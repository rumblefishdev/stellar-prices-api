//! Doorbell-cursor reconcile loop.
//!
//! Mirrors BE's indexer (`crates/indexer/src/handler/mod.rs:201`):
//! read cursor, derive next S3 key, fetch, decode, dispatch, bucket,
//! sink, advance cursor last. Stops at the first gap or
//! `max_iterations`. The cursor write is the **ordering barrier** —
//! a crash before it leaves the cursor unchanged and the next
//! invocation re-processes the same ledger (idempotent via the
//! ReplacingMergeTree / merge semantics in production; via the
//! pure-function bucketer in the prototype).

use std::future::Future;

use extractors_core::{SorobanEventRow, VenueRegistry};
use ledger_processor::dispatch::{DispatchError, dispatch};
use phoenix_extractor::PhoenixPoolRegistry;
use soroswap_extractor::SoroswapPoolRegistry;
use tracing::{info, warn};

use crate::bucket::Bucketer;
use crate::cursor::{Cursor, CursorError};
use crate::galexie_key::ledger_s3_key;
use crate::object_fetcher::{FetchError, ObjectFetcher};
use crate::sink::{OhlcvSink, SinkError};

#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    #[error("cursor error: {0}")]
    Cursor(#[from] CursorError),
    #[error("fetch error: {0}")]
    Fetch(#[from] FetchError),
    #[error("decode error: {0}")]
    Decode(String),
    #[error("dispatch error: {0}")]
    Dispatch(String),
    #[error("sink error: {0}")]
    Sink(#[from] SinkError),
}

#[derive(Debug, Clone)]
pub struct DecodedLedger {
    pub ledger_sequence: u64,
    pub closed_at_unix_seconds: i64,
    /// Soroban events grouped by `(transaction_id, contract_id)` — the
    /// shape the kernel from task 0037 dispatches on.
    pub event_groups: Vec<Vec<SorobanEventRow>>,
}

pub trait LedgerDecoder {
    fn decode(
        &self,
        bytes: &[u8],
    ) -> impl Future<Output = Result<Vec<DecodedLedger>, String>> + Send;
}

#[derive(Debug, Clone, Default)]
pub struct RunStats {
    pub start_cursor: u64,
    pub end_cursor: u64,
    pub ledgers_persisted: u64,
    pub rows_emitted: u64,
}

pub struct Reconciler<F, C, S, D> {
    pub fetcher: F,
    pub cursor: C,
    pub sink: S,
    pub decoder: D,
    pub venue_registry: VenueRegistry,
    pub phoenix_registry: PhoenixPoolRegistry,
    pub soroswap_registry: SoroswapPoolRegistry,
}

impl<F, C, S, D> Reconciler<F, C, S, D>
where
    F: ObjectFetcher + Sync,
    C: Cursor + Sync,
    S: OhlcvSink + Sync,
    D: LedgerDecoder + Sync,
{
    pub async fn run(&self, max_iterations: usize) -> Result<RunStats, ReconcileError> {
        let start = self.cursor.read().await?;
        let mut current = start;
        let mut persisted = 0u64;
        let mut rows_emitted = 0u64;

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

            let ledgers = self
                .decoder
                .decode(&bytes)
                .await
                .map_err(ReconcileError::Decode)?;

            let mut bucketer = Bucketer::new();
            let mut max_seq = current;
            for ledger in ledgers {
                for group in &ledger.event_groups {
                    let trades = match dispatch(
                        group,
                        &self.venue_registry,
                        &self.phoenix_registry,
                        &self.soroswap_registry,
                    ) {
                        Ok(t) => t,
                        Err(DispatchError::VenueNotImplemented { venue, contract_id }) => {
                            warn!(?venue, %contract_id, "venue extractor not yet implemented — skipping");
                            Vec::new()
                        }
                        Err(e) => return Err(ReconcileError::Dispatch(e.to_string())),
                    };
                    for trade in &trades {
                        bucketer.ingest(ledger.closed_at_unix_seconds, trade);
                    }
                }
                if ledger.ledger_sequence > max_seq {
                    max_seq = ledger.ledger_sequence;
                }
            }

            let rows = bucketer.drain();
            rows_emitted += rows.len() as u64;
            self.sink.write(&rows).await?;
            self.cursor.write(max_seq).await?;
            info!(ledger = max_seq, rows = rows.len(), "ledger persisted");
            current = max_seq;
            persisted += 1;
        }

        Ok(RunStats {
            start_cursor: start,
            end_cursor: current,
            ledgers_persisted: persisted,
            rows_emitted,
        })
    }
}
