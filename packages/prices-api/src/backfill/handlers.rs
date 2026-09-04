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

/// The one `status` value that licenses a 100% claim. Matches
/// `sdex-backfill`'s `ProgressStatus::as_ch`, which is what writes the column.
const STATUS_COMPLETED: &str = "completed";

/// Ceiling for a stream that is not `completed`. See [`progress_pct`] — a
/// backward stream finishes at a *low* `current_ledger`, and a partial run
/// writes exactly that while still `running`, so the arithmetic alone cannot
/// tell a finished archive from a genesis-anchored chunk.
const PCT_RUNNING_CEILING: f64 = 99.9;

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
/// Guarded on three counts: a zero span yields `0.0`; `current_ledger == 0` is
/// the unset sentinel rather than a stream that has reached genesis (without
/// that check a brand-new row reads as 100% done); and a stream that is not
/// `completed` can never publish a full 100%, for the reason below.
///
/// ⚠️ **`current_ledger` is the lowest run *start* ever completed, not proven
/// contiguous coverage.** `progress.rs` writes `Current::SetBackward(start)` at
/// `Phase::Completed` unconditionally, so the documented chunking pattern
/// `--mode sdex-only --start 1 --end 20_000_000` sets `current_ledger = 1`
/// while `status` correctly stays `running` and `[20_000_000, activation)` is
/// still missing. This function cannot detect that from one row — the ledger
/// ledger inventory lives in `prices.backfill_sdex_ledgers` and reading it here
/// would cost the O(1) contract this endpoint is built on ([[0263]]).
///
/// What it can do is refuse to *claim* completion the status does not support,
/// which is why a non-`completed` stream is held just under 100. A reviewer
/// reading Tranche 2 AC 5 off this payload then sees "nearly done, still
/// running" rather than "100% done, still running" — misleading in degree, but
/// not self-contradictory, and never an assertion of coverage we cannot back.
fn progress_pct(r: &ProgressRow) -> f64 {
    let span = r.target_ledger.saturating_sub(r.start_ledger);
    if span == 0 || r.current_ledger == CURRENT_UNSET {
        return 0.0;
    }
    let done = r.target_ledger.saturating_sub(r.current_ledger);
    let pct = ((done as f64 / span as f64) * 100.0).clamp(0.0, 100.0);
    if r.status != STATUS_COMPLETED {
        return pct.min(PCT_RUNNING_CEILING);
    }
    pct
}

/// Ledgers the SDEX archive has still to reach, i.e. how far its floor sits
/// above genesis: `current_ledger - start_ledger`.
///
/// The mirror of [`progress_pct`] and backward for the same reason. The
/// previous `target - current` answered "how far below the tip is the floor",
/// which for this stream is the span already *done* — it reported 63,795,748
/// remaining on a completed archive. An unset `current_ledger` means nothing is
/// covered yet, so the whole span remains.
///
/// Clamped to the span, because `current_ledger > target_ledger` is reachable:
/// `sink.rs` rewrites `target_ledger` on every write while a mid-run update
/// leaves `current_ledger` alone (`Current::Keep`), so a chunked run without
/// `--tip` can collapse the denominator below a stored floor. Unclamped, that
/// published more remaining than the whole span exists — "0% covered" beside
/// "50,457,423 remaining" out of a 30,000,000 span. The clamp keeps
/// `covered + remaining <= span` in every reachable state.
fn ledgers_remaining(r: &ProgressRow) -> u64 {
    let span = r.target_ledger.saturating_sub(r.start_ledger);
    if r.current_ledger == CURRENT_UNSET {
        return span;
    }
    r.current_ledger.saturating_sub(r.start_ledger).min(span)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(start: u64, current: u64, target: u64, status: &str) -> ProgressRow {
        ProgressRow {
            task_name: "sdex_archive".to_string(),
            start_ledger: start,
            target_ledger: target,
            current_ledger: current,
            status: status.to_string(),
            last_push_at: None,
            completed_at: None,
            earliest_data_available: None,
        }
    }

    fn running(start: u64, current: u64, target: u64) -> ProgressRow {
        row(start, current, target, "running")
    }

    /// The production row on 2026-09-04: the archive walked all the way to
    /// genesis, so `current == start == 1`. The forward formula called this
    /// `0.0` beside `status: "completed"`.
    #[test]
    fn a_completed_backward_archive_reads_as_one_hundred_percent() {
        let r = row(1, 1, 63_795_749, "completed");
        assert!((progress_pct(&r) - 100.0).abs() < f64::EPSILON);
        assert_eq!(ledgers_remaining(&r), 0);
    }

    /// Mid-run: the floor has descended to 34,891,234 of a 57,234,198 tip, so
    /// the covered span is the part ABOVE the floor.
    #[test]
    fn a_mid_run_archive_reports_the_span_above_its_floor() {
        let r = running(1, 34_891_234, 57_234_198);
        let pct = progress_pct(&r);
        assert!((pct - 39.04).abs() < 0.01, "pct={pct}");
        assert_eq!(ledgers_remaining(&r), 34_891_233);
    }

    /// `current_ledger = 0` is the seeded placeholder, not a stream that has
    /// reached genesis. Without the sentinel check this reads as ~100% done —
    /// the exact inverse of the bug being fixed.
    #[test]
    fn a_freshly_seeded_row_is_zero_percent_not_complete() {
        let r = running(1, CURRENT_UNSET, 63_795_749);
        assert_eq!(progress_pct(&r), 0.0);
        assert_eq!(ledgers_remaining(&r), 63_795_748);
    }

    /// 🔴 A genesis-anchored chunk (`--start 1 --end 20_000_000`) writes
    /// `current_ledger = 1` at `Phase::Completed` while `status` stays
    /// `running`, with `[20_000_000, activation)` still missing. The arithmetic
    /// says 100%; the status says otherwise, and the status wins.
    #[test]
    fn a_partial_genesis_anchored_run_cannot_claim_one_hundred() {
        let r = running(1, 1, 63_795_749);
        let pct = progress_pct(&r);
        assert!(pct < 100.0, "a running stream published {pct}");
        assert!(
            (pct - PCT_RUNNING_CEILING).abs() < f64::EPSILON,
            "pct={pct}"
        );
    }

    /// The ceiling applies to `paused` too — the state a completed combined
    /// pass leaves the archive in before the pre-Soroban tail starts.
    #[test]
    fn a_paused_stream_is_held_under_the_ceiling() {
        let r = row(1, 1, 63_795_749, "paused");
        assert!(progress_pct(&r) < 100.0);
    }

    /// Placeholder rows seed every ledger field to 0; a zero span must not
    /// divide.
    #[test]
    fn a_zero_span_does_not_divide() {
        assert_eq!(progress_pct(&running(0, 0, 0)), 0.0);
        assert_eq!(ledgers_remaining(&running(0, 0, 0)), 0);
    }

    /// `current > target` is reachable: the sink rewrites `target_ledger` on
    /// every write while a mid-run update keeps `current_ledger`, so a chunked
    /// run without `--tip` collapses the denominator below a stored floor.
    /// Unclamped, `ledgers_remaining` exceeded the whole span.
    #[test]
    fn a_current_above_the_tip_never_exceeds_the_span() {
        let r = running(1, 50_457_424, 30_000_000);
        assert_eq!(progress_pct(&r), 0.0, "nothing is covered above the tip");
        assert_eq!(
            ledgers_remaining(&r),
            29_999_999,
            "remaining must not exceed target - start"
        );
        assert!(ledgers_remaining(&r) <= r.target_ledger - r.start_ledger);
    }

    /// `covered + remaining <= span` in every reachable state (task 0176).
    #[test]
    fn covered_and_remaining_never_exceed_the_span() {
        let cases = [
            running(1, 34_891_234, 57_234_198),
            running(1, CURRENT_UNSET, 63_795_749),
            running(1, 50_457_424, 30_000_000),
            row(1, 1, 63_795_749, "completed"),
            running(0, 0, 0),
        ];
        for r in &cases {
            let span = r.target_ledger.saturating_sub(r.start_ledger);
            let covered = ((progress_pct(r) / 100.0) * span as f64).round() as u64;
            assert!(
                covered + ledgers_remaining(r) <= span + 1,
                "covered={covered} remaining={} span={span} row={r:?}",
                ledgers_remaining(r)
            );
        }
    }
}
