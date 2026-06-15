use std::collections::HashSet;

use clickhouse::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::bucket::OhlcvCandle;
use crate::canonical::{AssetIdentity, AssetRegistry};
use crate::error::BackfillError;

fn decimal_to_i128(d: Decimal) -> i128 {
    let d = d.round_dp(14);
    // `Decimal(38,14)` holds at most 38 significant digits. AMM amounts/prices
    // are i128-derived and can be far larger than SDEX stroops, so a naive
    // `mantissa * 10^(14-scale)` can overflow i128 and panic. Saturate instead:
    // an out-of-range value is clamped to the representable bound rather than
    // aborting the whole backfill.
    let factor = 10i128.pow(14 - d.scale());
    d.mantissa().saturating_mul(factor)
}

pub struct Sink {
    client: Client,
}

impl Sink {
    pub fn new(url: &str) -> Self {
        let client = Client::default().with_url(url);
        Self { client }
    }

    pub async fn preflight(&self) -> Result<(), BackfillError> {
        self.client.query("SELECT 1").execute().await?;
        Ok(())
    }

    pub async fn load_completed(
        &self,
        start: u32,
        end: u32,
    ) -> Result<HashSet<u32>, BackfillError> {
        let rows = self
            .client
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

    pub async fn load_assets(&self) -> Result<Vec<(u32, AssetIdentity)>, BackfillError> {
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

    pub async fn write_candles(
        &self,
        candles: &[OhlcvCandle],
        source: &str,
    ) -> Result<(), BackfillError> {
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

    pub async fn write_assets(&self, registry: &AssetRegistry) -> Result<(), BackfillError> {
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

            insert
                .write(&AssetRow {
                    asset_id: id,
                    asset_code,
                    asset_type: asset_type.to_string(),
                    issuer_address,
                    contract_address,
                    home_domain: String::new(),
                    is_active: 1,
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }

    pub async fn write_oracle(&self, samples: &[OracleSample]) -> Result<(), BackfillError> {
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

    pub async fn write_completed_ledgers(&self, sequences: &[u32]) -> Result<(), BackfillError> {
        if sequences.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert("prices.backfill_sdex_ledgers")?;
        for &seq in sequences {
            insert.write(&LedgerRow { sequence: seq }).await?;
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
    home_domain: String,
    is_active: u8,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct LedgerRow {
    sequence: u32,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct ExistingAssetRow {
    asset_id: u32,
    asset_code: String,
    issuer_address: String,
    contract_address: String,
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
