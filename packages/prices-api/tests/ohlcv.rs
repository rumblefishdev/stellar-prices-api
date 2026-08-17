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

// ---- start/end + window rule (task 0119: explicit rejection replaces the
// silent truncation at OHLCV_MAX_POINTS) ----

#[tokio::test]
async fn ohlcv_impossible_calendar_date_is_400() {
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?start=2026-02-30").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn ohlcv_impossible_time_is_400() {
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?end=2026-06-15T99:99:99").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn ohlcv_garbage_start_is_400() {
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?start=notadate").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn ohlcv_end_not_after_start_is_400() {
    let (status, _, json) =
        common::get("/v1/assets/native/ohlcv?start=2026-06-15&end=2026-06-15").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    assert!(
        json["message"].as_str().unwrap().contains("before"),
        "message should say start must be before end: {json}"
    );
}

#[tokio::test]
async fn ohlcv_future_start_without_end_is_400() {
    // eff_end = now, so a future-only start inverts the window.
    let (status, _, json) = common::get("/v1/assets/native/ohlcv?start=2099-01-01").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// Fencepost: the SQL bounds are inclusive, so a span of exactly 5000×1m
/// holds 5001 bucket starts — must be an explicit 400, not a silent LIMIT
/// trim of the oldest candle.
#[tokio::test]
async fn ohlcv_exact_5000_bucket_span_is_400() {
    let (status, _, json) = common::get(
        "/v1/assets/native/ohlcv?start=2026-01-01T00:00:00Z&end=2026-01-04T11:20:00Z&granularity=1m",
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// 6 years at 1m ≈ 3.1M candles — must be an explicit 400 naming the count,
/// not a silent newest-5000 page.
#[tokio::test]
async fn ohlcv_explicit_range_over_max_points_is_400() {
    let (status, _, json) =
        common::get("/v1/assets/native/ohlcv?start=2020-01-01&end=2026-01-01&granularity=1m").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    let msg = json["message"].as_str().unwrap();
    assert!(
        msg.contains("5000") && msg.contains("granularity"),
        "message should name the cap and suggest coarser granularity: {msg}"
    );
}

#[tokio::test]
async fn ohlcv_timeframe_1y_granularity_1m_is_400() {
    let (status, _, json) =
        common::get("/v1/assets/native/ohlcv?timeframe=1y&granularity=1m").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn ohlcv_timeframe_all_granularity_1m_is_400() {
    let (status, _, json) =
        common::get("/v1/assets/native/ohlcv?timeframe=all&granularity=1m").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}
