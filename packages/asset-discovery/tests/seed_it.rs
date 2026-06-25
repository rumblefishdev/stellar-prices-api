//! Integration test for the seed path (task 0054), against a local Docker
//! ClickHouse with the `prices` schema applied.
//!
//!     docker compose up -d clickhouse
//!     cargo test -p asset-discovery --test seed_it -- --ignored
//!
//! Uses the real `prices` database (the writer addresses `prices.assets`
//! literally), so it is destructive to that local table — fine for the
//! ephemeral Docker instance, never run against a shared/prod cluster.

use prices_ingest_core::OhlcvWriter;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn seed_populates_assets_idempotently() {
    let writer = OhlcvWriter::plaintext(&ch_url());

    // Ensure the schema is present (idempotent), then start from an empty table.
    prices_clickhouse::apply_sql(writer.client(), prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    writer
        .client()
        .query("TRUNCATE TABLE prices.assets")
        .execute()
        .await
        .expect("truncate assets");

    let seed = asset_discovery::seed_identities().expect("parse seed");

    // First run seeds the table.
    let n1 = asset_discovery::ensure_seed(&writer, &seed)
        .await
        .expect("first seed run");
    assert_eq!(
        n1,
        seed.len(),
        "registry should hold exactly the seed assets"
    );

    // Second run is a no-op: same count, no duplicate rows after FINAL collapse.
    let n2 = asset_discovery::ensure_seed(&writer, &seed)
        .await
        .expect("second seed run");
    assert_eq!(n1, n2, "re-run must not change the asset count");

    let count: u64 = writer
        .client()
        .query("SELECT count() FROM prices.assets FINAL")
        .fetch_one()
        .await
        .expect("count assets");
    assert_eq!(
        count as usize,
        seed.len(),
        "ReplacingMergeTree FINAL must collapse re-runs to one row per asset"
    );
}
