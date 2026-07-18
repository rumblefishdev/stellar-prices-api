//! Integration test for per-source candle coexistence + idempotency (task 0053,
//! Step 6), against a local Docker ClickHouse. Proves that candles for the same
//! (asset, quote, minute) written under different `source`s coexist in
//! `prices.price_ohlcv_1m` (source is part of the ReplacingMergeTree key), and
//! that re-writing the same candle is idempotent — the "re-run → count FINAL
//! stable" acceptance criterion — through the real backfill `Sink` write path.
//!
//!     docker compose up -d clickhouse
//!     cargo test -p sdex-backfill --test candles_it -- --ignored --nocapture
//!
//! Destructive to local `prices.price_ohlcv_1m` (truncates it); never run
//! against a shared/prod cluster.

use clickhouse::Client;
use prices_ingest_core::{CandleAccumulator, OhlcvCandle, TradeTick};
use rust_decimal::Decimal;
use sdex_backfill::sink::Sink;

// Two timestamps in distinct minutes (floor(ts/60)*60).
const T_M0: i64 = 1_700_000_000; // minute 1_699_999_980
const T_M1: i64 = 1_700_000_100; // minute 1_700_000_100

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// One flushed candle for (asset, quote) at the minute containing `closed_at`.
fn candle(asset: u32, quote: u32, closed_at: i64, ledger: u32) -> Vec<OhlcvCandle> {
    let mut acc = CandleAccumulator::new();
    acc.merge(&TradeTick {
        ledger_sequence: ledger,
        closed_at,
        operation_index: 0,
        claim_index: 0,
        base_id: asset,
        quote_id: quote,
        price: Decimal::from(10),
        volume_base: Decimal::from(1),
        volume_quote: Decimal::from(10),
    });
    acc.flush_all()
}

async fn count(c: &Client, where_sql: &str) -> u64 {
    c.query(&format!(
        "SELECT count() FROM prices.price_ohlcv_1m FINAL WHERE {where_sql}"
    ))
    .fetch_one::<u64>()
    .await
    .expect("count")
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn per_source_candles_coexist_and_rewrites_are_idempotent() {
    let c = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    c.query("TRUNCATE TABLE prices.price_ohlcv_1m")
        .execute()
        .await
        .expect("truncate price_ohlcv_1m");

    let sink = Sink::new(&ch_url());
    let pair = "asset_id = 1 AND quote_asset_id = 2";

    // Same (asset, quote, minute) under two different sources → two rows: source
    // is part of the RMT key, so they must not collapse into one.
    sink.write_candles(&candle(1, 2, T_M0, 100), "sdex")
        .await
        .expect("write sdex");
    sink.write_candles(&candle(1, 2, T_M0, 100), "phoenix")
        .await
        .expect("write phoenix");

    assert_eq!(
        count(&c, pair).await,
        2,
        "sdex + phoenix coexist for one minute"
    );
    let distinct_sources: u64 = c
        .query(&format!(
            "SELECT uniqExact(source) FROM prices.price_ohlcv_1m FINAL WHERE {pair}"
        ))
        .fetch_one()
        .await
        .expect("distinct sources");
    assert_eq!(distinct_sources, 2, "both sources present");

    // Idempotent re-write of the identical sdex candle (same key + version) →
    // ReplacingMergeTree collapses it; the count does not grow.
    sink.write_candles(&candle(1, 2, T_M0, 100), "sdex")
        .await
        .expect("re-write sdex");
    assert_eq!(count(&c, pair).await, 2, "re-run must not duplicate rows");

    // A second minute for the same source is a distinct key → a new row.
    sink.write_candles(&candle(1, 2, T_M1, 101), "sdex")
        .await
        .expect("write sdex minute 2");
    assert_eq!(count(&c, pair).await, 3, "distinct minute adds one row");
}
