//! Live-ClickHouse integration tests for the Phase 2 endpoints (asset detail,
//! batch, oracles, backfill). Gated `#[ignore]`:
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-api --test endpoints_it -- --ignored
//!
//! Each test owns an isolated scratch database, dropped at the end.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clickhouse::Client;
use prices_api::{AppConfig, AppState, app};
use serde_json::{Value, json};
use tower::ServiceExt;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

fn issuer() -> &'static str {
    prices_clickhouse::USDC_ISSUER
}

/// Create + seed a scratch db with assets, current prices, oracle prices, and
/// backfill progress; return a client scoped to it.
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
             (2, 'USDC', 'credit', '{iss}', '')",
            iss = issuer()
        ))
        .execute()
        .await
        .unwrap();
    // home_domain is enrichment — it lives in the single-writer asset_metadata
    // table, not on the assets identity row (task 0067). The read path LEFT JOINs
    // it back in.
    admin
        .query(&format!(
            "INSERT INTO {db}.asset_metadata (asset_id, home_domain) VALUES (2, 'centre.io')"
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
    admin
        .query(&format!(
            "INSERT INTO {db}.oracle_prices \
             (timestamp, asset_id, oracle_name, price_usd, raw_data) VALUES \
             ('2026-02-10 11:55:00', 2, 'reflector', 1.0, ''), \
             ('2026-02-10 11:58:00', 2, 'redstone', 1.0001, '')"
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.backfill_progress \
             (task_name, start_ledger, target_ledger, current_ledger, status, last_push_at, completed_at, earliest_data_available) VALUES \
             ('sdex_archive', 1, 57234198, 34891234, 'running', '2026-06-15 11:30:00', NULL, '2015-11-18 03:47:00'), \
             ('soroban_amm', 0, 0, 0, 'completed', '2026-04-14 08:23:11', '2026-04-14 08:23:11', '2024-02-20 17:00:00')"
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

async fn get(client: Client, uri: &str) -> (StatusCode, Value) {
    send(
        client,
        Request::builder().uri(uri).body(Body::empty()).unwrap(),
    )
    .await
}

async fn post(client: Client, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    send(client, req).await
}

async fn send(client: Client, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app(&config(), AppState::new(client))
        .oneshot(req)
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, json)
}

fn approx(v: &Value, expected: f64) {
    let got: f64 = v.as_str().expect("string-typed number").parse().unwrap();
    assert!(
        (got - expected).abs() < 1e-9,
        "expected ~{expected}, got {got}"
    );
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn asset_detail_native() {
    let db = "it_ep_detail_0040";
    let client = setup(db).await;
    let (status, json) = get(client, "/v1/assets/native").await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["asset"], "native");
    assert_eq!(json["asset_kind"], "native");
    assert_eq!(json["code"], "XLM");
    assert_eq!(json["is_active"], true);
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn asset_detail_returns_home_domain_from_metadata() {
    // Task 0067: home_domain is served from the asset_metadata LEFT JOIN, not the
    // assets identity row. The fixture only seeds it in asset_metadata.
    let db = "it_ep_detail_hd_0067";
    let client = setup(db).await;
    let (status, json) = get(client, &format!("/v1/assets/USDC:{}", issuer())).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["code"], "USDC");
    assert_eq!(
        json["home_domain"], "centre.io",
        "home_domain must be joined in from asset_metadata"
    );
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn asset_detail_unknown_is_404() {
    let db = "it_ep_detail_unknown_0040";
    let client = setup(db).await;
    let (status, _) = get(client, &format!("/v1/assets/FOO:{}", issuer())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn batch_returns_found_and_not_found() {
    let db = "it_ep_batch_0040";
    let client = setup(db).await;
    let body =
        json!({ "assets": ["native", format!("USDC:{}", issuer()), format!("FOO:{}", issuer())] });
    let (status, json) = post(client, "/v1/prices/batch", body).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["prices"].as_array().unwrap().len(), 2);
    assert_eq!(json["not_found"].as_array().unwrap().len(), 1);
    assert_eq!(json["not_found"][0], format!("FOO:{}", issuer()));
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn batch_empty_is_400() {
    let db = "it_ep_batch_empty_0040";
    let client = setup(db).await;
    let (status, json) = post(client, "/v1/prices/batch", json!({ "assets": [] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn oracles_returns_latest_per_name() {
    let db = "it_ep_oracles_0040";
    let client = setup(db).await;
    let (status, json) = get(client, &format!("/v1/oracles/USDC:{}", issuer())).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let oracles = json["oracles"].as_array().unwrap();
    assert_eq!(oracles.len(), 2);
    // ORDER BY oracle_name → redstone, reflector.
    assert_eq!(oracles[0]["name"], "redstone");
    assert_eq!(oracles[1]["name"], "reflector");
    approx(&oracles[1]["price_usd"], 1.0);
    assert_eq!(oracles[1]["updated_at"], "2026-02-10T11:55:00Z");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn oracles_unknown_asset_is_404() {
    let db = "it_ep_oracles_unknown_0040";
    let client = setup(db).await;
    let (status, _) = get(client, &format!("/v1/oracles/FOO:{}", issuer())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn backfill_status_maps_both_streams() {
    let db = "it_ep_backfill_0040";
    let client = setup(db).await;
    let (status, json) = get(client, "/v1/backfill/status").await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    assert_eq!(json["realtime_tip_ledger"], 57234198u64);
    assert_eq!(json["sdex"]["status"], "running");
    assert_eq!(json["sdex"]["current_ledger"], 34891234u64);
    // remaining = target - current = 57234198 - 34891234
    assert_eq!(json["sdex"]["ledgers_remaining"], 22342964u64);
    assert_eq!(json["sdex"]["last_push_at"], "2026-06-15T11:30:00Z");
    // earliest_data_available = oldest OHLCV row this stream has landed (AC 6)
    assert_eq!(
        json["sdex"]["earliest_data_available"],
        "2015-11-18T03:47:00Z"
    );
    // done = (current - start) / (target - start) * 100
    //      = (34891234 - 1) / (57234198 - 1) * 100 ≈ 60.96
    let pct = json["sdex"]["progress_pct"].as_f64().unwrap();
    assert!((pct - 60.96).abs() < 0.1, "pct={pct}");

    assert_eq!(json["soroban_amm"]["status"], "completed");
    assert_eq!(json["soroban_amm"]["completed_at"], "2026-04-14T08:23:11Z");
    assert_eq!(
        json["soroban_amm"]["earliest_data_available"],
        "2024-02-20T17:00:00Z"
    );
    teardown(db).await;
}
