//! Task 0139 — `OhlcvWriter::write_absent_assets`, the oracle worker's asset
//! write once it no longer loads the registry.
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-ingest-core --test absent_assets_it -- --ignored --test-threads=1
//!
//! The writer hardcodes `prices.assets`, so this resets the real `prices`
//! database's `assets` table. Never run against a shared cluster.

use clickhouse::Client;
use prices_clickhouse::USDC_ISSUER;
use prices_clickhouse::asset_id::fixture::{AssetFixture, assets_insert};
use prices_ingest_core::{AssetIdentity, OhlcvWriter};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Raw rows (not `FINAL`), so a re-emitted row would show as a second one.
async fn raw_rows(c: &Client, code: &str) -> u64 {
    c.query("SELECT count() FROM prices.assets WHERE asset_code = ?")
        .bind(code)
        .fetch_one()
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn only_absent_assets_are_written_and_a_present_one_is_never_re_emitted() {
    let c = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    c.query("TRUNCATE TABLE prices.assets")
        .execute()
        .await
        .unwrap();
    c.query(&assets_insert(
        "prices",
        &[AssetFixture::new("XLM", "classic", "", "")],
    ))
    .execute()
    .await
    .unwrap();

    let writer = OhlcvWriter::new(c.clone());
    let usdc = AssetIdentity::Credit {
        code: "USDC".to_string(),
        issuer: USDC_ISSUER.to_string(),
    };
    let both = [AssetIdentity::Native, usdc];

    assert_eq!(writer.write_absent_assets(&both).await.unwrap(), 1);
    assert_eq!(
        (raw_rows(&c, "XLM").await, raw_rows(&c, "USDC").await),
        (1, 1)
    );

    // Steady state: both present, nothing written.
    assert_eq!(writer.write_absent_assets(&both).await.unwrap(), 0);
    assert_eq!(
        (raw_rows(&c, "XLM").await, raw_rows(&c, "USDC").await),
        (1, 1)
    );

    // The row it wrote carries the SAC, like every other writer's.
    let sac: String = c
        .query("SELECT sac_address FROM prices.assets FINAL WHERE asset_code = 'USDC'")
        .fetch_one()
        .await
        .unwrap();
    assert!(sac.starts_with('C'), "USDC's SAC, got {sac:?}");

    c.query("TRUNCATE TABLE prices.assets")
        .execute()
        .await
        .unwrap();
}
