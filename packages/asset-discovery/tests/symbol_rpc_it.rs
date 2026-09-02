//! Live Soroban RPC check for the task-0210 symbol resolver. Gated `#[ignore]`
//! because it talks to the public mainnet endpoint:
//!
//!     cargo test -p asset-discovery --test symbol_rpc_it -- --ignored
//!
//! Precedent and reasoning: `oracle-worker/tests/oracle_it.rs`. CI cannot depend
//! on a third-party RPC, so the parser and envelope builder are pinned by offline
//! unit tests in `symbols.rs`; this test is what proves the two ends actually
//! meet against a real contract.

use asset_discovery::symbols::{DEFAULT_SOROBAN_RPC, Outcome, http_client, resolve_symbol};

/// The Soroban asset task 0120 flagged as nameless — highest 24h volume of the
/// API-visible Soroban rows on prod. BE's `default.soroban_contract_metadata`
/// independently records its symbol as `SolvBTC`, so this doubles as a
/// cross-check of the decode path against another team's reading of the chain.
const SOLVBTC: &str = "CBIJBDNZNF4X35BJ4FFZWCDBSCKOP5NB4PLG4SNENRMLAPYG4P5FM6VN";

fn rpc_url() -> String {
    std::env::var("SOROBAN_RPC_URL").unwrap_or_else(|_| DEFAULT_SOROBAN_RPC.to_string())
}

#[tokio::test]
#[ignore = "hits the public Soroban RPC endpoint"]
async fn resolves_a_real_token_symbol() {
    let got = resolve_symbol(&http_client(), &rpc_url(), SOLVBTC).await;
    assert_eq!(
        got,
        Outcome::Symbol("SolvBTC".to_string()),
        "CBIJ… must resolve to the symbol BE's soroban_contract_metadata also holds"
    );
}

#[tokio::test]
#[ignore = "hits the public Soroban RPC endpoint"]
async fn a_contract_that_is_not_a_token_is_absent_not_transient() {
    // A well-formed C-address that was never deployed: the simulation comes back
    // with an error rather than a value. That is a fact about the contract, so it
    // must sentinel (Absent) — classifying it Transient would re-poll it hourly
    // forever.
    let never_deployed = stellar_strkey::Contract([3u8; 32]).to_string();
    let got = resolve_symbol(&http_client(), &rpc_url(), &never_deployed).await;
    assert_eq!(got, Outcome::Absent, "got {got:?}");
}

#[tokio::test]
#[ignore = "hits the public Soroban RPC endpoint"]
async fn an_unreachable_endpoint_is_transient_not_absent() {
    // The other half of the split, and the one that matters most: an RPC outage
    // must NOT sentinel every contract it touches, which would name the whole
    // Soroban population `""` permanently.
    let got = resolve_symbol(&http_client(), "http://127.0.0.1:1/", SOLVBTC).await;
    assert_eq!(got, Outcome::Transient, "got {got:?}");
}
