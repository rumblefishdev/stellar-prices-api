//! Core enrichment algorithm.
//!
//! Given a `Candidate` row (a `price_ohlcv` row with
//! `volume_quote_usd = 0`) and an `OraclePriceLookup`, produce an
//! `EnrichedRow` whose `volume_quote_usd = oracle.price_usd *
//! candidate.volume_quote`, OR an `Outcome::OracleMiss` to be
//! retried on the next pass.
//!
//! Translation choice from the G-note Part A.2: the production form
//! INSERTs the EnrichedRow into a `ReplacingMergeTree(_inserted_at)`,
//! so two passes that pick the same candidate produce two equivalent
//! enriched rows that collapse to one on merge. The prototype emits
//! the same row through a stub sink.

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::oracle::{OracleError, OraclePriceLookup};

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Candidate {
    pub timestamp: i64,
    pub asset_id: String,
    pub granularity: String,
    pub quote_asset_id: String,
    pub source: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub volume_base: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub volume_quote: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub open: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub high: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub low: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub close: Decimal,
    pub trade_count: u64,
    #[serde(with = "rust_decimal::serde::str")]
    pub vwap_numerator: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub vwap_denominator: Decimal,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EnrichedRow {
    pub timestamp: i64,
    pub asset_id: String,
    pub granularity: String,
    pub quote_asset_id: String,
    pub source: String,
    #[serde(with = "rust_decimal::serde::str")]
    pub volume_base: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub volume_quote: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub volume_quote_usd: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub open: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub high: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub low: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub close: Decimal,
    pub trade_count: u64,
    #[serde(with = "rust_decimal::serde::str")]
    pub vwap_numerator: Decimal,
    #[serde(with = "rust_decimal::serde::str")]
    pub vwap_denominator: Decimal,
    /// `_inserted_at` for the production ReplacingMergeTree version.
    pub inserted_at_unix_seconds: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Enriched(Box<EnrichedRow>),
    OracleMiss {
        quote_asset_id: String,
        oracle_name: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum EnrichError {
    #[error("oracle lookup error: {0}")]
    Oracle(#[from] OracleError),
}

pub async fn enrich_one<O: OraclePriceLookup + Sync>(
    candidate: &Candidate,
    oracle: &O,
    oracle_name: &str,
    window_s: u32,
    now_unix_seconds: i64,
) -> Result<Outcome, EnrichError> {
    let maybe_price = oracle
        .lookup(&candidate.quote_asset_id, candidate.timestamp, window_s)
        .await?;
    let Some(price) = maybe_price else {
        return Ok(Outcome::OracleMiss {
            quote_asset_id: candidate.quote_asset_id.clone(),
            oracle_name: oracle_name.to_string(),
        });
    };
    let volume_quote_usd = price.price_usd * candidate.volume_quote;
    Ok(Outcome::Enriched(Box::new(EnrichedRow {
        timestamp: candidate.timestamp,
        asset_id: candidate.asset_id.clone(),
        granularity: candidate.granularity.clone(),
        quote_asset_id: candidate.quote_asset_id.clone(),
        source: candidate.source.clone(),
        volume_base: candidate.volume_base,
        volume_quote: candidate.volume_quote,
        volume_quote_usd,
        open: candidate.open,
        high: candidate.high,
        low: candidate.low,
        close: candidate.close,
        trade_count: candidate.trade_count,
        vwap_numerator: candidate.vwap_numerator,
        vwap_denominator: candidate.vwap_denominator,
        inserted_at_unix_seconds: now_unix_seconds,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oracle::{InMemoryOracleLookup, OracleEntry};
    use std::str::FromStr;

    fn candidate(quote: &str, vq: &str, ts: i64) -> Candidate {
        Candidate {
            timestamp: ts,
            asset_id: "CXLM".into(),
            granularity: "1m".into(),
            quote_asset_id: quote.into(),
            source: "phoenix".into(),
            volume_base: Decimal::from_str("1000").unwrap(),
            volume_quote: Decimal::from_str(vq).unwrap(),
            open: Decimal::from_str("1.25").unwrap(),
            high: Decimal::from_str("1.26").unwrap(),
            low: Decimal::from_str("1.24").unwrap(),
            close: Decimal::from_str("1.255").unwrap(),
            trade_count: 1,
            vwap_numerator: Decimal::from_str("1250").unwrap(),
            vwap_denominator: Decimal::from_str("1000").unwrap(),
        }
    }

    fn oracle_entry(asset: &str, ts: i64, price: &str) -> OracleEntry {
        OracleEntry {
            asset_id: asset.into(),
            oracle_name: "reflector".into(),
            timestamp: ts,
            price_usd: Decimal::from_str(price).unwrap(),
        }
    }

    #[tokio::test]
    async fn hit_produces_enriched_row() {
        let oracle =
            InMemoryOracleLookup::from_entries(vec![oracle_entry("CUSDC", 1_700_000_000, "1.0")]);
        let cand = candidate("CUSDC", "500", 1_700_000_000);
        let out = enrich_one(&cand, &oracle, "reflector", 300, 1_700_000_100)
            .await
            .unwrap();
        match out {
            Outcome::Enriched(row) => {
                assert_eq!(row.volume_quote_usd, Decimal::from_str("500").unwrap());
                assert_eq!(row.inserted_at_unix_seconds, 1_700_000_100);
            }
            other => panic!("expected Enriched, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn miss_when_oracle_absent() {
        let oracle = InMemoryOracleLookup::from_entries(Vec::new());
        let cand = candidate("CAQUA", "100", 1_700_000_000);
        let out = enrich_one(&cand, &oracle, "reflector", 300, 1_700_000_100)
            .await
            .unwrap();
        assert!(matches!(out, Outcome::OracleMiss { .. }));
    }

    #[tokio::test]
    async fn miss_when_oracle_outside_window() {
        // Oracle bar at t=1700000000, candidate at t=1700000600 (10min later),
        // window = 300s → out of window.
        let oracle =
            InMemoryOracleLookup::from_entries(vec![oracle_entry("CUSDC", 1_700_000_000, "1.0")]);
        let cand = candidate("CUSDC", "500", 1_700_000_600);
        let out = enrich_one(&cand, &oracle, "reflector", 300, 1_700_000_700)
            .await
            .unwrap();
        assert!(matches!(out, Outcome::OracleMiss { .. }));
    }

    #[tokio::test]
    async fn decimal_precision_no_float_drift() {
        // 12.345 * 6.789 = 83.810205 exactly. Float would drift.
        let oracle = InMemoryOracleLookup::from_entries(vec![oracle_entry(
            "CUSDC",
            1_700_000_000,
            "12.345",
        )]);
        let mut cand = candidate("CUSDC", "6.789", 1_700_000_000);
        cand.volume_quote = Decimal::from_str("6.789").unwrap();
        let out = enrich_one(&cand, &oracle, "reflector", 300, 1_700_000_100)
            .await
            .unwrap();
        match out {
            Outcome::Enriched(row) => {
                assert_eq!(
                    row.volume_quote_usd,
                    Decimal::from_str("83.810205").unwrap()
                );
            }
            other => panic!("expected Enriched, got {other:?}"),
        }
    }
}
