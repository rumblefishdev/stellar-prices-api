//! Live-ClickHouse integration tests for `GET /v1/assets` (listing). Gated
//! `#[ignore]`:
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-api --test list_it -- --ignored

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

/// Seed 4 assets with distinct 24h volumes:
///   USDC=3000, token(soroban)=2000, XLM=1000, FOO=500.
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
             (3, '', 'contract', '', 'CCONTRACTTOKEN'), \
             (4, 'FOO', 'credit', '{i}', '')",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at) VALUES \
             (1, 0.5, 0.5, 1000, '2026-02-10 12:00:00'), \
             (2, 1.0, 1.0, 3000, '2026-02-10 12:00:00'), \
             (3, 2.0, 2.0, 2000, '2026-02-10 12:00:00'), \
             (4, 9.0, 9.0, 500,  '2026-02-10 12:00:00')"
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

#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn default_sort_volume_desc_paginates() {
    let db = "it_list_paginate_0040";
    let client = setup(db).await;

    // Page 1: top 2 by volume desc → USDC(3000), token(2000).
    let (status, page1) = get(client, "/v1/assets?limit=2").await;
    assert_eq!(status, StatusCode::OK, "body={page1}");
    let d1 = page1["data"].as_array().unwrap();
    assert_eq!(d1.len(), 2);
    assert_eq!(d1[0]["asset_code"], "USDC");
    assert_eq!(d1[1]["asset_type"], "soroban");
    assert_eq!(page1["has_more"], true);
    let cursor = page1["cursor"].as_str().unwrap().to_string();

    // Page 2: XLM(1000), FOO(500); no more.
    let client = Client::default().with_url(ch_url()).with_database(db);
    let (status, page2) = get(client, &format!("/v1/assets?limit=2&cursor={cursor}")).await;
    assert_eq!(status, StatusCode::OK, "body={page2}");
    let d2 = page2["data"].as_array().unwrap();
    assert_eq!(d2.len(), 2);
    assert_eq!(d2[0]["asset_code"], "XLM");
    assert_eq!(d2[1]["asset_code"], "FOO");
    assert_eq!(page2["has_more"], false);
    assert!(page2["cursor"].is_null());

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn filter_by_type() {
    let db = "it_list_filter_0040";
    let client = setup(db).await;

    let (_, soroban) = get(client, "/v1/assets?type=soroban").await;
    assert_eq!(soroban["data"].as_array().unwrap().len(), 1);
    assert_eq!(soroban["data"][0]["asset_type"], "soroban");

    let client = Client::default().with_url(ch_url()).with_database(db);
    let (_, classic) = get(client, "/v1/assets?type=classic").await;
    assert_eq!(classic["data"].as_array().unwrap().len(), 3);

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn search_prefix() {
    let db = "it_list_search_0040";
    let client = setup(db).await;
    let (_, json) = get(client, "/v1/assets?search=US").await;
    let d = json["data"].as_array().unwrap();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0]["asset_code"], "USDC");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn invalid_sort_is_400() {
    let db = "it_list_badsort_0040";
    let client = setup(db).await;
    let (status, json) = get(client, "/v1/assets?sort=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    teardown(db).await;
}
