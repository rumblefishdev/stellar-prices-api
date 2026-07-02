//! Backfill ClickHouse sink — a thin wrapper over the shared
//! [`prices_ingest_core::OhlcvWriter`].
//!
//! The candle / asset / oracle writes (and the asset-registry load) are shared
//! with the live Lambda and live in the core writer, so both paths emit
//! byte-identical `prices.*` rows. This wrapper adds only the **backfill-only**
//! resume bookkeeping against `prices.backfill_sdex_ledgers` (the live Lambda
//! uses its own doorbell cursor instead). `OracleSample` is re-exported so the
//! rest of the backfill keeps importing it from `crate::sink`.

use std::collections::HashSet;

use clickhouse::Row;
use prices_ingest_core::canonical::AssetIdentity;
use prices_ingest_core::{
    AssetRegistry, DEFAULT_BACKOFF_MS, OhlcvCandle, OhlcvWriter, UnresolvedPool, retry_with_backoff,
};
use serde::Serialize;
use tracing::info;

pub use prices_ingest_core::OracleSample;

use crate::error::BackfillError;

pub struct Sink {
    writer: OhlcvWriter,
}

impl Sink {
    pub fn new(url: &str) -> Self {
        Self {
            writer: OhlcvWriter::plaintext(url),
        }
    }

    /// Direct-write sink to the Hetzner `prices.*` cluster over mTLS (ADR 0009).
    ///
    /// Unlike the live Lambdas' `client_from_lambda_env` — which fetches the
    /// bundle from the Parameters & Secrets Extension on `localhost:2773` — the
    /// backfill runs on an operator workstation, so it reads the client bundle
    /// from on-disk PEM files and builds the task-0052 client with
    /// [`prices_clickhouse::mtls::client_with_mtls`] (the same workstation entry
    /// point the 0052 round-trip smoke test uses). The key is read straight into
    /// rustls and never logged.
    #[cfg(feature = "aws-mtls")]
    pub fn mtls(
        domain: &str,
        cert_path: &std::path::Path,
        key_path: &std::path::Path,
        ca_path: &std::path::Path,
        database: &str,
    ) -> Result<Self, BackfillError> {
        use prices_clickhouse::mtls::{MtlsBundle, client_with_mtls};

        let read = |p: &std::path::Path| -> Result<String, BackfillError> {
            std::fs::read_to_string(p)
                .map_err(|e| BackfillError::Mtls(format!("read PEM at `{}`: {e}", p.display())))
        };
        let bundle = MtlsBundle {
            cert_pem: read(cert_path)?,
            key_pem: read(key_path)?,
            ca_pem: read(ca_path)?,
        };
        let client = client_with_mtls(domain, &bundle, database)
            .map_err(|e| BackfillError::Mtls(e.to_string()))?;
        Ok(Self {
            writer: OhlcvWriter::new(client),
        })
    }

    pub async fn preflight(&self) -> Result<(), BackfillError> {
        self.writer.preflight().await?;
        Ok(())
    }

    pub async fn load_assets(&self) -> Result<Vec<(u32, AssetIdentity)>, BackfillError> {
        Ok(self.writer.load_assets().await?)
    }

    // All `prices.*` writes below are idempotent (ReplacingMergeTree keyed by
    // `version`), so a retried INSERT can only replace, never duplicate. That
    // lets the sink retry every failure as transient (`|_| true`) — a bounded
    // `[50, 200, 800] ms` backoff so a passing CH/network blip does not abort a
    // multi-hour backfill. Same envelope and classifier as the live sink.

    pub async fn write_candles(
        &self,
        candles: &[OhlcvCandle],
        source: &str,
    ) -> Result<(), BackfillError> {
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async { self.writer.write_candles(candles, source).await },
        )
        .await
        .map(|_| ())
        .map_err(BackfillError::from)
    }

    pub async fn write_assets(&self, registry: &AssetRegistry) -> Result<(), BackfillError> {
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async { self.writer.write_assets(registry).await },
        )
        .await
        .map(|_| ())
        .map_err(BackfillError::from)
    }

    pub async fn write_oracle(&self, samples: &[OracleSample]) -> Result<(), BackfillError> {
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async { self.writer.write_oracle(samples).await },
        )
        .await
        .map(|_| ())
        .map_err(BackfillError::from)
    }

    pub async fn write_unresolved_pools(
        &self,
        pools: &[UnresolvedPool],
    ) -> Result<(), BackfillError> {
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async { self.writer.write_unresolved_pools(pools).await },
        )
        .await
        .map(|_| ())
        .map_err(BackfillError::from)
    }

    // --- backfill-only resume bookkeeping (prices.backfill_sdex_ledgers) ---

    pub async fn load_completed(
        &self,
        start: u32,
        end: u32,
    ) -> Result<HashSet<u32>, BackfillError> {
        let rows = self
            .writer
            .client()
            .query(
                "SELECT sequence FROM prices.backfill_sdex_ledgers \
                 WHERE sequence BETWEEN ? AND ?",
            )
            .bind(start)
            .bind(end)
            .fetch_all::<u32>()
            .await?;

        let set: HashSet<u32> = rows.into_iter().collect();
        info!(
            start,
            end,
            completed = set.len(),
            "loaded completed ledgers from backfill_sdex_ledgers"
        );
        Ok(set)
    }

    pub async fn write_completed_ledgers(&self, sequences: &[u32]) -> Result<(), BackfillError> {
        if sequences.is_empty() {
            return Ok(());
        }
        // Idempotent resume bookkeeping (RMT keyed by sequence) → retry the whole
        // batch INSERT as transient, same bounded backoff as the candle writes.
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async {
                let mut insert = self
                    .writer
                    .client()
                    .insert("prices.backfill_sdex_ledgers")?;
                for &seq in sequences {
                    insert.write(&LedgerRow { sequence: seq }).await?;
                }
                insert.end().await?;
                Ok::<(), clickhouse::error::Error>(())
            },
        )
        .await
        .map(|_| ())
        .map_err(BackfillError::from)
    }
}

#[derive(Debug, Serialize, Row)]
struct LedgerRow {
    sequence: u32,
}
