//! Axum handler for `/v1/prices/batch`.

use std::collections::HashMap;

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};

use crate::assets::dto::PriceResponse;
use crate::assets::queries_ch;
use crate::batch::dto::{BatchRequest, BatchResponse, MAX_BATCH};
use crate::common::errors::ErrorEnvelope;
use crate::common::extract::ValidatedJson;
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
    summary = "`POST /prices/batch` — current prices for many assets in one call.",
    description = "The same snapshot `GET /assets/{asset_identifier}/price` returns, for up to 100 assets\nin \
     one request. The whole list is validated first: a malformed identifier, an empty list\nor \
     more than 100 entries fails the entire request with a 400. Assets without a current\nprice \
     are listed in `not_found` instead of failing the call. Results follow the order of\nthe \
     request; a duplicated identifier is answered each time. Not cached.",
    request_body = BatchRequest,
    responses(
        (status = 200, description = "Current prices + not-found list", body = BatchResponse),
        (status = 400, description = "Malformed or oversized body (`invalid_body`), empty or over-cap list (`invalid_query`), \
          or an invalid identifier (`invalid_id`)",
         body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid `x-api-key` (`unauthorized`)", body = ErrorEnvelope),
        (status = 403, description = "Rejected at the API gateway: `x-api-key` missing, unknown, or not enabled for this API"),
        (status = 429, description = "Per-key rate limit or monthly quota exceeded"),
        (status = 500, description = "Database or upstream failure (`db_error`)", body = ErrorEnvelope),
    )
)]
pub async fn post_batch(
    State(state): State<AppState>,
    ValidatedJson(req): ValidatedJson<BatchRequest>,
) -> Response {
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

    // One query for the whole batch (vs. a per-asset N+1 loop), then map each
    // requested identifier back to its row by natural-identity key.
    let rows = match queries_ch::current_prices_batch(state.ch(), &ids).await {
        Ok(rows) => rows,
        Err(e) => return errors::db_error(&e, "batch price lookup"),
    };
    let by_key: HashMap<queries_ch::IdentKey, queries_ch::BatchPriceRow> =
        rows.into_iter().map(|r| (r.ident_key(), r)).collect();

    let mut prices = Vec::new();
    let mut not_found = Vec::new();
    for id in &ids {
        match by_key.get(&queries_ch::IdentKey::of(id)) {
            Some(row) => prices.push(PriceResponse::from_row(
                id.to_canonical(),
                queries_ch::CurrentPriceRow {
                    price_usd: row.price_usd.clone(),
                    price_xlm: row.price_xlm.clone(),
                    vwap_24h: row.vwap_24h.clone(),
                    volume_24h_usd: row.volume_24h_usd.clone(),
                    change_24h_pct: row.change_24h_pct.clone(),
                    sources: row.sources.clone(),
                    updated_at: row.updated_at.clone(),
                    method: row.method.clone(),
                },
            )),
            None => not_found.push(id.to_canonical()),
        }
    }

    let mut resp = Json(BatchResponse { prices, not_found }).into_response();
    cache_control::attach(&mut resp, cache_control::NO_STORE);
    resp
}
