//! Integration test for task 0064: the durable ClickHouse-backed cursor.
//!
//! Proves the property the freeze bug violated — the cursor **persists across
//! new client instances** (which stand in for Lambda execution-environment
//! recycles), instead of resetting to a seed. Runs against a local Docker
//! ClickHouse:
//!
//!     docker compose up -d clickhouse
//!     cargo test -p prices-ledger-processor --test cursor_ch_it -- --ignored --nocapture
//!
//! Isolation: these run in parallel against one shared `prices.ingest_cursor`
//! table, so each test uses a DISTINCT `id` and never truncates (a global
//! TRUNCATE would race the other tests). Because the RMT version is `ledger`,
//! every assertion is "highest ledger for this id" — stable across re-runs and
//! against a non-empty table, so no clean-slate step is needed. Never point at a
//! shared/prod cluster.

use clickhouse::Client;
use prices_ledger_processor::cursor::{ClickHouseCursor, Cursor, CursorError};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

async fn schema() -> Client {
    let c = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    c
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn empty_id_reads_as_empty_not_read_error() {
    // A never-written id → 0 rows → `Empty` (the first-run seed signal), which is
    // distinct from a `Read` error (a transient/failed query) so main seeds ONLY
    // on emptiness, never on a transient failure that would clobber the cursor.
    let cursor = ClickHouseCursor::new(schema().await, "it-never-written");
    assert!(matches!(cursor.read().await, Err(CursorError::Empty)));
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn write_then_read_roundtrips_and_advances() {
    let cursor = ClickHouseCursor::new(schema().await, "it-advance");
    // Seed, then advance to a higher ledger — FINAL returns the highest.
    cursor.write(63_352_611).await.expect("seed write");
    cursor.write(63_400_000).await.expect("advance write");
    assert_eq!(cursor.read().await.unwrap(), 63_400_000);
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn cursor_survives_a_new_client_instance() {
    // The whole point of 0064: a brand-new client (≙ a recycled Lambda
    // execution environment with a wiped /tmp) reads the SAME persisted value —
    // it does NOT reset to a seed.
    let writer = ClickHouseCursor::new(Client::default().with_url(ch_url()), "it-survive");
    ClickHouseCursor::new(schema().await, "it-survive")
        .write(63_390_000)
        .await
        .expect("persist cursor");

    let resumed = writer.read().await.expect("resume across a new client");
    assert_eq!(
        resumed, 63_390_000,
        "a fresh client must resume the persisted cursor, not rewind"
    );
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn a_lower_write_never_rewinds_the_cursor() {
    // The RMT(ledger) guard: even if a stray lower write lands (e.g. a spurious
    // re-seed to the floor after a transient read error), FINAL keeps the HIGHEST
    // ledger, so the cursor can't rewind. This is the storage-layer backstop
    // behind the main.rs "don't seed on a Read error" logic.
    let cursor = ClickHouseCursor::new(schema().await, "it-rewind-guard");
    cursor.write(63_490_000).await.expect("advance to tip");
    cursor.write(63_352_611).await.expect("stray floor re-seed");
    assert_eq!(
        cursor.read().await.unwrap(),
        63_490_000,
        "a lower write must not rewind the persisted cursor",
    );
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn distinct_ids_are_independent() {
    let c = schema().await;
    ClickHouseCursor::new(c.clone(), "it-consumer-a")
        .write(63_100_000)
        .await
        .unwrap();
    ClickHouseCursor::new(c.clone(), "it-consumer-b")
        .write(63_200_000)
        .await
        .unwrap();

    assert_eq!(
        ClickHouseCursor::new(c.clone(), "it-consumer-a")
            .read()
            .await
            .unwrap(),
        63_100_000
    );
    assert_eq!(
        ClickHouseCursor::new(c, "it-consumer-b")
            .read()
            .await
            .unwrap(),
        63_200_000
    );
}
