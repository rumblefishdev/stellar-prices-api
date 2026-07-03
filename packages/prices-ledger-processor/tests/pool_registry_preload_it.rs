//! Integration test for task 0078: the LIVE ledger-processor sink preloads the
//! discovered AMM pool registry from `prices.pool_registry` at cold start, so
//! pools created before the cursor start are resolvable instead of being sent to
//! `prices.unresolved_pools`. Mirrors the backfill's `pool_registry_it` but
//! exercises the live [`ClickHouseSink`], against a local Docker ClickHouse.
//!
//!     docker compose up -d clickhouse
//!     cargo test -p prices-ledger-processor --test pool_registry_preload_it -- --ignored --nocapture
//!
//! Destructive to the local `prices.pool_registry` table (truncates it); never
//! run against a shared/prod cluster.

use clickhouse::Client;
use extractors_core::Venue;
use prices_ingest_core::Registries;
use prices_ledger_processor::sink::ClickHouseSink;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// One discovered pool of each venue (matches the backfill IT's fixture).
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
async fn live_sink_preloads_seeded_pool_registry() {
    let c = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    c.query("TRUNCATE TABLE prices.pool_registry")
        .execute()
        .await
        .expect("truncate pool_registry");

    let sink = ClickHouseSink::plaintext(&ch_url());

    // Unseeded table → empty registry. This is the SDEX-only case, which must
    // not error (the live processor still runs, just pool-blind for AMM).
    let empty = sink
        .load_pool_registry()
        .await
        .expect("load empty registry");
    assert_eq!(empty.venue.len(), 0, "unseeded registry must be empty");

    // Seed the registry as the backfill (task 0053) would have persisted it —
    // via the same `to_pool_rows` serialisation the writer uses.
    let rows = sample_registry().to_pool_rows();
    let mut insert = c.insert("prices.pool_registry").expect("insert handle");
    for row in &rows {
        insert.write(row).await.expect("write pool row");
    }
    insert.end().await.expect("end insert");

    // The live sink now rehydrates the pool classification at cold start — the
    // whole point of task 0078.
    let reg = sink
        .load_pool_registry()
        .await
        .expect("load seeded registry");
    assert_eq!(reg.venue.len(), 3, "all three venues rehydrated");
    assert_eq!(reg.venue.get("CSOROSWAP"), Some(&Venue::Soroswap));
    assert_eq!(reg.venue.get("CPHOENIX"), Some(&Venue::Phoenix));
    assert_eq!(reg.venue.get("CAQUARIUS"), Some(&Venue::Aquarius));
    let sw = reg
        .soroswap
        .lookup("CSOROSWAP")
        .expect("soroswap pair present");
    assert_eq!(
        (sw.token0.as_str(), sw.token1.as_str()),
        ("CTOKEN0", "CTOKEN1"),
        "soroswap pair tokens round-trip",
    );
}
