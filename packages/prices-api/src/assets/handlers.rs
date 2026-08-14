//! Axum handlers for the `/v1/assets` resource.

use axum::Json;
use axum::extract::State;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::assets::dto::{
    AssetDetail, AssetListItem, AssetListResponse, OhlcvResponse, PriceResponse,
};
use crate::assets::queries_ch::{
    self, BaseCurrency, Granularity, ListArgs, OhlcvArgs, Order, SortCol, Timeframe, TypeFilter,
};
use crate::common::errors::ErrorEnvelope;
use crate::common::extract::{ValidatedPath, ValidatedQuery};
use crate::common::{cache_control, cursor, errors};
use crate::identity::AssetIdentifier;
use crate::state::AppState;

/// Cap on candles returned per OHLCV request (bounds response size).
const OHLCV_MAX_POINTS: u64 = 5000;

/// Default / maximum page size for `GET /assets` (overview §4.1).
const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

/// `GET /assets/{asset_identifier}/price` — current price for one asset.
///
/// Parsing/validation runs before any DB call, so a malformed identifier 400s
/// without touching ClickHouse. `price_xlm`, `change_24h_pct` and `sources` are
/// materialized producer-side by `mv_current_prices` (task 0072) and pass
/// straight through, so this stays a point lookup.
#[utoipa::path(
    get,
    path = "/assets/{asset_identifier}/price",
    tag = "prices",
    params(
        ("asset_identifier" = String, Path,
         description = "native, CODE:ISSUER, or a C… contract address")
    ),
    responses(
        (status = 200, description = "Current price", body = PriceResponse),
        (status = 400, description = "Invalid asset identifier", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid `x-api-key`", body = ErrorEnvelope),
        (status = 403, description = "API key missing, invalid, or not authorized for this API"),
        (status = 404, description = "No current price for the asset", body = ErrorEnvelope),
        (status = 429, description = "Per-key rate limit or monthly quota exceeded (API Gateway usage plan)"),
        (status = 500, description = "Query or upstream failure (`db_error`)", body = ErrorEnvelope),
    )
)]
pub async fn get_price(
    State(state): State<AppState>,
    ValidatedPath(raw): ValidatedPath<String>,
) -> Response {
    let id = match AssetIdentifier::parse(&raw) {
        Ok(id) => id,
        Err(e) => return errors::bad_request(errors::INVALID_ID, e.to_string()),
    };

    match queries_ch::current_price(state.ch(), &id).await {
        Ok(Some(row)) => {
            let body = PriceResponse::from_row(id.to_canonical(), row);
            let mut resp = Json(body).into_response();
            cache_control::attach(&mut resp, cache_control::SHORT);
            resp
        }
        Ok(None) => errors::not_found("no current price for the asset"),
        Err(e) => errors::db_error(&e, "price lookup"),
    }
}

/// `GET /assets/{asset_identifier}` — single-asset metadata.
#[utoipa::path(
    get,
    path = "/assets/{asset_identifier}",
    tag = "assets",
    params(
        ("asset_identifier" = String, Path,
         description = "native, CODE:ISSUER, or a C… contract address")
    ),
    responses(
        (status = 200, description = "Asset detail", body = AssetDetail),
        (status = 400, description = "Invalid asset identifier", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid `x-api-key`", body = ErrorEnvelope),
        (status = 403, description = "API key missing, invalid, or not authorized for this API"),
        (status = 404, description = "Unknown asset", body = ErrorEnvelope),
        (status = 429, description = "Per-key rate limit or monthly quota exceeded (API Gateway usage plan)"),
        (status = 500, description = "Query or upstream failure (`db_error`)", body = ErrorEnvelope),
    )
)]
pub async fn get_asset(
    State(state): State<AppState>,
    ValidatedPath(raw): ValidatedPath<String>,
) -> Response {
    let id = match AssetIdentifier::parse(&raw) {
        Ok(id) => id,
        Err(e) => return errors::bad_request(errors::INVALID_ID, e.to_string()),
    };

    match queries_ch::asset_detail(state.ch(), &id).await {
        Ok(Some(row)) => {
            let asset_kind = if !row.contract_address.is_empty() {
                "contract"
            } else if row.asset_code == "XLM" && row.issuer_address.is_empty() {
                "native"
            } else {
                "credit"
            };
            let body = AssetDetail {
                asset: id.to_canonical(),
                asset_kind: asset_kind.to_string(),
                code: row.asset_code,
                issuer: row.issuer_address,
                contract: row.contract_address,
                home_domain: row.home_domain,
                is_active: row.is_active != 0,
            };
            let mut resp = Json(body).into_response();
            cache_control::attach(&mut resp, cache_control::MEDIUM);
            resp
        }
        Ok(None) => errors::not_found("unknown asset"),
        Err(e) => errors::db_error(&e, "asset lookup"),
    }
}

/// Cap on the `?search` prefix: SEP-11 `alphanum12` — an asset-code prefix
/// longer than 12 chars can never match anything.
const MAX_SEARCH_LEN: usize = 12;

/// Query parameters for `GET /assets`. Enum params deserialize straight into
/// their typed forms — an unknown token fails serde and surfaces as a 400
/// through `ValidatedQuery`; unknown query *keys* are deliberately ignored
/// (forward-compatible, matches API Gateway cache-key behavior).
#[derive(Debug, Deserialize)]
pub struct ListParams {
    #[serde(rename = "type")]
    pub asset_type: Option<TypeFilter>,
    pub search: Option<String>,
    pub sort: Option<SortCol>,
    pub order: Option<Order>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}

/// `GET /assets` — paginated, sortable, filterable list of tracked assets.
#[utoipa::path(
    get,
    path = "/assets",
    tag = "assets",
    params(
        ("type" = Option<TypeFilter>, Query, description = "classic | soroban | all (default all)"),
        ("search" = Option<String>, Query,
         description = "asset code prefix match (1-12 ASCII alphanumeric)",
         min_length = 1, max_length = 12),
        ("sort" = Option<SortCol>, Query,
         description = "price | volume_24h | change_24h | code (default volume_24h)"),
        ("order" = Option<Order>, Query, description = "asc | desc (default desc)"),
        ("cursor" = Option<String>, Query, description = "opaque pagination cursor"),
        ("limit" = Option<u32>, Query, description = "1..=200 (default 50)",
         minimum = 1, maximum = 200),
    ),
    responses(
        (status = 200, description = "Asset list page", body = AssetListResponse),
        (status = 400, description = "Invalid query parameter", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid `x-api-key`", body = ErrorEnvelope),
        (status = 403, description = "API key missing, invalid, or not authorized for this API"),
        (status = 429, description = "Per-key rate limit or monthly quota exceeded (API Gateway usage plan)"),
        (status = 500, description = "Query or upstream failure (`db_error`)", body = ErrorEnvelope),
    )
)]
pub async fn get_assets(
    State(state): State<AppState>,
    ValidatedQuery(p): ValidatedQuery<ListParams>,
) -> Response {
    let sort = p.sort.unwrap_or(SortCol::Volume24h);
    let order = p.order.unwrap_or(Order::Desc);
    let type_filter = p.asset_type.unwrap_or(TypeFilter::All);
    let limit = p.limit.unwrap_or(DEFAULT_LIMIT);
    if limit == 0 || limit > MAX_LIMIT {
        return errors::bad_request(errors::INVALID_QUERY, "limit must be 1..=200");
    }
    let cursor = match p.cursor.as_deref() {
        Some(tok) => match cursor::decode(tok) {
            Some(c) => Some(c),
            None => return errors::bad_request(errors::INVALID_QUERY, "invalid cursor"),
        },
        None => None,
    };
    // Empty search is treated as absent; a non-empty one must be a plausible
    // asset-code prefix — anything longer or outside SEP-11's alphanumeric set
    // can never match and only costs a ClickHouse scan.
    let search = p.search.filter(|s| !s.is_empty());
    if let Some(s) = &search
        && (s.len() > MAX_SEARCH_LEN || !s.bytes().all(|b| b.is_ascii_alphanumeric()))
    {
        return errors::bad_request(
            errors::INVALID_QUERY,
            format!("search must be 1-{MAX_SEARCH_LEN} ASCII alphanumeric characters"),
        );
    }

    let args = ListArgs {
        sort,
        order,
        type_filter,
        search,
        cursor,
        fetch_limit: limit as u64 + 1,
    };
    let mut rows = match queries_ch::list_assets(state.ch(), args).await {
        Ok(rows) => rows,
        Err(e) => return errors::db_error(&e, "asset list"),
    };

    let has_more = rows.len() as u64 > limit as u64;
    if has_more {
        rows.truncate(limit as usize);
    }
    let next_cursor = if has_more {
        rows.last().map(|r| cursor::encode(&r.sort_key, r.asset_id))
    } else {
        None
    };

    let data = rows
        .into_iter()
        .map(|r| AssetListItem {
            asset_type: if r.contract_address.is_empty() {
                "classic".to_string()
            } else {
                "soroban".to_string()
            },
            asset_code: r.asset_code,
            issuer_address: r.issuer_address,
            contract_address: r.contract_address,
            home_domain: r.home_domain,
            price_usd: r.price_usd,
            change_24h_pct: r.change_24h_pct,
            change_7d_pct: r.change_7d_pct,
            volume_24h_usd: r.volume_24h_usd,
            vwap_24h: r.vwap_24h,
            sources: crate::assets::dto::parse_sources(&r.sources),
            updated_at: r.updated_at,
        })
        .collect();

    let mut resp = Json(AssetListResponse {
        data,
        cursor: next_cursor,
        has_more,
    })
    .into_response();
    cache_control::attach(&mut resp, cache_control::MEDIUM);
    resp
}

/// Query parameters for `GET /assets/{id}/ohlcv`. Enum params deserialize
/// straight into their typed forms (see [`ListParams`] for the policy).
#[derive(Debug, Deserialize)]
pub struct OhlcvParams {
    pub timeframe: Option<Timeframe>,
    pub granularity: Option<Granularity>,
    pub start: Option<String>,
    pub end: Option<String>,
    pub base_currency: Option<BaseCurrency>,
}

/// `GET /assets/{asset_identifier}/ohlcv` — candlestick history.
///
/// O/H/L/C are denominated in `base_currency` (USD→USDC quote, XLM→native quote)
/// and returned as stored — no conversion. Candles merge across sources per
/// bucket. `backfill_note` appears only for `timeframe=all` while the backfill
/// is still running.
#[utoipa::path(
    get,
    path = "/assets/{asset_identifier}/ohlcv",
    tag = "prices",
    params(
        ("asset_identifier" = String, Path, description = "native, CODE:ISSUER, or a C… contract"),
        ("timeframe" = Option<Timeframe>, Query,
         description = "1h | 24h | 7d | 30d | 1y | all (default 24h)"),
        ("granularity" = Option<Granularity>, Query,
         description = "1m | 15m | 1h | 4h | 1d | 1w | 1M (auto from timeframe if omitted)"),
        ("start" = Option<String>, Query, description = "ISO-8601 range start (overrides timeframe)"),
        ("end" = Option<String>, Query, description = "ISO-8601 range end"),
        ("base_currency" = Option<BaseCurrency>, Query, description = "USD (default) | XLM"),
    ),
    responses(
        (status = 200, description = "Candlestick series", body = OhlcvResponse),
        (status = 400, description = "Invalid parameter", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid `x-api-key`", body = ErrorEnvelope),
        (status = 403, description = "API key missing, invalid, or not authorized for this API"),
        (status = 404, description = "Unknown asset", body = ErrorEnvelope),
        (status = 429, description = "Per-key rate limit or monthly quota exceeded (API Gateway usage plan)"),
        (status = 500, description = "Query or upstream failure (`db_error`)", body = ErrorEnvelope),
        (status = 503, description = "The requested `base_currency` quote asset is not \
                                      tracked (`quote_unavailable`)", body = ErrorEnvelope),
    )
)]
pub async fn get_ohlcv(
    State(state): State<AppState>,
    ValidatedPath(raw): ValidatedPath<String>,
    ValidatedQuery(p): ValidatedQuery<OhlcvParams>,
) -> Response {
    let id = match AssetIdentifier::parse(&raw) {
        Ok(id) => id,
        Err(e) => return errors::bad_request(errors::INVALID_ID, e.to_string()),
    };
    let timeframe = p.timeframe.unwrap_or(Timeframe::H24);
    let base_currency = p.base_currency.unwrap_or(BaseCurrency::Usd);
    let granularity = p
        .granularity
        .unwrap_or_else(|| timeframe.default_granularity());
    // Validate ?start / ?end up front — otherwise a malformed value is bound into
    // ClickHouse `parseDateTimeBestEffort(?)`, which throws → a 500 for what is a
    // client input error (should be 400).
    if let Some(s) = p.start.as_deref()
        && !valid_iso8601(s)
    {
        return errors::bad_request(errors::INVALID_QUERY, "invalid start (expected ISO-8601)");
    }
    if let Some(e) = p.end.as_deref()
        && !valid_iso8601(e)
    {
        return errors::bad_request(errors::INVALID_QUERY, "invalid end (expected ISO-8601)");
    }

    // Resolve the base asset.
    let asset_id = match queries_ch::resolve_asset_id(state.ch(), &id).await {
        Ok(Some(a)) => a,
        Ok(None) => return errors::not_found("unknown asset"),
        Err(e) => return errors::db_error(&e, "asset lookup"),
    };

    // Resolve the quote leg from base_currency.
    let quote_ident = match base_currency {
        BaseCurrency::Usd => AssetIdentifier::Classic {
            code: "USDC".to_string(),
            issuer: prices_clickhouse::USDC_ISSUER.to_string(),
        },
        BaseCurrency::Xlm => AssetIdentifier::Native,
    };
    let quote_asset_id = match queries_ch::resolve_asset_id(state.ch(), &quote_ident).await {
        Ok(Some(q)) => q,
        // The quote leg (USDC for USD, native for XLM) must be tracked — its
        // absence is a server-side data gap, not "no candles". Surface it as a
        // 503 instead of masking it as an empty 200 (which looks like a healthy
        // asset with no history).
        Ok(None) => {
            tracing::error!(
                base_currency = base_currency.as_str(),
                "ohlcv quote asset not tracked"
            );
            return errors::service_unavailable(
                errors::QUOTE_UNAVAILABLE,
                "pricing in the requested base_currency is unavailable",
            );
        }
        Err(e) => return errors::db_error(&e, "quote lookup"),
    };

    let since_interval = if p.start.is_some() {
        None
    } else {
        timeframe.interval()
    };
    let args = OhlcvArgs {
        asset_id,
        quote_asset_id,
        granularity,
        start: p.start.clone(),
        end: p.end.clone(),
        since_interval,
        limit: OHLCV_MAX_POINTS,
    };
    let data = match queries_ch::ohlcv(state.ch(), args).await {
        Ok(d) => d,
        Err(e) => return errors::db_error(&e, "ohlcv lookup"),
    };

    // backfill_note: only for timeframe=all, with data, while SDEX still running.
    let note = if timeframe.is_all() && !data.is_empty() && sdex_backfill_running(state.ch()).await
    {
        let from = data
            .first()
            .map(|c| {
                c.timestamp
                    .split('T')
                    .next()
                    .unwrap_or(&c.timestamp)
                    .to_string()
            })
            .unwrap_or_default();
        Some(format!(
            "Historical data available from {from}. Backfill in progress — see GET /backfill/status."
        ))
    } else {
        None
    };

    ohlcv_response(&id, granularity, base_currency, note, data)
}

/// Build the OHLCV 200 response with a MEDIUM cache header.
fn ohlcv_response(
    id: &AssetIdentifier,
    granularity: Granularity,
    base_currency: BaseCurrency,
    backfill_note: Option<String>,
    data: Vec<crate::assets::dto::Candle>,
) -> Response {
    let mut resp = Json(OhlcvResponse {
        asset: id.to_canonical(),
        granularity: granularity.as_str().to_string(),
        base_currency: base_currency.as_str().to_string(),
        backfill_note,
        data,
    })
    .into_response();
    cache_control::attach(&mut resp, cache_control::MEDIUM);
    resp
}

/// True if the SDEX backfill stream is still running (best-effort; false on
/// error, so a status hiccup just omits the note).
async fn sdex_backfill_running(ch: &clickhouse::Client) -> bool {
    match crate::backfill::queries_ch::all_progress(ch).await {
        Ok(rows) => rows
            .iter()
            .any(|r| r.task_name == "sdex_archive" && r.status == "running"),
        Err(e) => {
            tracing::warn!(error = %e, "ohlcv backfill-status check failed; omitting note");
            false
        }
    }
}

/// Lightweight ISO-8601 / epoch validation for `?start` / `?end`. Accepts what
/// our clients send (and what ClickHouse `parseDateTimeBestEffort` consumes)
/// without pulling in a datetime crate: a bare epoch (all digits), or a
/// `YYYY-MM-DD` date optionally followed by `T`/space and an `HH:MM[:SS][.fff]`
/// time with an optional `Z` / `±HH:MM` offset. Anything else (e.g. `notadate`)
/// is rejected so the handler returns 400 instead of letting CH error → 500.
fn valid_iso8601(s: &str) -> bool {
    let s = s.trim();
    if s.is_empty() {
        return false;
    }
    // Bare unix epoch (seconds / millis).
    if s.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    let bytes = s.as_bytes();
    // Date prefix: exactly `YYYY-MM-DD`.
    if bytes.len() < 10 {
        return false;
    }
    let digit = |i: usize| bytes[i].is_ascii_digit();
    if !(digit(0)
        && digit(1)
        && digit(2)
        && digit(3)
        && bytes[4] == b'-'
        && digit(5)
        && digit(6)
        && bytes[7] == b'-'
        && digit(8)
        && digit(9))
    {
        return false;
    }
    let month = (bytes[5] - b'0') * 10 + (bytes[6] - b'0');
    let day = (bytes[8] - b'0') * 10 + (bytes[9] - b'0');
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return false;
    }
    if s.len() == 10 {
        return true;
    }
    // Optional time component: separator (`T` or space) then time-ish chars only.
    if bytes[10] != b'T' && bytes[10] != b' ' {
        return false;
    }
    let time = &s[11..];
    time.len() >= 5 // at least HH:MM
        && time
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b':' | b'.' | b'Z' | b'+' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::valid_iso8601;

    #[test]
    fn iso8601_accepts_common_forms() {
        for s in [
            "2026-06-15",
            "2026-06-15T11:30:00Z",
            "2026-06-15 11:30:00",
            "2026-06-15T11:30:00.123+02:00",
            "1718450000",
        ] {
            assert!(valid_iso8601(s), "{s} should be valid");
        }
    }

    #[test]
    fn iso8601_rejects_garbage() {
        for s in [
            "notadate",
            "",
            "2026/06/15",
            "2026-13-01",
            "2026-06-32",
            "06-15-2026",
            "T12:00:00",
        ] {
            assert!(!valid_iso8601(s), "{s} should be invalid");
        }
    }
}
