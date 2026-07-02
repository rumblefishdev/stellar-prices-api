//! Transport-agnostic ClickHouse writer for `prices.*`.
//!
//! Holds a `clickhouse::Client` and knows how to write the shared row shapes
//! (`price_ohlcv_1m`, `assets`, `oracle_prices`) and load the asset registry.
//! It does **not** care how the client was built: a plaintext local-dev client
//! ([`OhlcvWriter::plaintext`]) and the task-0052 mTLS client (passed to
//! [`OhlcvWriter::new`]) are both just a `clickhouse::Client`, so the same
//! writer serves the local backfill and the live Lambda's remote mTLS sink.
//!
//! Backfill-only bookkeeping (`backfill_sdex_ledgers` resume set) is **not**
//! here — it lives in `sdex-backfill`'s thin wrapper, since the live Lambda
//! uses its own doorbell cursor instead.

use clickhouse::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::bucket::OhlcvCandle;
use crate::canonical::{AssetIdentity, AssetRegistry};
use crate::error::IngestError;

/// Convert a `Decimal` to the `i128` mantissa ClickHouse expects for a
/// `Decimal(38, 14)` column. Saturates rather than panicking: AMM
/// amounts/prices are i128-derived and can exceed the 38-digit budget, and an
/// out-of-range value should clamp, not abort the whole run.
pub fn decimal_to_i128(d: Decimal) -> i128 {
    let d = d.round_dp(14);
    let factor = 10i128.pow(14 - d.scale());
    d.mantissa().saturating_mul(factor)
}

/// A ClickHouse writer over `prices.*`. Cheap to clone (the client is).
pub struct OhlcvWriter {
    client: Client,
}

impl OhlcvWriter {
    /// Wrap an already-built client (e.g. the mTLS client from
    /// `prices_clickhouse::mtls::client_from_lambda_env`).
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Build a plaintext client for local-dev / Docker ClickHouse.
    pub fn plaintext(url: &str) -> Self {
        Self {
            client: Client::default().with_url(url),
        }
    }

    /// Borrow the underlying client (e.g. for backfill-only resume queries).
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Cheap connectivity probe (`SELECT 1`).
    pub async fn preflight(&self) -> Result<(), IngestError> {
        self.client.query("SELECT 1").execute().await?;
        Ok(())
    }

    /// Load the existing `prices.assets` rows as `(asset_id, identity)` so a
    /// run reuses surrogate ids rather than reassigning them.
    pub async fn load_assets(&self) -> Result<Vec<(u32, AssetIdentity)>, IngestError> {
        let rows = self
            .client
            .query(
                "SELECT asset_id, asset_code, issuer_address, contract_address FROM prices.assets",
            )
            .fetch_all::<ExistingAssetRow>()
            .await?;

        let assets: Vec<(u32, AssetIdentity)> = rows
            .into_iter()
            .map(|r| {
                let identity = if !r.contract_address.is_empty() {
                    AssetIdentity::Contract(r.contract_address)
                } else if r.asset_code == "XLM" && r.issuer_address.is_empty() {
                    AssetIdentity::Native
                } else {
                    AssetIdentity::Credit {
                        code: r.asset_code,
                        issuer: r.issuer_address,
                    }
                };
                (r.asset_id, identity)
            })
            .collect();

        info!(
            existing_assets = assets.len(),
            "loaded asset registry from ClickHouse"
        );
        Ok(assets)
    }

    /// Write a batch of candles for one `source` into `prices.price_ohlcv_1m`.
    pub async fn write_candles(
        &self,
        candles: &[OhlcvCandle],
        source: &str,
    ) -> Result<(), IngestError> {
        if candles.is_empty() {
            return Ok(());
        }

        let mut insert = self.client.insert("prices.price_ohlcv_1m")?;

        for candle in candles {
            insert
                .write(&OhlcvRow {
                    timestamp: candle.minute_start,
                    asset_id: candle.asset_id,
                    quote_asset_id: candle.quote_asset_id,
                    source: source.to_string(),
                    open: decimal_to_i128(candle.open),
                    high: decimal_to_i128(candle.high),
                    low: decimal_to_i128(candle.low),
                    close: decimal_to_i128(candle.close),
                    volume_base: decimal_to_i128(candle.volume_base),
                    volume_quote: decimal_to_i128(candle.volume_quote),
                    // DEFAULT 0 — the 0026 enrichment Lambda fills this
                    // (volume_quote_usd = oracle_price * volume_quote).
                    volume_quote_usd: 0,
                    // DEFAULT 0 — the enrichment pass fills this (task 0061,
                    // close_usd = oracle_price * close), same as volume_quote_usd.
                    close_usd: 0,
                    vwap: decimal_to_i128(candle.vwap),
                    trade_count: candle.trade_count,
                    version: candle.version,
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }

    /// Write the asset registry into `prices.assets` (idempotent via
    /// ReplacingMergeTree on the asset sort key).
    pub async fn write_assets(&self, registry: &AssetRegistry) -> Result<(), IngestError> {
        let mut insert = self.client.insert("prices.assets")?;

        for (identity, &id) in registry.assets() {
            let (asset_code, asset_type, issuer_address, contract_address) = match identity {
                AssetIdentity::Native => {
                    ("XLM".to_string(), "classic", String::new(), String::new())
                }
                AssetIdentity::Credit { code, issuer } => {
                    (code.clone(), "classic", issuer.clone(), String::new())
                }
                AssetIdentity::Contract(addr) => {
                    (String::new(), "soroban", String::new(), addr.clone())
                }
            };
            // The SAC that wraps this classic asset (§12.4) — '' for a pure
            // Soroban token. Lets a read-time consumer resolve a SAC-wrapped leg.
            let sac_address = registry.sac_address_of(identity).unwrap_or_default();

            insert
                .write(&AssetRow {
                    asset_id: id,
                    asset_code,
                    asset_type: asset_type.to_string(),
                    issuer_address,
                    contract_address,
                    sac_address,
                    home_domain: String::new(),
                    is_active: 1,
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }

    /// Write aggregated unresolved-pool observations into
    /// `prices.unresolved_pools` (see [`UnresolvedPool`]). Idempotent via
    /// `ReplacingMergeTree(version)` on `(contract_id, source)`.
    pub async fn write_unresolved_pools(
        &self,
        pools: &[UnresolvedPool],
    ) -> Result<(), IngestError> {
        if pools.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert("prices.unresolved_pools")?;
        for p in pools {
            insert
                .write(&UnresolvedPoolRow {
                    contract_id: p.contract_id.clone(),
                    source: p.source.clone(),
                    first_ledger: p.first_ledger,
                    last_ledger: p.last_ledger,
                    swap_count: p.swap_count,
                    sample_topics: p.sample_topics.clone(),
                    still_unresolved: p.still_unresolved,
                    // Latest observation wins the RMT collapse.
                    version: p.last_ledger as u64,
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }

    /// Write decoded oracle price samples into `prices.oracle_prices`.
    pub async fn write_oracle(&self, samples: &[OracleSample]) -> Result<(), IngestError> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert("prices.oracle_prices")?;
        for s in samples {
            insert
                .write(&OracleRow {
                    timestamp: s.timestamp,
                    asset_id: s.asset_id,
                    oracle_name: s.oracle_name.clone(),
                    price_usd: s.price_usd,
                    raw_data: s.raw_data.clone(),
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct OhlcvRow {
    timestamp: u32,
    asset_id: u32,
    quote_asset_id: u32,
    source: String,
    open: i128,
    high: i128,
    low: i128,
    close: i128,
    volume_base: i128,
    volume_quote: i128,
    volume_quote_usd: i128,
    close_usd: i128,
    vwap: i128,
    trade_count: u32,
    version: u64,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct AssetRow {
    asset_id: u32,
    asset_code: String,
    asset_type: String,
    issuer_address: String,
    contract_address: String,
    sac_address: String,
    home_domain: String,
    is_active: u8,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct ExistingAssetRow {
    asset_id: u32,
    asset_code: String,
    issuer_address: String,
    contract_address: String,
}

/// One aggregated unresolved-pool observation, ready for
/// `prices.unresolved_pools`. A caller aggregates the per-ledger
/// [`crate::soroban::UnresolvedPoolSwap`] records by contract and re-checks each
/// against the final registry to set `still_unresolved`.
#[derive(Debug, Clone)]
pub struct UnresolvedPool {
    pub contract_id: String,
    /// Which pipeline observed it: `"backfill"` or `"live"`.
    pub source: String,
    pub first_ledger: u32,
    pub last_ledger: u32,
    pub swap_count: u64,
    pub sample_topics: String,
    /// `1` = still absent from the registry at run-end (a genuine extractor gap
    /// to investigate); `0` = the pool registered later in the run, so only its
    /// early swaps were dropped (recoverable, informational).
    pub still_unresolved: u8,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct UnresolvedPoolRow {
    contract_id: String,
    source: String,
    first_ledger: u32,
    last_ledger: u32,
    swap_count: u64,
    sample_topics: String,
    still_unresolved: u8,
    version: u64,
}

/// One decoded oracle price sample, ready for `prices.oracle_prices`.
#[derive(Debug, Clone)]
pub struct OracleSample {
    pub timestamp: u32,
    pub asset_id: u32,
    pub oracle_name: String,
    /// price_usd scaled to 14 decimals (matches Decimal(38,14)).
    pub price_usd: i128,
    pub raw_data: String,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct OracleRow {
    timestamp: u32,
    asset_id: u32,
    oracle_name: String,
    price_usd: i128,
    raw_data: String,
}
