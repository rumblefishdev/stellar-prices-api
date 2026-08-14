//! Request/response DTOs for `/v1/prices/batch`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::assets::dto::PriceResponse;

/// Maximum identifiers per batch request.
pub const MAX_BATCH: usize = 100;

/// Request-body size limit for `/prices/batch`, enforced by a
/// `DefaultBodyLimit` layer on the resource router. Sized from the payload it
/// bounds: `MAX_BATCH` identifiers × ~70 chars (`CODE:ISSUER`) ≈ 7.5 KB, so
/// 16 KB is generous while stopping a multi-megabyte body from being parsed
/// just to fail the `MAX_BATCH` check.
pub const MAX_BATCH_BODY_BYTES: usize = 16 * 1024;

/// `POST /prices/batch` request body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchRequest {
    /// Asset identifiers (`native`, `CODE:ISSUER`, or a C… contract).
    /// Bounds mirror the handler's checks (`MAX_BATCH`); attribute macros
    /// cannot read the const, so the literals are asserted back out of the
    /// served document by `tests/openapi.rs`.
    #[schema(min_items = 1, max_items = 100)]
    pub assets: Vec<String>,
}

/// `POST /prices/batch` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct BatchResponse {
    /// Current prices for assets that have one.
    pub prices: Vec<PriceResponse>,
    /// Echoed identifiers with no current-price row.
    pub not_found: Vec<String>,
}
