//! The supply worker's only network test (task 0039): a read-only public GET
//! to horizon.stellar.org.
//!
//! Recorded in the `#[ignore]` inventory (tools/scripts/ignored-tests.sh) and
//! deliberately never run by CI — third-party uptime must not gate a PR (task
//! 0275). It lives in its own target because one target holds one class: in
//! `supply_it.rs` it would run whenever that file's ClickHouse test does.
//!
//! Run it by hand:
//!
//!   cargo test -p supply-worker --test supply_net_it -- --ignored

use rust_decimal::Decimal;

#[tokio::test]
#[ignore = "requires public network — third-party uptime; never gates a PR"]
async fn fetch_real_usdc_supply() {
    let http = reqwest::Client::builder()
        .user_agent("stellar-prices-supply-worker-test/0.1")
        .build()
        .unwrap();
    let supply = supply_worker::fetch_supply(
        &http,
        supply_worker::DEFAULT_HORIZON,
        "USDC",
        "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN",
    )
    .await
    .expect("horizon fetch")
    .expect("USDC has a Horizon record");
    assert!(
        supply > Decimal::ZERO,
        "USDC circulating supply should be positive, got {supply}"
    );
}
