//! Request/response DTOs for `/v1/prices/batch`.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::assets::dto::PriceResponse;

/// Maximum identifiers per batch request.
pub const MAX_BATCH: usize = 100;

/// `POST /prices/batch` request body.
#[derive(Debug, Deserialize, ToSchema)]
pub struct BatchRequest {
    /// Asset identifiers (`native`, `CODE:ISSUER`, or a C… contract).
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
