//! Integration tests for the supply worker (task 0039).
//!
//!   docker compose up -d clickhouse
//!   cargo test -p supply-worker -- --ignored          # CH + network
//!
//! `load_and_write_supply_roundtrip` needs local ClickHouse (destructive to
//! prices.assets / prices.asset_supply). `fetch_real_usdc_supply` makes a
//! read-only public GET to horizon.stellar.org.

use clickhouse::Client;
use rust_decimal::Decimal;
use std::str::FromStr;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn load_and_write_supply_roundtrip() {
    let client = Client::default().with_url(ch_url());
    prices_clickhouse::apply_init_sql(&client)
        .await
        .expect("init schema");
    for t in ["prices.assets", "prices.asset_supply"] {
        client
            .query(&format!("TRUNCATE TABLE {t}"))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("truncate {t}: {e}"));
    }

    // A credit asset (loads), native XLM (no issuer → excluded), a Soroban
    // contract (excluded). Only the credit asset should come back.
    client
        .query(
            "INSERT INTO prices.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (1, 'USDC', 'classic', 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN', ''), \
             (2, 'XLM', 'classic', '', ''), \
             (3, '', 'soroban', '', 'CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34M7JQNS7VJK4D5DA73G5')",
        )
        .execute()
        .await
        .expect("seed assets");

    let assets = supply_worker::load_credit_assets(&client)
        .await
        .expect("load");
    assert_eq!(
        assets.len(),
        1,
        "only the credit asset is a supply candidate"
    );
    assert_eq!(assets[0].asset_code, "USDC");

    supply_worker::write_supplies(&client, &[(1, Decimal::from_str("999.5").unwrap())])
        .await
        .expect("write supply");

    let supply: f64 = client
        .query("SELECT toFloat64(token_supply) FROM prices.asset_supply FINAL WHERE asset_id = 1")
        .fetch_one()
        .await
        .expect("read supply");
    assert!(
        (supply - 999.5).abs() < 1e-6,
        "round-tripped supply, got {supply}"
    );
}

#[tokio::test]
#[ignore = "read-only public network GET to horizon.stellar.org"]
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
