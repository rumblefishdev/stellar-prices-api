//! Regression test for task 0067 — an enrichment value written to the
//! single-writer `prices.asset_metadata` table must survive a subsequent full
//! `write_assets` re-emit (which used to clobber a `home_domain` column living
//! on the shared `prices.assets` ReplacingMergeTree row). Needs a local
//! ClickHouse with the `prices` schema:
//!
//!     docker compose up -d clickhouse
//!     cargo test -p asset-discovery --test enrichment_survives_it -- --ignored
//!
//! Destructive to the local `prices.assets` / `prices.asset_metadata` tables —
//! never run against a shared/prod cluster.

use prices_ingest_core::{AssetIdentity, AssetMetadata, AssetRegistry, OhlcvWriter};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn home_domain_survives_a_second_write_assets() {
    let writer = OhlcvWriter::plaintext(&ch_url());
    prices_clickhouse::apply_sql(writer.client(), prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    for table in ["prices.assets", "prices.asset_metadata"] {
        writer
            .client()
            .query(&format!("TRUNCATE TABLE {table}"))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("truncate {table}: {e}"));
    }

    // Register an asset and write its identity row (as the ledger processor does).
    let mut registry = AssetRegistry::from_existing(Vec::new());
    let asset_id = registry.get_or_assign(&AssetIdentity::Credit {
        code: "USDC".to_string(),
        issuer: prices_clickhouse::USDC_ISSUER.to_string(),
    });
    writer
        .write_assets(&registry)
        .await
        .expect("write identity");

    // Enrich home_domain via the single-writer enrichment table.
    writer
        .write_asset_metadata(&[AssetMetadata {
            asset_id,
            home_domain: "centre.io".to_string(),
        }])
        .await
        .expect("write enrichment");

    // Re-emit the full asset registry — the historical clobber trigger. Under the
    // old single-table design this reset home_domain to '' on the shared row.
    writer
        .write_assets(&registry)
        .await
        .expect("second identity re-emit");

    // The enrichment must still be there: it lives in a table `write_assets`
    // never touches.
    let survived: u64 = writer
        .client()
        .query(
            "SELECT count() FROM prices.asset_metadata FINAL \
             WHERE asset_id = ? AND home_domain = 'centre.io'",
        )
        .bind(asset_id)
        .fetch_one()
        .await
        .expect("count enrichment");
    assert_eq!(
        survived, 1,
        "home_domain must survive a subsequent write_assets re-emit"
    );
}
