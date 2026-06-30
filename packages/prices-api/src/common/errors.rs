//! Canonical JSON error envelope + builders.
//!
//! Flat shape (fields at the top level, not nested under an `error` key),
//! matching BE so clients see one error format across both APIs. Codes are
//! machine-readable constants extended per phase as endpoints land.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

/// The wire shape for every error response.
#[derive(Debug, Serialize)]
pub struct ErrorEnvelope {
    /// Stable machine-readable code (see the `*` constants below).
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
/// An upstream ClickHouse query failed.
pub const DB_ERROR: &str = "db_error";

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
