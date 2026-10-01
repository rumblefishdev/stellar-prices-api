//! ClickHouse integration test for the supply worker (task 0039).
//!
//!   tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!   cargo test -p supply-worker --test supply_it -- --ignored --test-threads=1
//!
//! `load_and_write_supply_roundtrip` needs local ClickHouse (destructive to
//! prices.assets / prices.asset_supply). The crate's network test lives in
//! `supply_net_it.rs`, so that `--ignored` here never reaches Horizon.

use clickhouse::Client;
use prices_clickhouse::asset_id::fixture::{AssetFixture, assets_insert, fetch_ids};
use rust_decimal::Decimal;
use std::str::FromStr;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
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

    // Two credit assets (load), native XLM (no issuer → excluded), a Soroban
    // contract (excluded). EURC already has a supply row; USDC never did.
    let usdc = AssetFixture::new("USDC", "classic", prices_clickhouse::USDC_ISSUER, "");
    let eurc = AssetFixture::new(
        "EURC",
        "classic",
        "GDHU6WRG4IEQXM5NZ4BMPKOXHW76MZM4Y2IEMFDVXBSDP6SJY4ITNPP2",
        "",
    );
    client
        .query(&assets_insert(
            "prices",
            &[
                usdc,
                AssetFixture::new("XLM", "classic", "", ""),
                AssetFixture::new(
                    "",
                    "soroban",
                    "",
                    "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34M7JQNS7VJK4D5DA73G5",
                ),
                eurc,
            ],
        ))
        .execute()
        .await
        .expect("seed assets");
    let [usdc_id, eurc_id] = fetch_ids(&client, &[usdc, eurc]).await;

    // EURC has a (stale) supply row; USDC has none. Never-fetched must sort
    // first under the stalest-first checkpoint ordering (task 0084).
    client
        .query(&format!(
            "INSERT INTO prices.asset_supply (asset_id, token_supply, fetched_at) VALUES \
             ({eurc_id}, 100.0, toDateTime('2020-01-01 00:00:00'))"
        ))
        .execute()
        .await
        .expect("seed stale EURC supply");

    let assets = supply_worker::load_stalest_credit_assets(&client, 10)
        .await
        .expect("load");
    assert_eq!(
        assets.len(),
        2,
        "both credit assets are supply candidates (XLM + soroban excluded)"
    );
    assert_eq!(
        assets[0].asset_code, "USDC",
        "never-fetched USDC must lead the stalest-first ordering"
    );
    assert_eq!(assets[1].asset_code, "EURC", "already-fetched asset trails");
    // Task 0139: the id is read back as u64 (`toUInt64` in the SELECT) and is
    // exactly the one the database derived, so writing it back keys the same
    // asset.
    assert_eq!((assets[0].asset_id, assets[1].asset_id), (usdc_id, eurc_id));

    // The `limit` caps the slice — only the single stalest is returned.
    let capped = supply_worker::load_stalest_credit_assets(&client, 1)
        .await
        .expect("load capped");
    assert_eq!(capped.len(), 1, "limit bounds the per-run slice");
    assert_eq!(capped[0].asset_code, "USDC");

    supply_worker::write_supplies(
        &client,
        &[(assets[0].asset_id, Decimal::from_str("999.5").unwrap())],
    )
    .await
    .expect("write supply");

    let supply: f64 = client
        .query(&format!(
            "SELECT toFloat64(token_supply) FROM prices.asset_supply FINAL WHERE asset_id = {usdc_id}"
        ))
        .fetch_one()
        .await
        .expect("read supply");
    assert!(
        (supply - 999.5).abs() < 1e-6,
        "round-tripped supply, got {supply}"
    );
}
