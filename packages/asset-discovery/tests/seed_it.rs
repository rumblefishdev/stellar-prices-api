//! Integration test for the seed path (task 0054), against a local Docker
//! ClickHouse with the `prices` schema applied.
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p asset-discovery --test seed_it -- --ignored --test-threads=1
//!
//! Uses the real `prices` database (the writer addresses `prices.assets`
//! literally), so it is destructive to that local table — fine for the
//! ephemeral Docker instance, never run against a shared/prod cluster.

use prices_ingest_core::OhlcvWriter;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
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

    // Hold off background merges for the duration of this test. The raw-count
    // assertion at the end only bites while the duplicate part is still
    // separate: ReplacingMergeTree collapses two small parts within seconds, and
    // a merge landing first would make a re-emitting regression look clean.
    writer
        .client()
        .query("SYSTEM STOP MERGES prices.assets")
        .execute()
        .await
        .expect("stop merges");

    let seed = asset_discovery::seed_identities().expect("parse seed");

    // First run writes every seed asset.
    let n1 = asset_discovery::ensure_seed(&writer, &seed)
        .await
        .expect("first seed run");
    assert_eq!(n1, seed.len(), "the first run writes the whole seed");

    // Second run is a no-op: it writes nothing.
    let n2 = asset_discovery::ensure_seed(&writer, &seed)
        .await
        .expect("second seed run");
    assert_eq!(n2, 0, "a steady-state run writes no asset");

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

    // The assertion above passes even if the second run re-emitted the whole
    // registry, because FINAL collapses it either way — so it cannot catch a
    // regression. This one can: a steady-state run must write NO rows at all,
    // so with merges stopped the raw count still has to equal the seed. A full
    // re-emit here is what piled a fresh ~209k-row part into `prices.assets`
    // every hour and drove the oracle into Runtime.OutOfMemory (task 0256).
    let raw: u64 = writer
        .client()
        .query("SELECT count() FROM prices.assets")
        .fetch_one()
        .await
        .expect("count assets without FINAL");
    assert_eq!(
        raw as usize,
        seed.len(),
        "a re-run must write no rows — un-merged count must still equal the seed"
    );

    writer
        .client()
        .query("SYSTEM START MERGES prices.assets")
        .execute()
        .await
        .expect("restart merges");
}

/// Assets the table already holds, seed or not, are neither re-emitted nor
/// read back: only the absent seed identities are written.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn seed_writes_only_the_absent_identities() {
    use prices_ingest_core::{AssetIdentity, AssetRegistry};

    let writer = OhlcvWriter::plaintext(&ch_url());
    prices_clickhouse::apply_sql(writer.client(), prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    let ch = writer.client();
    ch.query("TRUNCATE TABLE prices.assets")
        .execute()
        .await
        .expect("truncate assets");
    ch.query("SYSTEM STOP MERGES prices.assets")
        .execute()
        .await
        .expect("stop merges");

    let seed = asset_discovery::seed_identities().expect("parse seed");
    let (held, absent) = seed.split_at(seed.len() / 2);
    let mut pre = AssetRegistry::from_existing(Vec::new());
    for identity in held {
        pre.intern(identity);
    }
    for c in ["C1", "C2", "C3"] {
        pre.intern(&AssetIdentity::Contract(c.to_string()));
    }
    writer.write_new_assets(&pre).await.expect("pre-seed");

    let written = asset_discovery::ensure_seed(&writer, &seed)
        .await
        .expect("seed run");
    assert_eq!(written, absent.len(), "only the absent half is written");

    let raw: u64 = ch
        .query("SELECT count() FROM prices.assets")
        .fetch_one()
        .await
        .expect("raw count");
    assert_eq!(
        raw as usize,
        seed.len() + 3,
        "no held row is re-emitted, unmerged"
    );

    ch.query("SYSTEM START MERGES prices.assets")
        .execute()
        .await
        .expect("restart merges");
}
