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

/// Seed `n` assets + `current_prices` rows for the pagination walk. Each asset
/// gets a unique `asset_code` (`A0001`…) — the response item exposes `asset_code`
/// (not the internal `asset_id`), so it is the identity we assert set-completeness
/// on. Volumes are bucketed into 13 tie-groups (`(asset_id % 13) * 100`): the
/// groups are large (~19 rows) and non-monotonic in `asset_id`, so equal-volume
/// rows both *reorder* relative to id and *straddle* the 50-row page boundary —
/// which is exactly what exercises the cursor's `(sort_col, asset_id)` tie-break.
async fn setup_n(db: &str, n: u32) -> Client {
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

    let mut assets = Vec::with_capacity(n as usize);
    let mut prices = Vec::with_capacity(n as usize);
    for i in 1..=n {
        assets.push(format!(
            "({i}, 'A{i:04}', 'credit', '{iss}', '')",
            iss = iss()
        ));
        let vol = (i % 13) * 100;
        prices.push(format!("({i}, 1.0, 1.0, {vol}, '2026-02-10 12:00:00')"));
    }
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES {}",
            assets.join(", ")
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at) VALUES {}",
            prices.join(", ")
        ))
        .execute()
        .await
        .unwrap();
    Client::default().with_url(ch_url()).with_database(db)
}

/// Percent-encode the reserved Base64 characters in an opaque cursor before it
/// goes into a query string. The cursor is STANDARD Base64 (`common/cursor.rs`),
/// whose alphabet includes `+ / =`; in an `application/x-www-form-urlencoded`
/// query, a raw `+` decodes to a space, corrupting the token → a 400. Encoding
/// these three makes the walk robust to whatever bytes the fixture produces.
fn enc_cursor(c: &str) -> String {
    c.replace('+', "%2B")
        .replace('/', "%2F")
        .replace('=', "%3D")
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
        portal_eligibility: None,
        portal_rate_limit: None,
        portal_web_origin: None,
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

/// Task 0074 — walk the FULL default (`volume_24h DESC`) keyset result set over a
/// 250-row fixture at `limit=50`, following `?cursor` until `has_more == false`,
/// and assert every asset appears **exactly once**: no duplicates, no skips. The
/// fixture packs equal-volume rows into large tie-groups that straddle the page
/// boundary (see [`setup_n`]), so a broken `(sort_col, asset_id)` tie-break would
/// surface here as a dropped or repeated row.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn keyset_pagination_250_rows_no_dup_no_skip() {
    let db = "it_list_paginate_250_0074";
    let _ = setup_n(db, 250).await;

    let mut seen: Vec<String> = Vec::new();
    let mut pages = 0;
    let mut uri = "/v1/assets?limit=50".to_string();
    loop {
        // `get` consumes the client, so build a fresh one per request.
        let client = Client::default().with_url(ch_url()).with_database(db);
        let (status, page) = get(client, &uri).await;
        assert_eq!(status, StatusCode::OK, "page {} body={page}", pages + 1);
        pages += 1;

        let data = page["data"].as_array().unwrap();
        // 250 / 50 = 5 exact pages, so every page (including the last) is full.
        assert_eq!(data.len(), 50, "page {pages} must be a full 50-row page");
        for item in data {
            seen.push(item["asset_code"].as_str().unwrap().to_string());
        }

        if page["has_more"].as_bool().unwrap() {
            let cursor = page["cursor"]
                .as_str()
                .expect("cursor present when has_more");
            uri = format!("/v1/assets?limit=50&cursor={}", enc_cursor(cursor));
        } else {
            assert!(page["cursor"].is_null(), "last page cursor must be null");
            break;
        }
    }

    // Page count: 250 rows at 50/page.
    assert_eq!(pages, 5, "expected exactly 5 pages");
    // No skips (count): every seeded row was returned.
    assert_eq!(seen.len(), 250, "collected 250 rows across the walk");
    // No duplicates: the multiset collapses to 250 distinct codes.
    let got: std::collections::BTreeSet<String> = seen.iter().cloned().collect();
    assert_eq!(
        got.len(),
        250,
        "no asset appeared twice across the cursor walk"
    );
    // No skips (set): the collected set equals the seeded set exactly.
    let expected: std::collections::BTreeSet<String> =
        (1..=250u32).map(|i| format!("A{i:04}")).collect();
    assert_eq!(got, expected, "collected set equals the seeded set");

    teardown(db).await;
}
