//! OHLCV sink — the seam that turns bucketed candles into ClickHouse rows.
//!
//! The real sink ([`ClickHouseSink`]) wraps the shared
//! [`prices_ingest_core::OhlcvWriter`], so it writes the exact same
//! `prices.price_ohlcv_1m` rows as the SDEX backfill. It is transport-agnostic:
//! [`ClickHouseSink::plaintext`] talks to a local Docker ClickHouse, and
//! (with the `aws-mtls` feature) [`ClickHouseSink::from_lambda_env`] talks to the
//! shared Hetzner cluster over mTLS via the task-0052 client. Tests use the
//! in-memory [`CountingSink`].

use std::future::Future;

use prices_ingest_core::{
    AssetRegistry, DEFAULT_BACKOFF_MS, OhlcvCandle, OhlcvWriter, OracleSample, Registries,
    retry_with_backoff,
};

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("sink write failed: {0}")]
    Write(String),
}

/// Writes the three shared `prices.*` outputs of a reconcile run. Candle writes
/// are idempotent (ReplacingMergeTree keyed by `version`), so the sink may be
/// retried freely.
pub trait CandleSink {
    fn write_candles(
        &self,
        candles: &[OhlcvCandle],
        source: &str,
    ) -> impl Future<Output = Result<(), SinkError>> + Send;

    fn write_oracle(
        &self,
        samples: &[OracleSample],
    ) -> impl Future<Output = Result<(), SinkError>> + Send;

    /// Persist assets interned on/after the `since` watermark — only the ones
    /// this run newly discovered, not the whole registry (task 0132). `since` is
    /// [`AssetRegistry::watermark`] captured before the run. A no-op write when
    /// no new assets were seen.
    fn write_new_assets(
        &self,
        registry: &AssetRegistry,
        since: u32,
    ) -> impl Future<Output = Result<(), SinkError>> + Send;
}

/// ClickHouse sink backed by the shared [`OhlcvWriter`]. Works against either a
/// plaintext local client or the mTLS remote client — both are a
/// `clickhouse::Client`.
pub struct ClickHouseSink {
    writer: OhlcvWriter,
}

impl ClickHouseSink {
    /// Local / Docker ClickHouse over plain HTTP (no TLS). Used by the CLI
    /// fixture runner and the local integration test.
    pub fn plaintext(url: &str) -> Self {
        Self {
            writer: OhlcvWriter::plaintext(url),
        }
    }

    /// Remote Hetzner ClickHouse over mTLS, built from the Lambda's
    /// `MTLS_SECRET_NAME` / `CH_DOMAIN` env vars via the task-0052 client.
    #[cfg(feature = "aws-mtls")]
    pub async fn from_lambda_env() -> Result<Self, SinkError> {
        let client =
            prices_clickhouse::mtls::client_from_lambda_env(prices_clickhouse::PROD_DATABASE)
                .await
                .map_err(|e| SinkError::Write(format!("mtls client init: {e}")))?;
        Ok(Self {
            writer: OhlcvWriter::new(client),
        })
    }

    /// Probe connectivity (`SELECT 1`). Call once at cold start so an
    /// unreachable cluster surfaces as a Lambda Init error, not per-event.
    pub async fn preflight(&self) -> Result<(), SinkError> {
        self.writer.preflight().await.map_err(redact)
    }

    /// The underlying ClickHouse client, so a [`crate::cursor::ClickHouseCursor`]
    /// can share this sink's connection (mTLS or plaintext) instead of opening a
    /// second one. Clients are cheap to clone — the hyper pool is shared.
    pub fn client(&self) -> &clickhouse::Client {
        self.writer.client()
    }

    /// Load the existing asset registry from `prices.assets` so surrogate ids
    /// are reused (not reassigned) across cold starts — the load-bearing
    /// guarantee that live ids match the backfill's.
    pub async fn load_registry(&self) -> Result<AssetRegistry, SinkError> {
        let existing = self.writer.load_assets().await.map_err(redact)?;
        Ok(AssetRegistry::from_existing(existing))
    }

    /// Preload the discovered AMM pool registry from `prices.pool_registry` so the
    /// live processor resolves pools created before the cursor start (task 0078)
    /// — otherwise it only knows pools whose factory events appear in its own live
    /// stream, and every pre-existing pool's swaps land in `unresolved_pools`.
    /// Empty when the registry has not been seeded (SDEX-only operation is fine).
    pub async fn load_pool_registry(&self) -> Result<Registries, SinkError> {
        self.writer.load_pool_registry().await.map_err(redact)
    }
}

impl CandleSink for ClickHouseSink {
    async fn write_candles(&self, candles: &[OhlcvCandle], source: &str) -> Result<(), SinkError> {
        // Idempotent (RMT by version) → retry every failure as transient.
        // Finer permanent-vs-transient classification is a follow-up.
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async {
                self.writer
                    .write_candles(candles, source)
                    .await
                    .map_err(redact)
            },
        )
        .await
        .map(|_| ())
    }

    async fn write_oracle(&self, samples: &[OracleSample]) -> Result<(), SinkError> {
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async { self.writer.write_oracle(samples).await.map_err(redact) },
        )
        .await
        .map(|_| ())
    }

    async fn write_new_assets(
        &self,
        registry: &AssetRegistry,
        since: u32,
    ) -> Result<(), SinkError> {
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async {
                self.writer
                    .write_new_assets(registry, since)
                    .await
                    .map_err(redact)
            },
        )
        .await
        .map(|_| ())
    }
}

/// Map an ingest error into a sink error. `IngestError`'s `Display` is already
/// leak-safe — its ClickHouse variant redacts the `BadResponse` body down to the
/// leading `Code: NNN` / status token (see
/// [`prices_ingest_core::safe_response_token`]) — so this is a plain string map.
fn redact(e: prices_ingest_core::IngestError) -> SinkError {
    SinkError::Write(e.to_string())
}

/// In-memory sink for tests and `--dry-run`: counts rows, touches no network.
#[derive(Default)]
pub struct CountingSink {
    pub candles: std::sync::atomic::AtomicU64,
    pub oracle: std::sync::atomic::AtomicU64,
    pub assets: std::sync::atomic::AtomicU64,
}

impl CandleSink for CountingSink {
    async fn write_candles(&self, candles: &[OhlcvCandle], _source: &str) -> Result<(), SinkError> {
        self.candles
            .fetch_add(candles.len() as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn write_oracle(&self, samples: &[OracleSample]) -> Result<(), SinkError> {
        self.oracle
            .fetch_add(samples.len() as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn write_new_assets(
        &self,
        registry: &AssetRegistry,
        since: u32,
    ) -> Result<(), SinkError> {
        let n = registry.assets_since(since).count() as u64;
        self.assets
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}
