//! Canonical JSON error envelope + builders.
//!
//! Flat shape (fields at the top level, not nested under an `error` key),
//! matching BE so clients see one error format across both APIs. Codes are
//! machine-readable constants extended per phase as endpoints land.

use axum::Json;
use axum::http::StatusCode;
use axum::http::header::CACHE_CONTROL;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::common::cache_control;

/// The wire shape for every error response. Published in the OpenAPI document
/// as the `body` of every 4xx/5xx response (task 0124), so a client can code
/// against `code` rather than parsing `message`.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorEnvelope {
    /// Stable machine-readable code (see the `*` constants below).
    #[schema(example = "invalid_id")]
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Optional structured context (omitted from the JSON when absent).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

impl ErrorEnvelope {
    fn into_response_with(self, status: StatusCode) -> Response {
        (status, Json(self)).into_response()
    }
}

// ---- canonical error codes (extended per phase) ----

/// A path/query identifier was malformed.
pub const INVALID_ID: &str = "invalid_id";
/// Query parameters failed validation.
pub const INVALID_QUERY: &str = "invalid_query";
/// The requested resource does not exist.
pub const NOT_FOUND: &str = "not_found";
/// Missing or invalid `X-API-Key`.
pub const UNAUTHORIZED: &str = "unauthorized";
/// An upstream ClickHouse query failed.
pub const DB_ERROR: &str = "db_error";
/// A required quote asset (e.g. USDC for USD) is not tracked — pricing in the
/// requested `base_currency` cannot be served.
pub const QUOTE_UNAVAILABLE: &str = "quote_unavailable";

/// 400 Bad Request with a machine-readable `code`.
pub fn bad_request(code: &'static str, message: impl Into<String>) -> Response {
    ErrorEnvelope {
        code,
        message: message.into(),
        details: None,
    }
    .into_response_with(StatusCode::BAD_REQUEST)
}

/// 404 Not Found.
pub fn not_found(message: impl Into<String>) -> Response {
    ErrorEnvelope {
        code: NOT_FOUND,
        message: message.into(),
        details: None,
    }
    .into_response_with(StatusCode::NOT_FOUND)
}

/// 500 Internal Server Error with a machine-readable `code`.
pub fn internal_error(code: &'static str, message: impl Into<String>) -> Response {
    ErrorEnvelope {
        code,
        message: message.into(),
        details: None,
    }
    .into_response_with(StatusCode::INTERNAL_SERVER_ERROR)
}

/// Map a failed ClickHouse query to a `DB_ERROR` 500. Logs the underlying error
/// (with `context` for correlation) and never leaks it to the client. `context`
/// names the operation, e.g. `db_error(&e, "price lookup")` → "price lookup
/// failed". One place to change DB-error policy (status, fields) for every
/// handler, instead of re-deriving the `tracing::error! + internal_error` arm.
pub fn db_error(e: &clickhouse::error::Error, context: &str) -> Response {
    tracing::error!(error = %e, context, "clickhouse query failed");
    internal_error(DB_ERROR, format!("{context} failed"))
}

/// 503 Service Unavailable with a machine-readable `code`. For a transient or
/// data-availability condition the client may retry (e.g. a required quote
/// asset not yet tracked).
pub fn service_unavailable(code: &'static str, message: impl Into<String>) -> Response {
    ErrorEnvelope {
        code,
        message: message.into(),
        details: None,
    }
    .into_response_with(StatusCode::SERVICE_UNAVAILABLE)
}

/// 401 Unauthorized. Carries `Cache-Control: no-store` so a rejection is never
/// cached by the client or the gateway.
pub fn unauthorized(message: impl Into<String>) -> Response {
    let mut resp = ErrorEnvelope {
        code: UNAUTHORIZED,
        message: message.into(),
        details: None,
    }
    .into_response_with(StatusCode::UNAUTHORIZED);
    resp.headers_mut()
        .insert(CACHE_CONTROL, cache_control::NO_STORE);
    resp
}
