//! `GET /v1/assets/{id}/ohlcv` negative-input tests (task 0119). CH-less: a
//! clean 400 proves validation rejects before any ClickHouse call (see
//! `tests/common/mod.rs`).

mod common;

use axum::http::StatusCode;

#[tokio::test]
async fn ohlcv_invalid_timeframe_is_400_envelope() {
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?timeframe=17m").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn ohlcv_invalid_granularity_is_400_envelope() {
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?granularity=17m").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// `1H` is not `1h` — and case cannot be forgiven here, because `1m` (minute)
/// and `1M` (month) differ only by case.
#[tokio::test]
async fn ohlcv_granularity_is_case_sensitive() {
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?granularity=1H").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn ohlcv_invalid_base_currency_is_400_envelope() {
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?base_currency=EUR").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// Only the documented `USD`/`XLM` and their all-lowercase aliases are
/// accepted; mixed case is a 400 under the exact-token policy.
#[tokio::test]
async fn ohlcv_mixed_case_base_currency_is_400() {
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?base_currency=uSd").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}
