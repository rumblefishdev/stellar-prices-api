//! Soroban token decimals for the AMM path (task 0329).
//!
//! A swap event carries each leg as a raw i128 in its token's own decimals.
//! Classic assets and their SACs are 7 by protocol; a pure Soroban token's
//! decimals are whatever its SEP-41 `decimals()` returns. Decoding is
//! synchronous and never guesses, so a trade whose token is unknown is dropped
//! and reported ([`crate::LedgerSoroban::missing_decimals`]). The caller then
//! asks [`DecimalsResolver`], persists what it learned to
//! `prices.asset_decimals`, records it in the [`AssetRegistry`], and decodes
//! the ledger again.
//!
//! [`AssetRegistry`]: crate::AssetRegistry

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;
use stellar_xdr::ScVal;

use crate::canonical::AssetRegistry;
use crate::soroban_rpc::{Simulated, http_client, rpc_url_from_env, simulate};

/// The largest scale `rust_decimal` holds. A token claiming more cannot be
/// represented, so it is treated as unresolved rather than truncated.
pub const MAX_DECIMALS: u32 = 28;

/// How long a contract whose `decimals()` gave no usable answer is left alone
/// before it is asked again. Bounds the RPC cost of a token that trades in
/// every ledger and never answers, without parking a transient failure for a
/// warm container's whole lifetime.
pub const RETRY_AFTER: Duration = Duration::from_secs(600);

/// One `prices.asset_decimals` row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, clickhouse::Row)]
pub struct DecimalsRow {
    pub contract_address: String,
    pub decimals: u8,
}

impl DecimalsRow {
    /// Record this row in `assets`, after it is durably written.
    pub fn record(&self, assets: &mut AssetRegistry) {
        assets.set_decimals(self.contract_address.clone(), self.decimals.into());
    }
}

/// Asks Soroban RPC for the decimals a decode reported missing.
pub struct DecimalsResolver {
    http: reqwest::Client,
    rpc_url: String,
    /// When each contract last failed to resolve.
    failed: HashMap<String, Instant>,
}

impl DecimalsResolver {
    pub fn new(rpc_url: String) -> Self {
        Self {
            http: http_client(),
            rpc_url,
            failed: HashMap::new(),
        }
    }

    /// Against `SOROBAN_RPC_URL`, or the public default.
    pub fn from_env() -> Self {
        Self::new(rpc_url_from_env())
    }

    /// Resolve each distinct contract in `missing` that has not failed within
    /// [`RETRY_AFTER`]. Returns the answers, which the caller persists and then
    /// [`record`](DecimalsRow::record)s. Empty means nothing new was learned,
    /// so decoding again would price nothing more.
    pub async fn resolve(&mut self, missing: &[String]) -> Vec<DecimalsRow> {
        let mut rows = Vec::new();
        let mut asked = HashSet::new();
        for contract in missing {
            if !asked.insert(contract)
                || self
                    .failed
                    .get(contract)
                    .is_some_and(|at| at.elapsed() < RETRY_AFTER)
            {
                continue;
            }
            match decimals_of(&self.http, &self.rpc_url, contract).await {
                Some(decimals) => {
                    tracing::info!(contract, decimals, "resolved soroban token decimals");
                    self.failed.remove(contract);
                    rows.push(DecimalsRow {
                        contract_address: contract.clone(),
                        decimals,
                    });
                }
                None => {
                    tracing::warn!(
                        contract,
                        retry_after_secs = RETRY_AFTER.as_secs(),
                        "decimals() unresolved — this token's AMM trades are dropped until it resolves"
                    );
                    self.failed.insert(contract.clone(), Instant::now());
                }
            }
        }
        rows
    }
}

/// `decimals()` of `contract`, or `None` when there is no usable answer.
pub async fn decimals_of(http: &reqwest::Client, rpc_url: &str, contract: &str) -> Option<u8> {
    match simulate(http, rpc_url, contract, "decimals").await {
        Simulated::Value(v) => usable_decimals(&v, contract),
        Simulated::Absent | Simulated::Transient => None,
    }
}

/// SEP-41 declares `decimals() -> u32`; anything else, or more than
/// [`MAX_DECIMALS`], is not a scale we can apply.
fn usable_decimals(v: &ScVal, contract: &str) -> Option<u8> {
    match v {
        ScVal::U32(d) if *d <= MAX_DECIMALS => Some(*d as u8),
        other => {
            tracing::warn!(contract, value = ?other, "decimals() returned no usable scale");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_u32_within_the_decimal_scale_is_usable() {
        assert_eq!(usable_decimals(&ScVal::U32(8), "C"), Some(8));
        assert_eq!(usable_decimals(&ScVal::U32(0), "C"), Some(0));
        assert_eq!(usable_decimals(&ScVal::U32(28), "C"), Some(28));
        assert_eq!(usable_decimals(&ScVal::U32(29), "C"), None);
        assert_eq!(usable_decimals(&ScVal::I32(7), "C"), None);
        assert_eq!(usable_decimals(&ScVal::Void, "C"), None);
    }

    #[tokio::test]
    async fn a_failed_contract_is_not_asked_again_inside_the_retry_window() {
        // Port 1 refuses at once, so each attempt is a fast Transient.
        let mut r = DecimalsResolver::new("http://127.0.0.1:1/".to_string());
        let c = "CBIJBDNZNF4X35BJ4FFZWCDBSCKOP5NB4PLG4SNENRMLAPYG4P5FM6VN".to_string();
        assert!(r.resolve(&[c.clone(), c.clone()]).await.is_empty());
        let first = r.failed[&c];
        assert!(r.resolve(std::slice::from_ref(&c)).await.is_empty());
        assert_eq!(r.failed[&c], first, "not asked again inside RETRY_AFTER");

        r.failed.insert(c.clone(), Instant::now() - RETRY_AFTER);
        assert!(r.resolve(std::slice::from_ref(&c)).await.is_empty());
        assert!(
            r.failed[&c] > first,
            "asked again once the window has passed"
        );
    }
}
