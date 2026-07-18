//! Live validation for the oracle worker (task 0039) — a read-only public
//! call to a Soroban RPC + the real Reflector contract. This is the only true
//! validation of the `simulateTransaction` envelope + ScVal parsing (there is
//! no local Reflector mock).
//!
//!   cargo test -p oracle-worker --test oracle_it -- --ignored --nocapture
//!
//! Override the endpoint with SOROBAN_RPC_URL if the default is rate-limited.

#[tokio::test]
#[ignore = "read-only public network call to Soroban RPC + Reflector"]
async fn fetch_real_xlm_price_from_reflector() {
    let http = reqwest::Client::builder().build().unwrap();
    let rpc = std::env::var("SOROBAN_RPC_URL")
        .unwrap_or_else(|_| oracle_worker::DEFAULT_SOROBAN_RPC.to_string());

    let pd = oracle_worker::fetch_lastprice(&http, &rpc, oracle_worker::REFLECTOR_CEX_DEX, "XLM")
        .await
        .expect("simulateTransaction call")
        .expect("Reflector should have an XLM price");

    eprintln!(
        "Reflector XLM lastprice = {} (1e{} scaled), timestamp = {}",
        pd.price,
        oracle_worker::REFLECTOR_DECIMALS,
        pd.timestamp,
    );
    assert!(pd.price > 0, "price should be positive, got {}", pd.price);
    assert!(
        pd.timestamp > 0,
        "timestamp should be set, got {}",
        pd.timestamp
    );
}
