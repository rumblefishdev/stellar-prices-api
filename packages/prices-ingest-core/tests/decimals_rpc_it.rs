//! Live Soroban RPC check for the task-0329 `decimals()` lookup. Gated
//! `#[ignore]` because it talks to the public mainnet endpoint:
//!
//!     cargo test -p prices-ingest-core --test decimals_rpc_it -- --ignored
//!
//! Recorded in the `#[ignore]` inventory (tools/scripts/ignored-tests.sh) as a
//! public-network test and never run by CI: third-party uptime must not gate a
//! PR. The parser is pinned offline in `decimals.rs` and `soroban_rpc.rs`; this
//! proves the two ends meet against real tokens.

use prices_ingest_core::decimals::{Decimals, decimals_of};
use prices_ingest_core::soroban_rpc::{http_client, rpc_url_from_env};

#[tokio::test]
#[ignore = "requires public network — third-party uptime; never gates a PR"]
async fn reads_the_decimals_sbe_checked_on_chain() {
    // The tokens SBE reported mispriced (task 0329), with the decimals they
    // read by calling decimals() themselves.
    for (name, contract, want) in [
        (
            "SolvBTC",
            "CBIJBDNZNF4X35BJ4FFZWCDBSCKOP5NB4PLG4SNENRMLAPYG4P5FM6VN",
            8,
        ),
        (
            "XAUM",
            "CC2RBGYNCFBCVENIDL5BFBWPH4OUZM2UA3OD2K2N54GLMWCC4KWPVAGO",
            9,
        ),
        (
            "XRP",
            "CB7OOP3VSAWBZOOTOG2YEFANVU45GVWYUUM5HI32DKLHVKUDOFVQ37XP",
            6,
        ),
    ] {
        let got = decimals_of(&http_client(), &rpc_url_from_env(), contract, 1).await;
        assert_eq!(got, Decimals::Known(want), "{name} ({contract})");
    }
}

#[tokio::test]
#[ignore = "requires public network — third-party uptime; never gates a PR"]
async fn a_contract_that_was_never_deployed_is_absent() {
    let never_deployed = stellar_strkey::Contract([3u8; 32]).to_string();
    let got = decimals_of(&http_client(), &rpc_url_from_env(), &never_deployed, 1).await;
    assert_eq!(
        got,
        Decimals::Absent,
        "a fact about the contract, not a retry"
    );
}
