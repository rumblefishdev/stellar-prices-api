use std::collections::HashSet;

use clickhouse::Client;
use serde::Serialize;
use tracing::info;

use crate::bucket::OhlcvCandle;
use crate::canonical::{AssetIdentity, AssetRegistry};
use crate::error::BackfillError;

pub struct Sink {
    client: Client,
}

impl Sink {
    pub fn new(url: &str) -> Self {
        let client = Client::default().with_url(url);
        Self { client }
    }

    pub async fn preflight(&self) -> Result<(), BackfillError> {
        self.client
            .query("SELECT 1")
            .execute()
            .await?;
        Ok(())
    }

    pub async fn load_completed(&self, start: u32, end: u32) -> Result<HashSet<u32>, BackfillError> {
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
            start, end,
            completed = set.len(),
            "loaded completed ledgers from backfill_sdex_ledgers"
        );
        Ok(set)
    }

    pub async fn write_candles(&self, candles: &[OhlcvCandle]) -> Result<(), BackfillError> {
        if candles.is_empty() {
            return Ok(());
        }

        let mut insert = self
            .client
            .insert("prices.price_ohlcv_1m")?;

        for candle in candles {
            insert
                .write(&OhlcvRow {
                    timestamp: candle.minute_start,
                    asset_id: candle.asset_id,
                    quote_asset_id: candle.quote_asset_id,
                    source: "sdex".to_string(),
                    open: candle.open.to_string(),
                    high: candle.high.to_string(),
                    low: candle.low.to_string(),
                    close: candle.close.to_string(),
                    volume_base: candle.volume_base.to_string(),
                    volume_quote_usd: candle.volume_quote.to_string(),
                    vwap: candle.vwap.to_string(),
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
            let (asset_code, asset_type, issuer_address) = match identity {
                AssetIdentity::Native => ("XLM".to_string(), "classic", String::new()),
                AssetIdentity::Credit { code, issuer } => {
                    (code.clone(), "classic", issuer.clone())
                }
            };

            insert
                .write(&AssetRow {
                    asset_id: id,
                    asset_code,
                    asset_type: asset_type.to_string(),
                    issuer_address,
                    contract_address: String::new(),
                    home_domain: String::new(),
                    is_active: 1,
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
    open: String,
    high: String,
    low: String,
    close: String,
    volume_base: String,
    volume_quote_usd: String,
    vwap: String,
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
