//! Pure computation of the two `prices.backfill_progress` rows a run advances.
//!
//! Kept transport-free (no ClickHouse) so the subtle §3.5 / decision-6
//! semantics are unit-tested in isolation; [`crate::sink::Sink::write_progress`]
//! is the thin writer that persists a [`ProgressUpdate`].
//!
//! ## Row semantics (overview §3.5)
//!
//! - `soroban_amm` fills `[activation, tip]` **forward**; `current_ledger` is
//!   the *newest* ledger reflected, so it advances honestly per-partition.
//! - `sdex_archive` fills `[1, tip]` **backward** (tip→genesis) across runs;
//!   `current_ledger` is the *oldest* ledger reflected. A forward single-pass
//!   has no honest intermediate value for it, so we only *set* it at
//!   completion and otherwise leave it unchanged ([`Current::Keep`]). The live
//!   signal for both streams is instead the direction-agnostic
//!   `[earliest, newest]_data_available` time-window (task 0053).

use crate::ingest::ExtractMode;

pub const SDEX_ARCHIVE: &str = "sdex_archive";
pub const SOROBAN_AMM: &str = "soroban_amm";

/// `prices.backfill_progress.status` enum values we write.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ProgressStatus {
    Running,
    /// The stream is resting between the two range runs — recent data is landed
    /// but the other range hasn't started. Set on `sdex_archive` when the
    /// combined pass finishes so the §5.6 `last_push_at` freshness alarm does not
    /// false-fire during the (possibly long) gap before the sdex-only tail run.
    Paused,
    Completed,
}

impl ProgressStatus {
    /// The literal ClickHouse `Enum8` string.
    pub fn as_ch(self) -> &'static str {
        match self {
            ProgressStatus::Running => "running",
            ProgressStatus::Paused => "paused",
            ProgressStatus::Completed => "completed",
        }
    }
}

/// What to do with `current_ledger` for a row. The two `Set*` variants are
/// merged **monotonically against the stored value by the sink**, so an
/// out-of-order, partial, or resumed run can never move `current_ledger` the
/// wrong way (see [`crate::sink`]):
///
/// - [`Current::SetForward`] — a forward stream (`soroban_amm`, oldest→newest):
///   the sink keeps `max(new, stored)`, so a resume never regresses it.
/// - [`Current::SetBackward`] — a backward stream (`sdex_archive`, tip→genesis):
///   the sink keeps `min(new, stored)` (treating the seeded `0` placeholder as
///   unset), so a combined pass never un-does an archive a prior `sdex-only`
///   run already carried down to genesis.
/// - [`Current::Keep`] — leave the stored value untouched (the backward stream
///   mid-run, where no per-partition value is truthful).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Current {
    /// Forward watermark: sink keeps `max(new, stored)`.
    SetForward(u64),
    /// Backward watermark: sink keeps `min(new, stored)`.
    SetBackward(u64),
    /// Leave the stored value untouched.
    Keep,
}

/// Whether this is a mid-run update or the run's final update.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Running,
    Completed,
}

/// One `backfill_progress` row's intended state after this update. `earliest` /
/// `newest` are the run's landed-candle window (unix-second minute), merged
/// monotonically against the stored value by the sink.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressUpdate {
    pub task_name: &'static str,
    pub start_ledger: u64,
    pub target_ledger: u64,
    pub current_ledger: Current,
    pub status: ProgressStatus,
    pub earliest_minute: Option<u32>,
    pub newest_minute: Option<u32>,
}

/// Inputs observed by the run when it computes an update.
#[derive(Copy, Clone, Debug)]
pub struct Observed {
    /// Highest ledger fully indexed so far this run (forward watermark).
    pub highest_indexed: u32,
    /// Oldest / newest candle-minute landed so far (unix seconds), **per
    /// stream**. Each `backfill_progress` row is stamped with its own stream's
    /// window; a mixed window is what made `soroban_amm` claim 17 days of
    /// coverage that exists at no granularity (task 0264).
    pub sdex_earliest: Option<u32>,
    pub sdex_latest: Option<u32>,
    pub amm_earliest: Option<u32>,
    pub amm_latest: Option<u32>,
}

/// Compute the `backfill_progress` rows this run should write, given its mode,
/// range, the chain tip (progress denominator), the activation split, the
/// observed watermarks, and the phase.
///
/// - **Combined** (`[activation, tip]`) advances `soroban_amm` forward and — so
///   recent SDEX is not under-reported — carries `sdex_archive.current` down to
///   the run floor at completion, leaving that stream `paused` (the "between the
///   two runs" resting state) so the freshness alarm doesn't false-fire.
///   `soroban_amm` rests the same way once its run finishes short of the tip,
///   which is the normal outcome: the documented `--end` is the live handoff
///   floor, not the tip, so `reached_tip` is unreachable in ordinary operation
///   and `completed` is reserved for a run that genuinely caught the chain.
/// - **SdexOnly** (`[1, activation)`) advances only `sdex_archive`; it completes
///   the stream only when the run covered the whole pre-Soroban tail — started
///   at genesis (`start == 1`) and reached the activation boundary.
///
/// A stream is only marked `completed` when the run actually reached the bound
/// it targets — a partial or resumed sub-window stays `running`, and the sink's
/// monotonic merge (plus its refusal to downgrade a stored `completed`) makes
/// these updates safe to apply in any run order.
pub fn progress_updates(
    mode: ExtractMode,
    start: u32,
    tip: u32,
    activation: u32,
    observed: Observed,
    phase: Phase,
) -> Vec<ProgressUpdate> {
    let (sdex_earliest, sdex_newest) = (observed.sdex_earliest, observed.sdex_latest);
    let (amm_earliest, amm_newest) = (observed.amm_earliest, observed.amm_latest);
    match mode {
        ExtractMode::Combined => {
            // Completed only once the forward pass actually reached the tip — a
            // partial / resumed sub-window (or a run that indexed nothing) stays
            // `running` instead of falsely claiming the whole stream is done.
            let reached_tip =
                observed.highest_indexed > 0 && observed.highest_indexed as u64 >= tip as u64;
            // Forward watermark, always clamped to the tip denominator so a
            // partition whose `last` overshoots the range never reports past it.
            let soroban_current = (observed.highest_indexed as u64).min(tip as u64);
            vec![
                // Forward stream: current_ledger = newest reflected.
                ProgressUpdate {
                    task_name: SOROBAN_AMM,
                    start_ledger: activation as u64,
                    target_ledger: tip as u64,
                    current_ledger: Current::SetForward(soroban_current),
                    // `running` used to be the only non-completed outcome here,
                    // and `reached_tip` is unreachable in the documented
                    // operating model: the combined run stops at the live
                    // handoff floor (`--end` = SDEX live floor − 1), never at
                    // the tip, because the range above it belongs to the live
                    // processor. So a run that finished its planned range
                    // reported `running` forever.
                    //
                    // Production sat exactly there: `current_ledger` =
                    // 63,352,611, the documented floor to the ledger, with
                    // `status: running` and no push since 2026-07-14. The run
                    // had not died — it had finished, and nothing could say so.
                    //
                    // `paused` is the same resting state the sdex_archive row
                    // below already uses for "between the two runs", and it is
                    // what stops the freshness alarm firing on a stream that is
                    // deliberately at rest (task 0176).
                    //
                    // ⚠️ Gated on having indexed something. A `Completed` run
                    // that advanced nothing — every remaining partition
                    // S3-incomplete — is not resting, it is failing, and must
                    // keep reporting `running` so the alarm still sees it.
                    status: match phase {
                        Phase::Completed if reached_tip => ProgressStatus::Completed,
                        Phase::Completed if observed.highest_indexed > 0 => ProgressStatus::Paused,
                        _ => ProgressStatus::Running,
                    },
                    // AMM-only window. This row used to be stamped with the
                    // run's mixed window, and since a Combined run lands SDEX
                    // and AMM candles from one parse — SDEX predating AMM in
                    // every Soroban-era range — it inherited the earliest SDEX
                    // minute at the activation boundary (task 0264).
                    earliest_minute: amm_earliest,
                    newest_minute: amm_newest,
                },
                // Backward stream: recent SDEX is reflected down to the run
                // floor once the pass is done (the floor is `start`, not a
                // hard-coded `activation`, so a partial `[X, tip]` window does
                // not over-claim coverage below `X`). Stays put mid-run. At
                // completion the stream goes `paused` — the combined pass is done
                // but the pre-Soroban tail hasn't started, and this is exactly
                // the "between the two runs" state (decision 6) that must not
                // trip the freshness alarm. Never `completed` here; the sink also
                // won't downgrade a stored `completed` a prior sdex-only run set.
                ProgressUpdate {
                    task_name: SDEX_ARCHIVE,
                    start_ledger: 1,
                    target_ledger: tip as u64,
                    current_ledger: match phase {
                        Phase::Running => Current::Keep,
                        Phase::Completed => Current::SetBackward(start as u64),
                    },
                    status: match phase {
                        Phase::Running => ProgressStatus::Running,
                        Phase::Completed => ProgressStatus::Paused,
                    },
                    earliest_minute: sdex_earliest,
                    newest_minute: sdex_newest,
                },
            ]
        }
        ExtractMode::SdexOnly => {
            // The archive is only complete when the pre-Soroban tail is fully
            // covered: the run started at genesis (`start == 1`) AND reached up to
            // the activation boundary (`highest_indexed >= activation - 1`), where
            // the combined pass takes over. A chunked genesis-first run that stops
            // short (e.g. `--start 1 --end 20_000_000`) stays `running` instead of
            // falsely completing while `[end, activation)` is still missing.
            let reached_genesis =
                start == 1 && observed.highest_indexed >= activation.saturating_sub(1);
            vec![ProgressUpdate {
                task_name: SDEX_ARCHIVE,
                start_ledger: 1,
                target_ledger: tip as u64,
                // Backward: oldest reflected = the run's floor, known only at
                // the end — and only carried down when the run actually proved
                // coverage from genesis (task 0263).
                //
                // `SetBackward(start)` used to be written unconditionally at
                // `Phase::Completed`, while `reached_genesis` gated only
                // `status`. So the chunking pattern this module's own doc
                // comment recommends — `--start 1 --end 20_000_000` — wrote
                // `current_ledger = 1` while `[20_000_000, activation)` had
                // never been touched. `status` correctly stayed `running`; the
                // two disagreed, and only `status` was telling the truth.
                //
                // The column is a floor claim that `/backfill/status` reads as
                // proven coverage: `progress_pct` and `ledgers_remaining` are
                // both derived from it. Gating it on the same condition keeps
                // the two fields consistent by construction.
                //
                // ⚠️ Trade-off, deliberate: a chunked run now never advances
                // the floor, because `reached_genesis` requires one run to both
                // start at genesis AND reach the activation boundary. That
                // under-claims — the stored floor stays where it is — which is
                // the safe direction. 0263 records the alternative (carry the
                // floor when it is adjacent to the stored one) as the follow-up
                // if chunked runs ever become the normal path.
                current_ledger: match phase {
                    Phase::Running => Current::Keep,
                    Phase::Completed if reached_genesis => Current::SetBackward(start as u64),
                    Phase::Completed => Current::Keep,
                },
                status: match phase {
                    Phase::Completed if reached_genesis => ProgressStatus::Completed,
                    _ => ProgressStatus::Running,
                },
                earliest_minute: sdex_earliest,
                newest_minute: sdex_newest,
            }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVATION: u32 = 50_457_424;
    const TIP: u32 = 55_000_000;

    fn observed(highest: u32) -> Observed {
        Observed {
            highest_indexed: highest,
            sdex_earliest: Some(1_700_000_000),
            sdex_latest: Some(1_720_000_000),
            amm_earliest: Some(1_710_000_000),
            amm_latest: Some(1_720_000_000),
        }
    }

    fn row<'a>(rows: &'a [ProgressUpdate], name: &str) -> &'a ProgressUpdate {
        rows.iter()
            .find(|r| r.task_name == name)
            .expect("row present")
    }

    #[test]
    fn combined_running_advances_soroban_forward_and_keeps_sdex() {
        let rows = progress_updates(
            ExtractMode::Combined,
            ACTIVATION,
            TIP,
            ACTIVATION,
            observed(50_600_000),
            Phase::Running,
        );
        assert_eq!(rows.len(), 2);

        let amm = row(&rows, SOROBAN_AMM);
        assert_eq!(amm.start_ledger, ACTIVATION as u64);
        assert_eq!(amm.target_ledger, TIP as u64);
        assert_eq!(amm.current_ledger, Current::SetForward(50_600_000));
        assert_eq!(amm.status, ProgressStatus::Running);

        // sdex_archive: no honest mid-run current; window still flows.
        let sdex = row(&rows, SDEX_ARCHIVE);
        assert_eq!(sdex.start_ledger, 1);
        assert_eq!(sdex.current_ledger, Current::Keep);
        assert_eq!(sdex.status, ProgressStatus::Running);
        assert_eq!(sdex.newest_minute, Some(1_720_000_000));
    }

    #[test]
    fn combined_completed_finishes_soroban_and_jumps_sdex_to_activation() {
        let rows = progress_updates(
            ExtractMode::Combined,
            ACTIVATION,
            TIP,
            ACTIVATION,
            observed(TIP),
            Phase::Completed,
        );
        let amm = row(&rows, SOROBAN_AMM);
        assert_eq!(amm.current_ledger, Current::SetForward(TIP as u64));
        assert_eq!(amm.status, ProgressStatus::Completed);

        let sdex = row(&rows, SDEX_ARCHIVE);
        // The AC: recent SDEX is reflected → oldest reflected = the run floor
        // (activation for a full combined pass). Backward-merged by the sink.
        assert_eq!(sdex.current_ledger, Current::SetBackward(ACTIVATION as u64));
        // Not completed — the pre-Soroban tail still remains; paused between runs.
        assert_eq!(sdex.status, ProgressStatus::Paused);
    }

    #[test]
    fn combined_partial_window_does_not_complete_soroban() {
        // A combined sub-window that stops short of the tip must NOT mark the
        // Soroban stream completed, and its sdex_archive floor is `start`, not a
        // hard-coded activation.
        let rows = progress_updates(
            ExtractMode::Combined,
            50_500_000, // start after activation
            TIP,        // tip denominator
            ACTIVATION,
            observed(50_600_000), // stopped well short of tip
            Phase::Completed,
        );
        let amm = row(&rows, SOROBAN_AMM);
        assert_eq!(amm.current_ledger, Current::SetForward(50_600_000));
        // CHANGED by task 0176: was `Running`. A finished run that stopped short
        // of the tip has reached its planned end — the live handoff floor — so
        // it rests, exactly as the sdex_archive row beside it does. Reporting
        // `running` was what made a completed stream look like a live one
        // forever.
        assert_eq!(
            amm.status,
            ProgressStatus::Paused,
            "finished short of the tip → resting at the handoff, not working"
        );

        let sdex = row(&rows, SDEX_ARCHIVE);
        assert_eq!(
            sdex.current_ledger,
            Current::SetBackward(50_500_000),
            "backward floor = run start, not activation"
        );
    }

    #[test]
    fn combined_completed_with_nothing_indexed_stays_running() {
        // Degenerate resume where every remaining partition was S3-incomplete:
        // highest_indexed never advanced past 0 → must not complete at 0.
        let rows = progress_updates(
            ExtractMode::Combined,
            ACTIVATION,
            TIP,
            ACTIVATION,
            observed(0),
            Phase::Completed,
        );
        let amm = row(&rows, SOROBAN_AMM);
        assert_eq!(amm.current_ledger, Current::SetForward(0));
        // Deliberately still `Running`, not `Paused`: a run that indexed nothing
        // is failing, not resting, and `paused` would silence the freshness
        // alarm on it (task 0176).
        assert_eq!(amm.status, ProgressStatus::Running);
    }

    #[test]
    fn combined_completed_clamps_soroban_current_to_tip() {
        // A partition whose `last` overshoots the tip must not report past it.
        let rows = progress_updates(
            ExtractMode::Combined,
            ACTIVATION,
            TIP,
            ACTIVATION,
            observed(TIP + 999),
            Phase::Completed,
        );
        assert_eq!(
            row(&rows, SOROBAN_AMM).current_ledger,
            Current::SetForward(TIP as u64)
        );
    }

    #[test]
    fn sdex_only_touches_one_row_and_keeps_current_mid_run() {
        let rows = progress_updates(
            ExtractMode::SdexOnly,
            1,
            TIP,
            ACTIVATION,
            observed(10_000_000),
            Phase::Running,
        );
        assert_eq!(rows.len(), 1);
        let sdex = row(&rows, SDEX_ARCHIVE);
        assert_eq!(sdex.task_name, SDEX_ARCHIVE);
        assert_eq!(sdex.current_ledger, Current::Keep);
        assert_eq!(sdex.status, ProgressStatus::Running);
    }

    #[test]
    fn sdex_only_completes_when_run_reached_genesis() {
        let rows = progress_updates(
            ExtractMode::SdexOnly,
            1,
            TIP,
            ACTIVATION,
            observed(ACTIVATION - 1),
            Phase::Completed,
        );
        let sdex = row(&rows, SDEX_ARCHIVE);
        assert_eq!(sdex.current_ledger, Current::SetBackward(1));
        assert_eq!(sdex.status, ProgressStatus::Completed);
    }

    #[test]
    fn sdex_only_partial_tail_does_not_complete() {
        // A sub-range that does not reach ledger 1 stays running.
        let rows = progress_updates(
            ExtractMode::SdexOnly,
            40_000_000,
            TIP,
            ACTIVATION,
            observed(45_000_000),
            Phase::Completed,
        );
        let sdex = row(&rows, SDEX_ARCHIVE);
        // CHANGED by task 0263: was `SetBackward(40_000_000)`. A run that did
        // not start at genesis has not proven coverage below its own floor, so
        // it no longer carries one down. Under-claims rather than over-claims.
        assert_eq!(sdex.current_ledger, Current::Keep);
        assert_eq!(sdex.status, ProgressStatus::Running);
    }

    #[test]
    fn sdex_only_genesis_chunk_that_stops_short_does_not_complete() {
        // A chunk from genesis that stops well below activation must NOT mark the
        // archive complete — [end, activation) is still missing.
        let rows = progress_updates(
            ExtractMode::SdexOnly,
            1,
            TIP,
            ACTIVATION,
            observed(20_000_000), // far short of activation
            Phase::Completed,
        );
        let sdex = row(&rows, SDEX_ARCHIVE);
        // CHANGED by task 0263 — this assertion WAS the defect. It pinned
        // `SetBackward(1)`: a chunk that touched nothing above 20,000,000 wrote
        // a genesis floor, and `/backfill/status` read it as a complete
        // archive. `status` said `running` at the same time. Only `status` was
        // right, and the corrected arithmetic (PR #283) would have turned the
        // disagreement from a pessimistic 0.0% into an optimistic 100%.
        assert_eq!(sdex.current_ledger, Current::Keep);
        assert_eq!(
            sdex.status,
            ProgressStatus::Running,
            "did not reach the activation boundary → not completed"
        );
    }

    #[test]
    fn status_ch_strings_match_schema_enum() {
        assert_eq!(ProgressStatus::Running.as_ch(), "running");
        assert_eq!(ProgressStatus::Completed.as_ch(), "completed");
    }

    /// The floor and the status now move together, in every reachable shape.
    /// They disagreed before task 0263, and `/backfill/status` derives
    /// `progress_pct` and `ledgers_remaining` from the floor while publishing
    /// the status beside them — so a consumer saw two fields contradict.
    #[test]
    fn the_floor_advances_only_when_the_status_completes() {
        let cases = [
            // (start, highest_indexed, should_complete)
            (1, ACTIVATION - 1, true),       // full genesis pass
            (1, 20_000_000, false),          // genesis-anchored chunk, stops short
            (40_000_000, 45_000_000, false), // mid-range chunk
        ];
        for (start, highest, should_complete) in cases {
            let rows = progress_updates(
                ExtractMode::SdexOnly,
                start,
                TIP,
                ACTIVATION,
                observed(highest),
                Phase::Completed,
            );
            let sdex = row(&rows, SDEX_ARCHIVE);
            let completed = sdex.status == ProgressStatus::Completed;
            let carried = sdex.current_ledger != Current::Keep;
            assert_eq!(
                completed, should_complete,
                "status for start={start} highest={highest}"
            );
            assert_eq!(
                carried, completed,
                "floor and status disagreed for start={start} highest={highest}"
            );
        }
    }

    /// The production row must survive the stricter writer. `sdex_archive`
    /// really did walk to genesis, corroborated in task 0127 against
    /// `min(timestamp) = 2015-11-18` and the `201511` partition, so a run that
    /// reaches the boundary must still carry the floor down to 1.
    #[test]
    fn a_genuine_full_pass_still_reaches_genesis() {
        let rows = progress_updates(
            ExtractMode::SdexOnly,
            1,
            TIP,
            ACTIVATION,
            observed(ACTIVATION - 1),
            Phase::Completed,
        );
        let sdex = row(&rows, SDEX_ARCHIVE);
        assert_eq!(sdex.current_ledger, Current::SetBackward(1));
        assert_eq!(sdex.status, ProgressStatus::Completed);
    }

    /// Mid-run is untouched: nothing is truthful per-partition for a backward
    /// walk, so the floor stays put regardless of the gate.
    #[test]
    fn a_running_pass_still_keeps_the_stored_floor() {
        let rows = progress_updates(
            ExtractMode::SdexOnly,
            1,
            TIP,
            ACTIVATION,
            observed(20_000_000),
            Phase::Running,
        );
        assert_eq!(row(&rows, SDEX_ARCHIVE).current_ledger, Current::Keep);
    }

    /// 🔴 The production defect, pinned. A `Combined` run lands SDEX and AMM
    /// candles from one parse, and SDEX predates AMM in every Soroban-era
    /// window, so a single mixed watermark stamped the earliest **SDEX** minute
    /// onto `soroban_amm`.
    ///
    /// Production carried `2024-02-20 17:00` — the activation boundary, where
    /// prod holds 141 SDEX candles and zero AMM ones — while the first real AMM
    /// candle is `2024-03-08 19:00`. 17 days of coverage claimed at no
    /// granularity, measured 2026-09-08 (task 0264).
    #[test]
    fn the_amm_row_is_stamped_with_amm_candles_not_sdex_ones() {
        const SDEX_FIRST: u32 = 1_708_448_400; // 2024-02-20 17:00 UTC
        const AMM_FIRST: u32 = 1_709_924_400; // 2024-03-08 19:00 UTC
        const NEWEST: u32 = 1_788_825_600; // 2026-09-08 00:00 UTC

        let rows = progress_updates(
            ExtractMode::Combined,
            ACTIVATION,
            TIP,
            ACTIVATION,
            Observed {
                highest_indexed: TIP,
                sdex_earliest: Some(SDEX_FIRST),
                sdex_latest: Some(NEWEST),
                amm_earliest: Some(AMM_FIRST),
                amm_latest: Some(NEWEST),
            },
            Phase::Completed,
        );

        assert_eq!(
            row(&rows, SOROBAN_AMM).earliest_minute,
            Some(AMM_FIRST),
            "the AMM row must carry the first AMM candle, not the first SDEX one"
        );
        assert_eq!(
            row(&rows, SDEX_ARCHIVE).earliest_minute,
            Some(SDEX_FIRST),
            "the SDEX row keeps its own window"
        );
    }

    /// A `Combined` run that lands SDEX candles but no AMM ones leaves the AMM
    /// window unset rather than borrowing the SDEX one. `sink.rs` merges with
    /// `merge_min`, which treats `None` as "no claim" — so an empty window
    /// stays empty instead of writing a coverage claim the stream cannot back.
    #[test]
    fn a_run_with_no_amm_candles_claims_no_amm_window() {
        let rows = progress_updates(
            ExtractMode::Combined,
            ACTIVATION,
            TIP,
            ACTIVATION,
            Observed {
                highest_indexed: TIP,
                sdex_earliest: Some(1_708_448_400),
                sdex_latest: Some(1_788_825_600),
                amm_earliest: None,
                amm_latest: None,
            },
            Phase::Completed,
        );
        assert_eq!(row(&rows, SOROBAN_AMM).earliest_minute, None);
        assert_eq!(row(&rows, SOROBAN_AMM).newest_minute, None);
        assert!(row(&rows, SDEX_ARCHIVE).earliest_minute.is_some());
    }

    /// The production shape, to the ledger. The combined run stops at the
    /// documented live handoff floor — `soroban_amm.current_ledger` read
    /// exactly 63,352,611 on 2026-09-08, with `target_ledger` 63,475,475 being
    /// only the tip as it stood at the last push, not a goal.
    ///
    /// Before task 0176 this shape reported `running` indefinitely, and the
    /// 122,864-ledger difference read as a stalled backfill rather than the
    /// deliberate margin handed to live ingestion.
    #[test]
    fn the_production_handoff_shape_rests_rather_than_running() {
        const LIVE_FLOOR: u32 = 63_352_611;
        const TIP_AT_LAST_PUSH: u32 = 63_475_475;

        let rows = progress_updates(
            ExtractMode::Combined,
            ACTIVATION,
            TIP_AT_LAST_PUSH,
            ACTIVATION,
            observed(LIVE_FLOOR),
            Phase::Completed,
        );
        let amm = row(&rows, SOROBAN_AMM);
        assert_eq!(amm.current_ledger, Current::SetForward(LIVE_FLOOR as u64));
        assert_eq!(
            amm.status,
            ProgressStatus::Paused,
            "stopped at the live handoff floor → resting, not working"
        );
    }
}
