//! Live-ClickHouse tests of the paid routes under `/x402/v1/` (task 0356): the
//! answers that need data. Gated `#[ignore]` like the other `_it` targets:
//!
//!   tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!   cargo test -p prices-api --test x402_it -- --ignored --test-threads=1
//!
//! Each test owns a scratch database and drops it at the end. The facilitator
//! is mocked on loopback (`tests/common/x402.rs`), so what is measured is the
//! rule the routes exist for: an answer with data is settled once, and an
//! answer with nothing in it is a `404` that is never settled.

#[path = "common/x402.rs"]
mod common;

use axum::Router;
use axum::http::StatusCode;
use clickhouse::Client;
use common::{MockFacilitator, TRANSACTION, config, pay, request, seller, send, settings};
use prices_api::AppState;
use prices_clickhouse::USDC_ISSUER;
use prices_clickhouse::asset_id::fixture::{AssetFixture, assets_insert};
use serde_json::json;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// A scratch database with XLM priced, DUST tracked with the `unpriced`
/// status, and USDC and GHOST tracked with no price row and no candles.
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
        AssetFixture::new("XLM", "native", "", ""),
        AssetFixture::new("USDC", "credit", USDC_ISSUER, ""),
        AssetFixture::new("DUST", "credit", USDC_ISSUER, ""),
        AssetFixture::new("GHOST", "credit", USDC_ISSUER, ""),
    ];
    admin
        .query(&assets_insert(db, &assets))
        .execute()
        .await
        .unwrap();
    let [xlm, _, dust, _] = assets.map(|asset| asset.id());
    admin
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct, \
              vwap_24h, volume_24h_usd, sources, updated_at, method, as_of, price_status) \
             VALUES \
             ({xlm}, 0.5, 1, -2.5, 7.25, 0.51, 1234.5, '', \
              '2026-02-10 12:00:30', 'traded', '2026-02-10 12:00:00', 'priced'), \
             ({dust}, 0, 0, 0, 0, 0, 0, '', \
              '2026-02-10 12:00:30', '', toDateTime(0), 'unpriced')"
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

/// The application over `client`, selling on `network` through a fresh mock.
async fn seller_on(client: Client, network: &str) -> (Router, MockFacilitator) {
    let facilitator = MockFacilitator::start().await;
    let seller = seller(
        Some(&settings(&facilitator, network)),
        &config(&[]),
        AppState::new(client),
    );
    (seller, facilitator)
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_paid_price_is_answered_settled_once_and_never_cached() {
    let db = "it_x402_price_0356";
    let (seller, facilitator) = seller_on(setup(db).await, "stellar:testnet").await;

    let (status, headers, answer) = pay(&seller, "/x402/v1/assets/native/price", None).await;

    assert_eq!(status, StatusCode::OK, "body={answer}");
    assert_eq!(answer["asset"], "native");
    assert_eq!(answer["price_status"], "priced");
    // The keyed route answers `public, max-age=10`; the next caller has not paid.
    assert_eq!(headers["cache-control"], "no-store, private");
    let receipt = common::decoded(&headers, "payment-response");
    assert_eq!(receipt["transaction"], TRANSACTION);
    assert_eq!(facilitator.settled(), 1);

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_unpriced_asset_is_a_404_that_is_not_charged() {
    let db = "it_x402_unpriced_0356";
    // Pubnet: no sandbox, so an asset off its list can be asked for.
    let (seller, facilitator) = seller_on(setup(db).await, "stellar:pubnet").await;
    let dust = format!("DUST:{USDC_ISSUER}");

    // The keyed route keeps its answer: a 200 that says `unpriced`.
    let (keyed, _, body) = send(
        &seller,
        request(&format!("/v1/assets/{dust}/price"), None, None),
    )
    .await;
    assert_eq!(keyed, StatusCode::OK, "body={body}");
    assert_eq!(body["price_status"], "unpriced");

    let (status, headers, answer) =
        pay(&seller, &format!("/x402/v1/assets/{dust}/price"), None).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body={answer}");
    assert_eq!(answer["code"], "unpriced");
    assert!(headers.get("payment-response").is_none());
    assert_eq!(facilitator.settled(), 0);

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_asset_without_a_price_row_is_a_404_that_is_not_charged() {
    let db = "it_x402_unknown_0356";
    let (seller, facilitator) = seller_on(setup(db).await, "stellar:pubnet").await;
    let ghost = format!("GHOST:{USDC_ISSUER}");

    let (status, _, answer) = pay(&seller, &format!("/x402/v1/assets/{ghost}/price"), None).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body={answer}");
    assert_eq!(answer["code"], "not_found");
    assert_eq!(facilitator.settled(), 0);

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_ohlcv_answer_without_candles_is_a_404_that_is_not_charged() {
    let db = "it_x402_ohlcv_0356";
    let (seller, facilitator) = seller_on(setup(db).await, "stellar:pubnet").await;
    let query = format!("assets/GHOST:{USDC_ISSUER}/ohlcv?timeframe=all&base_currency=XLM");

    // The keyed route keeps its answer: a 200 with an empty series.
    let (keyed, _, body) = send(&seller, request(&format!("/v1/{query}"), None, None)).await;
    assert_eq!(keyed, StatusCode::OK, "body={body}");
    assert_eq!(body["data"], json!([]));

    let (status, _, answer) = pay(&seller, &format!("/x402/v1/{query}"), None).await;

    assert_eq!(status, StatusCode::NOT_FOUND, "body={answer}");
    assert_eq!(answer["code"], "no_data");
    assert_eq!(facilitator.settled(), 0);

    teardown(db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_batch_is_charged_when_it_finds_at_least_one_price() {
    let db = "it_x402_batch_0356";
    let (seller, facilitator) = seller_on(setup(db).await, "stellar:pubnet").await;
    let ghost = format!("GHOST:{USDC_ISSUER}");
    let dust = format!("DUST:{USDC_ISSUER}");

    // Unknown and unpriced assets only: no price to sell.
    let nothing = json!({ "assets": [ghost, dust] });
    let (status, _, answer) = pay(&seller, "/x402/v1/prices/batch", Some(&nothing)).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "body={answer}");
    assert_eq!(answer["code"], "no_data");
    assert_eq!(facilitator.settled(), 0);

    let some = json!({ "assets": ["native", ghost] });
    let (status, headers, answer) = pay(&seller, "/x402/v1/prices/batch", Some(&some)).await;
    assert_eq!(status, StatusCode::OK, "body={answer}");
    assert_eq!(answer["prices"][0]["asset"], "native");
    assert_eq!(answer["not_found"], json!([ghost]));
    assert_eq!(headers["cache-control"], "no-store, private");
    assert_eq!(facilitator.settled(), 1);

    teardown(db).await;
}
