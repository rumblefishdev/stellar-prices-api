//! Soroban token decimals for the AMM path (task 0329).
//!
//! A swap event carries each leg as a raw i128 in its token's own decimals.
//! Classic assets and their SACs are 7 by protocol; a pure Soroban token's
//! decimals are whatever its SEP-41 `decimals()` returns. Decoding is
//! synchronous and never guesses, so a trade whose token is unknown is dropped
//! and reported ([`crate::LedgerSoroban::missing_decimals`]).
//! [`decode_resolving`] is the step every caller runs around a decode: ask the
//! resolver, persist what it learned to `prices.asset_decimals`, record it in
//! the [`AssetRegistry`], and decode the ledger again.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;
use stellar_xdr::ScVal;

use crate::canonical::AssetRegistry;
use crate::soroban::LedgerSoroban;
use crate::soroban_rpc::{Simulated, http_client, rpc_url_from_env, simulate};

/// The largest scale `rust_decimal` holds. A token claiming more cannot be
/// represented, so it is treated as unresolved rather than truncated.
pub const MAX_DECIMALS: u32 = 28;

/// How long the live processor leaves a contract that gave no answer at all (a
/// Transient failure) before asking again. Bounds the RPC cost of an outage for
/// a token that trades in every ledger, without parking a blip for a warm
/// container's whole lifetime.
pub const LIVE_RETRY_AFTER: Duration = Duration::from_secs(600);

/// A backfill's inline retries of a Transient failure, in milliseconds. Ten
/// minutes of wall clock is days of ledgers to a backfill, so it does not park:
/// it retries on the spot and then asks again on the token's next trade.
pub const BACKFILL_BACKOFF_MS: [u64; 3] = [1_000, 5_000, 20_000];

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
    /// The node has not reached the ledger being decoded, so a just-deployed
    /// token may not exist for it yet. Asked again on the token's next trade.
    Behind,
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

/// What [`decode_resolving`] asks for missing decimals: [`DecimalsResolver`] in
/// production, a fake in tests.
#[allow(async_fn_in_trait)] // only ever used through generics, never as `dyn`
pub trait ResolveDecimals {
    /// Answers for the contracts in `missing`, met while decoding `ledger`.
    async fn resolve(&mut self, missing: &[String], ledger: u32) -> Vec<DecimalsRow>;
}

/// Decode once. If the decode dropped trades for unknown decimals, resolve
/// them, `persist` the answers, record them in `assets` — only after `persist`
/// succeeds — and decode again. Decoding is deterministic and the first result
/// is discarded whole, so nothing is counted twice. Whatever is still in
/// `trades_missing_decimals` afterwards is volume the caller did not price.
pub async fn decode_resolving<E>(
    resolver: &mut impl ResolveDecimals,
    assets: &mut AssetRegistry,
    ledger: u32,
    mut decode: impl FnMut(&mut AssetRegistry) -> LedgerSoroban,
    persist: impl AsyncFnOnce(&[DecimalsRow]) -> Result<(), E>,
) -> Result<LedgerSoroban, E> {
    let out = decode(assets);
    if out.missing_decimals.is_empty() {
        return Ok(out);
    }
    let rows = resolver.resolve(&out.missing_decimals, ledger).await;
    if rows.is_empty() {
        return Ok(out);
    }
    persist(&rows).await?;
    for row in &rows {
        row.record(assets);
    }
    Ok(decode(assets))
}

/// Asks Soroban RPC for the decimals a decode reported missing.
pub struct DecimalsResolver {
    http: reqwest::Client,
    rpc_url: String,
    /// How long a Transient failure parks a contract.
    retry_after: Duration,
    /// Inline retries of a Transient failure before parking it.
    backoff_ms: &'static [u64],
    /// Contracts whose answer was [`Decimals::Absent`]. Never asked again by
    /// this process: a fact does not change, and a cold start asks once more.
    absent: HashSet<String>,
    /// When each contract last gave a [`Decimals::Transient`] failure.
    failed: HashMap<String, Instant>,
}

impl DecimalsResolver {
    /// For the live processor: no inline retries, a Transient failure parks the
    /// contract for [`LIVE_RETRY_AFTER`].
    pub fn live(rpc_url: String) -> Self {
        Self::with(rpc_url, LIVE_RETRY_AFTER, &[])
    }

    /// For a backfill: [`BACKFILL_BACKOFF_MS`] inline, then no parking.
    pub fn backfill(rpc_url: String) -> Self {
        Self::with(rpc_url, Duration::ZERO, &BACKFILL_BACKOFF_MS)
    }

    /// [`live`](Self::live) against `SOROBAN_RPC_URL`, or the public default.
    pub fn live_from_env() -> Self {
        Self::live(rpc_url_from_env())
    }

    /// [`backfill`](Self::backfill) against `SOROBAN_RPC_URL`, or the public default.
    pub fn backfill_from_env() -> Self {
        Self::backfill(rpc_url_from_env())
    }

    fn with(rpc_url: String, retry_after: Duration, backoff_ms: &'static [u64]) -> Self {
        Self {
            http: http_client(),
            rpc_url,
            retry_after,
            backoff_ms,
            absent: HashSet::new(),
            failed: HashMap::new(),
        }
    }

    /// Resolve each distinct contract in `missing` that is neither known Absent
    /// nor parked after a Transient failure. Returns the answers, which the
    /// caller persists and then [`record`](DecimalsRow::record)s. Empty means
    /// nothing new was learned, so decoding again would price nothing more.
    ///
    /// The calls run concurrently, so a ledger naming N new tokens costs one
    /// RPC timeout ([`crate::soroban_rpc::RPC_TIMEOUT_SECS`]) at worst, not N.
    pub async fn resolve(&mut self, missing: &[String], ledger: u32) -> Vec<DecimalsRow> {
        let mut asked = HashSet::new();
        let mut calls = tokio::task::JoinSet::new();
        for contract in missing {
            let parked = self.absent.contains(contract)
                || self
                    .failed
                    .get(contract)
                    .is_some_and(|at| at.elapsed() < self.retry_after);
            if parked || !asked.insert(contract) {
                continue;
            }
            let (http, rpc_url, contract) =
                (self.http.clone(), self.rpc_url.clone(), contract.clone());
            let backoff_ms = self.backoff_ms;
            calls.spawn(async move {
                let mut answer = decimals_of(&http, &rpc_url, &contract, ledger).await;
                for ms in backoff_ms {
                    if answer != Decimals::Transient {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(*ms)).await;
                    answer = decimals_of(&http, &rpc_url, &contract, ledger).await;
                }
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
                        retry_after_secs = self.retry_after.as_secs(),
                        "decimals() got no answer — this token's AMM trades are dropped until it resolves"
                    );
                    self.failed.insert(contract, Instant::now());
                }
                // Not parked: the node catches up within a ledger or two.
                Decimals::Behind => {
                    tracing::info!(
                        contract,
                        ledger,
                        "rpc node behind; decimals() asked again next trade"
                    );
                }
            }
        }
        rows.sort_by(|a, b| a.contract_address.cmp(&b.contract_address));
        rows
    }
}

impl ResolveDecimals for DecimalsResolver {
    async fn resolve(&mut self, missing: &[String], ledger: u32) -> Vec<DecimalsRow> {
        DecimalsResolver::resolve(self, missing, ledger).await
    }
}

/// `decimals()` of `contract` as of `ledger`, classified.
pub async fn decimals_of(
    http: &reqwest::Client,
    rpc_url: &str,
    contract: &str,
    ledger: u32,
) -> Decimals {
    classify(
        simulate(http, rpc_url, contract, "decimals", Some(ledger)).await,
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
        Simulated::Behind => Decimals::Behind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOLVBTC: &str = "CBIJBDNZNF4X35BJ4FFZWCDBSCKOP5NB4PLG4SNENRMLAPYG4P5FM6VN";

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
        assert_eq!(classify(Simulated::Behind, "C"), Decimals::Behind);
    }

    #[tokio::test]
    async fn an_absent_contract_is_never_asked_again() {
        // Port 1 would answer Transient; a contract already known Absent must
        // not reach it at all.
        let mut r = DecimalsResolver::live("http://127.0.0.1:1/".to_string());
        let c = SOLVBTC.to_string();
        r.absent.insert(c.clone());
        assert!(r.resolve(std::slice::from_ref(&c), 1).await.is_empty());
        assert!(
            !r.failed.contains_key(&c),
            "not asked, so no Transient recorded"
        );
    }

    #[tokio::test]
    async fn a_failed_contract_is_parked_for_its_retry_window() {
        // Port 1 refuses at once, so each attempt is a fast Transient.
        let mut r = DecimalsResolver::live("http://127.0.0.1:1/".to_string());
        let c = SOLVBTC.to_string();
        assert!(r.resolve(&[c.clone(), c.clone()], 1).await.is_empty());
        let first = r.failed[&c];
        assert!(r.resolve(std::slice::from_ref(&c), 1).await.is_empty());
        assert_eq!(r.failed[&c], first, "not asked again inside the window");

        // A backfill's window is zero: the next trade asks again.
        r.retry_after = Duration::ZERO;
        assert!(r.resolve(std::slice::from_ref(&c), 1).await.is_empty());
        assert!(
            r.failed[&c] > first,
            "asked again once the window has passed"
        );
    }
}
