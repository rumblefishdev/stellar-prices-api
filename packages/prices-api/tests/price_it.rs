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
    // asset 1 carries the task-0072 columns populated (the mv_current_prices
    // shape); asset 2 deliberately leaves them at their table DEFAULTs, so both
    // the pass-through path and the empty-producer path are covered.
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct, \
              vwap_24h, volume_24h_usd, sources, updated_at, method) VALUES \
             (1, 0.5, 1.25, -2.5, 7.25, 0.51, 1234.5, \
              '{{\"sdex\":{{\"price\":\"0.5\",\"volume_24h\":\"1000\"}}}}', \
              '2026-02-10 12:00:30', 'traded'), \
             (2, 1.0001, 0, 0, 0, 1.0002, 999999.25, '', '2026-02-10 12:00:30', '')"
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
        portal_eligibility: None,
        portal_rate_limit: None,
        portal_web_origin: None,
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

    // Task 0072 — these three were hardcoded stubs ("0"/"0"/{}) until the MV
    // materialized them. They now PASS THROUGH from current_prices, so the
    // assertions must be seeded values, not constants: a test asserting "0"
    // would pass just as happily against a handler that still hardcodes it.
    approx(&json["price_xlm"], 1.25);
    approx(&json["change_24h_pct"], -2.5);
    assert_eq!(
        json["sources"],
        serde_json::json!({"sdex": {"price": "0.5", "volume_24h": "1000"}}),
        "sources must be parsed from the MV's JSON string into an object"
    );

    teardown(db).await;
}

/// The producer has not written `sources` yet (table DEFAULT `''`). The response
/// must still be a well-formed empty object — never `null`, never a 500. This is
/// also the live shape for an exotic-quote asset, where no source has a
/// USD-priceable close (~62% of candles per task 0114).
#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn price_empty_sources_degrades_to_empty_object() {
    let db = "it_price_empty_sources_0072";
    let client = setup(db).await;

    let uri = format!("/v1/assets/USDC:{}/price", prices_clickhouse::USDC_ISSUER);
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["sources"], serde_json::json!({}));
    assert!(json["sources"].is_object(), "must be {{}}, not null");

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

/// Task 0118 — the `?min_volume_usd=` override, end to end over a seeded
/// three-source row: byte-identical bodies at/below the system default on a
/// FUNDED asset, narrowing above it, and the all-excluded sentinels.
///
/// The byte-identity here is a consequence, not a pass-through band: every
/// venue on this asset already clears $100 (the MV applied that cut), so the
/// strict filter finds nothing to drop. `min_volume_usd_cuts_an_all_dust_asset`
/// below covers the asset where the two differ.
#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn price_min_volume_override_narrows_sources_and_reweights() {
    let db = "it_price_min_volume_0118";
    let client = setup(db).await;

    // A third asset with three sources: $100k, $20k, and a $150 dust venue
    // quoting an absurd 5. The seeded vwap is what the MV (at the $100 system
    // default) would publish: (1*100000 + 1.02*20000 + 5*150) / 120150.
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (3, 'MULTI', 'credit', '{issuer}', '')",
            issuer = prices_clickhouse::USDC_ISSUER
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct, \
              vwap_24h, volume_24h_usd, sources, updated_at) VALUES \
             (3, 5, 0, 0, 0, 1.00832292967124, 120150, \
              '{{\"sdex\":{{\"price\":\"1\",\"volume_24h\":\"100000\"}},\
                 \"aquarius\":{{\"price\":\"1.02\",\"volume_24h\":\"20000\"}},\
                 \"soroswap\":{{\"price\":\"5\",\"volume_24h\":\"150\"}}}}', \
              '2026-02-10 12:00:30')"
        ))
        .execute()
        .await
        .unwrap();

    let base = format!("/v1/assets/MULTI:{}/price", prices_clickhouse::USDC_ISSUER);
    let raw = |uri: String| {
        let client = client.clone();
        async move {
            let resp = app(&config(), AppState::new(client))
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            let status = resp.status();
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            (status, bytes)
        }
    };

    // AC: on a funded asset the body is byte-identical whether the param is
    // omitted, set to the system default, or below it — the MV already made
    // that cut, so the strict filter drops nothing and never reformats the
    // producer's Decimal strings.
    let (s0, b0) = raw(base.clone()).await;
    assert_eq!(s0, StatusCode::OK);
    for suffix in ["?min_volume_usd=100", "?min_volume_usd=0"] {
        let (s, b) = raw(format!("{base}{suffix}")).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(
            b, b0,
            "{suffix} must be byte-identical to the param-less response"
        );
    }

    // AC: a higher value provably narrows the source set and reweights.
    let (s2, b2) = raw(format!("{base}?min_volume_usd=2000")).await;
    assert_eq!(s2, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&b2).unwrap();
    assert!(
        json["sources"].get("soroswap").is_none(),
        "the $150 venue must be excluded, got {}",
        json["sources"]
    );
    assert!(
        json["sources"].get("sdex").is_some() && json["sources"].get("aquarius").is_some(),
        "got {}",
        json["sources"]
    );
    approx(&json["vwap_24h"], 120_400.0 / 120_000.0);
    // The threshold is a weighting rule only.
    approx(&json["price_usd"], 5.0);
    approx(&json["volume_24h_usd"], 120_150.0);

    // Everything excluded → the MV's own sentinels for that shape.
    let (s3, b3) = raw(format!("{base}?min_volume_usd=1000000")).await;
    assert_eq!(s3, StatusCode::OK);
    let json: serde_json::Value = serde_json::from_slice(&b3).unwrap();
    assert_eq!(json["sources"], serde_json::json!({}));
    assert_eq!(json["vwap_24h"], "0");

    teardown(db).await;
}

/// Task 0118, code review 2026-08-27 — an explicit `?min_volume_usd=` filters
/// STRICTLY even at the system default.
///
/// The MV applies its $100 default conditionally, so an asset whose every
/// venue is dust keeps those venues in `sources`. A caller who asks for
/// `min_volume_usd=100` must not be handed a $50 venue back: before this was
/// fixed the handler treated `<= 100` as a pass-through and did exactly that,
/// while `100.01` emptied the object — a cliff at the documented default.
#[tokio::test]
#[ignore = "requires a local ClickHouse"]
async fn price_min_volume_cuts_an_all_dust_asset_at_the_system_default() {
    let db = "it_price_min_volume_dust_0118";
    let client = setup(db).await;

    // Two venues, $50 and $30: below the threshold, but present because the
    // producer's conditional arm keeps them (nothing funded to defend).
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (4, 'DUST', 'credit', '{issuer}', '')",
            issuer = prices_clickhouse::USDC_ISSUER
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct, \
              vwap_24h, volume_24h_usd, sources, updated_at) VALUES \
             (4, 3, 0, 0, 0, 3.375, 80, \
              '{{\"sdex\":{{\"price\":\"3\",\"volume_24h\":\"50\"}},\
                 \"soroswap\":{{\"price\":\"4\",\"volume_24h\":\"30\"}}}}', \
              '2026-02-10 12:00:30')"
        ))
        .execute()
        .await
        .unwrap();

    let base = format!("/v1/assets/DUST:{}/price", prices_clickhouse::USDC_ISSUER);

    // Without the param: the conditional default keeps both dust venues.
    let (s0, j0) = get(client.clone(), &base).await;
    assert_eq!(s0, StatusCode::OK, "body={j0}");
    assert!(
        j0["sources"].get("sdex").is_some() && j0["sources"].get("soroswap").is_some(),
        "precondition: the MV's conditional arm must keep the dust venues, got {}",
        j0["sources"]
    );

    // With an explicit 100: strict, so both are cut and the row lands on the
    // MV's own sentinels for "no source qualified".
    let (s1, j1) = get(client.clone(), &format!("{base}?min_volume_usd=100")).await;
    assert_eq!(s1, StatusCode::OK, "body={j1}");
    assert_eq!(
        j1["sources"],
        serde_json::json!({}),
        "an explicit threshold must filter strictly even at the system default"
    );
    assert_eq!(j1["vwap_24h"], "0");

    // Untouched by the threshold, which is a weighting rule only.
    approx(&j1["price_usd"], 3.0);
    approx(&j1["volume_24h_usd"], 80.0);

    teardown(db).await;
}

/// Task 0178 — `method` must reach the wire on `/price`, and it must be the
/// SEEDED value rather than anything the handler invents. Without this column a
/// consumer cannot tell a measured 1.0000 from a filled one, which is the whole
/// reason it exists.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn price_surfaces_the_provenance_method() {
    let db = "it_price_method_0178";
    let client = setup(db).await;

    // A third asset priced from the oracle arm, as canonical USDC is on prod.
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) \
             VALUES (3, 'ORC', 'credit', '{issuer}', '')",
            issuer = prices_clickhouse::USDC_ISSUER
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct, \
              vwap_24h, volume_24h_usd, sources, updated_at, method) VALUES \
             (3, 0.9993, 0, 0, 0, 0, 12000, '{{}}', '2026-02-10 12:00:30', 'oracle')"
        ))
        .execute()
        .await
        .unwrap();

    // A traded asset reports 'traded'.
    let (status, json) = get(client.clone(), "/v1/assets/native/price").await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["method"], "traded");

    // The oracle-priced asset reports 'oracle' — distinguishable from a real
    // aggregate at the same price.
    // Identity is (code, issuer), so ORC at this issuer is a distinct asset from
    // USDC at the same one — a real strkey is required, the handler validates.
    let (status, json) = get(
        client.clone(),
        &format!("/v1/assets/ORC:{}/price", prices_clickhouse::USDC_ISSUER),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["method"], "oracle");
    approx(&json["price_usd"], 0.9993);

    // The producer has written no method: the '' sentinel must survive to the
    // wire as an empty string, never as null and never coerced to 'traded'.
    let (status, json) = get(
        client,
        &format!("/v1/assets/USDC:{}/price", prices_clickhouse::USDC_ISSUER),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["method"], "");

    teardown(db).await;
}
