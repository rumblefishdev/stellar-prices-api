//! Live-ClickHouse integration tests for the Phase 2 endpoints (asset detail,
//! batch, oracles, backfill). Gated `#[ignore]`:
//!
//!   tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!   cargo test -p prices-api --test endpoints_it -- --ignored --test-threads=1
//!
//! Each test owns an isolated scratch database, dropped at the end.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clickhouse::Client;
use prices_api::{AppConfig, AppState, app};
use prices_clickhouse::asset_id::fixture::{AssetFixture, assets_insert};
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

/// XLM's SAC, stored on the XLM row as prod holds it (task 0242).
const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
const XCR_ISSUER: &str = "GBLJBHWVORDFI4J7CLBDRPECMYT3XO5S6GERXGC74VXOJMZPLI6ZU3S7";
const XCR_SAC: &str = "CDJQXBQO5ICVQUPHZHW7SHOM56K2UNNPPAIXUUSA3XACEI6Q4JQLXNVI";

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

    let assets = [
        AssetFixture::new("XLM", "native", "", "").with_sac(XLM_SAC),
        AssetFixture::new("USDC", "credit", issuer(), ""),
    ];
    admin
        .query(&assets_insert(db, &assets))
        .execute()
        .await
        .unwrap();
    let [xlm, usdc] = assets.map(|a| a.id());
    // home_domain is enrichment — it lives in the single-writer asset_metadata
    // table, not on the assets identity row (task 0067). The read path LEFT JOINs
    // it back in.
    admin
        .query(&format!(
            "INSERT INTO {db}.asset_metadata (asset_id, home_domain) VALUES ({usdc}, 'centre.io')"
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            // Task 0216: XLM carries a REAL as_of/price_status pair dated
            // behind its updated_at.
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at, as_of, price_status, \
              price_basis) \
             VALUES \
             ({xlm}, 0.5, 0.51, 1234.5, '2026-02-10 12:00:30', '2026-02-10 11:30:00', 'carried', \
              'offer_dust')"
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            // USDC names neither new column, so it really takes the table
            // DEFAULT pair (the epoch and ''), which the wire must render as
            // ""/"" — not a hand-written copy of those values.
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at) \
             VALUES \
             ({usdc}, 1.0001, 1.0002, 999999.25, '2026-02-10 12:00:30')"
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.oracle_prices \
             (timestamp, asset_id, oracle_name, price_usd, raw_data) VALUES \
             ('2026-02-10 11:55:00', {usdc}, 'reflector', 1.0, ''), \
             ('2026-02-10 11:58:00', {usdc}, 'redstone', 1.0001, '')"
        ))
        .execute()
        .await
        .unwrap();
    // `INSERT … SELECT` so `now()` evaluates (the idiom of
    // backfill-freshness-probe/tests/freshness_it.rs). The running stream's
    // `last_push_at` is now()-relative: `/v1/backfill/status` reports a
    // `running` stream whose last push is older than 7 days as `stalled`, and
    // the literal this seed used to carry ('2026-06-15 11:30:00') aged past that
    // threshold and turned `backfill_status_maps_both_streams` red (task 0275).
    // Every other value is a literal on purpose — none of them is compared
    // against the clock. Siblings checked for the same rot by reading what
    // their seeds compare against `now()`, not by waiting: `ohlcv_it` seeds no
    // `last_push_at` (NULL is never stalled), prices-clickhouse `views_it` uses
    // year 2096, enrichment-worker `ch_enrich_it` is all now()-relative.
    admin
        .query(&format!(
            "INSERT INTO {db}.backfill_progress \
             (task_name, start_ledger, target_ledger, current_ledger, status, last_push_at, completed_at, earliest_data_available) \
             SELECT 'sdex_archive', 1, 57234198, 34891234, 'running', \
                    toDateTime(now() - INTERVAL 1 DAY), CAST(NULL AS Nullable(DateTime)), toDateTime('2015-11-18 03:47:00') \
             UNION ALL \
             SELECT 'soroban_amm', 0, 0, 0, 'completed', \
                    toDateTime('2026-04-14 08:23:11'), toDateTime('2026-04-14 08:23:11'), toDateTime('2024-02-20 17:00:00')"
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
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
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
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
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

/// A valid Soroban C-strkey. The detail route parses the identifier through
/// `stellar_strkey::Contract`, so a placeholder like list_it's `CCONTRACTTOKEN`
/// would 400 before reaching the query.
fn contract() -> String {
    stellar_strkey::Contract([9u8; 32]).to_string()
}

/// Seed a Soroban asset plus, optionally, its resolved symbol.
async fn seed_soroban(db: &str, symbol: Option<&str>) {
    let admin = Client::default().with_url(ch_url());
    admin
        .query(&assets_insert(
            db,
            &[AssetFixture::new("", "contract", "", &contract())],
        ))
        .execute()
        .await
        .unwrap();
    if let Some(symbol) = symbol {
        admin
            .query(&format!(
                "INSERT INTO {db}.asset_symbol (contract_address, symbol) VALUES ('{c}', '{symbol}')",
                c = contract()
            ))
            .execute()
            .await
            .unwrap();
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn asset_detail_returns_soroban_symbol_as_code() {
    // Task 0210. The symbol is served from the asset_symbol LEFT JOIN, not the
    // assets identity row — the same single-writer shape 0067 gave home_domain,
    // but keyed on `contract_address` because 10 of the 52 soroban rows share an
    // `asset_id` with another row (0139).
    let db = "it_ep_detail_symbol_0210";
    let client = setup(db).await;
    seed_soroban(db, Some("SolvBTC")).await;

    let (status, json) = get(client, &format!("/v1/assets/{}", contract())).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["code"], "SolvBTC");
    assert_eq!(json["contract"], contract());
    assert_eq!(json["issuer"], "");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn asset_detail_unresolved_soroban_code_is_empty() {
    // No asset_symbol row: the join misses and `code` stays `""`, which is the
    // pre-0210 behaviour. Consumers must not see a partially-composed value.
    let db = "it_ep_detail_symbol_miss_0210";
    let client = setup(db).await;
    seed_soroban(db, None).await;

    let (status, json) = get(client, &format!("/v1/assets/{}", contract())).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["code"], "");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn asset_detail_unknown_is_404() {
    let db = "it_ep_detail_unknown_0040";
    let client = setup(db).await;
    let (status, _) = get(client, &format!("/v1/assets/FOO:{}", issuer())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn asset_detail_unknown_contract_is_404() {
    let db = "it_ep_detail_unknown_c_0242";
    let client = setup(db).await;
    let (status, _) = get(client, &format!("/v1/assets/{}", contract())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    teardown(db).await;
}

/// Seed XCR as a classic row carrying its SAC, priced at 0.25. `pre_heal` adds
/// the second identity task 0242 heals: a Contract row for the SAC, priced at
/// 9.0, so a lookup that reached it would show.
async fn seed_xcr(db: &str, pre_heal: bool) {
    let admin = Client::default().with_url(ch_url());
    let classic = AssetFixture::new("XCR", "credit", XCR_ISSUER, "").with_sac(XCR_SAC);
    let sac = AssetFixture::new("", "contract", "", XCR_SAC);
    let usdc = AssetFixture::new("USDC", "credit", issuer(), "").id();
    let mut rows = vec![classic];
    let mut prices = vec![format!(
        "({}, 0.25, 0.25, 10, '2026-02-10 12:00:30')",
        classic.id()
    )];
    // One XCR/USDC hour per identity, `close_usd = close` (par quote).
    let candle = |id: String, px: f64, trades: u32| {
        format!(
            "('2026-02-10 12:00:00', {id}, {usdc}, 'soroswap', {px}, {px}, {px}, {px}, \
             10, 10, {px}, {px}, {trades}, 1)"
        )
    };
    let mut candles = vec![candle(classic.id(), 0.25, 7)];
    if pre_heal {
        rows.push(sac);
        prices.push(format!(
            "({}, 9.0, 9.0, 1, '2026-02-10 12:00:30')",
            sac.id()
        ));
        candles.push(candle(sac.id(), 9.0, 1));
    }
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES {}",
            candles.join(", ")
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&assets_insert(db, &rows))
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
}

/// Task 0242 (D5): a classic asset's SAC address answers as the classic, and
/// the classic wins over a pre-heal Contract row for the same address.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn sac_address_answers_as_its_classic_asset() {
    let xcr = format!("XCR:{XCR_ISSUER}");
    for (db, pre_heal) in [
        ("it_ep_sac_alias_0242", false),
        ("it_ep_sac_alias_pre_heal_0242", true),
    ] {
        let client = setup(db).await;
        seed_xcr(db, pre_heal).await;

        let (status, json) = get(client.clone(), &format!("/v1/assets/{XCR_SAC}")).await;
        assert_eq!(status, StatusCode::OK, "{db}: body={json}");
        assert_eq!(json["asset"], xcr, "{db}");
        assert_eq!(json["asset_kind"], "credit", "{db}");
        assert_eq!(json["code"], "XCR", "{db}");
        assert_eq!(json["issuer"], XCR_ISSUER, "{db}");
        assert_eq!(json["contract"], "", "{db}");

        let (status, json) = get(client.clone(), &format!("/v1/assets/{XCR_SAC}/price")).await;
        assert_eq!(status, StatusCode::OK, "{db}: body={json}");
        assert_eq!(json["asset"], xcr, "{db}");
        approx(&json["price_usd"], 0.25);

        let (status, json) = get(client.clone(), &format!("/v1/oracles/{XCR_SAC}")).await;
        assert_eq!(status, StatusCode::OK, "{db}: body={json}");
        assert_eq!(json["asset"], xcr, "{db}");

        // The classic's series only: without the alias the clean case is 404
        // and the pre-heal case answers the SAC row's 9.0 candle.
        let uri = format!(
            "/v1/assets/{XCR_SAC}/ohlcv?granularity=1h\
             &start=2026-02-10T00:00:00Z&end=2026-02-11T00:00:00Z&base_currency=USD"
        );
        let (status, json) = get(client, &uri).await;
        assert_eq!(status, StatusCode::OK, "{db}: body={json}");
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "{db}: body={json}");
        approx(&data[0]["close"], 0.25);
        assert_eq!(data[0]["trade_count"], 7, "{db}");

        teardown(db).await;
    }
}

/// XLM's SAC answers as `native`, off the stored `sac_address` alone.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn xlm_sac_address_answers_as_native() {
    let db = "it_ep_xlm_sac_0242";
    let client = setup(db).await;

    let (status, json) = get(client.clone(), &format!("/v1/assets/{XLM_SAC}")).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["asset"], "native");
    assert_eq!(json["asset_kind"], "native");
    assert_eq!(json["code"], "XLM");
    assert_eq!(json["is_active"], true);

    let (status, json) = get(client, &format!("/v1/assets/{XLM_SAC}/price")).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["asset"], "native");
    approx(&json["price_usd"], 0.5);

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn batch_answers_sac_addresses_as_their_classic_assets() {
    let db = "it_ep_batch_sac_0242";
    let client = setup(db).await;
    seed_xcr(db, false).await;

    let body = json!({ "assets": [XCR_SAC, XLM_SAC] });
    let (status, json) = post(client, "/v1/prices/batch", body).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let assets: Vec<&str> = json["prices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["asset"].as_str().unwrap())
        .collect();
    assert_eq!(assets, [format!("XCR:{XCR_ISSUER}").as_str(), "native"]);
    assert_eq!(json["not_found"], json!([]));

    teardown(db).await;
}

/// Task 0242: an unpriced SAC is listed in `not_found` as the address the
/// client sent, not as its classic.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn batch_not_found_echoes_the_sac_address_asked() {
    let db = "it_ep_batch_sac_nf_0242";
    let client = setup(db).await;
    let classic = AssetFixture::new("XCR", "credit", XCR_ISSUER, "").with_sac(XCR_SAC);
    Client::default()
        .with_url(ch_url())
        .query(&assets_insert(db, &[classic]))
        .execute()
        .await
        .unwrap();

    let body = json!({ "assets": [XCR_SAC] });
    let (status, json) = post(client, "/v1/prices/batch", body).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["prices"], json!([]));
    assert_eq!(json["not_found"], json!([XCR_SAC]));

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
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

    // Task 0216 — the batch surface reads through its OWN row struct and then
    // hand-copies into the price response, so it can drift from `/price`
    // independently. Asserted by value on both arms: native carries a real
    // pair, USDC the DEFAULT pair that must publish as two empty strings.
    let by_asset: std::collections::HashMap<&str, &Value> = json["prices"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["asset"].as_str().unwrap(), p))
        .collect();
    let native = by_asset["native"];
    assert_eq!(native["as_of"], "2026-02-10T11:30:00Z");
    assert_ne!(
        native["as_of"], native["updated_at"],
        "the batch surface must publish the price's own time, not the snapshot's"
    );
    assert_eq!(native["price_status"], "carried");
    assert_eq!(native["price_basis"], "offer_dust", "task 0274, by value");
    let usdc = by_asset[format!("USDC:{}", issuer()).as_str()];
    assert_eq!(usdc["as_of"], "", "the epoch sentinel is never formatted");
    assert_eq!(usdc["price_status"], "");
    assert_eq!(usdc["price_basis"], "");

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn batch_empty_is_400() {
    let db = "it_ep_batch_empty_0040";
    let client = setup(db).await;
    let (status, json) = post(client, "/v1/prices/batch", json!({ "assets": [] })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
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
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn oracles_unknown_asset_is_404() {
    let db = "it_ep_oracles_unknown_0040";
    let client = setup(db).await;
    let (status, _) = get(client, &format!("/v1/oracles/FOO:{}", issuer())).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn backfill_status_maps_both_streams() {
    let db = "it_ep_backfill_0040";
    let client = setup(db).await;
    let (status, json) = get(client, "/v1/backfill/status").await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    assert_eq!(json["realtime_tip_ledger"], 57234198u64);
    assert_eq!(json["sdex"]["status"], "running");
    assert_eq!(json["sdex"]["current_ledger"], 34891234u64);
    // The archive walks BACKWARD (tip -> genesis), so `current_ledger` is the
    // oldest ledger reflected and what remains is the stretch still BELOW it:
    // remaining = current - start = 34891234 - 1
    assert_eq!(json["sdex"]["ledgers_remaining"], 34891233u64);
    // Seeded as `now() - INTERVAL 1 DAY`, so pin the wire format (RFC 3339,
    // UTC `Z`) and the value to within a few minutes, never its text.
    let last_push = json["sdex"]["last_push_at"]
        .as_str()
        .expect("last_push_at is a string");
    assert!(last_push.ends_with('Z'), "last_push_at={last_push}");
    let age = chrono::Utc::now()
        - chrono::DateTime::parse_from_rfc3339(last_push)
            .expect("last_push_at is RFC 3339")
            .with_timezone(&chrono::Utc);
    assert!(
        (age - chrono::Duration::days(1)).num_seconds().abs() < 300,
        "last_push_at={last_push} is not ~1 day ago (age {age})"
    );
    // earliest_data_available = oldest OHLCV row this stream has landed (AC 6)
    assert_eq!(
        json["sdex"]["earliest_data_available"],
        "2015-11-18T03:47:00Z"
    );
    // covered = (target - current) / (target - start) * 100
    //         = (57234198 - 34891234) / (57234198 - 1) * 100 ≈ 39.04
    //
    // Changed from 60.96 with the backward-direction fix (task 0127): the old
    // forward form reported the COMPLEMENT of the covered span, which read a
    // finished archive (current == start == 1) as 0.0% on production.
    let pct = json["sdex"]["progress_pct"].as_f64().unwrap();
    assert!((pct - 39.04).abs() < 0.1, "pct={pct}");

    assert_eq!(json["soroban_amm"]["status"], "completed");
    assert_eq!(json["soroban_amm"]["completed_at"], "2026-04-14T08:23:11Z");
    assert_eq!(
        json["soroban_amm"]["earliest_data_available"],
        "2024-02-20T17:00:00Z"
    );
    teardown(db).await;
}
