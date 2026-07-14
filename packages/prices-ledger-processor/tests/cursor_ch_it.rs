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
//! Destructive to the local `prices.ingest_cursor` table (truncates it); never
//! run against a shared/prod cluster.

use clickhouse::Client;
use prices_ledger_processor::cursor::{ClickHouseCursor, Cursor, CursorError};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

async fn fresh_schema() -> Client {
    let c = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    c.query("TRUNCATE TABLE prices.ingest_cursor")
        .execute()
        .await
        .expect("truncate ingest_cursor");
    c
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn read_errors_before_any_write() {
    let c = fresh_schema().await;
    let cursor = ClickHouseCursor::new(c, "test-consumer");
    // Empty table → read is an error, so main's seed path fires exactly once.
    assert!(matches!(cursor.read().await, Err(CursorError::Read(_))));
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn write_then_read_roundtrips_and_advances() {
    let c = fresh_schema().await;
    let cursor = ClickHouseCursor::new(c, "test-consumer");

    cursor.write(63_352_611).await.expect("seed write");
    assert_eq!(cursor.read().await.unwrap(), 63_352_611);

    // A later, higher write wins (the normal forward advance).
    cursor.write(63_400_000).await.expect("advance write");
    assert_eq!(cursor.read().await.unwrap(), 63_400_000);
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn cursor_survives_a_new_client_instance() {
    // The whole point of 0064: a brand-new client (≙ a recycled Lambda
    // execution environment with a wiped /tmp) reads the SAME persisted value —
    // it does NOT reset to a seed.
    let writer = ClickHouseCursor::new(Client::default().with_url(ch_url()), "ledger-processor");
    {
        let c = fresh_schema().await;
        ClickHouseCursor::new(c, "ledger-processor")
            .write(63_390_000)
            .await
            .expect("persist cursor");
    }

    let resumed = writer.read().await.expect("resume across a new client");
    assert_eq!(
        resumed, 63_390_000,
        "a fresh client must resume the persisted cursor, not rewind"
    );
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn distinct_ids_are_independent() {
    let c = fresh_schema().await;
    ClickHouseCursor::new(c.clone(), "consumer-a")
        .write(100)
        .await
        .unwrap();
    ClickHouseCursor::new(c.clone(), "consumer-b")
        .write(200)
        .await
        .unwrap();

    assert_eq!(
        ClickHouseCursor::new(c.clone(), "consumer-a")
            .read()
            .await
            .unwrap(),
        100
    );
    assert_eq!(
        ClickHouseCursor::new(c, "consumer-b").read().await.unwrap(),
        200
    );
}
