//! Integration test for the `mv_current_prices` refreshable MV (task 0039),
//! against a local Docker ClickHouse:
//!
//!     docker compose up -d clickhouse
//!     cargo test -p prices-clickhouse --test current_mv_it -- --ignored
//!
//! Verifies the MV computes price_usd / volume_24h_usd / market_cap_usd from
//! price_ohlcv_1m + asset_supply, and that a missing supply → market_cap 0
//! (best-effort). Destructive to the local prices.* tables.

use clickhouse::Client;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

async fn scalar_f64(client: &Client, sql: &str) -> f64 {
    client
        .query(sql)
        .fetch_one::<f64>()
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

fn insert_row(asset: u32, close_usd: &str, vol_usd: &str) -> String {
    format!(
        "INSERT INTO prices.price_ohlcv_1m \
         (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
          volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
         VALUES (now(), {asset}, 2, 'sdex', {close_usd}, {close_usd}, {close_usd}, {close_usd}, \
          50, {vol_usd}, {vol_usd}, {close_usd}, {close_usd}, 1, 1)"
    )
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn current_prices_mv_computes_price_volume_and_market_cap() {
    let admin = Client::default().with_url(ch_url());
    prices_clickhouse::apply_init_sql(&admin)
        .await
        .expect("init schema");
    for t in [
        "prices.price_ohlcv_1m",
        "prices.asset_supply",
        "prices.current_prices",
    ] {
        admin
            .query(&format!("TRUNCATE TABLE {t}"))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("truncate {t}: {e}"));
    }

    // Create the refreshable MV (experimental flag for older builds).
    let mv_client = admin
        .clone()
        .with_option("allow_experimental_refreshable_materialized_view", "1");
    prices_clickhouse::apply_sql(&mv_client, prices_clickhouse::CURRENT_SQL)
        .await
        .expect("create mv_current_prices");

    // asset 1 has supply (→ market_cap); asset 3 has none (→ 0).
    admin
        .query("INSERT INTO prices.asset_supply (asset_id, token_supply) VALUES (1, 1000)")
        .execute()
        .await
        .expect("insert supply");
    admin
        .query(&insert_row(1, "2", "100"))
        .execute()
        .await
        .expect("ohlcv 1");
    admin
        .query(&insert_row(3, "5", "40"))
        .execute()
        .await
        .expect("ohlcv 3");

    // Force a refresh and wait for current_prices to populate.
    admin
        .query("SYSTEM REFRESH VIEW prices.mv_current_prices")
        .execute()
        .await
        .expect("refresh view");
    let mut ready = false;
    for _ in 0..30 {
        let n: u64 = admin
            .query("SELECT count() FROM prices.current_prices FINAL")
            .fetch_one()
            .await
            .expect("count current_prices");
        if n >= 2 {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(ready, "MV did not populate current_prices in time");

    let where1 = "FROM prices.current_prices FINAL WHERE asset_id = 1";
    let p = scalar_f64(&admin, &format!("SELECT toFloat64(price_usd) {where1}")).await;
    let v = scalar_f64(
        &admin,
        &format!("SELECT toFloat64(volume_24h_usd) {where1}"),
    )
    .await;
    let m = scalar_f64(
        &admin,
        &format!("SELECT toFloat64(market_cap_usd) {where1}"),
    )
    .await;
    assert!((p - 2.0).abs() < 1e-6, "price_usd should be 2.0, got {p}");
    assert!(
        (v - 100.0).abs() < 1e-6,
        "volume_24h_usd should be 100, got {v}"
    );
    assert!(
        (m - 2000.0).abs() < 1e-6,
        "market_cap = 2 * 1000 = 2000, got {m}"
    );

    let m3 = scalar_f64(
        &admin,
        "SELECT toFloat64(market_cap_usd) FROM prices.current_prices FINAL WHERE asset_id = 3",
    )
    .await;
    assert!(
        m3.abs() < 1e-6,
        "market_cap must be 0 without supply, got {m3}"
    );
}
