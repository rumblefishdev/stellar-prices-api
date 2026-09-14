//! Doorbell-cursor reconcile loop.
//!
//! Mirrors BE's indexer: read cursor, derive the next S3 key, fetch, decode,
//! extract+bucket, write, advance the cursor **last**. Stops at the first gap or
//! `max_iterations`. The cursor write is the ordering barrier — a crash before
//! it leaves the cursor unchanged and the next invocation re-processes the run
//! (idempotent: ReplacingMergeTree collapses re-inserts by `version`).
//!
//! The decode→extract→canonicalise→bucket step is `prices_ingest_core` — the
//! same code the SDEX backfill runs — so live candles are byte-identical to
//! backfilled ones. Candles accumulate across the whole contiguous run and are
//! flushed at the end, so all ledgers sharing a minute aggregate into one
//! candle (matching the backfill's per-chunk accumulation).
//!
//! **A run ends on a whole minute, not wherever the feed ran out** (task 0282).
//! Only minutes the run saw the END of are flushed, and the cursor is rewound to
//! the last ledger of the last complete minute; the held-back ledgers are
//! re-read next run. Without that, the final partial minute was written as a
//! candle and the next run's write for the same bucket — carrying a higher
//! `version` — REPLACED it rather than summing, silently discarding the earlier
//! slice. This was not the small residual it was once described as: measured on
//! production 2026-09-14, buckets spanning 4+ ledgers retained **15%** of their
//! trades, and Aquarius was losing ~50% of its trades every day.
//!
//! ⚠️ The cost is latency: a minute's candles are published only once a ledger
//! from the FOLLOWING minute has been fetched, so the newest candle can be up to
//! ~1 minute behind rather than ~5 seconds. Check the
//! `prices-production-rollup-freshness-1m` threshold (task 0137) before deploy.

use std::collections::HashMap;
use std::time::Instant;

use prices_ingest_core::{
    AssetRegistry, CandleAccumulator, OracleSample, Registries, decode_object, extract_trades,
    ledger_close_time, ledger_sequence, process_ledger, raw_trade_to_tick,
};
use tokio::sync::Mutex;
use tracing::info;

use crate::cursor::{Cursor, CursorError};
use crate::galexie_key::ledger_s3_key;
use crate::metrics::WriteLatency;
use crate::object_fetcher::{FetchError, ObjectFetcher};
use crate::sink::{CandleSink, SinkError};

#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    #[error("cursor error: {0}")]
    Cursor(#[from] CursorError),
    #[error("fetch error: {0}")]
    Fetch(#[from] FetchError),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("sink error: {0}")]
    Sink(#[from] SinkError),
}

#[derive(Debug, Clone, Default)]
pub struct RunStats {
    pub start_cursor: u64,
    pub end_cursor: u64,
    pub ledgers_persisted: u64,
    /// Ledgers this run decoded but deliberately did NOT write, because they
    /// belong to a minute still being filled (task 0282). They are re-read next
    /// run. A steadily non-zero value is normal; a value that never falls to
    /// zero while the cursor never moves means the feed has stopped advancing.
    pub ledgers_held_back: u64,
    pub rows_emitted: u64,
    /// Candle-INSERT latency for this run, or `None` when the run wrote no
    /// candles at all. `None` rather than a zeroed struct so an idle run
    /// publishes no `ClickHouseWriteLatencyMs` datapoint instead of a 0 ms one
    /// (task 0125).
    pub ch_write: Option<WriteLatency>,
}

/// Warm per-container processing state: the surrogate-id registry (loaded from
/// `prices.assets` at cold start) and the incrementally-grown AMM venue/pool
/// registries. Persisting these across invocations lets a warm Lambda resolve
/// pools discovered earlier in its lifetime.
pub struct ProcessingState {
    pub assets: AssetRegistry,
    pub registries: Registries,
    /// Highest surrogate-id watermark whose assets are known **durably written**
    /// to `prices.assets`. Each run writes `assets_since(this)` and advances it
    /// only *after* that write succeeds (task 0132). Because the registry is warm
    /// across invocations, a run that interns new assets and then fails a later
    /// write leaves `next_id` advanced but this watermark unmoved — so the *next*
    /// run re-writes those assets instead of orphaning them (they would otherwise
    /// sit below a freshly-captured `next_id` and never be written, stranding the
    /// candles that reference their ids). Init: every asset loaded at cold start
    /// is already in `prices.assets`, so it starts at the loaded `watermark()`.
    pub persisted_asset_watermark: u32,
}

pub struct Reconciler<F, C, S> {
    fetcher: F,
    cursor: C,
    sink: S,
    state: Mutex<ProcessingState>,
}

impl<F, C, S> Reconciler<F, C, S>
where
    F: ObjectFetcher + Sync,
    C: Cursor + Sync,
    S: CandleSink + Sync,
{
    pub fn new(
        fetcher: F,
        cursor: C,
        sink: S,
        assets: AssetRegistry,
        registries: Registries,
    ) -> Self {
        // Everything loaded from `prices.assets` at cold start is already durable,
        // so the persisted watermark starts at the loaded registry's next id.
        let persisted_asset_watermark = assets.watermark();
        Self {
            fetcher,
            cursor,
            sink,
            state: Mutex::new(ProcessingState {
                assets,
                registries,
                persisted_asset_watermark,
            }),
        }
    }

    pub async fn run(&self, max_iterations: usize) -> Result<RunStats, ReconcileError> {
        let mut st = self.state.lock().await;
        // Deref the guard once so `registries` and `assets` can be borrowed as
        // disjoint fields (a borrow through the guard's DerefMut each time would
        // conflict).
        let state = &mut *st;

        let start = self.cursor.read().await?;
        // Tracks the highest ledger DECODED so far. The cursor is advanced to a
        // whole-minute boundary derived from `ledger_minutes` after the loop,
        // not to this — see the task-0282 block below.
        let mut current = start;
        let mut persisted = 0u64;

        // Accumulate across the whole contiguous run, then flush only the
        // minutes this run saw the END of — see `run_boundary` below.
        let mut sdex = CandleAccumulator::new();
        let mut amm: HashMap<&'static str, CandleAccumulator> = HashMap::new();
        let mut oracle: Vec<OracleSample> = Vec::new();
        // (ledger_sequence, minute_start) for every ledger this run decoded, so
        // the run can end on a whole-minute boundary instead of wherever the
        // feed happened to run out.
        let mut ledger_minutes: Vec<(u64, u32)> = Vec::new();

        for _ in 0..max_iterations {
            let next = current + 1;
            let key = ledger_s3_key(next as i64);
            let Some(bytes) = self.fetcher.fetch(&key).await? else {
                if persisted == 0 {
                    info!(next, "no new contiguous ledger — nothing to do");
                } else {
                    info!(next, persisted, "reached gap on S3 — contiguous run done");
                }
                break;
            };

            let lcms = decode_object(&bytes).map_err(|e| ReconcileError::Decode(e.to_string()))?;
            let mut obj_max = current;
            for lcm in &lcms {
                // Classic SDEX trades from operation results.
                for trade in extract_trades(lcm) {
                    sdex.merge(&raw_trade_to_tick(&trade, &mut state.assets));
                }
                // Soroban AMM trades + oracle samples.
                let sob = process_ledger(lcm, &mut state.registries, &mut state.assets);
                for (source, tick) in &sob.amm_ticks {
                    amm.entry(source).or_default().merge(tick);
                }
                oracle.extend(sob.oracle);
                let seq = ledger_sequence(lcm) as u64;
                // Same bucketing the accumulator uses (`bucket.rs::merge`), so
                // these minute keys and the candle keys cannot drift apart.
                let minute = (ledger_close_time(lcm) as u32 / 60) * 60;
                ledger_minutes.push((seq, minute));
                obj_max = obj_max.max(seq);
            }

            current = obj_max.max(next);
            persisted += 1;
        }

        if persisted == 0 {
            return Ok(RunStats {
                start_cursor: start,
                end_cursor: start,
                ledgers_persisted: 0,
                ledgers_held_back: 0,
                rows_emitted: 0,
                // Nothing was persisted, so no INSERT happened: no datapoint.
                ch_write: None,
            });
        }

        // --- Task 0282: never write a minute that is still being filled. ---
        //
        // The run ends wherever the feed ran out, which is almost never a minute
        // boundary. Flushing that last, partial minute writes a candle holding
        // only the slice this run happened to see. `price_ohlcv_1m` is
        // ReplacingMergeTree(`version`), and `version = max(ledger*1000 + op)`
        // over the bucket — so the NEXT run's write for the same bucket carries
        // a higher version and REPLACES this row instead of summing with it. The
        // trades in this slice are then gone, with no error and a candle that
        // still looks entirely plausible.
        //
        // Measured on production 2026-09-14 before this fix: buckets confined to
        // one ledger kept 100% of their trades, buckets spanning 2-3 kept 41.9%,
        // 4+ kept 15.1% — each matching "only the last write survives" to within
        // 0.6%. Aquarius, whose 488 pools crowd the same pairs, was losing ~50%
        // of its trades every day.
        //
        // The fix: flush only minutes this run saw the END of, and rewind the
        // cursor to the last ledger of the last COMPLETE minute. The held-back
        // ledgers are re-read next run and their minute is written once, whole.
        // Re-reading is what keeps this crash-safe: no accumulator state has to
        // survive between invocations, so a cold start behaves like a warm one.
        let (open_minute, complete_end) = run_boundary(&ledger_minutes);

        let held_back = ledger_minutes.iter().filter(|(s, _)| *s > current).count();

        // Write newly-interned assets FIRST — the candles below reference their
        // surrogate ids, so persisting the dimension row before the fact rows
        // keeps `prices.assets` referentially ahead of `price_ohlcv_*`. Only the
        // assets not yet durably written (id >= the persisted watermark), not the
        // whole registry (task 0132); a run that discovered nothing new writes
        // nothing. Advance the durable watermark ONLY after the write succeeds —
        // if it fails here the run returns early (cursor unmoved, doorbell
        // redelivered) and the next run retries these same assets.
        self.sink
            .write_new_assets(&state.assets, state.persisted_asset_watermark)
            .await?;
        state.persisted_asset_watermark = state.assets.watermark();

        let Some(current) = complete_end else {
            // Every ledger in this run belongs to the minute still being filled.
            // Write no CANDLES and leave the cursor where it was: the next run
            // re-reads these ledgers and completes the minute in one write.
            // Assets discovered above are already written — they are dimension
            // rows, harmless to persist early, and keeping `prices.assets`
            // referentially ahead of `price_ohlcv_*` is the existing contract.
            // Not an error, and not a stall — it resolves as soon as a ledger
            // from the next minute arrives.
            info!(
                start,
                ledgers = ledger_minutes.len(),
                open_minute,
                "run is entirely inside one open minute — holding back, cursor unmoved"
            );
            return Ok(RunStats {
                start_cursor: start,
                end_cursor: start,
                ledgers_persisted: 0,
                ledgers_held_back: ledger_minutes.len() as u64,
                rows_emitted: 0,
                ch_write: None,
            });
        };

        // Flush + write candles/oracle, then advance the cursor LAST (barrier).
        let mut rows_emitted = 0u64;

        // Task 0125: time the candle INSERTs — the ClickHouse-side write signal
        // the cluster itself cannot give us (no metric stream, no `system.*`
        // grant). Timed at the CALL SITE, not inside the sink, so the
        // `CandleSink` trait and both impls stay untouched and no feature cfg
        // leaks into the write path. Recorded only AFTER the `?`, so a failed
        // write is not measured and no error path changes. Note that
        // `ClickHouseSink::write_candles` wraps the write in
        // `retry_with_backoff`, so a retried write folds its backoff sleeps in
        // and reads as one long write.
        let mut ch_write = WriteLatency::default();

        // `flush_older_than` leaves the open minute in the accumulator, which
        // is then dropped — its ledgers are re-read next run, not lost.
        let sdex_candles = sdex.flush_older_than(open_minute);
        rows_emitted += sdex_candles.len() as u64;
        let t = Instant::now();
        self.sink.write_candles(&sdex_candles, "sdex").await?;
        // Only a write that had rows crossed the network: `write_candles`
        // short-circuits on an empty slice, so timing that would fold a ~0 ms
        // in-process no-op into the latency samples.
        if !sdex_candles.is_empty() {
            ch_write.record(t.elapsed().as_secs_f64() * 1000.0);
        }

        for (source, mut acc) in amm {
            let candles = acc.flush_older_than(open_minute);
            rows_emitted += candles.len() as u64;
            let t = Instant::now();
            self.sink.write_candles(&candles, source).await?;
            // Same guard per AMM source — a source with no trades in the window
            // is routine, and its no-op must not enter the samples.
            if !candles.is_empty() {
                ch_write.record(t.elapsed().as_secs_f64() * 1000.0);
            }
        }

        self.sink.write_oracle(&oracle).await?;
        self.cursor.write(current).await?;

        info!(
            start,
            end = current,
            persisted,
            held_back,
            open_minute,
            rows = rows_emitted,
            "reconcile run complete"
        );

        Ok(RunStats {
            start_cursor: start,
            end_cursor: current,
            ledgers_persisted: persisted,
            ledgers_held_back: held_back as u64,
            rows_emitted,
            ch_write: (!ch_write.samples_ms.is_empty()).then_some(ch_write),
        })
    }
}

/// Split a run's decoded ledgers at the last whole-minute boundary.
///
/// Returns `(open_minute, last_ledger_of_the_last_complete_minute)`. The open
/// minute is the newest one the run touched — the run cannot know whether it saw
/// all of it, because the next ledger of that minute may simply not be on S3
/// yet. Everything strictly older than it is complete and safe to write.
///
/// `None` for the second element means every ledger in the run belongs to the
/// open minute, so the run has nothing it can safely write. See the task-0282
/// block in [`Reconciler::run`] for why writing it anyway loses data.
fn run_boundary(ledger_minutes: &[(u64, u32)]) -> (u32, Option<u64>) {
    let open_minute = ledger_minutes.iter().map(|(_, m)| *m).max().unwrap_or(0);
    let complete_end = ledger_minutes
        .iter()
        .filter(|(_, m)| *m < open_minute)
        .map(|(s, _)| *s)
        .max();
    (open_minute, complete_end)
}

#[cfg(test)]
mod tests {
    use super::run_boundary;

    /// The ordinary case: a run straddles a minute boundary, so the earlier
    /// minute is complete and the later one is still being filled.
    #[test]
    fn a_run_straddling_a_boundary_ends_on_the_last_complete_minute() {
        let ledgers = vec![(100, 60), (101, 60), (102, 120), (103, 120)];
        let (open, end) = run_boundary(&ledgers);
        assert_eq!(open, 120, "the newest minute touched is the open one");
        assert_eq!(
            end,
            Some(101),
            "the cursor stops at the last ledger of minute 60, so 102-103 are re-read"
        );
    }

    /// Task 0282: a run entirely inside one minute must write NOTHING. Writing
    /// it would emit a partial candle that the next run's higher-version write
    /// replaces rather than sums.
    #[test]
    fn a_run_inside_a_single_minute_writes_nothing() {
        let ledgers = vec![(100, 60), (101, 60), (102, 60)];
        let (open, end) = run_boundary(&ledgers);
        assert_eq!(open, 60);
        assert_eq!(end, None, "no complete minute — hold everything back");
    }

    /// The steady-state shape in production: one ledger per doorbell. It holds
    /// back every time until a ledger from the next minute arrives, which is
    /// exactly the behaviour that stops the 15%-retention case.
    #[test]
    fn a_single_ledger_run_holds_back_until_the_minute_turns() {
        assert_eq!(run_boundary(&[(100, 60)]).1, None);
        assert_eq!(
            run_boundary(&[(100, 60), (101, 120)]).1,
            Some(100),
            "once the minute turns, the completed minute is released"
        );
    }

    /// Ledgers are pushed in fetch order, but the boundary must not depend on
    /// that: it is derived from close times, not from position in the vec.
    #[test]
    fn the_boundary_does_not_depend_on_arrival_order() {
        let forward = run_boundary(&[(100, 60), (101, 120), (102, 120)]);
        let shuffled = run_boundary(&[(102, 120), (100, 60), (101, 120)]);
        assert_eq!(forward, shuffled);
        assert_eq!(forward.1, Some(100));
    }

    /// A minute with no ledgers at all (an idle stretch) must not strand the
    /// run: minute 60 is still complete even though minute 120 never appears.
    #[test]
    fn a_gap_in_minutes_still_releases_the_older_one() {
        let (open, end) = run_boundary(&[(100, 60), (101, 60), (102, 300)]);
        assert_eq!(open, 300);
        assert_eq!(end, Some(101));
    }

    #[test]
    fn an_empty_run_has_no_boundary() {
        assert_eq!(run_boundary(&[]), (0, None));
    }
}
