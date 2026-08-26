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
    //
    // `close_usd` is seeded EQUAL TO `close` (task 0170). Two reasons, and the
    // second is why the expected numbers below did not have to change:
    //
    // 1. Since ADR 0011 the endpoint denominates rather than filtering, so a
    //    candle with `close_usd = 0` is "not priced yet" and comes back with its
    //    price fields absent. Before 0170 this column was never read and the
    //    fixture left it at its DEFAULT 0 — which now means the opposite of what
    //    the test intends.
    // 2. `close_usd = close` is exactly the $1 peg signature, so the derived
    //    rate is 1 and every scaled value equals the stored one. The merge
    //    assertions are therefore unchanged from the pre-0170 fixture, and it
    //    also pins `method = "peg"`.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-02-10 10:00:00', 3, 2, 'sdex',     1.0, 1.2, 0.9, 1.1, 100, 110, 1.1, 1.05, 10, 1), \
             ('2026-02-10 10:00:00', 3, 2, 'soroswap', 1.05, 1.3, 0.95, 1.15, 300, 345, 1.15, 1.10, 20, 1), \
             ('2026-02-10 11:00:00', 3, 2, 'sdex',     1.1, 1.15, 1.05, 1.12, 50, 56, 1.12, 1.1, 5, 1)"
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
        portal_eligibility: None,
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

/// Seed an asset that trades ONLY against XLM, plus the rows the classification
/// and ordering tests need. Separate from `setup` so the merge test's fixture
/// stays exactly what it was.
///
/// asset_id 4 = `BAR`, quoted in XLM (asset_id 1) throughout.
async fn seed_xlm_only(db: &str, admin: &Client) {
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (4, 'BAR', 'credit', '{i}', '')",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();
    // Three buckets, one per case the USD path has to get right. NB: ClickHouse
    // rejects `--` comments inside a VALUES list, so they live out here.
    //
    //   10:00  priced through the XLM pivot — 10 XLM/unit at $0.25 => $2.50
    //   11:00  enrichment has not reached it — close_usd stays at its DEFAULT 0
    //   12:00  close_usd = close on an XLM leg — claims XLM was worth exactly $1
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-02-10 10:00:00', 4, 1, 'sdex', 10.0, 12.0, 9.0, 10.0, 100, 250, 2.5, 10.0, 7, 1), \
             ('2026-02-10 11:00:00', 4, 1, 'sdex', 11.0, 11.0, 11.0, 11.0, 20, 0, 0, 11.0, 3, 1), \
             ('2026-02-10 12:00:00', 4, 1, 'sdex', 13.0, 13.0, 13.0, 13.0, 5, 5, 13.0, 13.0, 2, 1)"
        ))
        .execute()
        .await
        .unwrap();
}

/// 🔑 The regression test for the whole of task 0170.
///
/// `BAR` has never traded against USDC. Before the fix the endpoint filtered on
/// `quote_asset_id = <USDC>` and returned `200` with an empty array — the answer
/// 20,481 assets were getting, indistinguishable from "never traded".
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usd_serves_an_asset_that_never_traded_against_usdc() {
    let db = "it_ohlcv_xlm_only_0170";
    let client = setup(db).await;
    seed_xlm_only(db, &Client::default().with_url(ch_url()).with_database(db)).await;

    let uri = format!(
        "/v1/assets/BAR:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
         &end=2026-02-10T10:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "expected the XLM-quoted candle, got {json}");

    let c = &data[0];
    // rate = close_usd/close = 2.5/10 = 0.25, applied to every derived field.
    approx(&c["close"], 2.5); // exact: close_usd as stored
    approx(&c["open"], 2.5); // 10.0 * 0.25
    approx(&c["high"], 3.0); // 12.0 * 0.25
    approx(&c["low"], 2.25); // 9.0 * 0.25
    approx(&c["vwap"], 2.5); // 10.0 * 0.25
    assert_eq!(c["method"], "traded", "XLM leg is priced through the pivot");
    assert_eq!(c["derived"], true);

    teardown(db).await;
}

/// ADR 0011 §5: a bucket enrichment has not priced yet is RETURNED with its
/// price fields absent, never dropped. Dropping it would put a hole at the
/// right-hand edge of every chart and make "not yet priced" look like "did not
/// trade" — the confusion this task exists to remove.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_unpriced_bucket_is_returned_with_price_fields_absent() {
    let db = "it_ohlcv_unpriced_0170";
    let client = setup(db).await;
    seed_xlm_only(db, &Client::default().with_url(ch_url()).with_database(db)).await;

    let uri = format!(
        "/v1/assets/BAR:{}/ohlcv?granularity=1h&start=2026-02-10T11:00:00Z\
         &end=2026-02-10T11:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "the bucket must survive, not be dropped");

    let c = &data[0];
    for field in ["open", "high", "low", "close", "vwap", "method", "derived"] {
        assert!(
            c[field].is_null(),
            "{field} must be absent, got {}",
            c[field]
        );
    }
    // Activity that does not depend on the USD rate is still reported.
    approx(&c["volume_base"], 20.0);
    assert_eq!(c["trade_count"], 3);

    teardown(db).await;
}

/// The 5,921 rows measured on prod 2026-08-26: `close_usd = close` on an XLM or
/// USDT leg claims the reference asset was worth exactly $1.00000000000000. XLM
/// has never been near a dollar. Such a row is excluded from the USD
/// aggregation rather than labelled, because every available label — `peg` above
/// all — would assert something false.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_peg_signature_on_a_pivot_leg_is_not_labelled_peg() {
    let db = "it_ohlcv_anomaly_0170";
    let client = setup(db).await;
    seed_xlm_only(db, &Client::default().with_url(ch_url()).with_database(db)).await;

    let uri = format!(
        "/v1/assets/BAR:{}/ohlcv?granularity=1h&start=2026-02-10T12:00:00Z\
         &end=2026-02-10T12:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "the bucket still returns");
    let c = &data[0];
    assert!(
        c["method"].is_null() && c["close"].is_null(),
        "an XLM-leg row at exactly 1x must not be priced or labelled, got {c}"
    );
    assert_ne!(c["method"], "peg", "there is no peg on an XLM leg");

    teardown(db).await;
}

/// 🔑 Pins the convert-before-merge ordering, which fails SILENTLY if reversed.
///
/// One bucket, two quote legs at different rates. Converting after the merge
/// would compare a raw XLM-denominated high (12.0) against a raw USDC one (1.3)
/// and pick 12.0 — a number in the wrong unit that still looks like a price.
/// Converting first makes the comparison 3.0 vs 1.3, and the answer is 3.0 for
/// a reason rather than by luck.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_converts_each_leg_before_merging_across_them() {
    let db = "it_ohlcv_order_0170";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    seed_xlm_only(db, &admin).await;
    // Same asset, same bucket, second leg: BAR/USDC at the peg.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-02-10 10:00:00', 4, 2, 'sdex', 1.0, 1.3, 0.8, 1.2, 10, 12, 1.2, 1.2, 4, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/BAR:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
         &end=2026-02-10T10:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "both legs merge into ONE bucket");
    let c = &data[0];

    // Converted first: max(12.0*0.25, 1.3*1.0) = max(3.0, 1.3) = 3.0.
    // Converted after the merge it would have been max(12.0, 1.3) = 12.0.
    approx(&c["high"], 3.0);
    assert!(
        c["high"].as_str().unwrap().parse::<f64>().unwrap() < 11.0,
        "12.0 here means the merge happened before the conversion: {c}"
    );
    // min(9.0*0.25, 0.8*1.0) = min(2.25, 0.8) = 0.8 — the USDC leg wins the low
    // only once both are in the same unit.
    approx(&c["low"], 0.8);
    // Volumes sum across legs; volume_quote_usd is already USD on both.
    approx(&c["volume_base"], 110.0);
    approx(&c["volume_quote_usd"], 262.0);
    // argMax by volume_base → the XLM leg (100 > 10), so its method wins.
    assert_eq!(c["method"], "traded");

    teardown(db).await;
}
