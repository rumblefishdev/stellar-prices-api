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
//! flushed once at the end, so all ledgers sharing a minute aggregate into one
//! candle (matching the backfill's per-chunk accumulation). The only residual
//! is a minute split across two separate invocations/runs; that is the same
//! `version`-keyed characteristic the backfill has across partition boundaries,
//! and a periodic re-aggregation is tracked as a follow-up.

use std::collections::HashMap;

use prices_ingest_core::{
    AssetRegistry, CandleAccumulator, OracleSample, Registries, decode_object, extract_trades,
    ledger_sequence, process_ledger, raw_trade_to_tick,
};
use tokio::sync::Mutex;
use tracing::info;

use crate::cursor::{Cursor, CursorError};
use crate::galexie_key::ledger_s3_key;
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
    pub rows_emitted: u64,
}

/// Warm per-container processing state: the surrogate-id registry (loaded from
/// `prices.assets` at cold start) and the incrementally-grown AMM venue/pool
/// registries. Persisting these across invocations lets a warm Lambda resolve
/// pools discovered earlier in its lifetime.
pub struct ProcessingState {
    pub assets: AssetRegistry,
    pub registries: Registries,
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
        Self {
            fetcher,
            cursor,
            sink,
            state: Mutex::new(ProcessingState { assets, registries }),
        }
    }

    pub async fn run(&self, max_iterations: usize) -> Result<RunStats, ReconcileError> {
        let mut st = self.state.lock().await;
        // Deref the guard once so `registries` and `assets` can be borrowed as
        // disjoint fields (a borrow through the guard's DerefMut each time would
        // conflict).
        let state = &mut *st;

        let start = self.cursor.read().await?;
        let mut current = start;
        let mut persisted = 0u64;

        // Accumulate across the whole contiguous run, flush once at the end.
        let mut sdex = CandleAccumulator::new();
        let mut amm: HashMap<&'static str, CandleAccumulator> = HashMap::new();
        let mut oracle: Vec<OracleSample> = Vec::new();

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
                    amm.entry(source)
                        .or_insert_with(CandleAccumulator::new)
                        .merge(tick);
                }
                oracle.extend(sob.oracle);
                obj_max = obj_max.max(ledger_sequence(lcm) as u64);
            }

            current = obj_max.max(next);
            persisted += 1;
        }

        if persisted == 0 {
            return Ok(RunStats {
                start_cursor: start,
                end_cursor: start,
                ledgers_persisted: 0,
                rows_emitted: 0,
            });
        }

        // Flush + write, then advance the cursor LAST (ordering barrier).
        let mut rows_emitted = 0u64;

        let sdex_candles = sdex.flush_all();
        rows_emitted += sdex_candles.len() as u64;
        self.sink.write_candles(&sdex_candles, "sdex").await?;

        for (source, mut acc) in amm {
            let candles = acc.flush_all();
            rows_emitted += candles.len() as u64;
            self.sink.write_candles(&candles, source).await?;
        }

        self.sink.write_oracle(&oracle).await?;
        self.sink.write_assets(&state.assets).await?;
        self.cursor.write(current).await?;

        info!(
            start,
            end = current,
            persisted,
            rows = rows_emitted,
            "reconcile run complete"
        );

        Ok(RunStats {
            start_cursor: start,
            end_cursor: current,
            ledgers_persisted: persisted,
            rows_emitted,
        })
    }
}
