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

// ---- cursor (task 0119: malformed/truncated/foreign → 400, never a silent
// first page, never a 500 out of `toFloat64`) ----

fn cursor_token(json: &str) -> String {
    use base64::Engine;
    // STANDARD base64 can emit `+`/`=` — percent-encode like a client would.
    base64::engine::general_purpose::STANDARD
        .encode(json)
        .replace('+', "%2B")
        .replace('=', "%3D")
}

#[tokio::test]
async fn assets_cursor_not_base64_is_400() {
    let (status, _, json) = common::get("/v1/assets?cursor=!!!not-base64").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_cursor_valid_base64_wrong_json_is_400() {
    let (status, _, json) = common::get("/v1/assets?cursor=YWJj").await; // "abc"
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// A foreign token with extra keys must not be accepted as a lookalike.
#[tokio::test]
async fn assets_cursor_with_unknown_field_is_400() {
    let tok = cursor_token(r#"{"v":"1.5","id":1,"extra":true}"#);
    let (status, _, json) = common::get(&format!("/v1/assets?cursor={tok}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// Well-formed token, non-numeric `v`, numeric sort — the pre-0119 path to a
/// ClickHouse `toFloat64` throw (500).
#[tokio::test]
async fn assets_cursor_non_numeric_v_on_numeric_sort_is_400() {
    let tok = cursor_token(r#"{"v":"notanumber","id":1}"#);
    let (status, _, json) = common::get(&format!("/v1/assets?sort=volume_24h&cursor={tok}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

/// A numeric cursor replayed onto `sort=code` fails the asset-code shape.
#[tokio::test]
async fn assets_cursor_numeric_v_on_code_sort_is_400() {
    let tok = cursor_token(r#"{"v":"1523400.50","id":1}"#);
    let (status, _, json) = common::get(&format!("/v1/assets?sort=code&cursor={tok}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}

#[tokio::test]
async fn assets_cursor_over_long_token_is_400() {
    let tok = "A".repeat(300);
    let (status, _, json) = common::get(&format!("/v1/assets?cursor={tok}")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json["code"], "invalid_query");
}
