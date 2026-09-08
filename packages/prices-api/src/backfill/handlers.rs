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
     forward.\nA stream that has never reported is absent from the response.\n\nA stream \
     recorded as `running` whose last push is more than 7 days old is reported as `stalled`: \
     nothing writes a terminal state when a run dies, so `running` alone cannot be trusted to \
     mean the stream is making progress.",
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
        status: effective_status(r),
        current_ledger: r.current_ledger,
        start_ledger: r.start_ledger,
        target_ledger: r.target_ledger,
        progress_pct: progress_pct(r),
        ledgers_remaining: ledgers_remaining(r),
        last_push_at: r.last_push_at.clone(),
        earliest_data_available: r.earliest_data_available.clone(),
    });

    let soroban_amm = find("soroban_amm").map(|r| AmmStream {
        status: effective_status(r),
        last_push_at: r.last_push_at.clone(),
        completed_at: r.completed_at.clone(),
        earliest_data_available: r.earliest_data_available.clone(),
    });

    let realtime_tip_ledger = realtime_tip(&rows, sdex.as_ref().map(|s| s.target_ledger));

    let body = BackfillStatus {
        realtime_tip_ledger,
        sdex,
        soroban_amm,
    };
    let mut resp = Json(body).into_response();
    cache_control::attach(&mut resp, cache_control::MEDIUM);
    resp
}

/// The chain tip to publish as `realtime_tip_ledger`.
///
/// Reads the live processor's durable cursor (`prices.ingest_cursor`), which
/// advances every batch, and falls back to the SDEX `target_ledger` only when
/// the cursor is unset — a fresh deployment before the first batch.
///
/// The fallback used to be the *only* source, and it is not a chain tip: the
/// backfill sink rewrites `target_ledger` when it pushes, so the value freezes
/// the moment the backfill stops. The SDEX archive last pushed on 2026-08-11,
/// so a field named `realtime_tip_ledger` was publishing a tip **534,222
/// ledgers** — 28 days — behind reality (task 0176).
///
/// ⚠️ Deliberately NOT used as the denominator in [`progress_pct`]. The archive
/// completed against the tip as it stood when it ran; denominating a finished
/// archive by a live tip would drag it below 100% further every ledger.
fn realtime_tip(rows: &[ProgressRow], sdex_target: Option<u64>) -> u64 {
    rows.iter()
        .map(|r| r.live_tip_ledger)
        .max()
        .filter(|&tip| tip > 0)
        .or(sdex_target)
        .unwrap_or(0)
}

/// Push age past which a `running` stream is republished as [`STATUS_STALLED`].
///
/// 604 800 s (7 days), deliberately the same value as
/// `opsAlarms.sdexPushFreshnessSeconds` in `infra/envs/production.json` — what
/// the `prices-{env}-sdex-push-freshness` alarm fires on. The endpoint and the
/// alarm must not disagree about whether a stream is stalled, so retune the two
/// together.
const STALE_PUSH_SECONDS: i64 = 604_800;

/// Stored status for a stream that believes it is working.
const STATUS_RUNNING: &str = "running";

/// Published in place of `running` when the last push has aged past
/// [`STALE_PUSH_SECONDS`].
///
/// Task 0176 defect 2: `resolve_status` in `sdex-backfill`'s sink only
/// transitions on a push **from a live run**, so a crashed or killed run leaves
/// its row asserting `running` indefinitely — `soroban_amm` did exactly that
/// from 2026-07-14, and anything gating on `status != 'running'` waits forever.
///
/// This is a read-model correction. The stored row is deliberately left alone:
/// 0176 forbids hand-patching `backfill_progress`, because that repairs one row
/// and leaves the mechanism that produced it intact.
const STATUS_STALLED: &str = "stalled";

/// The status to publish for `row`.
///
/// Every stored status passes through unchanged except a `running` stream whose
/// last push has aged past [`STALE_PUSH_SECONDS`], which becomes
/// [`STATUS_STALLED`].
///
/// A stream that has **never** pushed (`push_age_seconds` is `NULL`) is not
/// stalled — it is a seeded row that has yet to start. That is the same case
/// `backfill-freshness-probe` excludes from its metric, and for the same
/// reason: "backfill overdue" and "no backfill at all" are different states,
/// and conflating them was the go-live false-page.
fn effective_status(r: &ProgressRow) -> String {
    match r.push_age_seconds {
        Some(age) if r.status == STATUS_RUNNING && age > STALE_PUSH_SECONDS => {
            STATUS_STALLED.to_string()
        }
        _ => r.status.clone(),
    }
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
///
/// 🔒 **Kept deliberately as a second line of defence, not redundant** (task
/// 0263's acceptance criterion asks this to be settled either way).
///
/// 0263 fixed the *writer*: `sdex-backfill`'s `progress.rs` now gates
/// `Current::SetBackward(start)` on the same `reached_genesis` condition that
/// gates `status`, so a genesis-anchored chunk no longer writes a floor of 1
/// while still `running`. That closes the source of the contradiction — but
/// only for rows written by a backfill binary carrying the fix.
///
/// The reader cannot know which binary wrote the row in front of it.
/// `backfill_progress` is a durable table, not a queue: a row predating the
/// writer fix, or written by an older build still in someone's path, keeps the
/// old shape indefinitely. Removing the ceiling would let exactly those rows
/// publish `progress_pct: 100.0` beside `status: "running"` on a
/// reviewer-facing endpoint.
///
/// Remove it only once no row of the old shape can reach this code — which in
/// practice means never, since nothing rewrites historical rows.
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
            push_age_seconds: None,
            live_tip_ledger: 0,
        }
    }

    fn running(start: u64, current: u64, target: u64) -> ProgressRow {
        row(start, current, target, "running")
    }

    /// A stream that has pushed, `age` seconds ago.
    fn pushed(status: &str, age: i64) -> ProgressRow {
        let mut r = row(1, 1, 63_795_749, status);
        r.last_push_at = Some("2026-07-14T17:54:24Z".to_string());
        r.push_age_seconds = Some(age);
        r
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

    /// The `soroban_amm` production row: `running` since 2026-07-14 with no
    /// push in eight weeks. `resolve_status` never writes a terminal state for
    /// a run that dies, so the row asserts it is working indefinitely.
    #[test]
    fn a_long_dead_running_stream_publishes_stalled() {
        let r = pushed(STATUS_RUNNING, 56 * 86_400);
        assert_eq!(effective_status(&r), STATUS_STALLED);
    }

    /// A run pushing on cadence is untouched — the threshold matches the
    /// production freshness alarm, so anything it would not page on stays
    /// `running`.
    #[test]
    fn a_recently_pushing_stream_stays_running() {
        let r = pushed(STATUS_RUNNING, 3_600);
        assert_eq!(effective_status(&r), STATUS_RUNNING);
    }

    /// Exactly at the threshold is not yet stalled; one second past it is. The
    /// comparison is strictly greater-than, matching the alarm's
    /// `GREATER_THAN_THRESHOLD`.
    #[test]
    fn the_threshold_is_exclusive_and_matches_the_alarm() {
        assert_eq!(
            effective_status(&pushed(STATUS_RUNNING, STALE_PUSH_SECONDS)),
            STATUS_RUNNING
        );
        assert_eq!(
            effective_status(&pushed(STATUS_RUNNING, STALE_PUSH_SECONDS + 1)),
            STATUS_STALLED
        );
    }

    /// A seeded row that has never pushed is not stalled — it has not started.
    /// Conflating "overdue" with "never ran" was the go-live false-page the
    /// freshness probe had to unpick.
    #[test]
    fn a_stream_that_never_pushed_is_not_stalled() {
        let r = running(1, CURRENT_UNSET, 63_795_749);
        assert_eq!(r.push_age_seconds, None);
        assert_eq!(effective_status(&r), STATUS_RUNNING);
    }

    /// Only `running` is rewritten. A finished stream whose last push is long
    /// past is simply an old completed archive, not a stalled one — the SDEX
    /// row has read `completed` with an ageing push since 2026-08-11.
    #[test]
    fn a_completed_stream_is_never_relabelled_however_old() {
        let r = pushed(STATUS_COMPLETED, 365 * 86_400);
        assert_eq!(effective_status(&r), STATUS_COMPLETED);
        assert_eq!(effective_status(&pushed("paused", 365 * 86_400)), "paused");
    }

    /// A stalled stream is still not `completed`, so the running ceiling keeps
    /// applying to it — relabelling the status must not hand a dead
    /// genesis-anchored chunk a 100% claim (task 0263).
    #[test]
    fn a_stalled_stream_still_cannot_publish_one_hundred_percent() {
        let mut r = pushed(STATUS_RUNNING, 56 * 86_400);
        r.current_ledger = 1;
        assert_eq!(effective_status(&r), STATUS_STALLED);
        assert!(progress_pct(&r) <= PCT_RUNNING_CEILING);
    }

    /// The live cursor wins over the SDEX `target_ledger`. On 2026-09-08 the
    /// two differed by 534,222 ledgers — 28 days — because the backfill stopped
    /// pushing and froze the column the tip used to be read from.
    #[test]
    fn the_tip_comes_from_the_live_cursor_not_the_backfill_column() {
        let mut r = row(1, 1, 63_795_749, "completed");
        r.live_tip_ledger = 64_329_971;
        assert_eq!(realtime_tip(&[r], Some(63_795_749)), 64_329_971);
    }

    /// Before the live processor has committed its first batch the cursor table
    /// is empty and the subquery yields 0. Falling back keeps the endpoint
    /// answering rather than publishing a tip of zero.
    #[test]
    fn an_unset_cursor_falls_back_to_the_backfill_target() {
        let r = row(1, 1, 63_795_749, "completed");
        assert_eq!(r.live_tip_ledger, 0);
        assert_eq!(realtime_tip(&[r], Some(63_795_749)), 63_795_749);
    }

    /// Neither source available — no rows at all — is 0, the same empty-state
    /// answer the endpoint gave before.
    #[test]
    fn no_rows_and_no_target_is_zero() {
        assert_eq!(realtime_tip(&[], None), 0);
    }
}
