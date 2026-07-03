//! Integration test for the pool-registry seeder (task 0079): map an
//! API-shaped payload → write to `prices.pool_registry` → reload and assert the
//! normalisation landed (aqua→aquarius, sdex + unknown poolType dropped). Needs
//! a local ClickHouse with the `prices` schema:
//!
//!     docker compose up -d clickhouse
//!     cargo test -p pool-registry-seed --test seed_it -- --ignored
//!
//! Destructive to the local `prices.pool_registry` table — never run against a
//! shared/prod cluster.

use pool_registry_seed::{ApiPool, build_registry, to_registry_rows};
use prices_ingest_core::OhlcvWriter;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn pool(protocol: &str, address: &str, ta: &str, tb: &str, pool_type: &str) -> ApiPool {
    ApiPool {
        protocol: protocol.into(),
        address: address.into(),
        token_a: ta.into(),
        token_b: tb.into(),
        pool_type: pool_type.into(),
    }
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn seeds_pool_registry_and_normalises_venues() {
    let writer = OhlcvWriter::plaintext(&ch_url());
    prices_clickhouse::apply_sql(writer.client(), prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    writer
        .client()
        .query("TRUNCATE TABLE prices.pool_registry")
        .execute()
        .await
        .expect("truncate");

    // An API-shaped payload spanning every venue-aware case.
    let pools = vec![
        pool("soroswap", "CSORO", "CTOKA", "CTOKB", "xyk"),
        pool("phoenix", "CPHO", "CPHOA", "CPHOB", "xyk"),
        pool("aqua", "CAQUA", "CAQA", "CAQB", "xyk"),
        pool("aqua", "CAQSTABLE", "CAS", "CBS", "stable"), // KEPT: Aquarius stableswap
        pool("aqua", "CAQCONC", "CAC", "CBC", "concentrated"), // dropped: held (task 0080)
        pool("sdex", "CSDEX", "", "", "xyk"),              // dropped: not an AMM venue
        pool("phoenix", "CPHOSTABLE", "CSA", "CSB", "stable"), // dropped: Phoenix stable unimplemented
    ];
    let (rows, stats) = to_registry_rows(&pools);
    assert_eq!(stats.kept, 4);
    let reg = build_registry(&rows);

    writer
        .write_pool_registry(&reg)
        .await
        .expect("write pool registry");

    // Row count in the table == the four kept AMM pools.
    let persisted: u64 = writer
        .client()
        .query("SELECT count() FROM prices.pool_registry FINAL")
        .fetch_one()
        .await
        .expect("count");
    assert_eq!(
        persisted, 4,
        "only the classified/seedable AMM pools persist"
    );

    // Reload and assert the venue-aware normalisation: aqua landed as canonical
    // 'aquarius', its stableswap pool is kept, concentrated + sdex + Phoenix
    // stable are absent, Soroswap pair tokens survived.
    let reloaded = writer.load_pool_registry().await.expect("reload");
    assert_eq!(
        reloaded.venue.get("CAQUA").map(|v| v.as_source()),
        Some("aquarius")
    );
    assert_eq!(
        reloaded.venue.get("CAQSTABLE").map(|v| v.as_source()),
        Some("aquarius"),
        "Aquarius stableswap pool must be seeded"
    );
    assert_eq!(
        reloaded.venue.get("CSORO").map(|v| v.as_source()),
        Some("soroswap")
    );
    assert_eq!(
        reloaded.venue.get("CPHO").map(|v| v.as_source()),
        Some("phoenix")
    );
    assert!(
        !reloaded.venue.contains_key("CAQCONC"),
        "Aquarius concentrated held back pending task 0080"
    );
    assert!(
        !reloaded.venue.contains_key("CSDEX"),
        "sdex must not be seeded"
    );
    assert!(
        !reloaded.venue.contains_key("CPHOSTABLE"),
        "Phoenix stable must not be seeded"
    );
    let soro = reloaded.soroswap.lookup("CSORO").expect("soroswap pair");
    assert_eq!(
        (soro.token0.as_str(), soro.token1.as_str()),
        ("CTOKA", "CTOKB")
    );

    // Idempotent: re-writing the same registry doesn't duplicate rows.
    writer.write_pool_registry(&reg).await.expect("re-write");
    let after: u64 = writer
        .client()
        .query("SELECT count() FROM prices.pool_registry FINAL")
        .fetch_one()
        .await
        .expect("count after rewrite");
    assert_eq!(after, 4, "re-run is idempotent (RMT on contract_id)");
}
