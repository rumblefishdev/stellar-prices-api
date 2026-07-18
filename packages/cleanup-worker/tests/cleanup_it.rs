//! Integration test for the cleanup worker (task 0039) against a local Docker
//! ClickHouse with the `prices` schema:
//!
//!     docker compose up -d clickhouse
//!     cargo test -p cleanup-worker --test cleanup_it -- --ignored
//!
//! Destructive to the local `prices.price_ohlcv_1m` table — never run against
//! a shared/prod cluster.

use clickhouse::Client;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

async fn insert_1m_row(client: &Client, ts_expr: &str) {
    // volume_quote_usd / close_usd have DEFAULT 0; the rest are provided.
    let q = format!(
        "INSERT INTO prices.price_ohlcv_1m \
         (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
          volume_base, volume_quote, vwap, trade_count, version) \
         VALUES ({ts_expr}, 1, 2, 'test', 1, 1, 1, 1, 1, 1, 1, 1, 1)"
    );
    client.query(&q).execute().await.expect("insert row");
}

async fn count_in_month(client: &Client, yyyymm: &str) -> u64 {
    client
        .query(&format!(
            "SELECT count() FROM prices.price_ohlcv_1m WHERE toYYYYMM(timestamp) = {yyyymm}"
        ))
        .fetch_one::<u64>()
        .await
        .expect("count")
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn drops_expired_partitions_keeps_recent() {
    let client = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&client, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    client
        .query("TRUNCATE TABLE prices.price_ohlcv_1m")
        .execute()
        .await
        .expect("truncate");

    // One row in a long-past month (well beyond the 7-day 1m window) and one now.
    insert_1m_row(&client, "toDateTime('2020-01-15 00:00:00')").await;
    insert_1m_row(&client, "now()").await;
    assert_eq!(
        count_in_month(&client, "202001").await,
        1,
        "old row present pre-cleanup"
    );

    let stats = cleanup_worker::run_cleanup(&client).await.expect("cleanup");
    assert!(
        stats.dropped.iter().any(|d| d == "price_ohlcv_1m=202001"),
        "the 2020-01 partition should be dropped, got {:?}",
        stats.dropped
    );

    // Old month gone, current month retained.
    assert_eq!(
        count_in_month(&client, "202001").await,
        0,
        "expired partition dropped"
    );
    let now_month: u64 = client
        .query(
            "SELECT count() FROM prices.price_ohlcv_1m WHERE toYYYYMM(timestamp) = toYYYYMM(now())",
        )
        .fetch_one()
        .await
        .expect("count now");
    assert!(now_month >= 1, "current-month data must be retained");

    // Idempotent: a second run drops nothing.
    let again = cleanup_worker::run_cleanup(&client)
        .await
        .expect("cleanup #2");
    assert!(
        !again.dropped.iter().any(|d| d == "price_ohlcv_1m=202001"),
        "second run must be a no-op for the already-dropped partition"
    );
}
