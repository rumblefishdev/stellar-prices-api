//! The paid routes under `/x402/v1/` (task 0356), with a facilitator mocked on
//! loopback and no ClickHouse: [`AppState::without_ch`] panics on any query, so
//! a clean refusal here also proves it was given before the database was
//! touched. The answers that need data are in `tests/x402_it.rs`.

#[path = "common/x402.rs"]
mod common;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{
    AQUA, MockFacilitator, PAY_TO, USDC, USDC_CONTRACT, config, pay, request, required, seller,
    send, settings,
};
use prices_api::AppState;
use prices_api::x402::Settings;
use serde_json::json;

async fn testnet_seller() -> (Router, MockFacilitator) {
    let facilitator = MockFacilitator::start().await;
    let seller = seller(
        Some(&settings(&facilitator, "stellar:testnet")),
        &config(&[]),
        AppState::without_ch(),
    );
    (seller, facilitator)
}

#[tokio::test]
async fn without_settings_the_paid_paths_do_not_exist() {
    let seller = seller(None, &config(&[]), AppState::without_ch());

    let (paid, _, _) = send(&seller, request("/x402/v1/assets/native/price", None, None)).await;
    let (unknown, _, _) = send(&seller, request("/no/such/path", None, None)).await;

    assert_eq!(paid, StatusCode::NOT_FOUND);
    assert_eq!(paid, unknown);
}

#[tokio::test]
async fn a_wrong_setting_leaves_the_application_running_without_paid_routes() {
    let facilitator = MockFacilitator::start().await;
    for wrong in [
        Settings {
            pay_to: "GABC".to_owned(),
            ..settings(&facilitator, "stellar:testnet")
        },
        Settings {
            price_usd: "free".to_owned(),
            ..settings(&facilitator, "stellar:testnet")
        },
        settings(&facilitator, "stellar:mainnet"),
    ] {
        let seller = seller(Some(&wrong), &config(&[]), AppState::without_ch());

        let (paid, _, _) = send(&seller, request("/x402/v1/assets/native/price", None, None)).await;
        let (health, _, _) = send(&seller, request("/health", None, None)).await;

        assert_eq!(paid, StatusCode::NOT_FOUND, "{wrong:?}");
        assert_eq!(health, StatusCode::OK, "{wrong:?}");
    }
    assert_eq!(facilitator.asked(), Vec::<String>::new());
}

#[tokio::test]
async fn an_unpaid_request_gets_the_terms_and_the_listing() {
    let (seller, facilitator) = testnet_seller().await;
    assert_eq!(facilitator.asked(), Vec::<String>::new());

    let (status, headers, _) =
        send(&seller, request("/x402/v1/assets/native/price", None, None)).await;

    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(headers["cache-control"], "no-store");
    let terms = required(&headers);
    assert_eq!(
        terms["resource"]["url"],
        "https://prices-api.example.com/x402/v1/assets/native/price"
    );
    assert_eq!(terms["accepts"][0]["network"], "stellar:testnet");
    assert_eq!(terms["accepts"][0]["payTo"], PAY_TO);
    // 0.001 USDC in its 7 decimals.
    assert_eq!(terms["accepts"][0]["amount"], "10000");
    // One catalog listing for every asset, not one each.
    let listing = &terms["extensions"]["bazaar"];
    assert_eq!(
        listing["routeTemplate"],
        "/x402/v1/assets/:asset_identifier/price"
    );
    assert_eq!(
        listing["info"]["input"]["pathParams"],
        json!({ "asset_identifier": "native" })
    );

    let (_, headers, _) = send(
        &seller,
        request("/x402/v1/assets/native/ohlcv?timeframe=1h", None, None),
    )
    .await;
    assert_eq!(required(&headers)["accepts"][0]["amount"], "50000");
    // The facilitator was asked what it supports once, and nothing else.
    assert_eq!(facilitator.asked(), ["/supported"]);
}

#[tokio::test]
async fn the_catalogs_own_check_of_the_batch_route_gets_402() {
    // The catalog checks a listed POST route with no body: the terms must come
    // before the body is read, or it would see a 400 and drop the listing.
    let (seller, _) = testnet_seller().await;
    let probe = Request::builder()
        .method("POST")
        .uri("/x402/v1/prices/batch")
        .body(Body::empty())
        .unwrap();

    let (status, headers, _) = send(&seller, probe).await;

    assert_eq!(status, StatusCode::PAYMENT_REQUIRED);
    let terms = required(&headers);
    assert_eq!(terms["accepts"][0]["amount"], "50000");
    assert_eq!(
        terms["extensions"]["bazaar"]["info"]["input"]["method"],
        "POST"
    );
    assert_eq!(terms["extensions"]["bazaar"].get("routeTemplate"), None);
}

#[tokio::test]
async fn the_paid_routes_need_no_api_key_and_the_keyed_routes_still_do() {
    let facilitator = MockFacilitator::start().await;
    let seller = seller(
        Some(&settings(&facilitator, "stellar:testnet")),
        &config(&["a-key"]),
        AppState::without_ch(),
    );

    let (paid, _, _) = send(&seller, request("/x402/v1/assets/native/price", None, None)).await;
    let (keyed, _, body) = send(&seller, request("/v1/assets/native/price", None, None)).await;
    let (unknown, _, _) = send(&seller, request("/x402/v1/assets", None, None)).await;
    let (elsewhere, _, _) = send(&seller, request("/no/such/path", None, None)).await;

    assert_eq!(paid, StatusCode::PAYMENT_REQUIRED);
    assert_eq!(keyed, StatusCode::UNAUTHORIZED);
    assert_eq!(body["code"], "unauthorized");
    // Only the three routes are keyless, not the prefix, and an unknown path
    // answers what it answered before the paid routes were there.
    assert_eq!(unknown, StatusCode::UNAUTHORIZED);
    assert_eq!(elsewhere, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn a_request_outside_the_sandbox_is_refused_and_not_charged() {
    let (seller, facilitator) = testnet_seller().await;
    let six = json!({ "assets": ["native", "native", "native", "native", "native", "native"] });
    let unlisted = json!({ "assets": ["native", AQUA] });

    for (uri, body) in [
        (format!("/x402/v1/assets/{AQUA}/price"), None),
        (format!("/x402/v1/assets/{USDC_CONTRACT}/price"), None),
        (format!("/x402/v1/assets/{USDC_CONTRACT}/ohlcv"), None),
        ("/x402/v1/assets/native/ohlcv?timeframe=7d".to_owned(), None),
        (
            "/x402/v1/assets/native/ohlcv?timeframe=all".to_owned(),
            None,
        ),
        (
            "/x402/v1/assets/native/ohlcv?start=2026-01-01T00:00:00Z".to_owned(),
            None,
        ),
        (
            "/x402/v1/assets/native/ohlcv?timeframe=1h&end=2026-01-01T00:00:00Z".to_owned(),
            None,
        ),
        ("/x402/v1/prices/batch".to_owned(), Some(&six)),
        ("/x402/v1/prices/batch".to_owned(), Some(&unlisted)),
    ] {
        let (status, headers, answer) = pay(&seller, &uri, body).await;

        assert_eq!(status, StatusCode::FORBIDDEN, "{uri} {body:?}");
        assert_eq!(answer["code"], "sandbox_scope", "{uri}");
        assert_eq!(headers["cache-control"], "no-store", "{uri}");
        assert!(headers.get("payment-response").is_none(), "{uri}");
    }
    // Every payment was verified, and none was settled.
    assert_eq!(facilitator.settled(), 0);
    assert!(facilitator.asked().contains(&"/verify".to_owned()));
}

#[tokio::test]
async fn a_listed_asset_passes_the_sandbox_however_the_path_writes_it() {
    let (seller, facilitator) = testnet_seller().await;

    // `min_volume_usd=-1` is refused by the keyed handler itself, after the
    // sandbox and before any query: a 400 here is the sandbox letting it by.
    for asset in ["native", "Native", &USDC.replace(':', "%3A"), USDC] {
        let uri = format!("/x402/v1/assets/{asset}/price?min_volume_usd=-1");

        let (status, _, answer) = pay(&seller, &uri, None).await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}");
        assert_eq!(answer["code"], "invalid_query", "{uri}");
    }
    // A request the route refuses costs nothing.
    assert_eq!(facilitator.settled(), 0);
}

#[tokio::test]
async fn on_pubnet_there_is_no_sandbox() {
    let facilitator = MockFacilitator::start().await;
    let seller = seller(
        Some(&settings(&facilitator, "stellar:pubnet")),
        &config(&[]),
        AppState::without_ch(),
    );
    let uri = format!("/x402/v1/assets/{AQUA}/price?min_volume_usd=-1");

    let (_, headers, _) = send(&seller, request(&uri, None, None)).await;
    let (status, _, answer) = pay(&seller, &uri, None).await;

    assert_eq!(
        required(&headers)["accepts"][0]["network"],
        "stellar:pubnet"
    );
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(answer["code"], "invalid_query");
    assert_eq!(facilitator.settled(), 0);
}

#[tokio::test]
async fn a_facilitator_outage_costs_the_paid_routes_only() {
    let (seller, facilitator) = testnet_seller().await;
    facilitator.go_down();

    let (paid, headers, _) =
        send(&seller, request("/x402/v1/assets/native/price", None, None)).await;
    let (health, _, _) = send(&seller, request("/health", None, None)).await;
    let (spec, _, _) = send(&seller, request("/api-docs-json", None, None)).await;

    // The crate answers as `@x402/express` does for a facilitator it could not read.
    assert_eq!(paid, StatusCode::INTERNAL_SERVER_ERROR);
    assert!(headers.get("payment-required").is_none());
    assert_eq!(health, StatusCode::OK);
    assert_eq!(spec, StatusCode::OK);
}
