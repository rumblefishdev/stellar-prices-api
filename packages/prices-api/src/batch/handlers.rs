//! Axum handler for `/v1/prices/batch`.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::assets::dto::PriceResponse;
use crate::assets::queries_ch;
use crate::batch::dto::{BatchRequest, BatchResponse, MAX_BATCH};
use crate::common::{cache_control, errors};
use crate::identity::AssetIdentifier;
use crate::state::AppState;

/// `POST /prices/batch` — current prices for many assets in one call.
///
/// Validates the whole list first (400 if any identifier is malformed or the
/// batch is empty / over `MAX_BATCH`). Assets with no current-price row are
/// returned in `not_found` rather than failing the request. Uncached.
#[utoipa::path(
    post,
    path = "/prices/batch",
    tag = "prices",
    request_body = BatchRequest,
    responses(
        (status = 200, description = "Current prices + not-found list", body = BatchResponse),
        (status = 400, description = "Empty/oversized batch or invalid identifier"),
    )
)]
pub async fn post_batch(State(state): State<AppState>, Json(req): Json<BatchRequest>) -> Response {
    if req.assets.is_empty() {
        return errors::bad_request(errors::INVALID_QUERY, "assets must not be empty");
    }
    if req.assets.len() > MAX_BATCH {
        return errors::bad_request(
            errors::INVALID_QUERY,
            format!("batch too large (max {MAX_BATCH})"),
        );
    }

    // Parse/validate all up front so a single bad identifier 400s the request.
    let mut ids = Vec::with_capacity(req.assets.len());
    for raw in &req.assets {
        match AssetIdentifier::parse(raw) {
            Ok(id) => ids.push(id),
            Err(e) => {
                return errors::bad_request(errors::INVALID_ID, format!("{raw}: {e}"));
            }
        }
    }

    let mut prices = Vec::new();
    let mut not_found = Vec::new();
    for id in &ids {
        match queries_ch::current_price(state.ch(), id).await {
            Ok(Some(row)) => prices.push(PriceResponse {
                asset: id.to_canonical(),
                price_usd: row.price_usd,
                price_xlm: "0".to_string(),
                vwap_24h: row.vwap_24h,
                volume_24h_usd: row.volume_24h_usd,
                change_24h_pct: "0".to_string(),
                sources: json!({}),
                updated_at: row.updated_at,
            }),
            Ok(None) => not_found.push(id.to_canonical()),
            Err(e) => {
                tracing::error!(error = %e, "batch current_price query failed");
                return errors::internal_error(errors::DB_ERROR, "batch price lookup failed");
            }
        }
    }

    let mut resp = Json(BatchResponse { prices, not_found }).into_response();
    cache_control::attach(&mut resp, cache_control::NO_STORE);
    resp
}
