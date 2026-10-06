//! OHLCV sink — the seam that turns bucketed candles into ClickHouse rows.
//!
//! The real sink ([`ClickHouseSink`]) wraps the shared
//! [`prices_ingest_core::OhlcvWriter`], so it writes the exact same
//! `prices.price_ohlcv_1m` rows as the SDEX backfill. It is transport-agnostic:
//! [`ClickHouseSink::plaintext`] talks to a local Docker ClickHouse, and
//! (with the `aws-mtls` feature) [`ClickHouseSink::from_lambda_env`] talks to the
//! shared Hetzner cluster over mTLS via the task-0052 client. Tests use the
//! in-memory [`CountingSink`].

use std::collections::HashSet;
use std::future::Future;

use prices_ingest_core::{
    AssetRegistry, DEFAULT_BACKOFF_MS, OhlcvCandle, OhlcvWriter, OracleSample, PoolRegistryRow,
    Registries, retry_with_backoff,
};

/// `registry` with `sac_contracts` as its `is_sac` candidates (task 0242 D2):
/// the step [`ClickHouseSink::load_ingest_registry`] adds to a plain load.
pub fn with_sac_candidates(
    mut registry: AssetRegistry,
    sac_contracts: HashSet<String>,
) -> AssetRegistry {
    tracing::info!(
        sac_contracts = sac_contracts.len(),
        "loaded the is_sac contract set"
    );
    registry.set_sac_candidates(sac_contracts);
    registry
}

/// [`with_sac_candidates`] when BE's `is_sac` set was read, else the registry
/// as loaded, with no candidates; the flag says which (task 0242 PC3,
/// reversed in review). An unreadable BE table must not stop live ingest: with
/// no candidates an unproven SAC mints a `Contract` identity, which the probe's
/// `SacContractIdentities` counts and the 0242 heal removes, while SDEX, oracle
/// and AMM candles keep flowing. The caller publishes
/// `SacCandidatesUnavailable` while the flag is false.
pub fn arm_sac_candidates(
    registry: AssetRegistry,
    sac_contracts: Result<HashSet<String>, SinkError>,
) -> (AssetRegistry, bool) {
    match sac_contracts {
        Ok(set) => (with_sac_candidates(registry, set), true),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "is_sac contract set unreadable: no SAC candidates until the next cold start, \
                 an unproven SAC mints a Contract identity (task 0242)"
            );
            (registry, false)
        }
    }
}

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

    /// Persist the registry's pending identities ([`AssetRegistry::pending_new`])
    /// — only the ones not yet written, not the whole registry (task 0132). A
    /// no-op write when nothing is pending. Does not clear the set: the caller
    /// does, after `Ok`.
    fn write_new_assets(
        &self,
        registry: &AssetRegistry,
    ) -> impl Future<Output = Result<(), SinkError>> + Send;

    /// Persist AMM pools this run learned from factory events — only the rows
    /// not yet durably in `prices.pool_registry` (task 0291; see
    /// [`Registries::pool_rows_unpersisted`]). A no-op write on an empty slice.
    fn write_pool_rows(
        &self,
        rows: &[PoolRegistryRow],
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

    /// Load the identities already in `prices.assets`, so a cold start writes
    /// only the assets it discovers. Ids are ClickHouse's, derived from the
    /// identity (task 0139), so live and backfill ids agree by construction.
    pub async fn load_registry(&self) -> Result<AssetRegistry, SinkError> {
        let existing = self.writer.load_assets().await.map_err(redact)?;
        Ok(AssetRegistry::from_existing(existing))
    }

    /// The contracts BE flags `is_sac` (task 0242), for
    /// [`AssetRegistry::set_sac_candidates`]. Separate from [`load_registry`]:
    /// the local CLI has no BE tables, so `prices-cli` never calls it and still
    /// mints an unproven SAC as a `Contract` identity. Local fixtures only.
    ///
    /// [`load_registry`]: ClickHouseSink::load_registry
    pub async fn load_sac_contracts(&self) -> Result<HashSet<String>, SinkError> {
        prices_ingest_core::load_sac_contracts(self.client(), prices_ingest_core::BE_DATABASE)
            .await
            .map_err(redact)
    }

    /// The registry the live processor ingests with: [`load_registry`] armed
    /// with BE's `is_sac` set by [`arm_sac_candidates`], the two reads joined,
    /// and whether the set was read. Only the `prices.assets` read fails the
    /// cold start; an unreadable BE set is a warning and a metric (task 0242).
    ///
    /// [`load_registry`]: ClickHouseSink::load_registry
    pub async fn load_ingest_registry(&self) -> Result<(AssetRegistry, bool), SinkError> {
        let (registry, sac_contracts) =
            tokio::join!(self.load_registry(), self.load_sac_contracts());
        Ok(arm_sac_candidates(registry?, sac_contracts))
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

    async fn write_new_assets(&self, registry: &AssetRegistry) -> Result<(), SinkError> {
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async { self.writer.write_new_assets(registry).await.map_err(redact) },
        )
        .await
        .map(|_| ())
    }

    async fn write_pool_rows(&self, rows: &[PoolRegistryRow]) -> Result<(), SinkError> {
        // Idempotent (RMT on contract_id) → retried like the other writes.
        retry_with_backoff(
            &DEFAULT_BACKOFF_MS,
            |_| true,
            || async { self.writer.write_pool_rows(rows).await.map_err(redact) },
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
    pub pools: std::sync::atomic::AtomicU64,
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

    async fn write_new_assets(&self, registry: &AssetRegistry) -> Result<(), SinkError> {
        let n = registry.pending_new().count() as u64;
        self.assets
            .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn write_pool_rows(&self, rows: &[PoolRegistryRow]) -> Result<(), SinkError> {
        self.pools
            .fetch_add(rows.len() as u64, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
}
