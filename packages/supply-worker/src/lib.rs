//! Supply worker (task 0039) — best-effort circulating supply →
//! `prices.asset_supply`, the table the `current_prices` MV multiplies by the
//! live price for `market_cap_usd`.
//!
//! v1 covers **classic (credit) assets** via Horizon `/assets`, summing total
//! issued supply across trustline balances + claimable-balance / liquidity-pool
//! / contract holdings (Horizon deprecated the flat `amount` field). Soroban-
//! contract token `total_supply` (RPC `simulateTransaction`) and native XLM are
//! follow-ups. Per ADR 0007 the worker is the **sole writer** of `asset_supply`.
//! Best-effort: a per-asset fetch failure is logged
//! and skipped — `market_cap_usd` simply stays 0 for that asset (NULL-
//! acceptable), and the run still succeeds.

use clickhouse::{Client, Row};
use rust_decimal::Decimal;
use serde::Deserialize;
use std::str::FromStr;

/// Public Horizon endpoint (overridable via `HORIZON_URL`).
pub const DEFAULT_HORIZON: &str = "https://horizon.stellar.org";

#[derive(Debug, thiserror::Error)]
pub enum SupplyError {
    #[error(transparent)]
    Clickhouse(#[from] clickhouse::error::Error),
    #[error("horizon request: {0}")]
    Http(#[from] reqwest::Error),
    #[error("horizon json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("supply not a decimal: {0}")]
    Decimal(#[from] rust_decimal::Error),
}

/// A classic asset that needs a supply lookup.
#[derive(Debug, Clone, Row, Deserialize)]
pub struct CreditAsset {
    pub asset_id: u32,
    pub asset_code: String,
    pub issuer_address: String,
}

/// Outcome of a [`run_supply`] pass.
#[derive(Debug, Default, Clone, Copy)]
pub struct SupplyStats {
    /// Credit assets considered.
    pub considered: usize,
    /// Supplies fetched + written to `asset_supply`.
    pub written: usize,
    /// Assets skipped (no Horizon record or a fetch error).
    pub skipped: usize,
}

/// Load classic credit assets (have both a code and a G-issuer) from
/// `prices.assets`. Native XLM and Soroban contracts are excluded.
pub async fn load_credit_assets(client: &Client) -> Result<Vec<CreditAsset>, SupplyError> {
    let rows = client
        .query(
            "SELECT asset_id, asset_code, issuer_address FROM prices.assets FINAL \
             WHERE asset_type = 'classic' AND issuer_address != '' AND asset_code != ''",
        )
        .fetch_all::<CreditAsset>()
        .await?;
    Ok(rows)
}

#[derive(Deserialize)]
struct HorizonAssets {
    #[serde(rename = "_embedded")]
    embedded: HorizonEmbedded,
}
#[derive(Deserialize)]
struct HorizonEmbedded {
    records: Vec<HorizonAssetRecord>,
}
// Horizon deprecated the flat `amount` field; total supply is now spread across
// trustline `balances` plus the amounts parked in claimable balances, liquidity
// pools, and Soroban contracts. Total issued = the sum of all of them.
#[derive(Deserialize)]
struct HorizonAssetRecord {
    balances: HorizonBalances,
    #[serde(default)]
    claimable_balances_amount: String,
    #[serde(default)]
    liquidity_pools_amount: String,
    #[serde(default)]
    contracts_amount: String,
}
#[derive(Deserialize)]
struct HorizonBalances {
    #[serde(default)]
    authorized: String,
    #[serde(default)]
    authorized_to_maintain_liabilities: String,
    #[serde(default)]
    unauthorized: String,
}

/// A Horizon amount string (possibly empty / absent) → `Decimal`. Empty is 0.
fn dec(s: &str) -> Result<Decimal, rust_decimal::Error> {
    if s.is_empty() {
        Ok(Decimal::ZERO)
    } else {
        Decimal::from_str(s)
    }
}

/// Parse an asset's **total issued supply** out of a Horizon `/assets` response
/// body — the sum of trustline balances + claimable-balance + liquidity-pool +
/// contract holdings. `Ok(None)` when there is no matching record (unknown
/// asset). Pure — no I/O, so it is unit-testable against a recorded body.
pub fn parse_horizon_amount(body: &str) -> Result<Option<Decimal>, SupplyError> {
    let resp: HorizonAssets = serde_json::from_str(body)?;
    match resp.embedded.records.into_iter().next() {
        Some(rec) => {
            let total = dec(&rec.balances.authorized)?
                + dec(&rec.balances.authorized_to_maintain_liabilities)?
                + dec(&rec.balances.unauthorized)?
                + dec(&rec.claimable_balances_amount)?
                + dec(&rec.liquidity_pools_amount)?
                + dec(&rec.contracts_amount)?;
            Ok(Some(total))
        }
        None => Ok(None),
    }
}

/// Fetch a classic asset's total supply from Horizon `/assets`.
pub async fn fetch_supply(
    http: &reqwest::Client,
    base_url: &str,
    code: &str,
    issuer: &str,
) -> Result<Option<Decimal>, SupplyError> {
    let body = http
        .get(format!("{base_url}/assets"))
        .query(&[
            ("asset_code", code),
            ("asset_issuer", issuer),
            ("limit", "1"),
        ])
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    parse_horizon_amount(&body)
}

/// Write a batch of `(asset_id, supply)` into `prices.asset_supply`. The supply
/// values are already-parsed `Decimal`s rendered as plain decimal literals — no
/// injection surface. Idempotent (ReplacingMergeTree on `asset_id`).
pub async fn write_supplies(
    client: &Client,
    supplies: &[(u32, Decimal)],
) -> Result<(), SupplyError> {
    if supplies.is_empty() {
        return Ok(());
    }
    let values: Vec<String> = supplies
        .iter()
        .map(|(id, supply)| format!("({id}, {supply})"))
        .collect();
    let sql = format!(
        "INSERT INTO prices.asset_supply (asset_id, token_supply) VALUES {}",
        values.join(", ")
    );
    client.query(&sql).execute().await?;
    Ok(())
}

/// Load credit assets, fetch each one's supply from Horizon (best-effort), and
/// write the successes to `asset_supply`. Only a ClickHouse load/write failure
/// fails the run; per-asset Horizon failures are skipped.
pub async fn run_supply(
    ch: &Client,
    http: &reqwest::Client,
    base_url: &str,
) -> Result<SupplyStats, SupplyError> {
    let assets = load_credit_assets(ch).await?;
    let mut supplies = Vec::new();
    let mut skipped = 0usize;

    for asset in &assets {
        match fetch_supply(http, base_url, &asset.asset_code, &asset.issuer_address).await {
            Ok(Some(supply)) => supplies.push((asset.asset_id, supply)),
            Ok(None) => {
                skipped += 1;
                tracing::debug!(code = %asset.asset_code, "no Horizon record");
            }
            Err(err) => {
                skipped += 1;
                tracing::warn!(code = %asset.asset_code, error = %err, "supply fetch failed; skipping");
            }
        }
    }

    let written = supplies.len();
    write_supplies(ch, &supplies).await?;
    Ok(SupplyStats {
        considered: assets.len(),
        written,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_total_supply_across_balances_and_holdings() {
        // Real /assets shape: balances + claimable/LP/contract holdings.
        let body = r#"{
            "_embedded": { "records": [ {
                "asset_code": "USDC",
                "claimable_balances_amount": "100.0000000",
                "liquidity_pools_amount": "200.0000000",
                "contracts_amount": "300.0000000",
                "balances": {
                    "authorized": "1000.0000000",
                    "authorized_to_maintain_liabilities": "10.0000000",
                    "unauthorized": "1.0000000"
                }
            } ] }
        }"#;
        let total = parse_horizon_amount(body).unwrap().expect("some supply");
        // 1000 + 10 + 1 + 100 + 200 + 300 = 1611
        assert_eq!(total, Decimal::from_str("1611").unwrap());
    }

    #[test]
    fn missing_holding_fields_default_to_zero() {
        // Only trustline balances present; the holding fields are absent.
        let body = r#"{
            "_embedded": { "records": [ {
                "balances": { "authorized": "42.5000000" }
            } ] }
        }"#;
        let total = parse_horizon_amount(body).unwrap().expect("some supply");
        assert_eq!(total, Decimal::from_str("42.5").unwrap());
    }

    #[test]
    fn empty_records_is_none() {
        let body = r#"{ "_embedded": { "records": [] } }"#;
        assert!(parse_horizon_amount(body).unwrap().is_none());
    }

    #[test]
    fn malformed_amount_errors() {
        let body = r#"{ "_embedded": { "records": [ { "balances": { "authorized": "not-a-number" } } ] } }"#;
        assert!(matches!(
            parse_horizon_amount(body),
            Err(SupplyError::Decimal(_))
        ));
    }
}
