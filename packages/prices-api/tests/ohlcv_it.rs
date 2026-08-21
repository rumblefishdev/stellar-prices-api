//! Live-ClickHouse integration tests for `GET /v1/assets/{id}/ohlcv`. Gated
//! `#[ignore]`:
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-api --test ohlcv_it -- --ignored

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clickhouse::Client;
use prices_api::{AppConfig, AppState, app};
use serde_json::Value;
use tower::ServiceExt;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

fn iss() -> &'static str {
    prices_clickhouse::USDC_ISSUER
}

/// Seed FOO/USDC candles in `price_ohlcv_1h`: bucket T1 has two sources (to
/// exercise the merge), T2 a single source. SDEX backfill = running.
async fn setup(db: &str) -> Client {
    let admin = Client::default().with_url(ch_url());
    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (1, 'XLM', 'native', '', ''), \
             (2, 'USDC', 'credit', '{i}', ''), \
             (3, 'FOO', 'credit', '{i}', '')",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();
    // asset_id=3 (FOO) quoted in asset_id=2 (USDC).
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, vwap, trade_count, version) VALUES \
             ('2026-02-10 10:00:00', 3, 2, 'sdex',     1.0, 1.2, 0.9, 1.1, 100, 110, 1.05, 10, 1), \
             ('2026-02-10 10:00:00', 3, 2, 'soroswap', 1.05, 1.3, 0.95, 1.15, 300, 345, 1.10, 20, 1), \
             ('2026-02-10 11:00:00', 3, 2, 'sdex',     1.1, 1.15, 1.05, 1.12, 50, 56, 1.1, 5, 1)"
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.backfill_progress \
             (task_name, start_ledger, target_ledger, current_ledger, status) VALUES \
             ('sdex_archive', 1, 100, 50, 'running')"
        ))
        .execute()
        .await
        .unwrap();
    Client::default().with_url(ch_url()).with_database(db)
}

async fn teardown(db: &str) {
    let admin = Client::default().with_url(ch_url());
    let _ = admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await;
}

fn config() -> AppConfig {
    AppConfig {
        ch_enabled: false,
        base_url: None,
        api_keys: vec![],
        portal_enabled: false,
        // Sign-in credentials are loaded asynchronously from Secrets Manager
        // (task 0186) and are never part of the environment; `None` is the shape
        // every non-portal test wants.
        portal_oauth: None,
        // Discord endpoints are part of the config now, not read from the
        // process environment per router — see `AppConfig::portal_endpoints`.
        portal_endpoints: Default::default(),
        // Task 0187: the control-plane client for self-service keys. `None`
        // is what every non-portal test wants — with no client in the
        // config there is no code path here that can reach API Gateway.
        portal_keys: None,
        portal_rate_limit: None,
    }
}

async fn get(client: Client, uri: &str) -> (StatusCode, Value) {
    let resp = app(&config(), AppState::new(client))
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn approx(v: &Value, expected: f64) {
    let got: f64 = v.as_str().expect("string number").parse().unwrap();
    assert!(
        (got - expected).abs() < 1e-6,
        "expected ~{expected}, got {got}"
    );
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_merges_sources_and_notes_backfill() {
    let db = "it_ohlcv_merge_0040";
    let client = setup(db).await;

    // Explicit start/end around the seeded candles: since task 0119 the window
    // rule rejects `timeframe=all&granularity=1h` up front (genesis → now is
    // ~95k hourly buckets against the 5000 cap — the old silent-truncation
    // semantics this test used to lean on). `timeframe=all` stays, narrowed by
    // the explicit bounds, so the backfill_note path is still exercised.
    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?timeframe=all&granularity=1h\
         &start=2026-02-01&end=2026-02-15&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["granularity"], "1h");
    assert_eq!(json["base_currency"], "USD");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);

    // Bucket T1 (oldest, merged across sdex + soroswap):
    let c0 = &data[0];
    approx(&c0["high"], 1.3); // max(1.2, 1.3)
    approx(&c0["low"], 0.9); // min(0.9, 0.95)
    approx(&c0["volume_base"], 400.0); // 100 + 300
    approx(&c0["volume_quote_usd"], 455.0); // 110 + 345
    approx(&c0["open"], 1.05); // argMax by volume → soroswap
    approx(&c0["close"], 1.15); // argMax by volume → soroswap
    approx(&c0["vwap"], 1.0875); // (1.05*100 + 1.10*300) / 400
    assert_eq!(c0["trade_count"], 30);

    // backfill_note present (timeframe=all + SDEX running).
    let note = json["backfill_note"].as_str().unwrap();
    assert!(note.contains("2026-02-10"), "note={note}");

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn ohlcv_xlm_quote_has_no_candles() {
    let db = "it_ohlcv_xlm_0040";
    let client = setup(db).await;
    // No FOO/XLM rows were seeded → empty series, no note.
    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?timeframe=all&base_currency=XLM",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["base_currency"], "XLM");
    assert_eq!(json["data"].as_array().unwrap().len(), 0);
    assert!(json["backfill_note"].is_null());
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn ohlcv_invalid_timeframe_is_400() {
    let db = "it_ohlcv_badtf_0040";
    let client = setup(db).await;
    let (status, json) = get(
        client,
        &format!("/v1/assets/FOO:{}/ohlcv?timeframe=99z", iss()),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn ohlcv_unknown_asset_is_404() {
    let db = "it_ohlcv_unknown_0040";
    let client = setup(db).await;
    let (status, _) = get(
        client,
        &format!("/v1/assets/BAR:{}/ohlcv?timeframe=all", iss()),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    teardown(db).await;
}
