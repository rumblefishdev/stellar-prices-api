//! Axum handler for `/v1/backfill/status`.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

use crate::backfill::dto::{AmmStream, BackfillStatus, SdexStream};
use crate::backfill::queries_ch::{self, ProgressRow};
use crate::common::errors::ErrorEnvelope;
use crate::common::{cache_control, errors};
use crate::state::AppState;

/// `GET /backfill/status` — push-state of both backfill streams.
#[utoipa::path(
    get,
    path = "/backfill/status",
    tag = "backfill",
    summary = "`GET /backfill/status` — progress of the historical backfill streams.",
    description = "Progress of the two streams that load history: the SDEX archive, which walks \
     backward\nfrom the chain tip toward genesis, and the one-shot Soroban AMM import, which walks \
     forward.\nA stream that has never reported is absent from the response.",
    responses(
        (status = 200, description = "Backfill progress", body = BackfillStatus),
        (status = 401, description = "Missing or invalid `x-api-key` (`unauthorized`)", body = ErrorEnvelope),
        (status = 403, description = "Rejected at the API gateway: `x-api-key` missing, unknown, or not enabled for this API"),
        (status = 429, description = "Per-key rate limit or monthly quota exceeded"),
        (status = 500, description = "Database or upstream failure (`db_error`)", body = ErrorEnvelope),
    )
)]
pub async fn get_status(State(state): State<AppState>) -> Response {
    let rows = match queries_ch::all_progress(state.ch()).await {
        Ok(rows) => rows,
        Err(e) => return errors::db_error(&e, "backfill status lookup"),
    };

    let find = |name: &str| rows.iter().find(|r| r.task_name == name);

    let sdex = find("sdex_archive").map(|r| SdexStream {
        status: r.status.clone(),
        current_ledger: r.current_ledger,
        start_ledger: r.start_ledger,
        target_ledger: r.target_ledger,
        progress_pct: progress_pct(r),
        ledgers_remaining: ledgers_remaining(r),
        last_push_at: r.last_push_at.clone(),
        earliest_data_available: r.earliest_data_available.clone(),
    });

    let soroban_amm = find("soroban_amm").map(|r| AmmStream {
        status: r.status.clone(),
        last_push_at: r.last_push_at.clone(),
        completed_at: r.completed_at.clone(),
        earliest_data_available: r.earliest_data_available.clone(),
    });

    let realtime_tip_ledger = sdex.as_ref().map(|s| s.target_ledger).unwrap_or(0);

    let body = BackfillStatus {
        realtime_tip_ledger,
        sdex,
        soroban_amm,
    };
    let mut resp = Json(body).into_response();
    cache_control::attach(&mut resp, cache_control::MEDIUM);
    resp
}

/// The seeded `current_ledger` placeholder meaning "nothing reflected yet".
///
/// Genesis is ledger 1, so `0` is never a real sequence. `sdex-backfill`'s sink
/// relies on the same sentinel when merging a backward watermark
/// (`resolve_current`: `Some(e) if e != 0`), and the two must agree — otherwise
/// a fresh row reads as a *finished* archive here, because a backward stream
/// finishes at a low `current_ledger`.
const CURRENT_UNSET: u64 = 0;

/// Fraction of the ledger span the SDEX archive has covered, in percent.
///
/// 🔴 **The archive walks backward** — tip → genesis. `sdex-backfill`'s
/// `progress.rs` writes `start_ledger = 1` (genesis) and `target_ledger = tip`
/// on every update, and moves `current_ledger` *down* via
/// `Current::SetBackward`, so `current_ledger` is the **oldest** ledger
/// reflected so far. Covered is therefore `[current, target]`, and the consumed
/// fraction is `(target - current) / (target - start)`.
///
/// It is **not** `(current - start) / (target - start)`. That is the forward
/// form, and it is what shipped: because the archive finishes at
/// `current == start == 1`, a *completed* stream reported `progress_pct: 0.0`
/// beside `status: "completed"` on production (task 0127). The number Tranche 2
/// AC 5 asks a reviewer to read sits on this same payload.
///
/// Guarded on both ends: a zero span yields `0.0`, and `current_ledger == 0` is
/// the unset sentinel rather than a stream that has reached genesis — without
/// that check a brand-new row would read as 100% done.
fn progress_pct(r: &ProgressRow) -> f64 {
    let span = r.target_ledger.saturating_sub(r.start_ledger);
    if span == 0 || r.current_ledger == CURRENT_UNSET {
        return 0.0;
    }
    let done = r.target_ledger.saturating_sub(r.current_ledger);
    ((done as f64 / span as f64) * 100.0).clamp(0.0, 100.0)
}

/// Ledgers the SDEX archive has still to reach, i.e. how far its floor sits
/// above genesis: `current_ledger - start_ledger`.
///
/// The mirror of [`progress_pct`] and backward for the same reason. The
/// previous `target - current` answered "how far below the tip is the floor",
/// which for this stream is the span already *done* — it reported 63,795,748
/// remaining on a completed archive. An unset `current_ledger` means nothing is
/// covered yet, so the whole span remains.
fn ledgers_remaining(r: &ProgressRow) -> u64 {
    if r.current_ledger == CURRENT_UNSET {
        return r.target_ledger.saturating_sub(r.start_ledger);
    }
    r.current_ledger.saturating_sub(r.start_ledger)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(start: u64, current: u64, target: u64) -> ProgressRow {
        ProgressRow {
            task_name: "sdex_archive".to_string(),
            start_ledger: start,
            target_ledger: target,
            current_ledger: current,
            status: "running".to_string(),
            last_push_at: None,
            completed_at: None,
            earliest_data_available: None,
        }
    }

    /// The production row on 2026-09-04: the archive walked all the way to
    /// genesis, so `current == start == 1`. The forward formula called this
    /// `0.0` beside `status: "completed"`.
    #[test]
    fn a_completed_backward_archive_reads_as_one_hundred_percent() {
        let r = row(1, 1, 63_795_749);
        assert!(
            (progress_pct(&r) - 100.0).abs() < f64::EPSILON,
            "{}",
            progress_pct(&r)
        );
        assert_eq!(ledgers_remaining(&r), 0);
    }

    /// Mid-run: the floor has descended to 34,891,234 of a 57,234,198 tip, so
    /// the covered span is the part ABOVE the floor.
    #[test]
    fn a_mid_run_archive_reports_the_span_above_its_floor() {
        let r = row(1, 34_891_234, 57_234_198);
        let pct = progress_pct(&r);
        assert!((pct - 39.04).abs() < 0.01, "pct={pct}");
        assert_eq!(ledgers_remaining(&r), 34_891_233);
    }

    /// `current_ledger = 0` is the seeded placeholder, not a stream that has
    /// reached genesis. Without the sentinel check this reads as ~100% done —
    /// the exact inverse of the bug being fixed.
    #[test]
    fn a_freshly_seeded_row_is_zero_percent_not_complete() {
        let r = row(1, CURRENT_UNSET, 63_795_749);
        assert_eq!(progress_pct(&r), 0.0);
        assert_eq!(ledgers_remaining(&r), 63_795_748);
    }

    /// Placeholder rows seed every ledger field to 0; a zero span must not
    /// divide.
    #[test]
    fn a_zero_span_does_not_divide() {
        assert_eq!(progress_pct(&row(0, 0, 0)), 0.0);
        assert_eq!(ledgers_remaining(&row(0, 0, 0)), 0);
    }

    /// A stored `current_ledger` above the tip (a resumed run that overshot, or
    /// a tip that moved back) must not publish a negative or >100 percentage.
    #[test]
    fn an_out_of_range_current_is_clamped() {
        let over = row(1, 70_000_000, 63_795_749);
        assert_eq!(progress_pct(&over), 0.0);
        let under = row(1, 1, 63_795_749);
        assert!(progress_pct(&under) <= 100.0);
    }
}
