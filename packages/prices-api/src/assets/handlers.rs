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

/// Cap on the `?search` prefix: an asset-code prefix longer than a code can
/// never match anything.
const MAX_SEARCH_LEN: usize = crate::identity::MAX_CODE_LEN;

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
         description = "asset code prefix match (1-12 ASCII alphanumeric; \
                        an empty value is treated as absent)",
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
    // `valid_for` type-checks the payload against the active sort — a numeric
    // sort binds `v` into `toFloat64(?)`, where a corrupt value would make
    // ClickHouse throw (500) instead of this 400.
    let cursor = match p.cursor.as_deref() {
        Some(tok) => match cursor::decode(tok) {
            Some(c) if c.valid_for(sort.is_numeric()) => Some(c),
            _ => return errors::bad_request(errors::INVALID_QUERY, "invalid cursor"),
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
        ("start" = Option<String>, Query,
         description = "range start: ISO-8601 or unix epoch (seconds; ms if 13+ digits); \
                        overrides timeframe"),
        ("end" = Option<String>, Query,
         description = "range end, same forms; with only `end`, the timeframe window \
                        ends there"),
        ("base_currency" = Option<BaseCurrency>, Query,
         description = "USD (default) | XLM; all-lowercase usd/xlm accepted as \
                        legacy aliases"),
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

    // Window rule (task 0119): parse ?start/?end to epochs, then bound-check the
    // whole window BEFORE the DB is touched. Explicit rejection replaces the old
    // silent truncation at OHLCV_MAX_POINTS, which looked like missing data.
    let start = match p.start.as_deref() {
        Some(s) => match parse_time(s) {
            Some(t) => Some(t),
            None => {
                return errors::bad_request(
                    errors::INVALID_QUERY,
                    "invalid start (expected ISO-8601 or epoch)",
                );
            }
        },
        None => None,
    };
    let end = match p.end.as_deref() {
        Some(e) => match parse_time(e) {
            Some(t) => Some(t),
            None => {
                return errors::bad_request(
                    errors::INVALID_QUERY,
                    "invalid end (expected ISO-8601 or epoch)",
                );
            }
        },
        None => None,
    };
    let now = chrono::Utc::now().timestamp();
    let eff_end = end.unwrap_or(now);
    // The timeframe window anchors to eff_end (not now), so `?end=…&timeframe=7d`
    // means "the 7d window ending there". `all` starts at Stellar genesis. The
    // derived branch is clamped to epoch 0: it is the one bound `parse_time`'s
    // range check never saw, and a negative value would reach `toDateTime(?)`
    // (ClickHouse DateTime is unsigned — throw or wraparound, both wrong).
    let eff_start = start.unwrap_or_else(|| {
        match timeframe.seconds() {
            Some(tf) => eff_end - tf as i64,
            None => queries_ch::STELLAR_GENESIS_EPOCH,
        }
        .max(0)
    });
    if eff_start >= eff_end {
        return errors::bad_request(errors::INVALID_QUERY, "start must be before end");
    }
    // `+ 1`: the SQL bounds are inclusive on both ends, so an aligned window
    // spanning exactly N buckets contains N + 1 bucket-start timestamps —
    // counting span/granularity alone would let a 5001-bucket request through
    // to be silently truncated by the LIMIT.
    let points = ((eff_end - eff_start) as u64).div_ceil(granularity.seconds()) + 1;
    if points > OHLCV_MAX_POINTS {
        return errors::bad_request(
            errors::INVALID_QUERY,
            format!(
                "window yields ~{points} candles at granularity {g} (max {OHLCV_MAX_POINTS}); \
                 use a coarser granularity or a narrower start/end",
                g = granularity.as_str()
            ),
        );
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

    // The validated window is exactly what binds into SQL — the same eff_start
    // the point-count check measured, so there is one interpretation of the
    // window, not a parallel now() - INTERVAL path that could drift from it
    // (INTERVAL 1 YEAR is calendar-aware; seconds() isn't). The upper bound
    // binds only when the client supplied one: a derived `end = now` from this
    // process's clock could, under skew, cut a bucket ClickHouse already has —
    // an open top costs nothing (future buckets don't exist).
    let args = OhlcvArgs {
        asset_id,
        quote_asset_id,
        granularity,
        start: Some(eff_start),
        end,
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

/// Latest epoch we accept for `?start` / `?end`: 2100-01-01. Bounds what gets
/// bound into ClickHouse `toDateTime` (DateTime tops out in 2106) and rejects
/// nonsense like a 12-digit "epoch" that is really a typo.
const MAX_EPOCH: i64 = 4_102_444_800;

/// Parse a `?start` / `?end` value to epoch seconds (UTC). Accepts a bare
/// epoch (seconds; milliseconds when 13+ digits), `YYYY-MM-DD` (midnight
/// UTC), and `YYYY-MM-DD[T ]HH:MM[:SS[.fff]]` with an optional `Z` / `±HH:MM`
/// / `±HHMM` offset (naive times are UTC). Unlike its shape-only predecessor,
/// this rejects semantically impossible dates (`2026-02-30`, `T99:99:99`) —
/// and the parsed epoch (not the raw string) is what reaches SQL, so there is
/// exactly one interpretation of the window.
fn parse_time(s: &str) -> Option<i64> {
    use std::borrow::Cow;

    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let epoch = if s.bytes().all(|b| b.is_ascii_digit()) {
        // Digit count, not magnitude, decides seconds vs milliseconds — the
        // documented rule. (10-digit millis, i.e. instants in 1970, are
        // indistinguishable from valid seconds and read as seconds.)
        let millis = s.len() >= 13;
        let n: i64 = s.parse().ok()?;
        if millis { n / 1000 } else { n }
    } else if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        d.and_hms_opt(0, 0, 0)?.and_utc().timestamp()
    } else {
        // Datetime forms: normalize the documented space separator to `T`
        // (a space anywhere else fails every parser below anyway).
        let norm: Cow<str> = if s.contains(' ') {
            Cow::Owned(s.replacen(' ', "T", 1))
        } else {
            Cow::Borrowed(s)
        };
        // Offset-carrying forms: RFC 3339 (`T`, seconds, `Z`/`±HH:MM`), then
        // the shapes it rejects — minute precision with an offset, and `±HHMM`
        // without the colon (`%#z` takes both colon styles).
        let with_offset = chrono::DateTime::parse_from_rfc3339(&norm)
            .ok()
            .or_else(|| {
                ["%Y-%m-%dT%H:%M:%S%.f%#z", "%Y-%m-%dT%H:%M%#z"]
                    .iter()
                    .find_map(|fmt| chrono::DateTime::parse_from_str(&norm, fmt).ok())
            });
        match with_offset {
            Some(dt) => dt.timestamp(),
            None => {
                // Naive forms (UTC), seconds optional; a trailing `Z` marks
                // the same UTC instant, so accept it on minute precision too.
                let naive = norm.strip_suffix('Z').unwrap_or(&norm);
                ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M"]
                    .iter()
                    .find_map(|fmt| chrono::NaiveDateTime::parse_from_str(naive, fmt).ok())?
                    .and_utc()
                    .timestamp()
            }
        }
    };
    (0..=MAX_EPOCH).contains(&epoch).then_some(epoch)
}

#[cfg(test)]
mod tests {
    use super::parse_time;

    #[test]
    fn parse_time_accepts_common_forms() {
        for s in [
            "2026-06-15",
            "2026-06-15T11:30:00Z",
            "2026-06-15 11:30:00",
            "2026-06-15T11:30",
            "2026-06-15T11:30:00.123+02:00",
            "1718450000",
            // Offset shapes RFC 3339 alone rejects — accepted by the old
            // parseDateTimeBestEffort pipeline, so still part of the contract.
            "2026-06-15T11:30Z",
            "2026-06-15T11:30+02:00",
            "2026-06-15T11:30:00+0200",
        ] {
            assert!(parse_time(s).is_some(), "{s} should be valid");
        }
    }

    #[test]
    fn parse_time_agrees_on_equivalent_forms() {
        // The same instant in four spellings — all must produce one epoch,
        // since the parsed value (not the raw string) is what reaches SQL.
        let epoch = parse_time("2026-06-15T11:13:20Z").unwrap();
        assert_eq!(parse_time("2026-06-15 11:13:20"), Some(epoch));
        assert_eq!(parse_time(&epoch.to_string()), Some(epoch));
        assert_eq!(parse_time(&format!("{}000", epoch)), Some(epoch)); // millis
        // Offsets shift correctly.
        assert_eq!(parse_time("2026-06-15T13:13:20+02:00"), Some(epoch));
        // Date-only is midnight UTC.
        assert_eq!(parse_time("1970-01-02"), Some(86_400));
    }

    #[test]
    fn parse_time_rejects_garbage_and_impossible_dates() {
        for s in [
            "notadate",
            "",
            "2026/06/15",
            "2026-13-01",
            "2026-06-32",
            "06-15-2026",
            "T12:00:00",
            "2026-02-30",          // impossible calendar date
            "2026-06-15T99:99:99", // impossible time
            "99999999999999999",   // over MAX_EPOCH even as millis
            "999999999999",        // 12 digits = seconds by the rule → > 2100
        ] {
            assert!(parse_time(s).is_none(), "{s} should be invalid");
        }
    }
}
