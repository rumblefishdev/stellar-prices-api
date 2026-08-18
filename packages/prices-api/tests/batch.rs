//! `POST /v1/prices/batch` negative-input tests (task 0119). CH-less: a clean
//! 400 proves validation rejects before any ClickHouse call (see
//! `tests/common/mod.rs`).

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};

fn post(body: impl Into<Body>, content_type: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("POST").uri("/v1/prices/batch");
    if let Some(ct) = content_type {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    builder.body(body.into()).unwrap()
}

fn post_json(body: &str) -> Request<Body> {
    post(body.to_string(), Some("application/json"))
}

#[tokio::test]
async fn batch_empty_assets_is_400() {
    let (status, headers, json) = common::send(post_json(r#"{"assets":[]}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    // Rejections are never cacheable.
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn batch_over_max_batch_is_400() {
    let assets: Vec<&str> = std::iter::repeat_n("native", 101).collect();
    let body = serde_json::json!({ "assets": assets }).to_string();
    let (status, _, json) = common::send(post_json(&body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
    assert!(
        json["message"].as_str().unwrap().contains("100"),
        "message should name the cap: {json}"
    );
}

#[tokio::test]
async fn batch_invalid_element_400_names_element() {
    let (status, _, json) =
        common::send(post_json(r#"{"assets":["native","not-an-asset!"]}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_id");
    assert!(
        json["message"].as_str().unwrap().contains("not-an-asset!"),
        "message should name the failing element: {json}"
    );
}

#[tokio::test]
async fn batch_malformed_json_is_400_envelope() {
    let (status, _, json) = common::send(post_json("{not json")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_body");
}

/// `{"assets":"x"}` deserializes the object but fails the field type — axum
/// answered 422 `text/plain` before task 0119.
#[tokio::test]
async fn batch_wrong_shape_is_400_envelope() {
    let (status, _, json) = common::send(post_json(r#"{"assets":"not-an-array"}"#)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_body");
}

#[tokio::test]
async fn batch_missing_field_is_400_envelope() {
    let (status, _, json) = common::send(post_json("{}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_body");
}

/// No `Content-Type: application/json` — axum answered 415 before task 0119.
#[tokio::test]
async fn batch_missing_content_type_is_400_envelope() {
    let (status, _, json) = common::send(post(r#"{"assets":["native"]}"#.to_string(), None)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_body");
}

/// A body over `MAX_BATCH_BODY_BYTES` (16 KB) is refused by the body-limit
/// layer instead of being parsed just to fail the `MAX_BATCH` check.
#[tokio::test]
async fn batch_oversized_body_is_400_envelope() {
    let filler = "x".repeat(100 * 1024);
    let body = format!(r#"{{"assets":["{filler}"]}}"#);
    let (status, _, json) = common::send(post_json(&body)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_body");
}
