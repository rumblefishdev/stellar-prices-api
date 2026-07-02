//! Integration test for the discovered pool-registry artifact (task 0053,
//! decision #4), against a local Docker ClickHouse. Validates the
//! `prices.pool_registry` write→load round-trip through real ClickHouse (the
//! RowBinary column-list insert, `LowCardinality(venue)`, and `FINAL` read) —
//! the durable output a partial re-backfill / the live processor loads.
//!
//!     docker compose up -d clickhouse
//!     cargo test -p sdex-backfill --test pool_registry_it -- --ignored --nocapture
//!
//! Destructive to the local `prices.pool_registry` table (truncates it); never
//! run against a shared/prod cluster.

use clickhouse::Client;
use extractors_core::Venue;
use prices_ingest_core::Registries;
use sdex_backfill::sink::Sink;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// One discovered pool of each venue.
fn sample_registry() -> Registries {
    let mut reg = Registries::new();
    reg.venue.insert("CSOROSWAP".into(), Venue::Soroswap);
    reg.soroswap
        .register("CSOROSWAP".into(), "CTOKEN0".into(), "CTOKEN1".into());
    reg.venue.insert("CPHOENIX".into(), Venue::Phoenix);
    reg.phoenix
        .register_with_wasm("CPHOENIX".into(), 0, [0xab; 32]);
    reg.venue.insert("CAQUARIUS".into(), Venue::Aquarius);
    reg
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn pool_registry_round_trips_through_clickhouse() {
    let c = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    c.query("TRUNCATE TABLE prices.pool_registry")
        .execute()
        .await
        .expect("truncate pool_registry");

    let sink = Sink::new(&ch_url());

    // Empty table → empty registries (the fresh full-run case).
    let empty = sink.load_pool_registry().await.expect("load empty");
    assert_eq!(empty.venue.len(), 0);

    // Persist, then reload and compare.
    sink.write_pool_registry(&sample_registry())
        .await
        .expect("write pool registry");

    let rows: u64 = c
        .query("SELECT count() FROM prices.pool_registry FINAL")
        .fetch_one()
        .await
        .expect("count");
    assert_eq!(rows, 3, "one row per discovered pool");

    let reg = sink.load_pool_registry().await.expect("load populated");
    assert_eq!(reg.venue.get("CSOROSWAP"), Some(&Venue::Soroswap));
    assert_eq!(reg.venue.get("CPHOENIX"), Some(&Venue::Phoenix));
    assert_eq!(reg.venue.get("CAQUARIUS"), Some(&Venue::Aquarius));

    let sw = reg.soroswap.lookup("CSOROSWAP").expect("soroswap pair");
    assert_eq!(
        (sw.token0.as_str(), sw.token1.as_str()),
        ("CTOKEN0", "CTOKEN1")
    );

    let ph = reg.phoenix.lookup("CPHOENIX").expect("phoenix pool");
    assert_eq!(ph.pool_type, 0);
    assert_eq!(
        ph.wasm_hash,
        Some([0xab; 32]),
        "wasm hash survives the hex round-trip"
    );

    assert_eq!(
        reg.pool_count(),
        2,
        "soroswap + phoenix (aquarius has no pool detail)"
    );

    // Idempotent re-write: FINAL still collapses to one row per pool.
    sink.write_pool_registry(&sample_registry())
        .await
        .expect("re-write pool registry");
    let rows_again: u64 = c
        .query("SELECT count() FROM prices.pool_registry FINAL")
        .fetch_one()
        .await
        .expect("count again");
    assert_eq!(rows_again, 3, "re-run must not duplicate rows after FINAL");
}
