//! Live-ClickHouse integration tests for `GET /v1/assets/{id}/ohlcv`. Gated
//! `#[ignore]`:
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-api --test ohlcv_it -- --ignored

use axum::body::Body;
use axum::http::{Request, StatusCode};
use clickhouse::Client;
use prices_api::{AppConfig, AppState, app};
use rust_decimal::Decimal;
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
    //    also pins the candle path's par label — which task 0268 renamed from
    //    the 0165 spelling to `assumed-par`, because on a quote leg the word
    //    has to name the ASSUMPTION rather than borrow USDC's own-series
    //    meaning. `ohlcv_peg_series` (a `USDC:...` request) still says `peg`.
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

/// Run a statement that returns nothing, failing loudly. Used for the account
/// management the read-only test needs; the data seeds spell their own out.
async fn exec(client: &Client, sql: &str) {
    client.query(sql).execute().await.unwrap();
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
    // Both par spellings, because task 0268 renamed this label on the candle
    // path: asserting only the retired word would leave the guard vacuously
    // true — the exact way a regression here would slip through unnoticed.
    assert_ne!(c["method"], "peg", "there is no peg on an XLM leg");
    assert_ne!(
        c["method"], "assumed-par",
        "no $1 assumption was ever applied to an XLM leg"
    );

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

/// Regression test for the RowBinary shape of `base_currency=XLM`.
///
/// The existing XLM test asserts an EMPTY series, so no row is ever decoded and
/// a column-type mismatch in that branch is invisible to it. `BAR` is quoted in
/// XLM, so this one actually decodes rows.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_xlm_denomination_decodes_rows() {
    let db = "it_ohlcv_xlm_rows_0170";
    let client = setup(db).await;
    seed_xlm_only(db, &Client::default().with_url(ch_url()).with_database(db)).await;

    let uri = format!(
        "/v1/assets/BAR:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
         &end=2026-02-10T10:00:00Z&base_currency=XLM",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "XLM-quoted candle must decode: {json}");
    approx(&data[0]["close"], 10.0);

    teardown(db).await;
}

/// Seed `prices.usd_rate` with a MOVING measured rate for canonical USDC, plus
/// a USDC-quoted candle in a bucket that predates every observation.
///
/// The rate deliberately wobbles (0.9993, 1.0007) rather than sitting at 1.0:
/// a test that asserted the constant would pass against a hardcoded peg and
/// prove nothing (ADR 0011 §6, task 0212).
async fn seed_peg_rate(db: &str, admin: &Client) {
    admin
        .query(&format!(
            // ⚠️ The explicit column list now names task 0267's `quality`, and
            // these ORACLE rows leave it at ''. That is not a placeholder: the
            // column carries the IMPORTING series' confidence in its own
            // observation, and a polled Reflector reading has no such series.
            // '' means "does not apply" here, never "unknown".
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2026-02-10 10:00:00', 0.9993, 'oracle', '', '', 0, 1), \
             ('credit', 'USDC', '{i}', '', '2026-02-10 11:00:00', 1.0007, 'oracle', '', '', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();
    // The peg series takes its buckets from the XLM/USDC reference market
    // (asset_id 1 quoted in asset_id 2), NOT from "any USDC-quoted candle":
    // price_ohlcv_* is ORDER BY (asset_id, quote_asset_id, …), so filtering on
    // the quote alone is not a key prefix and degenerates into a full FINAL
    // scan. `close_usd = 0.25` is XLM's USD price, which the XLM denomination
    // divides by.
    //
    //   2024-01-05 09:00  predates every observation → peg fallback
    //   2026-02-10 10:00  covered by the 0.9993 observation
    //   2026-02-10 11:00  covered by the 1.0007 observation
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2024-01-05 09:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-02-10 10:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-02-10 11:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();
}

/// 🔑 The epoch-day gap hour: the one bucket where the day-wide external net
/// used to outlive the oracle epoch.
///
/// The imported series stops at 13:00 on 2026-03-11 and the first poll is at
/// 14:00 (`USDC_ORACLE_EPOCH_S`). `e_ok` floored an imported row at
/// `toStartOfDay(bkt, 'UTC')`, so an hour at or after 14:00 THAT DAY whose own
/// oracle window found nothing reached back to the 13:00 import and published it
/// as a measurement — while `price_usd_series_1h`, which buckets strictly by
/// hour, published the $1 peg for the same hour. Two read surfaces, two
/// different answers for one bucket, and the candle path was the one claiming a
/// measurement for an hour the imported series does not cover.
///
/// A bound on the imported ROW cannot fix this: the offending row is at 13:00,
/// already below the epoch. The bucket is what must be bounded.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_peg_series_stops_importing_across_the_oracle_epoch() {
    let db = "it_ohlcv_epoch_gap_0267";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);

    // The last imported hour (13:00), and NO oracle row at all — the enrichment
    // gap this test is about. With a poll present the oracle rank would take
    // these buckets and the leak would be invisible, which is exactly how it
    // survived: the bound cannot be tested through a surface that outranks it.
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2026-03-11 13:00:00', 0.99123400000000, \
              'external', 'chainlink', 'measured', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    // Buckets for the series to render, on the reference market.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-11 13:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-03-11 14:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-03-11 15:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2026-03-11T13:00:00Z\
         &end=2026-03-11T15:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();

    let at = |ts: &str| {
        data.iter()
            .find(|c| c["timestamp"] == ts)
            .unwrap_or_else(|| panic!("{ts} missing: {json}"))
            .clone()
    };

    // 13:00 is below the epoch and the import covers it: measured.
    let below = at("2026-03-11T13:00:00Z");
    assert_eq!(below["method"], "external", "the imported hour: {below}");
    approx(&below["close"], 0.991234);

    // ⚠️ 14:00 is the first bucket the epoch opens. It is excluded by a bound on
    // either end — the SPANNING bucket below is what separates them.
    let boundary = at("2026-03-11T14:00:00Z");
    assert_eq!(
        boundary["method"], "peg",
        "the bucket the epoch opens must not take the import: {boundary}"
    );
    approx(&boundary["close"], 1.0);

    // 15:00 is wholly above the epoch. The imported series holds nothing there,
    // so the only honest answer is the labelled $1 peg.
    let above = at("2026-03-11T15:00:00Z");
    assert_eq!(
        above["method"], "peg",
        "an hour above the oracle epoch must not be priced from an import \
         stamped below it — the imported series does not cover it: {above}"
    );
    approx(&above["close"], 1.0);

    // 🔑 THE SPANNING BUCKET, and the reason the bound is on `bend` and not on
    // `bkt`. The 1d bucket of 2026-03-11 STARTS at 00:00 — below the epoch — and
    // runs to midnight, ten hours past it. A bound on the bucket's start leaves
    // it holding the day-wide net, so with no poll anywhere in the day the
    // 13:00 import wins the whole day and the API publishes it as a measurement
    // over the ten hours the series does not hold. At 1w and 1M the same shape
    // spans days and weeks. Normally the oracle rank hides this; an enrichment
    // gap is exactly when it does not, and relying on another surface to mask a
    // wrong answer is not a bound.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-11 00:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1d&start=2026-03-11T00:00:00Z\
         &end=2026-03-11T00:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(Client::default().with_url(ch_url()).with_database(db), &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let day = &json["data"].as_array().unwrap()[0];
    assert_eq!(
        day["method"], "peg",
        "a bucket that extends past the epoch must not be priced from an import \
         stamped below it, at any grain: {day}"
    );
    approx(&day["close"], 1.0);

    teardown(db).await;
}

/// 🔑 BRIEF acceptance criterion 4, and the falsifier for the whole of task
/// 0267: **2023-03-11 must stop publishing a dollar.**
///
/// USDC lost its peg that day — the composed series (task 0265) measured a
/// 0.96812 close from Chainlink — and until this task the API published a
/// literal 1.0 labelled `peg` for it, because `usd_rate` held no row and
/// `ohlcv_peg_series` read only `method = 'oracle'`. The row is seeded exactly
/// as `load-external-rate` writes it: stamped at the UTC day START, carrying the
/// CSV's source in `reference_asset` and its quality in `quality`.
///
/// ⚠️ The close is asserted as the STORED decimal, not as `0.9681`. The column
/// is `Decimal(38, 14)` and the value is returned through `toString`, which
/// TRIMS trailing zeros — so the wire carries `0.96812`, not
/// `0.96812000000000` (review WR-02; `views_it.rs` pins the same trimming for
/// the views). `0.9681` is a four-significant-figure QUOTATION of it from the
/// task text, and an equality assertion against that string WILL fail. The
/// exact string plus the numeric check together say what matters: the right
/// number, at full stored precision, in the form the operator's runbook expects.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_publishes_the_imported_measurement_for_the_2023_depeg() {
    let db = "it_ohlcv_0267_depeg";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);

    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2023-03-11 00:00:00', 0.96812, 'external', \
              'chainlink', 'measured', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    // The peg series takes its buckets from the XLM/USDC reference market, so
    // the day needs a candle there or there is no bucket to report at all.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2023-03-11 00:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1d&start=2023-03-11T00:00:00Z\
         &end=2023-03-11T00:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "the depeg day must have a bucket: {json}");
    let row = &data[0];

    let close = row["close"].as_str().expect("close is a string");
    assert_eq!(
        close, "0.96812",
        "the depeg day must publish the MEASURED rate, trailing zeros trimmed. \
         `1` means the imported row is not being read at all"
    );
    approx(&row["close"], 0.96812);

    assert_eq!(
        row["method"], "external",
        "an imported measurement must say so — a consumer has to be able to tell \
         it from a poll AND from the $1 assumption it replaces"
    );
    assert_eq!(row["source"], "chainlink", "{json}");
    assert_eq!(row["quality"], "measured", "{json}");

    teardown(db).await;
}

/// 🔑 BRIEF acceptance criterion 5: **no discontinuity at the oracle epoch, and
/// no cross-contamination in either direction.**
///
/// Task 0267 loads history strictly BELOW `USDC_ORACLE_EPOCH_S` and our own
/// polling is primary from it. The two populations share NO key, but they DO
/// share one daily bucket (review IN-01): the epoch is 14:00 UTC, the composed
/// series' last loadable row is the 00:00 of that same day, so the 1d bucket of
/// the epoch day holds one `external` row and the day's `oracle` rows. That is
/// the one bucket in production on which the preference rule does real work,
/// and it is the seam seeded here: the day before (import only) must read as
/// the import it is, and the epoch day (import at 00:00, poll at 14:00) must
/// read as the POLL, with the import's provenance nowhere on the wire.
///
/// ⚠️ `source`/`quality` on the polled bucket are asserted `null`, not `""`
/// (review CR-01): a poll's `reference_asset` is `''` and its `quality` the
/// column DEFAULT, and `toNullable('')` is `''`. Before the fix this assertion
/// could never have held.
///
/// ⚠️ Every timestamp is DERIVED from the shared constant, in SQL, rather than
/// typed. Two hand-written epochs is precisely the drift the constant exists to
/// prevent, and a test that restates the number would keep passing while the
/// code moved away from it.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_reads_each_side_of_the_oracle_epoch_with_its_own_method() {
    use prices_clickhouse::USDC_ORACLE_EPOCH_S as EPOCH;

    let db = "it_ohlcv_0267_epoch_seam";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);

    // Two imported days: the day START one day below the epoch, and the day
    // START of the epoch day itself — the last row the loader admits (it is
    // 00:00, strictly below a 14:00 epoch). The polled reading: the epoch
    // instant, which is the first one prod holds. The epoch-day import is
    // planted at a value no measured USDC rate could hold, so a rank regression
    // shows in the NUMBER, not only in the label.
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) \
             SELECT 'credit', 'USDC', '{i}', '', \
                    toStartOfDay(toDateTime({EPOCH} - 86400)), 0.98765, 'external', \
                    'chainlink', 'measured-disputed', 0, 1 \
             UNION ALL \
             SELECT 'credit', 'USDC', '{i}', '', \
                    toStartOfDay(toDateTime({EPOCH})), 0.5, 'external', \
                    'bitstamp', 'fallback', 0, 1 \
             UNION ALL \
             SELECT 'credit', 'USDC', '{i}', '', \
                    toDateTime({EPOCH}), 1.00042, 'oracle', '', '', 0, 1",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    // One XLM/USDC candle per day bucket, on both sides of the seam.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) \
             SELECT toStartOfDay(toDateTime({EPOCH} - 86400)), 1, 2, 'sdex', \
                    0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1 \
             UNION ALL \
             SELECT toStartOfDay(toDateTime({EPOCH})), 1, 2, 'sdex', \
                    0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1"
        ))
        .execute()
        .await
        .unwrap();

    // The request window, also derived: five days either side of the seam.
    let (from, to): (String, String) = admin
        .query(
            "SELECT formatDateTime(toDateTime(? - 5 * 86400), '%Y-%m-%dT%H:%i:%SZ'), \
                    formatDateTime(toDateTime(? + 5 * 86400), '%Y-%m-%dT%H:%i:%SZ')",
        )
        .bind(EPOCH)
        .bind(EPOCH)
        .fetch_one::<(String, String)>()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1d&start={from}&end={to}&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2, "one bucket each side of the seam: {json}");

    // Ascending by timestamp, so [0] is the imported day and [1] the polled one.
    let (imported, polled) = (&data[0], &data[1]);

    assert_eq!(
        imported["method"], "external",
        "the day below the epoch must read as the IMPORT it is: {json}"
    );
    approx(&imported["close"], 0.98765);
    assert_eq!(imported["source"], "chainlink");
    assert_eq!(
        imported["quality"], "measured-disputed",
        "a disputed cross-check must survive to the wire — it is what tells a \
         consumer the number is real but less certain: {json}"
    );

    assert_eq!(
        polled["method"], "oracle",
        "the bucket holding our own first reading must read as a POLL, not as \
         the import that shares its day: {json}"
    );
    approx(&polled["close"], 1.00042);
    assert_eq!(
        polled["source"],
        Value::Null,
        "a polled reading has no outside series, and naming one would attach a \
         provenance nobody measured. `\"\"` here is the '' DEFAULT leaking \
         through `toNullable` (review CR-01); `bitstamp` is the outranked \
         import's provenance leaking across methods: {json}"
    );
    assert_eq!(polled["quality"], Value::Null, "{json}");

    // Neither bucket may be labelled `peg`: both HAVE a measurement, and a `peg`
    // here would be the original defect returning in a new place.
    for row in data {
        assert_ne!(row["method"], "peg", "{json}");
    }

    teardown(db).await;
}

/// 🔑 Review WR-05 — a valid ORACLE reading wins the WHOLE bucket, even when
/// an import is stamped LATER in it; and `/ohlcv` and `price_usd_series`
/// resolve that bucket by the SAME rule.
///
/// This is `views_it.rs`'s `an_oracle_row_outranks_an_imported_row_in_the_same_
/// bucket` fixture, run through the API. Before the fix the peg series ranked
/// oracle only WITHIN one instant and let the ASOF's recency choose across
/// instants, so this seed published `0.5`/`external` here while the view
/// published `1.00066…`/`oracle` for the same bucket. The loader's day-start
/// stamping hid the divergence in production; a rule held only by another
/// tool's invariant is not a rule either surface can be trusted on.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_oracle_outranks_a_later_import_in_the_same_bucket() {
    let db = "it_ohlcv_0267_oracle_outranks_import";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    prices_clickhouse::apply_sql(
        &Client::default().with_url(ch_url()),
        &rewrite(prices_clickhouse::VIEWS_SQL, db),
    )
    .await
    .unwrap();

    const ORACLE: &str = "1.00066784838102";
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2026-08-10 23:30:00', {ORACLE}, 'oracle', '', '', 0, 1), \
             ('credit', 'USDC', '{i}', '', '2026-08-10 23:45:00', 0.5, 'external', \
              'chainlink', 'measured', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-08-10 00:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1d&start=2026-08-10T00:00:00Z\
         &end=2026-08-10T00:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "{json}");
    let row = &data[0];

    assert_eq!(
        row["close"], ORACLE,
        "the POLLED rate must win a bucket that holds both. `0.5` means recency \
         decided across instants and the later import outranked the poll: {json}"
    );
    assert_eq!(row["method"], "oracle", "{json}");
    assert_eq!(
        row["source"],
        Value::Null,
        "the outranked import's provenance must not leak onto the oracle's bucket: {json}"
    );
    assert_eq!(row["quality"], Value::Null, "{json}");

    // And the view says the same — the cross-surface criterion of task 0246,
    // now on a bucket that holds two provenances.
    let (view_close, view_method) = admin
        .query(&format!(
            "SELECT toString(close_usd), method FROM {db}.price_usd_series \
             WHERE asset_code = ? AND issuer_address = ? AND bucket = toDateTime(?)"
        ))
        .bind("USDC")
        .bind(iss())
        .bind("2026-08-10 00:00:00")
        .fetch_one::<(String, String)>()
        .await
        .unwrap();
    assert_eq!(row["close"], view_close.as_str(), "{json}");
    assert_eq!(row["method"], view_method.as_str(), "{json}");

    teardown(db).await;
}

/// 🔑 Review WR-06 — an imported day prices EVERY bucket of its UTC day, at a
/// finer grain than the daily row it came from.
///
/// The composed series is daily and stamped at 00:00. Task 0268's external tier
/// prices every hourly XLM/USDC candle of 2023-03-11 from that one row, so an
/// hourly candle's `close_usd` at 13:00 says USDC was worth 0.96812. Before this
/// fix USDC's OWN hourly series said `1`/`peg` for the same hour — a one-bucket
/// staleness window on a daily row served the 00:00 hour only. The two surfaces
/// must agree on what USDC was worth at 13:00.
///
/// The window still ends at the day's end: the next midnight, with no row of
/// its own, falls back to the labelled peg. Forward-filling a daily import
/// past its day would be task 0246's defect returning in a new place.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_serves_an_imported_day_at_every_hour_of_it() {
    let db = "it_ohlcv_0267_hourly_import_window";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);

    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2023-03-11 00:00:00', 0.96812, 'external', \
              'chainlink', 'measured', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();
    // Three hours of the depeg day and the first hour of the next.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2023-03-11 00:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2023-03-11 13:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2023-03-11 23:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2023-03-12 00:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2023-03-11T00:00:00Z\
         &end=2023-03-12T00:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 4, "{json}");

    for row in &data[..3] {
        let ts = row["timestamp"].as_str().unwrap();
        assert_eq!(
            row["close"], "0.96812",
            "{ts}: every hour of an imported day carries that day's rate. `1` \
             means the import's window is one bucket wide and only 00:00 saw it: {json}"
        );
        assert_eq!(row["method"], "external", "{ts}: {json}");
        assert_eq!(row["source"], "chainlink", "{ts}: {json}");
        assert_eq!(row["quality"], "measured", "{ts}: {json}");
    }

    let next = &data[3];
    assert_eq!(next["timestamp"], "2023-03-12T00:00:00Z", "{json}");
    assert_eq!(
        next["method"], "peg",
        "the day AFTER has no row of its own and must not inherit the \
         previous day's import: {json}"
    );
    assert_eq!(next["close"], "1", "{json}");
    assert_eq!(next["source"], Value::Null, "{json}");
    assert_eq!(next["quality"], Value::Null, "{json}");

    teardown(db).await;
}

/// 🔑 ADR 0011 §6 — the ORIGINAL narrow defect this task was named for.
///
/// USDC is never stored as a base leg, so `GET /assets/{USDC}/ohlcv` asked for a
/// USDC/USDC self-pair and matched zero rows. Dropping the quote filter does not
/// help; the series has to be synthesized.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_self_pair_is_synthesized_from_the_measured_rate() {
    let db = "it_ohlcv_peg_0170";
    let client = setup(db).await;
    seed_peg_rate(db, &Client::default().with_url(ch_url()).with_database(db)).await;

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
         &end=2026-02-10T11:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2, "USDC must have a series at all: {json}");

    // Each observation sits exactly on a bucket start, so each bucket contains
    // its own: 10:00 takes 0.9993, 11:00 takes 1.0007. ⚠️ This fixture cannot
    // tell the pre-0246 rule from the current one — both pick the same rows.
    // `seed_0246` is the one that can; see the tests at the end of this file.
    approx(&data[0]["close"], 0.9993);
    approx(&data[1]["close"], 1.0007);
    assert_eq!(data[0]["method"], "oracle");
    assert_ne!(
        data[0]["close"], data[1]["close"],
        "the series must track a MOVING rate, not a constant"
    );

    teardown(db).await;
}

/// ADR 0011 §6 fallback semantics: no rate available → peg, and it says `peg`.
///
/// `usd_rate` starts 2026-03-11 on prod while `timeframe=all` reads back to
/// 2021, so this is the majority of the real series — not an edge case.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_before_any_observation_falls_back_to_a_labelled_peg() {
    let db = "it_ohlcv_peg_fallback_0170";
    let client = setup(db).await;
    seed_peg_rate(db, &Client::default().with_url(ch_url()).with_database(db)).await;

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2024-01-05T09:00:00Z\
         &end=2024-01-05T09:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "the pre-observation bucket must still exist");
    approx(&data[0]["close"], 1.0);
    assert_eq!(
        data[0]["method"], "peg",
        "a fallback must be labelled, never rendered as a measurement"
    );

    teardown(db).await;
}

/// 🔑 The production failure of 2026-08-27, pinned so it cannot return.
///
/// The peg query used to end `SETTINGS join_use_nulls = 1`. `prices_reader` runs
/// read-only in production and a read-only user may not modify a setting, so
/// ClickHouse refused the query outright — `Code: 164 … (READONLY)` at
/// `ExceptionBeforeStart`, 40 ms, no rows read — and the deployed endpoint
/// answered `500` for canonical USDC. Every test in this file passed throughout,
/// because they all connect as a user that is **not** read-only.
///
/// That gap is what this closes. It is not a test of the fallback (two tests
/// above already own that) but of the privilege the query runs under: the same
/// request, as a `readonly = 1` user, must still answer. A future `SETTINGS`
/// clause fails here instead of on prod.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_peg_series_answers_for_a_readonly_user() {
    let db = "it_ohlcv_peg_readonly_0170";
    let _ = setup(db).await;
    seed_peg_rate(db, &Client::default().with_url(ch_url()).with_database(db)).await;

    let admin = Client::default().with_url(ch_url());
    exec(&admin, "DROP USER IF EXISTS ohlcv_ro_0170").await;
    exec(
        &admin,
        "CREATE USER ohlcv_ro_0170 IDENTIFIED WITH no_password SETTINGS readonly = 1",
    )
    .await;
    exec(&admin, &format!("GRANT SELECT ON {db}.* TO ohlcv_ro_0170")).await;

    let readonly = Client::default()
        .with_url(ch_url())
        .with_database(db)
        .with_user("ohlcv_ro_0170");

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
         &end=2026-02-10T11:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(readonly, &uri).await;

    // ⚠️ The user is dropped BEFORE the assertions. A failing assertion unwinds
    // past whatever follows it, and leaving a passwordless account holding
    // SELECT behind on the server is not an acceptable cost of a red test —
    // `ch_url()` honours `CLICKHOUSE_URL` and need not be a throwaway container.
    exec(&admin, "DROP USER IF EXISTS ohlcv_ro_0170").await;

    assert_eq!(
        status,
        StatusCode::OK,
        "a read-only user must be able to read the peg series: body={json}"
    );
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 2, "the synthesized series must survive: {json}");
    approx(&data[0]["close"], 0.9993);
    assert_eq!(data[0]["method"], "oracle");

    teardown(db).await;
}

/// The precision precondition (ADR 0011 §7 / task 0170).
///
/// Shaped on a real prod row: `close = 5e-14`, `close_usd = 4e-14` — five ticks
/// of the `Decimal(38,14)` floor over four. The implied rate is **1.25**, an
/// entirely ordinary-looking number, which is exactly why a band check on the
/// derived rate cannot catch it. The inputs are what is wrong, not the value.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_refuses_to_derive_a_rate_from_values_at_the_decimal_floor() {
    let db = "it_ohlcv_precision_0170";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-01 10:00:00', 3, 2, 'sdex', 0.00000000000005, 0.00000000000005, \
              0.00000000000005, 0.00000000000005, 9, 9, 0.00000000000004, \
              0.00000000000005, 1, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-01T10:00:00Z\
         &end=2026-03-01T10:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "the bucket still returns (§5)");
    let c = &data[0];
    assert!(
        c["close"].is_null() && c["method"].is_null(),
        "a rate from 5 ticks over 4 is quantisation noise, not a price: {c}"
    );
    // The activity itself is real and still reported.
    approx(&c["volume_base"], 9.0);

    teardown(db).await;
}

/// `close = 0` is a distinct population from `close_usd = 0` and needs its own
/// coverage: dividing by it is what the guard exists to prevent.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_guards_a_zero_close() {
    let db = "it_ohlcv_zero_close_0170";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-02 10:00:00', 3, 2, 'sdex', 0, 0, 0, 0, 4, 0, 2.0, 0, 1, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-02T10:00:00Z\
         &end=2026-03-02T10:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let c = &json["data"].as_array().unwrap()[0];
    assert!(c["close"].is_null(), "close = 0 must not divide: {c}");

    teardown(db).await;
}

/// USDT trades genuinely as a base in ~102 pools. The synthetic peg path must
/// NOT capture it — that is the trap 0165 documents and 0172 proved expensive:
/// USDT is not at par, and overriding real market data with an assumed rate is
/// how 44,657 candles came to be overstated ~7.4x.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdt_as_a_base_keeps_its_real_market_data() {
    let db = "it_ohlcv_usdt_base_0170";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (5, 'USDT', 'credit', '{u}', '')",
            u = prices_clickhouse::USDT_ISSUER
        ))
        .execute()
        .await
        .unwrap();
    // USDT/USDC at its real depegged level, not par.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-03 10:00:00', 5, 2, 'sdex', 0.13, 0.14, 0.12, 0.13, 100, 13, 0.13, 0.13, 6, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDT:{u}/ohlcv?granularity=1h&start=2026-03-03T10:00:00Z\
         &end=2026-03-03T10:00:00Z&base_currency=USD",
        u = prices_clickhouse::USDT_ISSUER
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "USDT has real candles as a base: {json}");
    approx(&data[0]["close"], 0.13);
    assert!(
        (data[0]["close"].as_str().unwrap().parse::<f64>().unwrap() - 1.0).abs() > 0.5,
        "USDT must not be synthesized at par: {}",
        data[0]
    );

    teardown(db).await;
}

/// The whole point of the task: an asset that genuinely never traded must stay
/// distinguishable from one that is merely unrepresentable in the requested
/// denomination. Before the fix both were an empty `200`.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_never_traded_is_distinguishable_from_unrepresentable() {
    let db = "it_ohlcv_never_traded_0170";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    seed_xlm_only(db, &admin).await;
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (6, 'GHOST', 'credit', '{i}', '')",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    // GHOST is tracked but has no candles anywhere: a genuine empty series.
    let (status, ghost) = get(
        client.clone(),
        &format!(
            "/v1/assets/GHOST:{}/ohlcv?granularity=1h&base_currency=USD",
            iss()
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={ghost}");
    assert!(
        ghost["data"].as_array().unwrap().is_empty(),
        "an asset that never traded is still an empty series"
    );

    // BAR trades only against XLM — the case that used to look identical.
    let (status, bar) = get(
        client,
        &format!(
            "/v1/assets/BAR:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
             &end=2026-02-10T10:00:00Z&base_currency=USD",
            iss()
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body={bar}");
    assert!(
        !bar["data"].as_array().unwrap().is_empty(),
        "an XLM-only asset must NOT read as never-traded: {bar}"
    );

    teardown(db).await;
}

/// ADR 0011 §6's second degenerate case: USDC denominated in XLM.
///
/// The market exists one way round only (base XLM, quote USDC), so the series is
/// DERIVED from two USD rates rather than inverted out of that candle —
/// `USDC_usd / XLM_usd`. Seeded so the answer is unambiguous: USDC at 0.9993 USD
/// and XLM at 0.25 USD gives 3.9972 XLM per USDC.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_in_xlm_is_derived_from_two_usd_rates() {
    let db = "it_ohlcv_usdc_xlm_0170";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    seed_peg_rate(db, &admin).await;

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
         &end=2026-02-10T10:00:00Z&base_currency=XLM",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_eq!(json["base_currency"], "XLM");

    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "USDC must have an XLM series: {json}");
    // 0.9993 / 0.25 = 3.9972 — NOT 1/0.25 = 4.0, which is what a naive
    // inversion of the candle would have produced.
    approx(&data[0]["close"], 3.9972);
    assert_eq!(data[0]["method"], "oracle");

    teardown(db).await;
}

/// Finding from PR #253's review: on the synthesized path `method` and `derived`
/// were computed independently of whether the price survived, so an XLM-mode
/// bucket whose denominator was floored out returned `close: null` next to
/// `method: "oracle"`, `derived: true`.
///
/// That inverts the contract the DTO states, and a client using `method != null`
/// as "this bucket is priced" would dereference a null. The unpriced right-hand
/// edge is exactly where it would have bitten.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_in_xlm_nulls_provenance_when_the_denominator_is_unpriced() {
    let db = "it_ohlcv_xlm_den_null_0170";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    seed_peg_rate(db, &admin).await;
    // An XLM/USDC bucket enrichment has not reached: close_usd = 0, so there is
    // no denominator. The rate observation at 10:00 still precedes it, so a
    // naive implementation would happily label this one.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-02-10 12:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 0, 0, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2026-02-10T12:00:00Z\
         &end=2026-02-10T12:00:00Z&base_currency=XLM",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 1, "the bucket still returns (§5)");
    let c = &data[0];
    assert!(c["close"].is_null(), "no denominator means no price: {c}");
    assert!(
        c["method"].is_null() && c["derived"].is_null(),
        "provenance must vanish with the price it describes: {c}"
    );

    teardown(db).await;
}

/// Assert `low <= open,close <= high` on one candle, **exactly**.
///
/// ⚠️ Deliberately NOT built on [`approx`]. Task 0229's violation is ~1e-12 at
/// five-figure prices; `approx` tolerates 1e-6 and f64 cannot even represent the
/// difference there, so an assertion routed through either would pass against the
/// bug and prove nothing. The comparison is exact decimal or it is vacuous.
fn assert_ohlc_ordered(c: &Value, label: &str) {
    let d = |k: &str| -> Decimal {
        Decimal::from_str_exact(
            c[k].as_str()
                .unwrap_or_else(|| panic!("{label}: {k} absent")),
        )
        .unwrap_or_else(|e| panic!("{label}: {k} unparseable: {e}"))
    };
    let (o, h, l, cl) = (d("open"), d("high"), d("low"), d("close"));
    assert!(l <= o, "{label}: low {l} > open {o}");
    assert!(l <= cl, "{label}: low {l} > close {cl}  (gap {})", l - cl);
    assert!(o <= h, "{label}: open {o} > high {h}");
    assert!(cl <= h, "{label}: close {cl} > high {h}  (gap {})", cl - h);

    // `vwap` is a volume-weighted mean of prices in the bucket, so it must lie
    // within the bucket's range — same bound, and it escapes it far more often
    // than the extremes do (0229's review, finding 1). Skipped on the zero
    // sentinel, which means "no weighted mean", not "a vwap of zero".
    let vw = d("vwap");
    if !vw.is_zero() {
        assert!(l <= vw, "{label}: low {l} > vwap {vw}  (gap {})", l - vw);
        assert!(vw <= h, "{label}: vwap {vw} > high {h}  (gap {})", vw - h);
    }
}

/// Task 0229 — the derived `low` rounds ABOVE the exact `close`.
///
/// The candle closes at its low (`low == close`), so the true derived low is
/// exactly `close_usd`; `toFloat64(low) * rate` then lands one ulp on the wrong
/// side of it. `68421.98765432109876` is chosen because it reproduces: the float
/// product renders as `68421.98765432110000`, **1.24e-12 above** the exact close.
///
/// 🔑 Non-vacuous by construction — against the pre-fix query this returns
/// `low > close` and the assertion fires. The magnitude matters: at three
/// figures the float has precision to spare and nothing crosses.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_derived_low_cannot_round_above_the_exact_close() {
    let db = "it_ohlcv_low_above_close_0229";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-03 10:00:00', 3, 2, 'sdex', 1.1, 1.2, 1.0, 1.0, \
              10, 10, 68421.98765432109876, 1.05, 3, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-03T10:00:00Z\
         &end=2026-03-03T10:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let c = &json["data"].as_array().unwrap()[0];

    assert_ohlc_ordered(c, "derived low vs exact close");

    // The exactness ADR 0011 §3 keeps is not sacrificed to get there.
    assert_eq!(
        c["close"].as_str().unwrap(),
        "68421.98765432109876",
        "close must stay EXACT — clamping moves the extremes, never the close"
    );
    assert_eq!(c["derived"], true);

    teardown(db).await;
}

/// Task 0229, the mirror — the derived `high` rounds BELOW the exact `close`.
///
/// The candle closes at its high, and `76943.51350417596657` (the value measured
/// on prod, BTC 1h) renders through the float product as `76943.51350417596`:
/// **6.57e-12 below** the exact close. Both directions are covered because a
/// clamp on only one of them is a fix that looks complete and is not.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_derived_high_cannot_round_below_the_exact_close() {
    let db = "it_ohlcv_high_below_close_0229";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-03 11:00:00', 3, 2, 'sdex', 0.9, 1.0, 0.8, 1.0, \
              10, 10, 76943.51350417596657, 0.95, 3, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-03T11:00:00Z\
         &end=2026-03-03T11:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let c = &json["data"].as_array().unwrap()[0];

    assert_ohlc_ordered(c, "derived high vs exact close");
    assert_eq!(
        c["close"].as_str().unwrap(),
        "76943.51350417596657",
        "close must stay EXACT"
    );

    teardown(db).await;
}

/// The invariant holds on the `base_currency=XLM` path too (0229 AC 2).
///
/// ⚠️ This path is structurally incapable of the defect — [`Denomination::QuoteLeg`]
/// emits the stored columns with no rate applied, so nothing is derived and
/// nothing is on a second scale. The test exists to PIN that, not to catch a
/// live bug: if the as-stored arm ever grows a conversion, this fails.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_xlm_denomination_keeps_ohlc_ordered() {
    let db = "it_ohlcv_xlm_ordered_0229";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    // FOO/XLM (quote_asset_id = 1), same awkward magnitude in the stored columns.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-03 12:00:00', 3, 1, 'sdex', \
              68421.98765432109876, 68421.98765432109876, 68421.98765432109876, \
              68421.98765432109876, 10, 10, 68421.98765432109876, \
              68421.98765432109876, 3, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-03T12:00:00Z\
         &end=2026-03-03T12:00:00Z&base_currency=XLM",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let c = &json["data"].as_array().unwrap()[0];

    assert_ohlc_ordered(c, "xlm denomination");
    assert_eq!(
        c["close"].as_str().unwrap(),
        "68421.98765432109876",
        "as-stored means as stored — no rate, no rounding"
    );

    teardown(db).await;
}

/// The synthesized USDC self-pair series keeps the invariant (0229 AC 2).
///
/// ⚠️ Also structurally safe: [`ohlcv_peg_series`] emits `o AS h, o AS l, o AS c`,
/// one value in four fields, so `low <= open,close <= high` is satisfied by
/// identity. Pinned rather than assumed, because ADR 0011 §6 could later give the
/// extremes their own derivation and the equality would stop being free.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_peg_series_keeps_ohlc_ordered() {
    let db = "it_ohlcv_peg_ordered_0229";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    // An XLM/USDC market to anchor the buckets, plus one awkward measured rate.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-03 13:00:00', 1, 2, 'sdex', 0.2, 0.21, 0.19, 0.2, \
              100, 20, 0.2, 0.2, 5, 1)"
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (timestamp, asset_kind, asset_code, issuer_address, contract_address, \
              usd_rate, method, version) VALUES \
             ('2026-03-03 13:00:00', 'credit', 'USDC', '{i}', '', \
              0.99987654321098, 'oracle', 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2026-03-03T13:00:00Z\
         &end=2026-03-03T13:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let c = &json["data"].as_array().unwrap()[0];

    assert_ohlc_ordered(c, "usdc self-pair");
    assert_eq!(
        c["method"], "oracle",
        "the measured rate must win, not the peg"
    );

    teardown(db).await;
}

/// Task 0229 review finding 2 — an extreme the query cannot compute stays `null`.
///
/// 🔴 ClickHouse's `least`/`greatest` **ignore** null arguments rather than
/// propagating them (verified on 26.3.10.60: `greatest(NULL, 2.5)` = `2.5`, while
/// `NULL + 2.5` = `NULL`). `h_x` is `toDecimal128OrNull` and overflows
/// `Decimal128(38, 14)` when `rate` is large — reachable here with a dust `close`
/// at the precision floor against a ten-figure `close_usd`, giving `rate = 1e22`.
///
/// Unguarded, the clamp would report `high = close`: a value the query failed to
/// compute, presented as if measured. The bucket is still returned; only the
/// unrepresentable field is absent, per ADR 0011 §5.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_an_unrepresentable_extreme_stays_null_rather_than_becoming_the_close() {
    let db = "it_ohlcv_overflow_null_0229";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    // close = 1e-12 (at PRECISION_FLOOR, so the row is `valid`), close_usd = 1e10
    // → rate = 1e22, and high * rate = 1e25 overflows Decimal128(38, 14).
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-03 14:00:00', 3, 2, 'sdex', 1000, 1000, 1000, 0.000000000001, \
              10, 10, 10000000000, 1000, 3, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-03T14:00:00Z\
         &end=2026-03-03T14:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let c = &json["data"].as_array().unwrap()[0];

    assert!(
        c["high"].is_null(),
        "an unrepresentable high must stay null, not become the close; got {}",
        c["high"]
    );
    assert_eq!(
        c["close"].as_str().unwrap(),
        "10000000000",
        "the close itself is representable and must still be served"
    );

    teardown(db).await;
}

/// Task 0229 review finding 1 — the merged `vwap` rounds ABOVE the clamped high.
///
/// `vwap` carries a **second** float round-trip on top of the one the extremes
/// get: `sum(w_x * volume) / sum(volume)`, and `(x*v)/v != x` in IEEE754. The
/// seed puts every price on the same value so the true vwap sits exactly on the
/// bound, which is where a single ulp decides it — `close_usd` and
/// `volume_base = 3` chosen by searching ClickHouse's own arithmetic for a
/// crossing.
///
/// 🔑 Nothing would have surfaced this: [[0120]]'s assertion is
/// `low <= open,close <= high` and never looks at vwap.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_vwap_cannot_round_above_the_high() {
    let db = "it_ohlcv_vwap_above_0229";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-04 10:00:00', 3, 2, 'sdex', 1.0, 1.0, 1.0, 1.0, \
              3, 3, 70000.00000299999232, 1.0, 1, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-04T10:00:00Z\
         &end=2026-03-04T10:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_ohlc_ordered(&json["data"].as_array().unwrap()[0], "vwap above high");

    teardown(db).await;
}

/// The mirror — the merged `vwap` rounds BELOW the clamped low.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_vwap_cannot_round_below_the_low() {
    let db = "it_ohlcv_vwap_below_0229";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-04 11:00:00', 3, 2, 'sdex', 1.0, 1.0, 1.0, 1.0, \
              7, 7, 70000.00001000000512, 1.0, 1, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-04T11:00:00Z\
         &end=2026-03-04T11:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_ohlc_ordered(&json["data"].as_array().unwrap()[0], "vwap below low");

    teardown(db).await;
}

/// The as-stored (`base_currency=XLM`) path needs the vwap clamp too — and this
/// test exists because the first version of it was a **false negative**.
///
/// 🔴 That arm applies no rate, so `o`/`h`/`l`/`c` are stored decimals and cannot
/// cross. A one-source bucket therefore reads clean, and measuring 200,000 of
/// them gave **0 violations** — which read as "structurally safe" and was wrong.
/// The merged vwap is a float weighted mean, and with **two** sources at equal
/// prices the same expression gave **12,017 above `high` and 12,026 below `low`
/// in 200,000 buckets**.
///
/// 🔑 The lesson is the seed, not the fix: a one-row probe of a MERGE aggregate
/// tests a path production does not have.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_xlm_merged_vwap_stays_inside_the_band() {
    let db = "it_ohlcv_xlm_vwap_0229";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    // TWO sources, same bucket, identical prices — the boundary case. The price
    // and the volume pair (3, 2) were found by searching this arm's own
    // expression for a crossing: they put the merged vwap **2.05e-11 below** the
    // stored low. ⚠️ The first seed tried here did NOT violate, and the test
    // passed against the unclamped query — a vacuous test that read as proof.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-03-04 12:00:00', 3, 1, 'sdex', \
              70000.00000099999744, 70000.00000099999744, 70000.00000099999744, \
              70000.00000099999744, 3, 3, 70000.00000099999744, \
              70000.00000099999744, 1, 1), \
             ('2026-03-04 12:00:00', 3, 1, 'soroswap', \
              70000.00000099999744, 70000.00000099999744, 70000.00000099999744, \
              70000.00000099999744, 2, 2, 70000.00000099999744, \
              70000.00000099999744, 1, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/FOO:{}/ohlcv?granularity=1h&start=2026-03-04T12:00:00Z\
         &end=2026-03-04T12:00:00Z&base_currency=XLM",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    assert_ohlc_ordered(&json["data"].as_array().unwrap()[0], "xlm merged vwap");

    teardown(db).await;
}

/// Seed one shared fixture for the task-0246 tests: an XLM/USDC hourly market
/// whose buckets both surfaces read, plus measured rates placed **inside** a
/// bucket rather than on its boundary.
///
/// 🔑 The boundary placement is the whole point. Every earlier fixture in this
/// file put its observations exactly on the bucket start (10:00, 11:00), where
/// resolving at the bucket's START and resolving at its END pick the same row —
/// which is why 26 tests passed against both rules and the divergence survived.
/// Here 10:05 and 10:55 sit strictly inside the 10:00 bucket, so the two rules
/// give different answers and a test can finally tell them apart.
async fn seed_0246(db: &str, admin: &Client) {
    // Three consecutive hourly buckets on the reference market. The view's peg
    // arm reads the same rows (USDC is the quote and never a base here, so
    // `sum(w) = 0` and the placeholder applies).
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-02-10 10:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-02-10 11:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-02-10 12:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();
    // Two observations, BOTH inside the 10:00 bucket. Nothing at all in 11:00
    // or 12:00 — that gap is the simulated oracle outage.
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2026-02-10 10:05:00', 0.99930000000000, 'oracle', '', 0, 1), \
             ('credit', 'USDC', '{i}', '', '2026-02-10 10:55:00', 1.00070000000000, 'oracle', '', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    // 🔴 Review round 2, WR-09 — a SECOND day, on which the rate is IMPORTED
    // rather than polled.
    //
    // The fixture used to seed `oracle` rows only, so the cross-surface test
    // below could not reach the population on which the two surfaces actually
    // diverged: `/ohlcv` floors an imported row at the UTC day start (one daily
    // row is valid for all 24 hours) and `price_usd_series_1h` buckets it by the
    // hour. That gap is what let the divergence ship. It is closed by loading at
    // HOURLY grain — `usd_rate` carries an imported row for every hour — and
    // this seed is that shape: three imported HOURS on 2026-02-11, each with a
    // candle of its own, so both surfaces resolve the same row per bucket.
    //
    // 2026-02-12 00:00 has a candle and NO rate of any method, and it is the
    // control: it must read `1`/'peg' on both surfaces. Without it, a regression
    // that forward-filled the previous day's import across the day boundary
    // would pass — and `/ohlcv`'s day floor is exactly the expression that would
    // do it.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2026-02-11 00:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-02-11 07:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-02-11 23:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2026-02-12 00:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();
    // The three values are the real 2023-03-11 shape, transplanted: the day
    // opens near par, troughs at 07:00 and closes at the daily close. Distinct
    // per hour on purpose — a surface that resolved the wrong hour's row would
    // otherwise still match.
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2026-02-11 00:00:00', 0.99503491000000, 'external', 'chainlink', 'measured', 0, 1), \
             ('credit', 'USDC', '{i}', '', '2026-02-11 07:00:00', 0.88330000000000, 'external', 'chainlink', 'measured', 0, 1), \
             ('credit', 'USDC', '{i}', '', '2026-02-11 23:00:00', 0.96812000000000, 'external', 'chainlink', 'measured', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();
}

/// 🔑 Task 0246 — `/ohlcv` and `price_usd_series_1h` must publish the SAME
/// value for the same identity in the same bucket.
///
/// This is the acceptance criterion the task exists for, and it is deliberately
/// asserted **across the two surfaces** rather than against literals. Each
/// surface already had tests pinning its own rule in isolation, and that
/// isolation is exactly why they drifted apart: both suites were green while the
/// two endpoints answered differently for the same request.
///
/// Against the pre-0246 query this fails on the 10:00 bucket, and not subtly —
/// the old `ASOF … ON r.rts <= b.bkt` found no observation at or before 10:00:00
/// (both sit later in the hour) and rendered the bucket as the `$1` peg, while
/// the view published the measured 1.0007.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_agrees_with_price_usd_series_on_the_same_bucket() {
    let db = "it_ohlcv_0246_cross_surface";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    prices_clickhouse::apply_sql(
        &Client::default().with_url(ch_url()),
        &rewrite(prices_clickhouse::VIEWS_SQL, db),
    )
    .await
    .unwrap();
    seed_0246(db, &admin).await;

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
         &end=2026-02-12T00:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(
        data.len(),
        7,
        "three polled buckets, three IMPORTED ones and the un-priced control: {json}"
    );

    for row in data {
        let bucket = row["timestamp"].as_str().unwrap().replace('T', " ");
        let bucket = bucket.trim_end_matches('Z').to_string();
        let (view_close, view_method) = admin
            .query(&format!(
                "SELECT toString(close_usd), method FROM {db}.price_usd_series_1h \
                 WHERE asset_code = ? AND issuer_address = ? AND bucket = toDateTime(?)"
            ))
            .bind("USDC")
            .bind(iss())
            .bind(bucket.as_str())
            .fetch_one::<(String, String)>()
            .await
            .unwrap_or_else(|e| panic!("view has no row for {bucket}: {e}"));

        let api_close: f64 = row["close"].as_str().unwrap().parse().unwrap();
        let view_close_f: f64 = view_close.parse().unwrap();
        assert!(
            (api_close - view_close_f).abs() < 1e-12,
            "{bucket}: /ohlcv published {api_close} but price_usd_series_1h \
             published {view_close} — the two surfaces must resolve the rate by \
             the same rule (task 0246)"
        );
        assert_eq!(
            row["method"].as_str().unwrap(),
            view_method,
            "{bucket}: the surfaces disagree about PROVENANCE, which is worse \
             than disagreeing about the value — one of them is calling a \
             fallback a measurement"
        );
    }

    // And the agreed value is the bucket's LAST observation, not its first and
    // not the previous bucket's. Asserted once, so a future change that made
    // both surfaces agree on the WRONG row would still fail here.
    approx(&data[0]["close"], 1.0007);
    assert_eq!(data[0]["method"], "oracle");

    // 🔴 Review round 2, WR-09 — the imported day, hour by hour. The loop above
    // proves the two surfaces AGREE; these four assertions prove they agree on
    // the right number, which "both wrong" would otherwise satisfy.
    for (i, (want, method)) in [
        (0.99503491, "external"),
        (0.8833, "external"),
        (0.96812, "external"),
        (1.0, "peg"),
    ]
    .into_iter()
    .enumerate()
    {
        let row = &data[3 + i];
        let ts = row["timestamp"].as_str().unwrap().to_string();
        assert_eq!(
            row["method"], method,
            "{ts}: an imported hour must read `external`, and the day AFTER the              import must NOT — `/ohlcv`'s day floor must not forward-fill across              a UTC day boundary: {json}"
        );
        approx(&row["close"], want);
    }

    teardown(db).await;
}

/// 🔑 Review round 2, WR-09 / the hourly load (Adam, 2026-09-09) — the depeg
/// day at `granularity=1h`, which is the whole argument for loading the hourly
/// grain.
///
/// From a DAILY-only load, every hour of 2023-03-11 reads 0.96812: one row,
/// valid for its whole UTC day by `/ohlcv`'s safety net. That is right for the
/// day and wrong for the hour — the peg troughed at **0.8833** at 07:00 UTC and
/// had largely recovered by 23:00. With the hourly file loaded, `/ohlcv` at
/// `1h` shows the trough where it happened.
///
/// The instants here are the REAL ones from `composed_usdc_usd_1h.csv`, not a
/// transplant: `DEPEG_HOUR_S` and its value are asserted against the versioned
/// file by `composed_usdc_csv.rs`, so this test and the loader cannot disagree
/// about what the answer should be.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_serves_the_depeg_day_hour_by_hour_from_the_hourly_import() {
    let db = "it_ohlcv_0267_hourly_depeg";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);

    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2023-03-11 00:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2023-03-11 07:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1), \
             ('2023-03-11 23:00:00', 1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1)"
        ))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2023-03-11 00:00:00', 0.99503491000000, 'external', 'chainlink', 'measured', 0, 1), \
             ('credit', 'USDC', '{i}', '', '2023-03-11 07:00:00', 0.88330000000000, 'external', 'chainlink', 'measured', 0, 1), \
             ('credit', 'USDC', '{i}', '', '2023-03-11 23:00:00', 0.96812000000000, 'external', 'chainlink', 'measured', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2023-03-11T00:00:00Z\
         &end=2023-03-11T23:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 3, "one bucket per seeded hour: {json}");

    for (row, want) in data.iter().zip([0.99503491, 0.8833, 0.96812]) {
        let ts = row["timestamp"].as_str().unwrap().to_string();
        approx(&row["close"], want);
        assert_eq!(row["method"], "external", "{ts}: {json}");
        assert_eq!(row["source"], "chainlink", "{ts}: {json}");
        assert_eq!(row["quality"], "measured", "{ts}: {json}");
    }

    // The point, stated as its own assertion: the trough hour and the day's
    // close are DIFFERENT numbers. A daily-only load makes them equal, and this
    // is the check that says so out loud.
    assert_ne!(
        data[1]["close"], data[2]["close"],
        "07:00 must not publish the day's close — that is the daily-only \
         behaviour this grain exists to replace: {json}"
    );

    teardown(db).await;
}

/// 🔑 Task 0246 — an oracle gap must fall back to a labelled peg, never
/// forward-fill the last reading as a measurement.
///
/// The pre-0246 query bounded its ASOF by nothing at all, so a bucket any
/// distance after the final observation still resolved to that observation and
/// was labelled `method = 'oracle'`. A dead oracle's last reading would have
/// been served as a live measurement for the entire length of the outage — and
/// the longer the outage, the more confidently wrong the answer.
///
/// Here 11:00 and 12:00 hold no observation of their own. Against the old query
/// both returned 1.0007/`oracle`; they must now return the `$1` fallback and say
/// so.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_does_not_forward_fill_a_stale_rate_into_later_buckets() {
    let db = "it_ohlcv_0246_no_forward_fill";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);
    seed_0246(db, &admin).await;

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1h&start=2026-02-10T10:00:00Z\
         &end=2026-02-10T12:00:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 3, "three buckets expected: {json}");

    // The gap buckets FIRST, so a regression fails on the defect this test is
    // named for rather than on the sanity check below it.
    for i in [1usize, 2usize] {
        let ts = data[i]["timestamp"].as_str().unwrap().to_string();
        assert_eq!(
            data[i]["method"], "peg",
            "{ts}: an oracle gap must render as the labelled peg. \
             'oracle' here means the 10:55 reading forward-filled past its own \
             bucket and is being presented as a measurement (task 0246)"
        );
        approx(&data[i]["close"], 1.0);
    }

    // And the bucket that HAS an observation still publishes the measured rate —
    // the fix must not have simply switched the measurement off.
    approx(&data[0]["close"], 1.0007);
    assert_eq!(data[0]["method"], "oracle");

    teardown(db).await;
}

/// 🔑 Task 0246 — at `1m` the window must be the ORACLE CADENCE, not the bucket.
///
/// The first cut of 0246 scoped the rate strictly to its bucket, copying
/// `price_usd_series`. That is safe for the view because the view exists only at
/// `1d` and `1h`. `/ohlcv` has no such floor: `Timeframe::H1`'s default
/// granularity **is** `1m`, so `?timeframe=1h` with no other parameter returns
/// sixty 1-minute buckets — while `oracleWatcher` polls every 5 minutes.
///
/// Strict bucket scoping therefore left roughly four buckets in five with no
/// observation of their own, and the series alternated between a measured rate
/// and the `$1` fallback every minute, flipping `method` with it. That is worse
/// than the unbounded forward-fill this task removed: a three-minute-old
/// measurement is better evidence than a literal `$1`.
///
/// One observation at 10:00:30, six 1-minute buckets. With the 300 s floor the
/// first five carry it and the sixth has aged past the window:
///
/// | bucket | window floor | verdict |
/// |---|---|---|
/// | 10:00 – 10:04 | 09:56 – 10:00 | `0.9993` / `oracle` |
/// | 10:05 | 10:01 | `1` / `peg` — the reading is now stale |
///
/// So this pins BOTH halves: the measurement carries across the poll gap, and it
/// still stops. A regression in either direction fails here.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_at_1m_carries_a_measurement_across_the_oracle_poll_gap() {
    let db = "it_ohlcv_0246_1m_cadence";
    let client = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);

    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) \
             SELECT toDateTime('2026-02-10 10:00:00') + INTERVAL number MINUTE, \
                    1, 2, 'sdex', 0.25, 0.26, 0.24, 0.25, 900, 225, 0.25, 0.25, 9, 1 \
             FROM numbers(6)"
        ))
        .execute()
        .await
        .unwrap();
    // A single poll, 30 s into the first bucket.
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2026-02-10 10:00:30', 0.99930000000000, 'oracle', '', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    let uri = format!(
        "/v1/assets/USDC:{}/ohlcv?granularity=1m&start=2026-02-10T10:00:00Z\
         &end=2026-02-10T10:05:00Z&base_currency=USD",
        iss()
    );
    let (status, json) = get(client, &uri).await;
    assert_eq!(status, StatusCode::OK, "body={json}");
    let data = json["data"].as_array().unwrap();
    assert_eq!(data.len(), 6, "six 1-minute buckets expected: {json}");

    for row in data.iter().take(5) {
        let ts = row["timestamp"].as_str().unwrap().to_string();
        assert_eq!(
            row["method"], "oracle",
            "{ts}: a measurement inside the oracle's 5-minute cadence must carry \
             across the buckets it spans. 'peg' here means the window was scoped \
             to the 1-minute bucket, and the series is a square wave (task 0246)"
        );
        approx(&row["close"], 0.9993);
    }

    // ...and it still stops. 10:05's window opens at 10:01, after the reading.
    assert_eq!(
        data[5]["method"], "peg",
        "the floor is a WINDOW, not a licence to forward-fill: once the reading \
         is older than ORACLE_POLL_FLOOR_S it must fall back and say so"
    );
    approx(&data[5]["close"], 1.0);

    teardown(db).await;
}

/// 🔑 Task 0268: the candle path's three USDC labels, in one scratch DB.
///
/// `method` is not stored — it is reconstructed from the quote leg, the
/// `close_usd = close` signature and the candle's timestamp
/// (`queries_ch::usd_method_expr`). The arms are ordered, every one of them is
/// valid SQL in any order, and a reordering relabels whole populations on the
/// wire without failing anywhere. This pins all three against a real query
/// planner rather than against the emitted string.
///
/// The three cases, all on a **non-USDC base** so the request takes the candle
/// path and not `ohlcv_peg_series` (a `USDC:...` request, which still says
/// `peg` and is asserted elsewhere in this file):
///
/// 1. **pre-epoch, `close_usd = close`** -> `assumed-par`. Nothing measured it;
///    the literal 1.0 supplied the value.
/// 2. **pre-epoch, scaled** -> `external`. Before the first measured oracle row
///    the only thing that can have scaled a USDC leg is task 0267's imported
///    series. The rate seeded here is USDC's real 2023-03-11 close, 0.9681 —
///    the 3% the whole task exists to stop discarding.
/// 3. **post-epoch, scaled** -> `oracle`. Same signature as case 2, different
///    side of the epoch, and that is the ONLY thing separating them.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn ohlcv_usdc_leg_labels_par_external_and_oracle_by_signature_and_epoch() {
    let db = "it_ohlcv_labels_0268";
    let _ = setup(db).await;
    let admin = Client::default().with_url(ch_url()).with_database(db);

    // asset_id=3 (FOO) quoted in asset_id=2 (USDC). `close = 10` throughout, so
    // the only difference between rows is close_usd and the timestamp.
    //
    // The epoch is 2026-03-11T14:00:00Z (prices_clickhouse::USDC_ORACLE_EPOCH_S).
    // 2026-03-11 13:00 is the last bucket BELOW it and 15:00 the first above —
    // deliberately adjacent, so an off-by-an-hour in the arm's comparison shows
    // up here rather than on prod.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ('2023-03-11 12:00:00', 3, 2, 'sdex', 10, 10, 10, 10, 5, 50, 10,    10, 1, 1), \
             ('2026-03-11 13:00:00', 3, 2, 'sdex', 10, 10, 10, 10, 5, 48, 9.681, 10, 1, 1), \
             ('2026-03-11 15:00:00', 3, 2, 'sdex', 10, 10, 10, 10, 5, 48, 9.681, 10, 1, 1), \
             ('2024-06-01 12:00:00', 3, 2, 'sdex', 10, 10, 10, 10, 5, 48, 9.900, 10, 1, 1)"
        ))
        .execute()
        .await
        .unwrap();

    // ⚠️ The imported series is what makes case 2 `external`, and seeding it is
    // the point of this fixture. The label is decided by whether an imported
    // rate covers the bucket's UTC DAY — not by `close_usd`'s bytes, which a
    // measured rate of exactly 1.0 makes indistinguishable from the assumption
    // (174 of the 2049 imported days close at exactly 1.00000000).
    //
    // 2026-03-11 is covered. 2023-03-11 and 2024-06-01 are deliberately NOT,
    // which is what separates cases 1 and 4 below.
    admin
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, hops, version) VALUES \
             ('credit', 'USDC', '{i}', '', '2026-03-11 13:00:00', 0.96810000000000, \
              'external', 'chainlink', 0, 1)",
            i = iss()
        ))
        .execute()
        .await
        .unwrap();

    for (ts, expected, why) in [
        (
            "2023-03-11T12:00:00Z",
            "assumed-par",
            "close_usd = close exactly: the $1 assumption supplied the value and \
             nothing measured it",
        ),
        (
            "2026-03-11T13:00:00Z",
            "external",
            "scaled below the epoch on a day the imported series covers: task \
             0267's rate priced this, and calling it 'oracle' would claim a poll \
             that never happened",
        ),
        (
            "2026-03-11T15:00:00Z",
            "oracle",
            "same signature as the row above, two hours later: at or after the \
             epoch a scaled USDC leg was priced by a measured Reflector reading",
        ),
        (
            // 🔑 Scaled, below the epoch, on a day NO imported rate covers. The
            // arm list used to answer `external` here on the timestamp alone —
            // asserting a measurement from a series that holds nothing for this
            // day. No tier can produce this state, so the honest answer is null.
            "2024-06-01T12:00:00Z",
            "null",
            "scaled below the epoch but no imported rate covers the day: nothing \
             can attribute this bucket, and a label would be a claim",
        ),
    ] {
        let uri = format!(
            "/v1/assets/FOO:{}/ohlcv?granularity=1h&start={ts}&end={ts}&base_currency=USD",
            iss()
        );
        let (status, json) =
            get(Client::default().with_url(ch_url()).with_database(db), &uri).await;
        assert_eq!(status, StatusCode::OK, "body={json}");
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "{ts}: one bucket expected: {json}");
        if expected == "null" {
            assert!(data[0]["method"].is_null(), "{ts}: {why}: {json}");
        } else {
            assert_eq!(data[0]["method"], expected, "{ts}: {why}");
        }
    }

    // The retired word must not reach the wire on a quote leg at all — a
    // regression that reinstated it would otherwise only show as an unexpected
    // string in one of the three assertions above.
    //
    // ⚠️ Swept as day-wide windows, one per seeded bucket, NOT as one span from
    // 2023 to 2026: that span is ~26329 buckets at `1h` and the handler refuses
    // anything over `OHLCV_MAX_POINTS` (5000) with a 400, so the sweep asserted
    // `OK` against `invalid_query` and this test could never reach its own
    // assertion. Each window still carries a seeded row, which is what the
    // sweep needs.
    for (start, end) in [
        ("2023-03-11T00:00:00Z", "2023-03-12T00:00:00Z"),
        ("2024-06-01T00:00:00Z", "2024-06-02T00:00:00Z"),
        ("2026-03-11T00:00:00Z", "2026-03-12T00:00:00Z"),
    ] {
        let uri = format!(
            "/v1/assets/FOO:{}/ohlcv?granularity=1h&start={start}&end={end}&base_currency=USD",
            iss()
        );
        let (status, json) =
            get(Client::default().with_url(ch_url()).with_database(db), &uri).await;
        assert_eq!(status, StatusCode::OK, "body={json}");
        let rows = json["data"].as_array().unwrap();
        assert!(
            !rows.is_empty(),
            "{start}: the sweep must see its seeded row"
        );
        for c in rows {
            assert_ne!(
                c["method"], "peg",
                "0268 retired this label from the candle path; it survives only \
                 on USDC's own series: {c}"
            );
        }
    }

    teardown(db).await;
}
