//! Live-ClickHouse integration test for `GET /v1/assets/{id}/price`. Gated
//! `#[ignore]` (matches the `prices-clickhouse` integration tests):
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-api --test price_it -- --ignored
//!
//! Each test owns an isolated scratch database (the `prices.*` schema rewritten
//! onto the scratch name) and drops it at the end. The handler's SQL uses
//! unqualified table names, so pointing the client at the scratch db is enough.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clickhouse::Client;
use prices_api::{AppConfig, AppState, app};
use tower::ServiceExt;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// Create + seed an isolated scratch db; return a client scoped to it.
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

    // 1 = native XLM, 2 = USDC classic. Each with a current-price row.
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (1, 'XLM', 'native', '', ''), \
             (2, 'USDC', 'credit', '{issuer}', '')",
            issuer = prices_clickhouse::USDC_ISSUER
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at) VALUES \
             (1, 0.5, 0.51, 1234.5, '2026-02-10 12:00:30'), \
             (2, 1.0001, 1.0002, 999999.25, '2026-02-10 12:00:30')"
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
    }
}

async fn get(client: Client, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app(&config(), AppState::new(client))
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

fn approx(v: &serde_json::Value, expected: f64) {
    let got: f64 = v.as_str().expect("string-typed number").parse().unwrap();
    assert!(
        (got - expected).abs() < 1e-9,
        "expected ~{expected}, got {got}"
    );
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn price_native_returns_seeded_row() {
    let db = "it_price_native_0040";
    let client = setup(db).await;

    let (status, json) = get(client, "/v1/assets/native/price").await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    assert_eq!(json["asset"], "native");
    approx(&json["price_usd"], 0.5);
    approx(&json["vwap_24h"], 0.51);
    approx(&json["volume_24h_usd"], 1234.5);
    assert_eq!(json["updated_at"], "2026-02-10T12:00:30Z");
    // v1 stubs (task 0072):
    assert_eq!(json["price_xlm"], "0");
    assert_eq!(json["change_24h_pct"], "0");
    assert_eq!(json["sources"], serde_json::json!({}));

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn price_classic_resolves_by_code_and_issuer() {
    let db = "it_price_classic_0040";
    let client = setup(db).await;

    let uri = format!("/v1/assets/USDC:{}/price", prices_clickhouse::USDC_ISSUER);
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    approx(&json["price_usd"], 1.0001);

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn price_unknown_asset_is_404() {
    let db = "it_price_unknown_0040";
    let client = setup(db).await;

    // Valid identifier (code + real issuer strkey) that was never seeded.
    let uri = format!("/v1/assets/FOO:{}/price", prices_clickhouse::USDC_ISSUER);
    let (status, _) = get(client, &uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    teardown(db).await;
}
