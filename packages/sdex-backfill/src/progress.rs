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
    Completed,
}

impl ProgressStatus {
    /// The literal ClickHouse `Enum8` string.
    pub fn as_ch(self) -> &'static str {
        match self {
            ProgressStatus::Running => "running",
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
    /// Oldest / newest candle-minute landed so far (unix seconds).
    pub earliest_minute: Option<u32>,
    pub newest_minute: Option<u32>,
}

/// Compute the `backfill_progress` rows this run should write, given its mode,
/// range, the chain tip (progress denominator), the activation split, the
/// observed watermarks, and the phase.
///
/// - **Combined** (`[activation, tip]`) advances `soroban_amm` forward and — so
///   recent SDEX is not under-reported — carries `sdex_archive.current` down to
///   the run floor at completion (its window/target still update live).
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
    let (earliest, newest) = (observed.earliest_minute, observed.newest_minute);
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
                    status: match phase {
                        Phase::Completed if reached_tip => ProgressStatus::Completed,
                        _ => ProgressStatus::Running,
                    },
                    earliest_minute: earliest,
                    newest_minute: newest,
                },
                // Backward stream: recent SDEX is reflected down to the run
                // floor once the pass is done (the floor is `start`, not a
                // hard-coded `activation`, so a partial `[X, tip]` window does
                // not over-claim coverage below `X`). Stays put mid-run. Never
                // `completed` here — the pre-Soroban tail remains for the
                // SdexOnly run, and the sink will not downgrade a stored
                // `completed` a prior sdex-only run may have set.
                ProgressUpdate {
                    task_name: SDEX_ARCHIVE,
                    start_ledger: 1,
                    target_ledger: tip as u64,
                    current_ledger: match phase {
                        Phase::Running => Current::Keep,
                        Phase::Completed => Current::SetBackward(start as u64),
                    },
                    status: ProgressStatus::Running,
                    earliest_minute: earliest,
                    newest_minute: newest,
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
                // the end.
                current_ledger: match phase {
                    Phase::Running => Current::Keep,
                    Phase::Completed => Current::SetBackward(start as u64),
                },
                status: match phase {
                    Phase::Completed if reached_genesis => ProgressStatus::Completed,
                    _ => ProgressStatus::Running,
                },
                earliest_minute: earliest,
                newest_minute: newest,
            }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVATION: u32 = 50_463_000;
    const TIP: u32 = 55_000_000;

    fn observed(highest: u32) -> Observed {
        Observed {
            highest_indexed: highest,
            earliest_minute: Some(1_700_000_000),
            newest_minute: Some(1_720_000_000),
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
        // Not completed — the pre-Soroban tail still remains.
        assert_eq!(sdex.status, ProgressStatus::Running);
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
        assert_eq!(
            amm.status,
            ProgressStatus::Running,
            "did not reach tip → not completed"
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
        assert_eq!(sdex.current_ledger, Current::SetBackward(40_000_000));
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
        assert_eq!(sdex.current_ledger, Current::SetBackward(1));
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
}
