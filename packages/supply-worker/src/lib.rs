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
use std::time::{Duration, Instant};

/// Public Horizon endpoint (overridable via `HORIZON_URL`).
pub const DEFAULT_HORIZON: &str = "https://horizon.stellar.org";

/// Default wall-clock budget for a single run's Horizon walk, in seconds. The
/// Lambda timeout is 300 s; stopping the walk at 240 s leaves headroom for the
/// final batch flush + runtime teardown so a run *never* ends `Status: timeout`
/// (task 0084). Override with `SUPPLY_TIME_BUDGET_SECS`.
pub const DEFAULT_TIME_BUDGET_SECS: u64 = 240;

/// Default upper bound on assets pulled per run — a safety cap on the CH query
/// so the in-memory list stays bounded as the registry grows. The *time* budget
/// is the real per-run limiter; this just keeps the query from returning tens of
/// thousands of rows. Override with `SUPPLY_MAX_ASSETS_PER_RUN`.
pub const DEFAULT_MAX_ASSETS_PER_RUN: usize = 5_000;

/// Per-run bounds that keep the supply walk inside the Lambda timeout (task
/// 0084). A run refreshes the **stalest** assets first (see
/// [`load_stalest_credit_assets`]) until either bound is hit, so successive
/// scheduled runs round-robin the whole registry over a few invocations instead
/// of re-walking from the top and timing out before reaching the tail.
#[derive(Debug, Clone, Copy)]
pub struct SupplyRunConfig {
    /// Stop starting new Horizon fetches once this much wall-clock has elapsed.
    pub time_budget: Duration,
    /// Upper bound on assets loaded (CH query `LIMIT`).
    pub max_assets: usize,
}

impl Default for SupplyRunConfig {
    fn default() -> Self {
        Self {
            time_budget: Duration::from_secs(DEFAULT_TIME_BUDGET_SECS),
            max_assets: DEFAULT_MAX_ASSETS_PER_RUN,
        }
    }
}

impl SupplyRunConfig {
    /// Build from `SUPPLY_TIME_BUDGET_SECS` / `SUPPLY_MAX_ASSETS_PER_RUN`,
    /// falling back to the defaults.
    pub fn from_env() -> Self {
        let default = Self::default();
        Self {
            time_budget: Duration::from_secs(prices_clickhouse::env::env_parse_or(
                "SUPPLY_TIME_BUDGET_SECS",
                DEFAULT_TIME_BUDGET_SECS,
            )),
            max_assets: prices_clickhouse::env::env_parse_or(
                "SUPPLY_MAX_ASSETS_PER_RUN",
                default.max_assets,
            ),
        }
    }
}

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
    /// Credit assets loaded for this run (the stalest slice, ≤ `max_assets`).
    pub considered: usize,
    /// Supplies fetched + written to `asset_supply`.
    pub written: usize,
    /// Assets skipped (no Horizon record or a fetch error).
    pub skipped: usize,
    /// Assets loaded but not attempted because the time budget was hit —
    /// deferred to the next scheduled run (they remain the stalest, so they lead
    /// the next slice). 0 on a run that finished its whole slice.
    pub deferred: usize,
    /// Whether the run stopped early on the wall-clock budget.
    pub deadline_hit: bool,
}

/// Load classic credit assets (have both a code and a G-issuer) from
/// `prices.assets`, **stalest first** — assets never fetched (no `asset_supply`
/// row) lead, then those with the oldest `fetched_at`. Capped at `limit`.
///
/// This ordering is the checkpoint (task 0084): each scheduled run refreshes the
/// most-out-of-date assets, and writing them stamps a fresh `fetched_at` so they
/// sort last next time. Over ⌈registry / per-run-throughput⌉ runs the whole set
/// is covered, then it round-robins oldest-first — no external cursor needed and
/// robust to assets being added or removed. Native XLM and Soroban contracts are
/// excluded.
pub async fn load_stalest_credit_assets(
    client: &Client,
    limit: usize,
) -> Result<Vec<CreditAsset>, SupplyError> {
    // LEFT JOIN the latest supply time per asset. Unmatched (never-fetched)
    // assets get the DateTime default (epoch 0), so they sort first under ASC.
    let rows = client
        .query(
            "SELECT a.asset_id, a.asset_code, a.issuer_address \
             FROM prices.assets AS a FINAL \
             LEFT JOIN ( \
                 SELECT asset_id, max(fetched_at) AS fetched_at \
                 FROM prices.asset_supply GROUP BY asset_id \
             ) AS s ON a.asset_id = s.asset_id \
             WHERE a.asset_type = 'classic' AND a.issuer_address != '' AND a.asset_code != '' \
             ORDER BY s.fetched_at ASC, a.asset_id ASC \
             LIMIT ?",
        )
        .bind(limit as u64)
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

/// How many fetched supplies to accumulate before flushing to ClickHouse. The
/// Horizon fetch loop is sequential and can run for minutes across a large asset
/// registry; flushing in batches means a Lambda timeout mid-loop still persists
/// everything fetched so far (only the in-flight batch is lost) rather than
/// discarding the whole run.
pub const SUPPLY_FLUSH_BATCH: usize = 50;

/// Load the stalest credit assets, fetch each one's supply from Horizon
/// (best-effort), and write the successes to `asset_supply` in batches — **within
/// a wall-clock budget** so the run never hits the Lambda timeout (task 0084).
///
/// Only a ClickHouse load/write failure fails the run; per-asset Horizon failures
/// are skipped. When the time budget is reached the walk stops early: everything
/// fetched so far is persisted (batches flush as they fill), and the untouched
/// assets — still the stalest — lead the next scheduled run.
pub async fn run_supply(
    ch: &Client,
    http: &reqwest::Client,
    base_url: &str,
    cfg: &SupplyRunConfig,
) -> Result<SupplyStats, SupplyError> {
    let assets = load_stalest_credit_assets(ch, cfg.max_assets).await?;
    let mut batch: Vec<(u32, Decimal)> = Vec::new();
    let mut written = 0usize;
    let mut skipped = 0usize;
    let mut deferred = 0usize;
    let mut deadline_hit = false;

    let deadline = Instant::now() + cfg.time_budget;
    for (i, asset) in assets.iter().enumerate() {
        if Instant::now() >= deadline {
            // Out of time: leave the remaining (still-stalest) assets for the
            // next run rather than risk a mid-fetch Lambda kill.
            deferred = assets.len() - i;
            deadline_hit = true;
            break;
        }
        match fetch_supply(http, base_url, &asset.asset_code, &asset.issuer_address).await {
            Ok(Some(supply)) => {
                batch.push((asset.asset_id, supply));
                if batch.len() >= SUPPLY_FLUSH_BATCH {
                    write_supplies(ch, &batch).await?;
                    written += batch.len();
                    batch.clear();
                }
            }
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

    // Flush the tail.
    if !batch.is_empty() {
        write_supplies(ch, &batch).await?;
        written += batch.len();
    }

    Ok(SupplyStats {
        considered: assets.len(),
        written,
        skipped,
        deferred,
        deadline_hit,
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

    #[test]
    fn config_default_stays_under_the_lambda_timeout() {
        let cfg = SupplyRunConfig::default();
        // 240 s walk budget + flush headroom must stay below the 300 s Lambda
        // timeout, else the run can still end `Status: timeout` (task 0084).
        assert!(
            cfg.time_budget < Duration::from_secs(300),
            "time budget must leave headroom under the 300 s Lambda limit"
        );
        assert_eq!(
            cfg.time_budget,
            Duration::from_secs(DEFAULT_TIME_BUDGET_SECS)
        );
        assert_eq!(cfg.max_assets, DEFAULT_MAX_ASSETS_PER_RUN);
    }
}
