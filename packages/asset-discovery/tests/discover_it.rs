//! Integration test for organic discovery (task 0054), driving `discover_window`
//! with a `LocalDiskFetcher` over the real Galexie fixtures bundled with
//! `prices-ledger-processor` (contiguous ledgers 62460540–62460542). Needs a
//! local ClickHouse with the `prices` schema:
//!
//!     docker compose up -d clickhouse
//!     cargo test -p asset-discovery --test discover_it -- --ignored
//!
//! Destructive to the local `prices.assets` / `prices.discovery_state` tables —
//! never run against a shared/prod cluster.

use prices_ingest_core::OhlcvWriter;
use prices_ledger_processor::object_fetcher::LocalDiskFetcher;
use std::path::PathBuf;

const FIRST_LEDGER: u64 = 62460540;
const LAST_LEDGER: u64 = 62460542;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

// The fixtures live in the sibling crate (shared, exact Galexie key scheme).
fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../prices-ledger-processor/fixtures/ledgers")
}

fn fixtures_present() -> bool {
    fixtures_dir()
        .join("FC47D9FF--62400000-62463999/FC46ED83--62460540.xdr.zst")
        .exists()
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn discover_window_scans_fixtures_and_advances_cursor() {
    if !fixtures_present() {
        eprintln!("skipping: bundled ledger fixtures not present");
        return;
    }

    let writer = OhlcvWriter::plaintext(&ch_url());
    prices_clickhouse::apply_sql(writer.client(), prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    for table in [
        "prices.assets",
        "prices.discovery_state",
        "prices.pool_registry",
    ] {
        writer
            .client()
            .query(&format!("TRUNCATE TABLE {table}"))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("truncate {table}: {e}"));
    }

    let fetcher = LocalDiskFetcher::new(fixtures_dir());

    // First run scans the 3 contiguous fixtures, then hits a gap.
    let stats = asset_discovery::discover_window(&writer, &fetcher, FIRST_LEDGER, 10)
        .await
        .expect("discover window");
    assert_eq!(
        stats.ledgers_scanned, 3,
        "all 3 contiguous fixtures scanned"
    );
    assert_eq!(
        stats.to_ledger, LAST_LEDGER,
        "cursor reaches the last fixture"
    );
    assert!(
        stats.assets_total > 0,
        "real mainnet fixtures should surface at least one traded asset"
    );

    // The high-water-mark was persisted.
    let cursor = asset_discovery::load_cursor(&writer)
        .await
        .expect("load cursor");
    assert_eq!(
        cursor,
        Some(LAST_LEDGER),
        "discovery_state advanced to the tip"
    );

    // Pool-registry maintenance (task 0069): whatever the scan discovered was
    // persisted to `prices.pool_registry`, and reloading it round-trips exactly
    // the count the run reported. (These SDEX-era fixtures may carry no AMM
    // factory events, so the count can legitimately be 0 — the invariant under
    // test is persist-then-reload consistency, not a specific pool count.)
    let reloaded = writer
        .load_pool_registry()
        .await
        .expect("reload pool registry");
    assert_eq!(
        reloaded.pool_count(),
        stats.pools_total,
        "persisted pool registry must round-trip the reported pools_total"
    );

    // Resuming at cursor+1 finds an immediate gap → no-op (idempotent tail).
    let stats2 = asset_discovery::discover_window(&writer, &fetcher, LAST_LEDGER + 1, 10)
        .await
        .expect("resume window");
    assert_eq!(
        stats2.ledgers_scanned, 0,
        "no ledgers past the fixture window"
    );
    assert_eq!(
        asset_discovery::load_cursor(&writer).await.unwrap(),
        Some(LAST_LEDGER),
        "an empty scan must not move the cursor"
    );
}
