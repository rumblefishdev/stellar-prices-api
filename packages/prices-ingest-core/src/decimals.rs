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

/// How long a contract that gave no answer at all (a Transient failure) is left
/// alone before it is asked again. Bounds the RPC cost of an outage for a token
/// that trades in every ledger, without parking a blip for a warm container's
/// whole lifetime.
pub const RETRY_AFTER: Duration = Duration::from_secs(600);

/// What one `decimals()` call told us.
#[derive(Debug, PartialEq, Eq)]
pub enum Decimals {
    Known(u8),
    /// The contract answered and the answer is not a usable scale: it has no
    /// `decimals()`, errored, returned something other than a `u32`, or more
    /// than [`MAX_DECIMALS`]. Repeats on every call.
    Absent,
    /// No answer about the contract (transport, JSON-RPC error, archived state).
    Transient,
}

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
    /// Contracts whose answer was [`Decimals::Absent`]. Never asked again by
    /// this process: a fact does not change, and a cold start asks once more.
    absent: HashSet<String>,
    /// When each contract last gave a [`Decimals::Transient`] failure.
    failed: HashMap<String, Instant>,
}

impl DecimalsResolver {
    pub fn new(rpc_url: String) -> Self {
        Self {
            http: http_client(),
            rpc_url,
            absent: HashSet::new(),
            failed: HashMap::new(),
        }
    }

    /// Against `SOROBAN_RPC_URL`, or the public default.
    pub fn from_env() -> Self {
        Self::new(rpc_url_from_env())
    }

    /// Resolve each distinct contract in `missing` that is neither known
    /// Absent nor inside [`RETRY_AFTER`] of a Transient failure. Returns the
    /// answers, which the caller persists and then
    /// [`record`](DecimalsRow::record)s. Empty means nothing new was learned,
    /// so decoding again would price nothing more.
    ///
    /// The calls run concurrently, so a ledger naming N new tokens costs one
    /// RPC timeout ([`crate::soroban_rpc::RPC_TIMEOUT_SECS`]) at worst, not N.
    pub async fn resolve(&mut self, missing: &[String]) -> Vec<DecimalsRow> {
        let mut asked = HashSet::new();
        let mut calls = tokio::task::JoinSet::new();
        for contract in missing {
            let parked = self.absent.contains(contract)
                || self
                    .failed
                    .get(contract)
                    .is_some_and(|at| at.elapsed() < RETRY_AFTER);
            if parked || !asked.insert(contract) {
                continue;
            }
            let (http, rpc_url, contract) =
                (self.http.clone(), self.rpc_url.clone(), contract.clone());
            calls.spawn(async move {
                let answer = decimals_of(&http, &rpc_url, &contract).await;
                (contract, answer)
            });
        }

        let mut rows = Vec::new();
        while let Some(joined) = calls.join_next().await {
            // A panicked call answered nothing; the token is asked again on its
            // next trade.
            let Ok((contract, answer)) = joined else {
                continue;
            };
            match answer {
                Decimals::Known(decimals) => {
                    tracing::info!(contract, decimals, "resolved soroban token decimals");
                    self.failed.remove(&contract);
                    rows.push(DecimalsRow {
                        contract_address: contract,
                        decimals,
                    });
                }
                Decimals::Absent => {
                    tracing::warn!(
                        contract,
                        "decimals() has no usable answer — this token's AMM trades are dropped; \
                         not asked again until the process restarts"
                    );
                    self.absent.insert(contract);
                }
                Decimals::Transient => {
                    tracing::warn!(
                        contract,
                        retry_after_secs = RETRY_AFTER.as_secs(),
                        "decimals() got no answer — this token's AMM trades are dropped until it resolves"
                    );
                    self.failed.insert(contract, Instant::now());
                }
            }
        }
        rows.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));
        rows
    }
}

/// `decimals()` of `contract`, classified.
pub async fn decimals_of(http: &reqwest::Client, rpc_url: &str, contract: &str) -> Decimals {
    classify(
        simulate(http, rpc_url, contract, "decimals").await,
        contract,
    )
}

/// SEP-41 declares `decimals() -> u32`; anything else, or more than
/// [`MAX_DECIMALS`], is not a scale we can apply.
fn classify(simulated: Simulated, contract: &str) -> Decimals {
    match simulated {
        Simulated::Value(ScVal::U32(d)) if d <= MAX_DECIMALS => Decimals::Known(d as u8),
        Simulated::Value(other) => {
            tracing::warn!(contract, value = ?other, "decimals() returned no usable scale");
            Decimals::Absent
        }
        Simulated::Absent => Decimals::Absent,
        Simulated::Transient => Decimals::Transient,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_u32_within_the_decimal_scale_is_known() {
        let v = |v| classify(Simulated::Value(v), "C");
        assert_eq!(v(ScVal::U32(8)), Decimals::Known(8));
        assert_eq!(v(ScVal::U32(0)), Decimals::Known(0));
        assert_eq!(v(ScVal::U32(28)), Decimals::Known(28));
        assert_eq!(v(ScVal::U32(29)), Decimals::Absent);
        assert_eq!(v(ScVal::I32(7)), Decimals::Absent);
        assert_eq!(v(ScVal::Void), Decimals::Absent);
        // The simulation's own verdict passes through: a fact stays a fact,
        // no answer stays retryable.
        assert_eq!(classify(Simulated::Absent, "C"), Decimals::Absent);
        assert_eq!(classify(Simulated::Transient, "C"), Decimals::Transient);
    }

    #[tokio::test]
    async fn an_absent_contract_is_never_asked_again() {
        // Port 1 would answer Transient; a contract already known Absent must
        // not reach it at all.
        let mut r = DecimalsResolver::new("http://127.0.0.1:1/".to_string());
        let c = "CBIJBDNZNF4X35BJ4FFZWCDBSCKOP5NB4PLG4SNENRMLAPYG4P5FM6VN".to_string();
        r.absent.insert(c.clone());
        assert!(r.resolve(std::slice::from_ref(&c)).await.is_empty());
        assert!(
            !r.failed.contains_key(&c),
            "not asked, so no Transient recorded"
        );
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
