//! Doorbell-cursor reconcile loop.
//!
//! Mirrors BE's indexer: read cursor, derive the next S3 key, fetch, decode,
//! extract+bucket, write, advance the cursor **last**. Stops at the first gap or
//! `max_iterations`. The cursor write is the ordering barrier — a crash before
//! it leaves the cursor unchanged and the next invocation re-processes the run.
//!
//! ⚠️ **Re-processing is near-idempotent, not idempotent.** A re-read minute is
//! rebuilt whole and carries the same payload, so the surviving row is correct.
//! But `operation_index` is not stable across re-processing, so the same trade
//! can come back with a different `version`, and ReplacingMergeTree then keeps
//! both rows until a merge rather than collapsing them — production holds such
//! byte-identical pairs (task 0282). Do not lean on `version` equality: what
//! makes a re-read safe is that it writes the WHOLE minute again, never a slice.
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
//! Two costs, both deliberate:
//!
//! ⚠️ **Latency.** A minute's candles are published only once a ledger from the
//! FOLLOWING minute has been fetched, so the newest candle can be up to ~1 minute
//! behind rather than ~5 seconds. Measured against the alarm that watches this:
//! `prices-production-rollup-freshness-1m` fires above **900 s** and production
//! sits at a steady **18 s**, so ~78 s worst case stays an order of magnitude
//! clear. The coarse tiers shift by the same ~60 s against thresholds of 3,600 s
//! and up.
//!
//! ⚠️ **Re-reads.** Each doorbell re-fetches and re-decodes every ledger held
//! back since the minute last turned — 1 object on the first doorbell of a
//! minute, growing to ~12 on the last, so roughly a 6-7x average increase in S3
//! GETs, zstd decompression and extraction work. That is the price of keeping no
//! accumulator state between invocations, which is what makes a cold start
//! behave like a warm one. It scales with ledgers-per-minute, the same factor
//! that drives the budget-exhausted escape hatch in `run_end` below.
//!
//! 🔑 **The cursor always parks on a minute boundary** (outside the escape
//! hatch). It is only ever written as the last ledger of a COMPLETE minute, so
//! whenever the process stops — crash, outage, redeploy — the next run resumes at
//! the first ledger of a minute and no minute is split across the stop. A backlog
//! therefore drains one or more whole minutes per run until the tip, where runs
//! go back to holding back the open minute. `tests/reconcile_drain.rs` pins this
//! across successive runs.
//!
//! ⚠️ The first run after deploying this over the old code is the exception: the
//! old code left the cursor mid-minute, so that one minute is written from its
//! tail only. It self-heals from the next minute on.

use std::collections::HashMap;
use std::time::Instant;

use prices_ingest_core::{
    AssetRegistry, CandleAccumulator, OracleSample, Registries, decode_object, extract_trades,
    ledger_close_time, ledger_sequence, offer_lookup_counts, process_ledger, raw_trade_to_tick,
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
    /// Whether this run hit the forced-progress escape hatch and flushed a
    /// PARTIAL minute to keep the cursor moving (task 0282).
    ///
    /// ⚠️ True means this run re-created the task-0282 loss for that one minute.
    /// It is published as `ForcedPartialFlushes` so an alarm can see it — a WARN
    /// line alone is not observable, and every other signal on this path reads
    /// healthy (the doorbell is consumed, the queue drains, no error is raised).
    pub forced_partial_flush: bool,
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

    /// One reconcile run for a LONG-RUNNING caller (the Lambda). Minutes the
    /// run did not see the end of are held back for the next run.
    pub async fn run(&self, max_iterations: usize) -> Result<RunStats, ReconcileError> {
        self.run_inner(max_iterations, false).await
    }

    /// One reconcile run for a ONE-SHOT caller (`bin/cli.rs`), which exits when
    /// this returns. There is no "next run" to re-read held-back ledgers, so the
    /// open minute is flushed rather than dropped.
    ///
    /// ⚠️ That last minute may therefore be PARTIAL — it carries only the trades
    /// this process saw. For a terminal replay that is the right trade (a partial
    /// candle beats a silently missing one), but it is why the CLI is not a way
    /// to repair a range: use `events-backfill`, which accumulates the whole
    /// range before writing.
    pub async fn run_terminal(&self, max_iterations: usize) -> Result<RunStats, ReconcileError> {
        self.run_inner(max_iterations, true).await
    }

    async fn run_inner(
        &self,
        max_iterations: usize,
        terminal: bool,
    ) -> Result<RunStats, ReconcileError> {
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
        let offer_lookups_before = offer_lookup_counts();

        // Accumulate across the whole contiguous run, then flush only the
        // minutes this run saw the END of — see `run_boundary` below.
        let mut sdex = CandleAccumulator::new();
        let mut amm: HashMap<&'static str, CandleAccumulator> = HashMap::new();
        let mut oracle: Vec<OracleSample> = Vec::new();
        // (ledger_sequence, minute_start, is_last_ledger_of_its_object) for every
        // ledger this run decoded, so the run can end on a whole-minute boundary
        // instead of wherever the feed happened to run out.
        //
        // The third element guards a latent coupling: `ledger_s3_key` assumes
        // `ledgers_per_file = 1`, so the cursor must land on an OBJECT boundary —
        // `ledger_s3_key(interior_ledger + 1)` would resolve to a key that does
        // not exist and the run would gap-stop forever. Before this task the
        // cursor was always `obj_max`, so that held for free; the minute-boundary
        // rewind can pick an interior ledger, so it has to be enforced.
        let mut ledger_minutes: Vec<(u64, u32, bool)> = Vec::new();

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
            let mut obj_ledgers: Vec<(u64, u32)> = Vec::new();
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
                obj_ledgers.push((seq, minute));
                obj_max = obj_max.max(seq);
            }
            // Flag only this object's HIGHEST ledger as a valid cursor landing
            // point (see `ledger_minutes` above).
            let obj_last = obj_ledgers.iter().map(|(s, _)| *s).max();
            ledger_minutes.extend(
                obj_ledgers
                    .into_iter()
                    .map(|(s, m)| (s, m, Some(s) == obj_last)),
            );

            current = obj_max.max(next);
            persisted += 1;
        }

        if persisted == 0 {
            return Ok(RunStats {
                start_cursor: start,
                end_cursor: start,
                ledgers_persisted: 0,
                ledgers_held_back: 0,
                forced_partial_flush: false,
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
        let (open_minute, split) = run_boundary(&ledger_minutes);

        // FIX 2 — forced progress. Holding back is only safe while a LATER run
        // can reach the next minute. If this run filled its whole iteration
        // budget and every ledger still shared one minute, waiting cannot help:
        // the next run fetches the same ledgers, finds the same open minute, and
        // holds back again — forever, with no error, no Err arm, and no alarm
        // (the SQS doorbell is consumed successfully every time, so
        // `ApproximateAgeOfOldestMessage` stays at 0). Ingestion would stop dead
        // while every signal read healthy.
        //
        // Production runs `maxIterations: 32` (`infra/envs/production.json`).
        // Measured over 7 days the network never closed more than 12 ledgers in a
        // minute, so the longest walk that must see a minute turn is 13. Crossing
        // 32 needs a sub-2 s close time — a deliberate network change, not drift.
        //
        // So in exactly that case, flush the open minute and advance anyway. That
        // re-exposes the task-0282 partial-write hazard for that ONE minute, and
        // that is the deliberate trade: a candle that may be undercounted beats a
        // pipeline that has silently stopped. It is logged at WARN because it
        // means `maxIterations` is now too small for the chain's block rate.
        let end = run_end(
            split,
            persisted,
            max_iterations,
            ledger_minutes.len(),
            terminal,
        );
        // Only the budget case re-creates the task-0282 loss, so only it is
        // reported as a forced partial flush. A terminal run flushing its last
        // minute is the designed behaviour of a one-shot caller, and an object
        // with no ledgers has nothing to flush at all — counting either would
        // fire an alarm whose advice (raise `maxIterations`) is wrong for them.
        let forced = end == RunEnd::FlushAll(FlushAllReason::BudgetExhausted);
        match end {
            RunEnd::FlushAll(FlushAllReason::BudgetExhausted) => tracing::warn!(
                start,
                max_iterations,
                ledgers = ledger_minutes.len(),
                open_minute,
                "iteration budget exhausted inside ONE minute — flushing a PARTIAL minute \
                 to keep the cursor moving; raise ledgerProcessor.maxIterations above \
                 the ledgers-per-minute rate (task 0282)"
            ),
            RunEnd::FlushAll(FlushAllReason::NoLedgersDecoded) => info!(
                start,
                persisted, "objects decoded to no ledgers — advancing past them, nothing to flush"
            ),
            _ => {}
        }

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

        // `unwrap_or(current)` not `start`: an object that decoded to no ledgers
        // still advanced `current` past it, and that advance must not be lost.
        let highest_decoded = ledger_minutes
            .iter()
            .map(|(s, _, _)| *s)
            .max()
            .unwrap_or(current);
        // `(cursor to write, flush minutes strictly older than this)`.
        let advance_to = match end {
            RunEnd::Split {
                cursor,
                flush_before,
            } => Some((cursor, flush_before)),
            // Everything decoded is flushed, so the cursor may pass all of it.
            RunEnd::FlushAll(_) => Some((highest_decoded, u32::MAX)),
            RunEnd::Hold => None,
        };

        let Some((current, flush_boundary)) = advance_to else {
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
                // Holding back is the DESIGNED path, not the escape hatch.
                forced_partial_flush: false,
                rows_emitted: 0,
                ch_write: None,
            });
        };

        // Computed AFTER `current` is bound to the run's real end — an earlier
        // revision of this computed it against the outer `current` (the highest
        // ledger DECODED), where the predicate is false by construction and the
        // field was always 0 on the path that matters.
        let held_back = held_back_count(&ledger_minutes, current);

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
        let sdex_candles = sdex.flush_older_than(flush_boundary);
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
            let candles = acc.flush_older_than(flush_boundary);
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

        // Task 0286 phase 2 (S4): how many order-book fills this run priced from
        // the offer they crossed, and how many fell back to their own amounts.
        // The rate, not a once-per-process warn line.
        let offer_lookups = offer_lookup_counts().since(offer_lookups_before);
        info!(
            start,
            end = current,
            persisted,
            held_back,
            open_minute,
            rows = rows_emitted,
            order_book_fills = offer_lookups.order_book_fills,
            offer_lookup_misses = offer_lookups.offer_lookup_misses,
            pool_fills = offer_lookups.pool_fills,
            "reconcile run complete"
        );

        Ok(RunStats {
            start_cursor: start,
            end_cursor: current,
            ledgers_persisted: persisted,
            ledgers_held_back: held_back as u64,
            forced_partial_flush: forced,
            rows_emitted,
            ch_write: (!ch_write.samples_ms.is_empty()).then_some(ch_write),
        })
    }
}

/// How a run ends: where the cursor goes, and which minutes are written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RunEnd {
    /// The normal case: park the cursor at `cursor` and write only minutes
    /// strictly older than `flush_before` — exactly the minutes whose ledgers
    /// all sit at or below `cursor`. See [`run_boundary`].
    Split { cursor: u64, flush_before: u32 },
    /// Nothing can be written safely yet: leave the cursor where it was.
    Hold,
    /// Write every minute decoded and move the cursor past all of it.
    FlushAll(FlushAllReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FlushAllReason {
    /// A one-shot caller (`bin/cli.rs`) exits when the run returns, so nobody
    /// would ever re-read a held-back minute. By design, not an escape hatch.
    Terminal,
    /// The whole iteration budget went inside one minute: a later run would
    /// fetch the same ledgers and hold back identically, forever. The ONLY
    /// reason that re-creates the task-0282 loss, so the only one alarmed.
    BudgetExhausted,
    /// Objects were consumed but decoded to no ledgers, leaving nothing to
    /// bound a minute with. Nothing partial is written.
    NoLedgersDecoded,
}

/// Decide how a run ends. See the WARN-logged block in [`Reconciler::run`] for
/// why a possibly-undercounted candle beats a stalled pipeline.
fn run_end(
    split: Option<(u64, u32)>,
    persisted: u64,
    max_iterations: usize,
    decoded_ledgers: usize,
    terminal: bool,
) -> RunEnd {
    // Checked FIRST: a one-shot caller must flush its last minute even when an
    // earlier minute is complete — otherwise that minute is silently dropped
    // and the cursor rewound behind it as the process exits.
    if terminal {
        return RunEnd::FlushAll(FlushAllReason::Terminal);
    }
    if let Some((cursor, flush_before)) = split {
        return RunEnd::Split {
            cursor,
            flush_before,
        };
    }
    if persisted as usize >= max_iterations {
        return RunEnd::FlushAll(FlushAllReason::BudgetExhausted);
    }
    // Pre-0282 an empty object advanced the cursor past itself; keep doing
    // that, or the run would hold back zero ledgers forever.
    if persisted > 0 && decoded_ledgers == 0 {
        return RunEnd::FlushAll(FlushAllReason::NoLedgersDecoded);
    }
    RunEnd::Hold
}

/// Ledgers decoded by this run that sit ABOVE the cursor it will write, and so
/// will be re-read next run.
///
/// Must be called with the run's FINAL cursor, not the highest ledger decoded —
/// against the latter the predicate is false by construction and this silently
/// returns 0 for every run.
fn held_back_count(ledger_minutes: &[(u64, u32, bool)], cursor_end: u64) -> usize {
    ledger_minutes
        .iter()
        .filter(|(s, _, _)| *s > cursor_end)
        .count()
}

/// Split a run's decoded ledgers at the last clean whole-minute boundary.
///
/// Returns `(open_minute, Some((cursor, flush_before)))`, or `None` in the
/// second slot when no safe split exists. The open minute is the newest one the
/// run touched — the run cannot know whether it saw all of it, because the next
/// ledger of that minute may simply not be on S3 yet.
///
/// A candidate cursor `c` is safe only when it cleanly divides the minutes:
///
/// - `c` ends an S3 object — `ledger_s3_key` assumes one ledger per object and
///   the next key is derived from the cursor, so an interior ledger would
///   produce a key that does not exist and the run would gap-stop forever;
/// - every minute at or below `c` is older than the open minute; and
/// - no ledger ABOVE `c` shares a minute with a ledger at or below it.
///
/// The last condition is what keeps the write and the cursor in agreement. The
/// run writes minutes up to `flush_before` and the next run re-reads everything
/// above `c`; if a minute had ledgers on both sides, it would be written now
/// from all of them and re-written next run from only the upper part — with a
/// higher `version`, so ReplacingMergeTree keeps the partial row. With one
/// ledger per object it always holds; it matters only for multi-ledger objects.
///
/// See the task-0282 block in [`Reconciler::run`] for why writing an open
/// minute loses data.
fn run_boundary(ledger_minutes: &[(u64, u32, bool)]) -> (u32, Option<(u64, u32)>) {
    let open_minute = ledger_minutes.iter().map(|(_, m, _)| *m).max().unwrap_or(0);
    let split = ledger_minutes
        .iter()
        .filter(|(_, _, obj_end)| *obj_end)
        .filter_map(|(c, _, _)| {
            let below = ledger_minutes
                .iter()
                .filter(|(s, _, _)| s <= c)
                .map(|(_, m, _)| *m)
                .max()?;
            let above = ledger_minutes
                .iter()
                .filter(|(s, _, _)| s > c)
                .map(|(_, m, _)| *m)
                .min();
            let clean = below < open_minute && above.is_none_or(|a| below < a);
            // Minutes are whole multiples of 60, so `< below + 1` is `<= below`.
            clean.then_some((*c, below + 1))
        })
        .max_by_key(|(c, _)| *c);
    (open_minute, split)
}

#[cfg(test)]
mod tests {
    use super::{FlushAllReason, RunEnd, held_back_count, run_boundary, run_end};

    /// The cursor half of `run_boundary`'s split, for tests that only care
    /// where the run parks.
    fn cursor(ledgers: &[(u64, u32, bool)]) -> Option<u64> {
        run_boundary(ledgers).1.map(|(c, _)| c)
    }

    /// Whether `run_end` flushes everything for the given reason.
    fn flushes_all(end: RunEnd, reason: FlushAllReason) -> bool {
        end == RunEnd::FlushAll(reason)
    }

    /// Finding 5: an object that decodes to NO ledgers used to leave
    /// `run_boundary` at `(0, None)`, so the run "held back" zero ledgers and
    /// the cursor sat still forever. Pre-0282 it advanced past the object.
    #[test]
    fn an_object_that_decodes_to_no_ledgers_still_advances() {
        assert!(
            flushes_all(
                run_end(None, 1, 16, 0, false),
                FlushAllReason::NoLedgersDecoded
            ),
            "nothing decoded but an object was consumed: advance, do not hold"
        );
        assert_eq!(
            run_end(None, 0, 16, 0, false),
            RunEnd::Hold,
            "no object consumed either — that is the ordinary idle run"
        );
    }

    /// Review of PR #313, 2026-09-17, finding 3: an empty object used to be
    /// reported as a forced PARTIAL flush — WARN, metric, and an alarm telling
    /// the operator to raise `maxIterations`. Nothing partial is written there,
    /// so it must be a distinct reason from the one that is alarmed.
    #[test]
    fn an_empty_object_is_not_reported_as_a_partial_flush() {
        assert_ne!(
            run_end(None, 1, 16, 0, false),
            RunEnd::FlushAll(FlushAllReason::BudgetExhausted)
        );
    }

    /// Finding 3: a one-shot caller (`bin/cli.rs`) exits when the run returns,
    /// so there is no next run to re-read held-back ledgers. It flushes.
    #[test]
    fn a_terminal_run_always_flushes() {
        assert!(flushes_all(
            run_end(None, 1, 16, 3, true),
            FlushAllReason::Terminal
        ));
        // Review of PR #313, 2026-09-17, finding 1. This used to assert the
        // OPPOSITE — "a complete minute needs no forcing, terminal or not" —
        // which pinned the bug: the terminal run parked at the complete minute
        // and exited, so its last minute was never written by anyone.
        assert!(
            flushes_all(
                run_end(Some((101, 61)), 1, 16, 3, true),
                FlushAllReason::Terminal
            ),
            "a terminal run flushes its last minute even when an earlier one is complete"
        );
    }

    /// Finding 7: `ledger_s3_key` assumes one ledger per object, so the cursor
    /// must land on an object boundary. With multi-ledger batches the minute
    /// boundary can fall inside an object, and parking there would make the
    /// next key resolve to something that does not exist.
    #[test]
    fn the_cursor_never_parks_inside_a_multi_ledger_object() {
        // One object holds 100-102; the minute turns at 102, mid-object.
        let ledgers = [(100, 60, false), (101, 60, false), (102, 120, true)];
        let (open, end) = run_boundary(&ledgers);
        assert_eq!(open, 120);
        assert_eq!(
            end, None,
            "101 completes a minute but is interior — refuse it and hold back"
        );

        // Same minutes, but now the object ends at 101: that IS a legal landing.
        let split = [(100, 60, false), (101, 60, true), (102, 120, true)];
        assert_eq!(run_boundary(&split).1, Some((101, 61)));
    }

    /// Review of PR #313, 2026-09-17, finding 2. Object A = [100], object B =
    /// [101, 102, 103]. The only object end below the open minute is 100, but
    /// 101 shares minute 60 with it. The old code parked at 100 and still wrote
    /// minute 60 (and 120) — then the next run re-read 101 and wrote minute 60
    /// again from 101 alone, with a higher `version`, replacing the whole
    /// candle with a slice. That is the task-0282 loss.
    ///
    /// Merely writing less (minutes before 60) would not do either: the cursor
    /// would still pass ledger 100, whose minute-60 trades nobody would write.
    /// The only safe answer is no split at all.
    #[test]
    fn a_minute_straddling_the_cursor_is_never_a_split() {
        let ledgers = [
            (100, 60, true),
            (101, 60, false),
            (102, 120, false),
            (103, 180, true),
        ];
        assert_eq!(run_boundary(&ledgers), (180, None));

        // Once a later minute arrives, 103 is a clean split and minute 60 is
        // written once, whole, from both 100 and 101.
        let later = [
            (100, 60, true),
            (101, 60, false),
            (102, 120, false),
            (103, 180, true),
            (104, 240, true),
        ];
        assert_eq!(run_boundary(&later), (240, Some((103, 181))));
    }

    /// The split writes exactly the minutes whose ledgers all sit at or below
    /// the cursor — no more (a straddled minute) and no fewer (a stranded one).
    #[test]
    fn the_split_writes_exactly_the_minutes_below_the_cursor() {
        let ledgers = [
            (100, 60, true),
            (101, 120, true),
            (102, 120, true),
            (103, 180, true),
        ];
        let (_, split) = run_boundary(&ledgers);
        let (c, flush_before) = split.unwrap();
        for (s, m, _) in ledgers {
            assert_eq!(
                m < flush_before,
                s <= c,
                "ledger {s} (minute {m}): written iff at or below the cursor"
            );
        }
    }

    /// Regression, code review of PR #313 finding 1: `held_back` was computed
    /// against the highest ledger DECODED rather than the cursor the run
    /// actually writes. `current >= s` holds for every decoded ledger by
    /// construction, so the count was always 0 — including on the normal path,
    /// where the whole point is that it is NOT 0.
    #[test]
    fn held_back_counts_ledgers_above_the_written_cursor() {
        let ledgers = [
            (100, 60, true),
            (101, 60, true),
            (102, 120, true),
            (103, 120, true),
        ];
        assert_eq!(
            held_back_count(&ledgers, 101),
            2,
            "102 and 103 are above the cursor and will be re-read"
        );
        assert_eq!(
            held_back_count(&ledgers, 103),
            0,
            "nothing held back when the cursor reaches the last decoded ledger"
        );
    }

    /// Regression, code review of PR #313 finding 2: holding back is only safe
    /// while a LATER run can reach the next minute. A run that spent its whole
    /// iteration budget inside one minute would otherwise re-read the same
    /// ledgers forever — silently, with no error and no alarm, because every
    /// doorbell is still consumed successfully.
    #[test]
    fn a_budget_exhausted_inside_one_minute_forces_progress() {
        assert!(
            flushes_all(
                run_end(None, 16, 16, 16, false),
                FlushAllReason::BudgetExhausted
            ),
            "budget spent, no complete minute: must flush and advance or deadlock"
        );
    }

    #[test]
    fn forced_progress_does_not_fire_while_the_run_can_still_grow() {
        assert_eq!(
            run_end(None, 3, 16, 3, false),
            RunEnd::Hold,
            "under budget — the next run fetches more and the minute will close"
        );
        let split = RunEnd::Split {
            cursor: 101,
            flush_before: 61,
        };
        assert_eq!(
            run_end(Some((101, 61)), 16, 16, 16, false),
            split,
            "a complete minute exists, so there is nothing to force"
        );
        assert_eq!(
            run_end(Some((101, 61)), 3, 16, 3, false),
            split,
            "the ordinary healthy run"
        );
    }

    /// `maxIterations: 1` is accepted by the config validator today
    /// (`infra/src/lib/types.ts:936` only enforces >= 1) and would deadlock on
    /// the first invocation without the escape hatch.
    #[test]
    fn the_smallest_legal_iteration_budget_does_not_deadlock() {
        assert!(flushes_all(
            run_end(None, 1, 1, 1, false),
            FlushAllReason::BudgetExhausted
        ));
    }

    /// The ordinary case: a run straddles a minute boundary, so the earlier
    /// minute is complete and the later one is still being filled.
    #[test]
    fn a_run_straddling_a_boundary_ends_on_the_last_complete_minute() {
        let ledgers = vec![
            (100, 60, true),
            (101, 60, true),
            (102, 120, true),
            (103, 120, true),
        ];
        let (open, end) = run_boundary(&ledgers);
        assert_eq!(open, 120, "the newest minute touched is the open one");
        assert_eq!(
            end,
            Some((101, 61)),
            "the cursor stops at the last ledger of minute 60, so 102-103 are re-read"
        );
    }

    /// Task 0282: a run entirely inside one minute must write NOTHING. Writing
    /// it would emit a partial candle that the next run's higher-version write
    /// replaces rather than sums.
    #[test]
    fn a_run_inside_a_single_minute_writes_nothing() {
        let ledgers = vec![(100, 60, true), (101, 60, true), (102, 60, true)];
        let (open, end) = run_boundary(&ledgers);
        assert_eq!(open, 60);
        assert_eq!(end, None, "no complete minute — hold everything back");
    }

    /// The steady-state shape in production: one ledger per doorbell. It holds
    /// back every time until a ledger from the next minute arrives, which is
    /// exactly the behaviour that stops the 15%-retention case.
    #[test]
    fn a_single_ledger_run_holds_back_until_the_minute_turns() {
        assert_eq!(cursor(&[(100, 60, true)]), None);
        assert_eq!(
            cursor(&[(100, 60, true), (101, 120, true)]),
            Some(100),
            "once the minute turns, the completed minute is released"
        );
    }

    /// Ledgers are pushed in fetch order, but the boundary must not depend on
    /// that: it is derived from close times, not from position in the vec.
    #[test]
    fn the_boundary_does_not_depend_on_arrival_order() {
        let forward = run_boundary(&[(100, 60, true), (101, 120, true), (102, 120, true)]);
        let shuffled = run_boundary(&[(102, 120, true), (100, 60, true), (101, 120, true)]);
        assert_eq!(forward, shuffled);
        assert_eq!(forward.1, Some((100, 61)));
    }

    /// A minute with no ledgers at all (an idle stretch) must not strand the
    /// run: minute 60 is still complete even though minute 120 never appears.
    #[test]
    fn a_gap_in_minutes_still_releases_the_older_one() {
        let (open, end) = run_boundary(&[(100, 60, true), (101, 60, true), (102, 300, true)]);
        assert_eq!(open, 300);
        assert_eq!(end, Some((101, 61)));
    }

    #[test]
    fn an_empty_run_has_no_boundary() {
        assert_eq!(run_boundary(&[]), (0, None));
    }
}
