//! `GET /v1/assets` negative-input tests (task 0119). CH-less: a clean 400
//! proves validation rejects before any ClickHouse call (see
//! `tests/common/mod.rs`).

mod common;

use axum::http::StatusCode;

#[tokio::test]
async fn assets_invalid_type_is_400_envelope() {
    let (status, _, json) = common::get("/v1/assets?type=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_invalid_sort_message_lists_valid_values() {
    let (status, _, json) = common::get("/v1/assets?sort=bogus").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    let msg = json["message"].as_str().unwrap();
    assert!(
        msg.contains("volume_24h"),
        "message should enumerate valid values: {msg}"
    );
}

#[tokio::test]
async fn assets_invalid_order_is_400_envelope() {
    let (status, _, json) = common::get("/v1/assets?order=upwards").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// Exact-token policy: documented tokens are lowercase; `PRICE` is not `price`.
#[tokio::test]
async fn assets_sort_is_case_sensitive() {
    let (status, _, json) = common::get("/v1/assets?sort=PRICE").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_limit_zero_is_400() {
    let (status, _, json) = common::get("/v1/assets?limit=0").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_limit_over_max_is_400() {
    let (status, _, json) = common::get("/v1/assets?limit=201").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// `limit=abc` fails in the `Query` extractor — axum answered `text/plain`
/// before task 0119.
#[tokio::test]
async fn assets_limit_non_numeric_is_400_envelope() {
    let (status, _, json) = common::get("/v1/assets?limit=abc").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_limit_negative_is_400_envelope() {
    let (status, _, json) = common::get("/v1/assets?limit=-1").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_search_too_long_is_400() {
    let (status, _, json) = common::get("/v1/assets?search=THIRTEENCHARS").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_search_bad_charset_is_400() {
    let (status, _, json) = common::get("/v1/assets?search=US%21").await; // "US!"
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_duplicate_query_key_is_400_envelope() {
    let (status, _, json) = common::get("/v1/assets?sort=price&sort=code").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}
