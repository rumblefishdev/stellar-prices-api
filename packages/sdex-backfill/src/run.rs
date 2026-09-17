use std::path::Path;
use std::time::{Duration, Instant};

use tokio::process::Command;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use prices_ingest_core::{AssetRegistry, Registries, UnresolvedPool, UnresolvedPoolSwap};

use crate::error::BackfillError;
use crate::ingest::{
    ExtractMode, PartitionEnd, PartitionStats, RunAccumulators, flush_open_minutes,
    index_partition, peek_ledger_minute,
};
use crate::partition::{Partition, partitions_for_range};
use crate::progress::{Observed, Phase, progress_updates};
use crate::sink::{Sink, merge_max, merge_min};
use crate::sync::{SyncOutcome, sync_partition};

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    sink: &Sink,
    temp_dir: &Path,
    start: u32,
    end: u32,
    keep_partitions: bool,
    mode: ExtractMode,
    activation_ledger: u32,
    tip: u32,
) -> Result<(), BackfillError> {
    assert!(start <= end, "invalid range: start ({start}) > end ({end})");

    // Sanity-check the mode against the range. Warn loudly rather than fail —
    // the operator may have a deliberate reason.
    match mode {
        // Combined over a purely pre-activation range extracts no AMM.
        ExtractMode::Combined if end < activation_ledger => warn!(
            end,
            activation_ledger,
            "combined mode but range is entirely pre-activation — no Soroban AMM to extract"
        ),
        // Forward pool discovery is only complete when the window begins at
        // activation: a pool's factory-create event must be decoded before any
        // of its swaps. A window that starts *after* activation never sees the
        // create events of earlier pools, so their swaps cannot be classified
        // unless a persisted pool registry is preloaded (decision #4; done at
        // run start from prices.pool_registry). Any swap for a still-unresolved
        // pool is recorded to prices.unresolved_pools (not silently dropped) and
        // — because a post-activation start makes such gaps expected — does not
        // fail the run. Seed prices.pool_registry from a prior full run to
        // actually capture that volume.
        ExtractMode::Combined if start > activation_ledger => warn!(
            start,
            activation_ledger,
            "combined mode starts after activation — pools created before start are resolved only from a preloaded pool_registry; swaps for any unresolved pool are recorded to prices.unresolved_pools (not dropped) but their volume is lost unless the registry was seeded"
        ),
        // SdexOnly over the Soroban era silently drops AMM swaps.
        ExtractMode::SdexOnly if end >= activation_ledger => warn!(
            activation_ledger,
            end,
            "sdex-only mode over the Soroban era — AMM swaps in [activation, end] will NOT be extracted"
        ),
        _ => {}
    }

    // The sdex_archive progress denominator (`target_ledger`) is the LIVE chain
    // tip. In a sdex-only tail run `--end` is the activation boundary, not the
    // tip, so `--tip <live tip>` must be passed explicitly; when it is omitted
    // `tip` defaults to `--end` (≈ activation) and progress_pct reads against
    // the wrong denominator. A tip at or below activation is the tell-tale of a
    // forgotten flag.
    //
    // The direction of the error is not fixed. `progress_pct` is
    // `(target - current) / (target - start)` (task 0127 — the archive walks
    // backward), so a collapsed `target` can push it either way: a terminal
    // update that carries `current` down to the run start reports ~100% of a
    // span that stops at activation, while a mid-run `Current::Keep` leaving a
    // stored floor at or above the collapsed target reports 0%. Both are wrong
    // about the real archive; neither is reliably conservative.
    if mode == ExtractMode::SdexOnly && tip <= activation_ledger {
        warn!(
            tip,
            activation_ledger,
            "sdex-only run without a live --tip: backfill_progress.target_ledger is the activation boundary, not the chain tip, so /backfill/status progress_pct will misreport (either direction) — pass --tip <live tip>"
        );
    }

    tokio::fs::create_dir_all(temp_dir).await?;

    preflight_aws().await;
    sink.preflight()
        .await
        .unwrap_or_else(|e| panic!("pre-flight: sink unreachable: {e}"));
    info!("pre-flight: all checks passed");

    let partitions = partitions_for_range(start, end);
    if partitions.is_empty() {
        info!("no partitions in range");
        return Ok(());
    }

    let completed = sink.load_completed(start, end).await?;

    let todo: Vec<&Partition> = partitions
        .iter()
        .filter(|p| !partition_fully_done(p, start, end, &completed))
        .collect();

    info!(
        start,
        end,
        total_partitions = partitions.len(),
        already_done = partitions.len() - todo.len(),
        to_process = todo.len(),
        "backfill starting"
    );

    if todo.is_empty() {
        info!("nothing to do — all partitions fully indexed");
        return Ok(());
    }

    let run_start = Instant::now();
    let mut totals = PartitionStats::default();
    let mut partitions_skipped_s3: usize = 0;
    // Highest ledger fully indexed so far — the forward watermark that drives
    // `soroban_amm.current_ledger` in the per-partition progress update.
    let mut highest_indexed: u32 = 0;
    // Fires the one-time activation-split minute-alignment check on whichever
    // partition straddles the split (decision 7).
    let mut checked_alignment = false;
    // The same check at the RUN's two ends (review CR-02). Phase 3 re-ingests
    // month by month, so `--end N` / `--start N+1` puts a run boundary on the
    // last minute of every month — ~120 of them across the chain — and the
    // minute open at `end` is written from this run's fills, then again from
    // the next run's. One shot each, on whichever partition holds the ledger.
    let mut checked_run_start = false;
    let mut checked_run_end = false;

    // Run-level candle state (task 0286 S5, review A BL-01): the minute open at
    // a partition boundary is CARRIED into the next partition instead of being
    // written twice — the second write would replace the first under
    // `ReplacingMergeTree`, and a dust-only second half erases a priced minute.
    // Same shape as `events-backfill`, which keeps its open minute across chunks.
    let mut accs = RunAccumulators::new();

    let existing_assets = sink.load_assets().await?;
    let mut registry = AssetRegistry::from_existing(existing_assets);
    // Venue / pool registries. Preloaded from the persisted `pool_registry`
    // artifact (decision #4) so a window starting after activation still
    // resolves earlier-created pools; empty on a fresh full run. Then grown
    // incrementally across partitions from in-window factory events.
    let mut reg = sink.load_pool_registry().await?;

    let mut current_complete = matches!(
        sync_partition(todo[0], temp_dir).await?,
        SyncOutcome::Complete
    );
    if !current_complete {
        warn!(
            partition = todo[0].start,
            "first partition S3 incomplete — will skip"
        );
        partitions_skipped_s3 += 1;
    }

    for (i, partition) in todo.iter().enumerate() {
        let next_handle: Option<JoinHandle<Result<SyncOutcome, BackfillError>>> =
            if let Some(next) = todo.get(i + 1) {
                let next = (*next).clone();
                let temp = temp_dir.to_path_buf();
                Some(tokio::spawn(
                    async move { sync_partition(&next, &temp).await },
                ))
            } else {
                None
            };

        if current_complete {
            let partition_end = partition_end(i, todo.len());
            let mut stats = index_partition(
                partition,
                temp_dir,
                sink,
                start,
                end,
                &completed,
                &mut registry,
                &mut reg,
                mode,
                &mut accs,
                partition_end,
            )
            .await?;

            totals.indexed += stats.indexed;
            totals.skipped += stats.skipped;
            totals.trade_ticks += stats.trade_ticks;
            totals.amm_ticks += stats.amm_ticks;
            totals.oracle_rows += stats.oracle_rows;
            totals.candles_written += stats.candles_written;
            totals.total_bytes += stats.total_bytes;
            totals.sdex_earliest = merge_min(totals.sdex_earliest, stats.sdex_earliest);
            totals.sdex_latest = merge_max(totals.sdex_latest, stats.sdex_latest);
            totals.amm_earliest = merge_min(totals.amm_earliest, stats.amm_earliest);
            totals.amm_latest = merge_max(totals.amm_latest, stats.amm_latest);
            totals.unresolved.append(&mut stats.unresolved);

            // Forward watermark = this partition's clamped upper bound.
            let (_, part_last) = partition.clamped(start, end);
            highest_indexed = highest_indexed.max(part_last);

            // Advance the progress row(s) live (Model B writes to Hetzner as we
            // go, so /backfill/status is truthful in real time). sdex_archive's
            // backward current_ledger stays put mid-run (Current::Keep); the
            // covered time-window advances for both streams.
            let observed = Observed {
                highest_indexed,
                sdex_earliest: totals.sdex_earliest,
                sdex_latest: totals.sdex_latest,
                amm_earliest: totals.amm_earliest,
                amm_latest: totals.amm_latest,
            };
            for u in progress_updates(
                mode,
                start,
                tip,
                activation_ledger,
                observed,
                Phase::Running,
            ) {
                sink.write_progress(&u).await?;
            }

            // One-time minute-alignment check at the activation split (decision
            // 7). The boundary partition is synced whole, so both ledgers
            // straddling the split are on disk here even though only one side is
            // in-range. If activation-1 and activation land in the same
            // candle-minute, that minute's source='sdex' candle is written
            // partially by BOTH range runs (ReplacingMergeTree replaces, not
            // sums) → silent undercount. Warn (best-effort) rather than fail: it
            // is a single boundary minute and the fix is a targeted reconcile.
            if !checked_alignment
                && activation_ledger > 1
                && partition.start <= activation_ledger
                && activation_ledger <= partition.end
            {
                checked_alignment = true;
                let m_prev = peek_ledger_minute(partition, activation_ledger - 1, temp_dir).await;
                let m_act = peek_ledger_minute(partition, activation_ledger, temp_dir).await;
                match boundary_is_minute_aligned(m_prev, m_act) {
                    Some(false) => warn!(
                        activation_ledger,
                        prev_ledger = activation_ledger - 1,
                        straddled_minute = m_act.unwrap_or_default(),
                        "activation split is NOT minute-aligned: the ledgers on either side of the split share one candle-minute, so its source='sdex' candle is written partially by both range runs (RMT replaces, not sums) — under ADR 0287 the second write can ERASE a priced minute, not merely undercount it. Rebuild that single minute from one pass after both runs, or accept the documented boundary artifact"
                    ),
                    Some(true) => info!(
                        activation_ledger,
                        "activation split is minute-aligned — no straddled sdex candle at the boundary"
                    ),
                    None => {}
                }
            }

            // The run's own two ends (review CR-02). A boundary that falls on a
            // 64k partition edge peeks `None` on the outside ledger and is
            // skipped — it needs no check, because the flush boundary and the
            // run boundary then coincide.
            if !checked_run_start && start > 1 && partition.start <= start && start <= partition.end
            {
                checked_run_start = true;
                check_boundary_alignment(partition, temp_dir, start - 1, start, "run_start").await;
            }
            if !checked_run_end && end < u32::MAX && partition.start <= end && end <= partition.end
            {
                checked_run_end = true;
                check_boundary_alignment(partition, temp_dir, end, end + 1, "run_end").await;
            }
        } else {
            info!(
                partition = partition.start,
                "skipping S3-incomplete partition"
            );
        }

        if !keep_partitions {
            let local = partition.local_folder(temp_dir);
            match tokio::fs::remove_dir_all(&local).await {
                Ok(()) => info!(partition = partition.start, "cleaned up local folder"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(BackfillError::Io(e)),
            }
        }

        current_complete = if let Some(h) = next_handle {
            match h.await.expect("prefetch task panicked")? {
                SyncOutcome::Complete => true,
                SyncOutcome::S3Incomplete { local, s3, need } => {
                    warn!(local, s3, need, "next partition S3 incomplete — will skip");
                    partitions_skipped_s3 += 1;
                    false
                }
            }
        } else {
            false
        };
    }

    // The last partition drains itself; this covers the run whose last
    // partition was skipped (S3-incomplete) and therefore never reached
    // `PartitionEnd::Drain`. A no-op when nothing is open.
    flush_open_minutes(&mut accs, sink, &mut totals).await?;

    sink.write_assets(&registry).await?;
    // Persist the discovered pool registry as a durable artifact (decision #4)
    // so a partial re-backfill / the live processor can load it.
    sink.write_pool_registry(&reg).await?;

    // Re-check the swaps that hit an unregistered pool against the *final*
    // registry and record them to prices.unresolved_pools so the operator can
    // investigate. Records are written either way; only the exit status reflects
    // a GENUINE gap.
    //
    // A window that begins at or before activation should — by forward-discovery
    // construction — have seen every pool's factory-create before its swaps, so
    // a still-unresolved pool is a true extractor gap that must fail the run. A
    // window that begins AFTER activation legitimately cannot see the create
    // events of earlier pools (the guard above warns about exactly this;
    // decision #4 seeds prices.pool_registry to cover them), so its
    // still-unresolved pools are expected — recorded, not fatal — matching the
    // guard's "operator may have a reason".
    let unresolved = aggregate_unresolved(&totals.unresolved, &reg);
    let fatal_gaps = if unresolved.is_empty() {
        0
    } else {
        let genuine_gaps = unresolved
            .iter()
            .filter(|u| u.still_unresolved == 1)
            .count();
        sink.write_unresolved_pools(&unresolved).await?;
        let fatal = if start <= activation_ledger {
            genuine_gaps
        } else {
            0
        };
        warn!(
            contracts = unresolved.len(),
            genuine_gaps, fatal, "unresolved AMM pools recorded to prices.unresolved_pools"
        );
        fatal
    };

    // Fail BEFORE writing the terminal `completed` progress, so a run that
    // aborts on a genuine gap is never reported as completed on /backfill/status.
    if fatal_gaps > 0 {
        return Err(BackfillError::UnresolvedPools(fatal_gaps));
    }

    // Terminal progress update: soroban_amm completes only if the forward pass
    // reached the tip; sdex_archive carries its backward floor down to the run
    // start (the sink merges both monotonically and won't downgrade a stored
    // `completed`), completing only when a sdex-only run reached genesis.
    let observed = Observed {
        highest_indexed,
        sdex_earliest: totals.sdex_earliest,
        sdex_latest: totals.sdex_latest,
        amm_earliest: totals.amm_earliest,
        amm_latest: totals.amm_latest,
    };
    for u in progress_updates(
        mode,
        start,
        tip,
        activation_ledger,
        observed,
        Phase::Completed,
    ) {
        sink.write_progress(&u).await?;
    }

    let elapsed = run_start.elapsed();
    print_run_summary(todo.len(), &totals, partitions_skipped_s3, elapsed);

    Ok(())
}

/// Aggregate per-ledger unresolved-swap records by contract and re-check each
/// against the final registry. A contract still absent from `reg.venue` is a
/// genuine extractor gap (`still_unresolved = 1`); one that registered later in
/// the run only lost its early swaps (`still_unresolved = 0`). Output is sorted
/// genuine-gaps-first then by contract id so the artifact is stable across runs.
fn aggregate_unresolved(raw: &[UnresolvedPoolSwap], reg: &Registries) -> Vec<UnresolvedPool> {
    struct Agg {
        first: u32,
        last: u32,
        count: u64,
        sample: String,
    }
    let mut by_contract: std::collections::HashMap<&str, Agg> = std::collections::HashMap::new();
    for u in raw {
        let e = by_contract.entry(&u.contract_id).or_insert(Agg {
            first: u.ledger_sequence,
            last: u.ledger_sequence,
            count: 0,
            sample: u.sample_topics.clone(),
        });
        e.first = e.first.min(u.ledger_sequence);
        e.last = e.last.max(u.ledger_sequence);
        e.count += u.swap_count as u64;
    }

    let mut out: Vec<UnresolvedPool> = by_contract
        .into_iter()
        .map(|(contract_id, a)| UnresolvedPool {
            contract_id: contract_id.to_string(),
            source: "backfill".to_string(),
            first_ledger: a.first,
            last_ledger: a.last,
            swap_count: a.count,
            sample_topics: a.sample,
            still_unresolved: u8::from(!reg.venue.contains_key(contract_id)),
        })
        .collect();

    out.sort_by(|a, b| {
        b.still_unresolved
            .cmp(&a.still_unresolved)
            .then_with(|| a.contract_id.cmp(&b.contract_id))
    });
    out
}

/// Which partition of a run may drain the minute it leaves open (review WR-02).
///
/// Only the LAST one: every earlier boundary carries the open minute into the
/// partition that holds its remaining fills, because writing it here and again
/// there would replace the first half with the second under
/// `ReplacingMergeTree` — and under ADR 0287 a dust-only second half erases a
/// priced minute rather than merely shortening it.
///
/// A single-partition run is that last partition, so it drains.
fn partition_end(index: usize, len: usize) -> PartitionEnd {
    if index + 1 == len {
        PartitionEnd::Drain
    } else {
        PartitionEnd::Carry
    }
}

/// Whether the two ledgers on either side of a range boundary fall in DIFFERENT
/// candle-minutes (review CR-02).
///
/// * `Some(true)`  — aligned: the boundary splits two minutes and no candle
///   straddles it.
/// * `Some(false)` — the two ledgers share one candle-minute, so that minute is
///   written partially by the run on each side of the boundary.
/// * `None` — one of the two ledgers could not be read, which in practice means
///   the boundary falls on a 64k partition edge (the neighbour is in a partition
///   this run never synced). Such a boundary needs no check: the answer is
///   simply unknown here and the caller skips silently.
fn boundary_is_minute_aligned(prev: Option<u32>, next: Option<u32>) -> Option<bool> {
    match (prev, next) {
        (Some(p), Some(n)) => Some(p != n),
        _ => None,
    }
}

/// Best-effort minute-alignment check at ONE end of the run's range (review
/// CR-02). `prev_ledger` is the last ledger below the boundary, `next_ledger`
/// the first above it; exactly one of the two is in this run's range and the
/// other is on disk whenever `partition` was synced whole.
///
/// Logs only — a straddling minute is a single bucket and the fix is a targeted
/// reconcile from one pass, not a failed run.
async fn check_boundary_alignment(
    partition: &Partition,
    temp_dir: &Path,
    prev_ledger: u32,
    next_ledger: u32,
    boundary: &'static str,
) {
    let m_prev = peek_ledger_minute(partition, prev_ledger, temp_dir).await;
    let m_next = peek_ledger_minute(partition, next_ledger, temp_dir).await;
    match boundary_is_minute_aligned(m_prev, m_next) {
        Some(false) => warn!(
            boundary,
            prev_ledger,
            next_ledger,
            straddled_minute = m_next.unwrap_or_default(),
            "run boundary is NOT minute-aligned: the ledgers on either side of it share one candle-minute, so that minute's candle is written partially by BOTH runs (RMT replaces, not sums) — under ADR 0287 the second write can ERASE a priced minute, not merely undercount it, if its half holds only dust. Rebuild that single minute from ONE pass covering both sides, or move the boundary onto a minute edge"
        ),
        Some(true) => info!(
            boundary,
            prev_ledger, next_ledger, "run boundary is minute-aligned — no candle straddles it"
        ),
        // Neighbour not on disk: a partition-aligned boundary, or archive
        // tail-lag. Nothing to say.
        None => {}
    }
}

fn partition_fully_done(
    partition: &Partition,
    start: u32,
    end: u32,
    completed: &std::collections::HashSet<u32>,
) -> bool {
    let (first, last) = partition.clamped(start, end);
    (first..=last).all(|s| completed.contains(&s))
}

fn print_run_summary(
    partitions_processed: usize,
    totals: &PartitionStats,
    partitions_skipped_s3: usize,
    elapsed: Duration,
) {
    println!();
    println!("=== sdex-backfill complete ===");
    println!("partitions processed:      {partitions_processed}");
    println!("partitions skipped (S3):   {partitions_skipped_s3}");
    println!("ledgers indexed:           {}", totals.indexed);
    println!("ledgers already in DB:     {}", totals.skipped);
    println!("SDEX trade ticks:          {}", totals.trade_ticks);
    println!("AMM trade ticks:           {}", totals.amm_ticks);
    println!("oracle rows:               {}", totals.oracle_rows);
    println!("price_ohlcv_1m rows:       {}", totals.candles_written);
    println!("total bytes downloaded:    {}", totals.total_bytes);
    println!("elapsed:                   {} s", elapsed.as_secs());
}

async fn preflight_aws() {
    let out = Command::new("aws")
        .arg("--version")
        .output()
        .await
        .unwrap_or_else(|err| {
            panic!("pre-flight: failed to spawn `aws --version`: {err}");
        });
    if !out.status.success() {
        panic!("pre-flight: `aws --version` exited non-zero");
    }
    info!(
        version = %String::from_utf8_lossy(&out.stdout).trim(),
        "pre-flight: aws CLI present"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use extractors_core::Venue;

    fn swap(contract: &str, ledger: u32, count: u32) -> UnresolvedPoolSwap {
        UnresolvedPoolSwap {
            contract_id: contract.to_string(),
            ledger_sequence: ledger,
            swap_count: count,
            sample_topics: format!("[Symbol(\"swap\")] @ {ledger}"),
        }
    }

    #[test]
    fn aggregates_by_contract_and_widens_ledger_range() {
        // Two ledgers for the same pool → one record summing counts and spanning
        // the full ledger range.
        let raw = vec![swap("POOL_A", 48_600_000, 2), swap("POOL_A", 48_600_050, 3)];
        let reg = Registries::new(); // empty → nothing registered

        let out = aggregate_unresolved(&raw, &reg);

        assert_eq!(out.len(), 1);
        let a = &out[0];
        assert_eq!(a.contract_id, "POOL_A");
        assert_eq!(a.source, "backfill");
        assert_eq!(a.first_ledger, 48_600_000);
        assert_eq!(a.last_ledger, 48_600_050);
        assert_eq!(a.swap_count, 5);
        assert_eq!(a.still_unresolved, 1, "absent from registry → genuine gap");
    }

    #[test]
    fn recheck_clears_pools_registered_later_in_the_run() {
        let raw = vec![swap("GAP", 1, 1), swap("LATE", 2, 4)];
        // LATE registered later in the forward pass; GAP never did.
        let mut reg = Registries::new();
        reg.venue.insert("LATE".to_string(), Venue::Soroswap);

        let out = aggregate_unresolved(&raw, &reg);

        // Sorted genuine-gaps-first, so GAP (still_unresolved=1) precedes LATE.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].contract_id, "GAP");
        assert_eq!(out[0].still_unresolved, 1);
        assert_eq!(out[1].contract_id, "LATE");
        assert_eq!(
            out[1].still_unresolved, 0,
            "registered by run-end → recoverable, not a gap"
        );
    }

    #[test]
    fn empty_input_yields_no_records() {
        assert!(aggregate_unresolved(&[], &Registries::new()).is_empty());
    }

    /// Review WR-02: the decision the whole carry-the-open-minute fix turns on.
    /// Inverted to an unconditional `Drain`, every other test in the crate still
    /// passes and the boundary minute is written twice again.
    ///
    /// ⚠️ What this does NOT cover: that `execute` calls `partition_end` with
    /// the loop index, and that it calls `flush_open_minutes` after the loop.
    /// `index_partition` takes a concrete `Sink` (a ClickHouse writer with no
    /// trait behind it), so neither can be driven from a unit test; both are
    /// exercised end to end by the ignored `candles_it` suite.
    #[test]
    fn only_the_last_partition_of_a_run_drains_the_open_minute() {
        assert_eq!(partition_end(0, 3), PartitionEnd::Carry);
        assert_eq!(partition_end(1, 3), PartitionEnd::Carry);
        assert_eq!(partition_end(2, 3), PartitionEnd::Drain);
    }

    #[test]
    fn a_single_partition_run_drains() {
        assert_eq!(partition_end(0, 1), PartitionEnd::Drain);
    }

    /// Review CR-02: the two ledgers on either side of a range boundary must
    /// fall in different candle-minutes, or the minute they share is written
    /// partially by both runs — and under ADR 0287 the second write can ERASE
    /// the first rather than merely undercount it.
    #[test]
    fn a_boundary_between_two_minutes_is_aligned() {
        assert_eq!(
            boundary_is_minute_aligned(Some(1_700_000_040), Some(1_700_000_100)),
            Some(true)
        );
    }

    #[test]
    fn a_boundary_inside_one_minute_is_not_aligned() {
        assert_eq!(
            boundary_is_minute_aligned(Some(1_700_000_040), Some(1_700_000_040)),
            Some(false)
        );
    }

    /// Either side unreadable — the ledger is outside the synced partition, i.e.
    /// the boundary falls on a 64k partition edge, which needs no check anyway.
    #[test]
    fn a_boundary_with_an_unreadable_side_answers_nothing() {
        assert_eq!(boundary_is_minute_aligned(None, Some(1_700_000_040)), None);
        assert_eq!(boundary_is_minute_aligned(Some(1_700_000_040), None), None);
        assert_eq!(boundary_is_minute_aligned(None, None), None);
    }
}
