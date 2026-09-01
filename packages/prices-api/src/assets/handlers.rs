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

/// Query parameters for `GET /assets/{id}/price` (task 0118).
#[derive(Debug, Deserialize)]
pub struct PriceParams {
    pub min_volume_usd: Option<f64>,
}

/// Validate `?min_volume_usd=` (task 0118, coordinated with 0119's "every
/// invalid input is a 400 in the standard envelope"). serde already rejects
/// non-numeric text, but `f64::from_str` accepts `NaN`/`inf`, so finiteness is
/// checked here, along with the sign and a sanity ceiling. Returns the 400 to
/// send when the value is invalid, `None` when it is fine.
fn min_volume_error(v: Option<f64>) -> Option<Response> {
    match v {
        Some(x)
            if !(x.is_finite() && (0.0..=crate::assets::dto::MAX_MIN_VOLUME_USD).contains(&x)) =>
        {
            Some(errors::bad_request(
                errors::INVALID_QUERY,
                "min_volume_usd must be a finite number in 0..=1e15",
            ))
        }
        _ => None,
    }
}

/// `GET /assets/{asset_identifier}/price` — current price for one asset.
///
/// Parsing/validation runs before any DB call, so a malformed identifier 400s
/// without touching ClickHouse. `price_xlm`, `change_24h_pct` and `sources` are
/// materialized producer-side by `mv_current_prices` (task 0072) and pass
/// straight through, so this stays a point lookup.
///
/// `?min_volume_usd=` (task 0118) re-weights `vwap_24h` from the row's own
/// `sources` JSON in the handler — no extra ClickHouse round-trip, so the p95
/// SLO that motivated the producer-side design is untouched. An explicit value
/// always filters strictly at exactly that value: the MV's $100 default is
/// applied *conditionally*, so on an all-dust asset a below-$100 venue is
/// still in the JSON and a caller asking for 100 must not be handed it back.
/// Omit the param on the common path — it is part of the API Gateway cache
/// key (§6), and the cached entry is shared only when the param is absent.
#[utoipa::path(
    get,
    path = "/assets/{asset_identifier}/price",
    tag = "prices",
    params(
        ("asset_identifier" = String, Path,
         description = "native, CODE:ISSUER, or a C… contract address"),
        ("min_volume_usd" = Option<f64>, Query, minimum = 0,
         description = "Exclude sources whose trailing-24h USD volume is at or \
                        below this value from `vwap_24h` weighting and from \
                        `sources` (§5.5). An explicit value ALWAYS filters \
                        strictly at exactly that value and can empty \
                        `sources`, unlike the producer-side $100 system \
                        default, which is conditional (a below-threshold \
                        source is kept when no source on the asset clears the \
                        threshold). On an asset with a funded venue the \
                        producer has already applied the $100 cut, so a value \
                        at or below 100 returns the same body as omitting the \
                        param; on an all-dust asset it does not. Omit on the \
                        common path to share the response cache entry."),
    ),
    responses(
        (status = 200, description = "Current price", body = PriceResponse),
        (status = 400, description = "Invalid asset identifier or query parameter", body = ErrorEnvelope),
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
    ValidatedQuery(q): ValidatedQuery<PriceParams>,
) -> Response {
    let id = match AssetIdentifier::parse(&raw) {
        Ok(id) => id,
        Err(e) => return errors::bad_request(errors::INVALID_ID, e.to_string()),
    };
    if let Some(resp) = min_volume_error(q.min_volume_usd) {
        return resp;
    }

    match queries_ch::current_price(state.ch(), &id).await {
        Ok(Some(row)) => {
            let mut body = PriceResponse::from_row(id.to_canonical(), row);
            if let Some(t) = q.min_volume_usd {
                crate::assets::dto::apply_min_volume(&mut body.sources, &mut body.vwap_24h, t);
            }
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

/// Cap on the `?search` prefix — the shared "compared against `asset_code`"
/// rule (see its definition for why length-only).
const MAX_SEARCH_LEN: usize = cursor::MAX_STRING_PAYLOAD_LEN;

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
    pub min_volume_usd: Option<f64>,
}

/// `GET /assets` — paginated, sortable, filterable list of tracked assets.
#[utoipa::path(
    get,
    path = "/assets",
    tag = "assets",
    params(
        ("type" = Option<TypeFilter>, Query, description = "classic | soroban | all (default all)"),
        ("search" = Option<String>, Query,
         description = "asset code prefix match (max 64 bytes; an empty value \
                        is treated as absent)",
         min_length = 1, max_length = 64),
        ("sort" = Option<SortCol>, Query,
         description = "price | volume_24h | change_24h | code (default volume_24h)"),
        ("order" = Option<Order>, Query, description = "asc | desc (default desc)"),
        ("cursor" = Option<String>, Query, description = "opaque pagination cursor"),
        ("limit" = Option<u32>, Query, description = "1..=200 (default 50)",
         minimum = 1, maximum = 200),
        ("min_volume_usd" = Option<f64>, Query, minimum = 0,
         description = "Exclude sources whose trailing-24h USD volume is at or \
                        below this value from `vwap_24h` weighting and from \
                        `sources` (§5.5) — identical semantics to the \
                        parameter on `GET /assets/{asset_identifier}/price`: \
                        an explicit value ALWAYS filters strictly at exactly \
                        that value and can empty `sources`, while the \
                        producer-side $100 default is conditional. Does not \
                        affect `price_usd`, `volume_24h_usd`, or the sort."),
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
    // Empty search is treated as absent. Length-only cap, no charset: stored
    // codes include lossy-decoded on-chain bytes, so a charset rule would make
    // listed assets unsearchable (PR #217 review; same rule as the cursor).
    let search = p.search.filter(|s| !s.is_empty());
    if let Some(s) = &search
        && s.len() > MAX_SEARCH_LEN
    {
        return errors::bad_request(
            errors::INVALID_QUERY,
            format!("search must be at most {MAX_SEARCH_LEN} bytes"),
        );
    }
    if let Some(resp) = min_volume_error(p.min_volume_usd) {
        return resp;
    }
    let min_volume = p.min_volume_usd;

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
        .map(|r| {
            let mut sources = crate::assets::dto::parse_sources(&r.sources);
            let mut vwap_24h = r.vwap_24h;
            if let Some(t) = min_volume {
                // 0118 override — reweights vwap_24h/sources only; the sort
                // ran in ClickHouse on columns the threshold never touches.
                crate::assets::dto::apply_min_volume(&mut sources, &mut vwap_24h, t);
            }
            AssetListItem {
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
                vwap_24h,
                sources,
                updated_at: r.updated_at,
                method: r.method,
            }
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
/// `base_currency` **denominates** the series; it does not select a quote leg
/// (ADR 0011). `USD` expresses every candle in USD whatever it traded against,
/// so an asset with no USDC market still returns its history — `close` is exact
/// and O/H/L/`vwap` are derived by scaling, flagged per candle. `XLM` still
/// filters to the native quote and returns candles as stored.
///
/// Price fields are **absent, not dropped**, on a bucket with no USD value.
/// Candles merge across sources — and, in USD mode, across quote legs — per
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
         description = "1m | 15m | 1h | 4h | 1d | 1w | 1M; omitted = timeframe default, or \
                        the finest fitting 5000 points for explicit windows and `all`"),
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
        (status = 503, description = "A reference asset the denomination needs is not \
                                      tracked — canonical USDC for `USD`, native for \
                                      `XLM` (`quote_unavailable`)", body = ErrorEnvelope),
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

    // Window rule (task 0119): parse ?start/?end to epochs, then bound-check the
    // whole window BEFORE the DB is touched. Explicit rejection replaces the old
    // silent truncation at OHLCV_MAX_POINTS, which looked like missing data.
    // Granularity is resolved AFTER the window (see below): when omitted, it
    // derives from what the caller actually asked for.
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
    // Strictly greater: `start == end` is a legitimate one-bucket window, since
    // the SQL bounds are inclusive on both ends.
    if eff_start > eff_end {
        // A future-only `start` trips this with an `end` the client never sent —
        // name the actual problem instead (review finding, PR #217).
        let message = if end.is_none() && start.is_some_and(|s| s > now) {
            "start is in the future"
        } else {
            "start must be before end"
        };
        return errors::bad_request(errors::INVALID_QUERY, message);
    }
    let span = (eff_end - eff_start) as u64;
    // Granularity, when omitted, follows what the caller asked for: a plain
    // timeframe keeps its documented default; an explicit window (and
    // `timeframe=all`, whose span grows with time) gets the finest granularity
    // that fits the point cap — so `?start=2020-01-01` alone is answerable
    // instead of 400ing at a granularity the caller never chose, and bare
    // `timeframe=all` self-coarsens instead of hitting a cliff around 2029.
    let explicit_window = start.is_some() || end.is_some();
    let granularity = match p.granularity {
        Some(g) => g,
        None if !explicit_window && !timeframe.is_all() => timeframe.default_granularity(),
        None => Granularity::finest_for_span(span, OHLCV_MAX_POINTS),
    };
    // `+ 1`: the SQL bounds are inclusive on both ends, so an aligned window
    // spanning exactly N buckets contains N + 1 bucket-start timestamps —
    // counting span/granularity alone would let a 5001-bucket request through
    // to be silently truncated by the LIMIT.
    let points = span.div_ceil(granularity.seconds()) + 1;
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

    // Resolve the denomination (ADR 0011 §1). In USD mode `base_currency` no
    // longer selects a quote leg — the reference ids are needed only to classify
    // each row's provenance, not to filter it. In XLM mode it is still the
    // pre-ADR pair filter; see Denomination::QuoteLeg.
    //
    // The three references are resolved by natural identity, mirroring the
    // enrichment worker's own resolve_reference_ids so read and write cannot
    // disagree about which USDC is canonical. All three must be tracked: their
    // absence is a server-side data gap, not "no candles", so it stays a 503
    // rather than being masked as an empty 200 (which looks like a healthy asset
    // with no history).
    let denomination =
        match base_currency {
            BaseCurrency::Usd => {
                // Resolved once per AppState and shared thereafter — see
                // AppState::usd_refs. The three identities are constants and their
                // surrogate ids never move, so re-reading `assets FINAL` on every
                // request is pure waste on a p95-bounded path.
                let refs = state
                .usd_refs()
                .get_or_try_init(|| async {
                    // Concurrent: they do not depend on each other.
                    let (usdc_id, xlm_id, usdt_id) =
                        (usdc_identifier(), AssetIdentifier::Native, usdt_identifier());
                    let (usdc, xlm, usdt) = tokio::join!(
                        queries_ch::resolve_asset_id(state.ch(), &usdc_id),
                        queries_ch::resolve_asset_id(state.ch(), &xlm_id),
                        queries_ch::resolve_asset_id(state.ch(), &usdt_id),
                    );
                    // The pivots are best-effort: an untracked reference cannot
                    // be any candle's quote leg, so it only costs the `traded`
                    // label. A lookup ERROR still fails — that is the database
                    // misbehaving, not an absent asset.
                    let mut pivots = Vec::new();
                    for found in [xlm, usdt] {
                        match found {
                            Ok(Some(id)) => pivots.push(id),
                            Ok(None) => tracing::debug!(
                                "ohlcv pivot reference not tracked; rows on this leg go unlabelled"
                            ),
                            Err(e) => return Err(e),
                        }
                    }
                    // USDC must resolve: see UsdRefs::usdc.
                    match usdc {
                        Ok(Some(usdc)) => Ok(queries_ch::UsdRefs { usdc, pivots }),
                        Ok(None) => Err(clickhouse::error::Error::Custom(
                            "canonical USDC is not tracked".to_string(),
                        )),
                        Err(e) => Err(e),
                    }
                })
                .await;
                match refs {
                    Ok(r) => queries_ch::Denomination::Usd(r.clone()),
                    Err(e) => {
                        tracing::error!(error = %e, "ohlcv reference resolution failed");
                        return errors::service_unavailable(
                            errors::QUOTE_UNAVAILABLE,
                            "pricing in the requested base_currency is unavailable",
                        );
                    }
                }
            }
            BaseCurrency::Xlm => {
                match resolve_reference(state.ch(), AssetIdentifier::Native, base_currency).await {
                    Ok(id) => queries_ch::Denomination::QuoteLeg(id),
                    Err(resp) => return resp,
                }
            }
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
        denomination,
        granularity,
        start: Some(eff_start),
        end,
        limit: OHLCV_MAX_POINTS,
    };
    // ADR 0011 §6: canonical USDC is only ever a quote leg, so a normal query
    // for it matches zero rows and no amount of filter-dropping helps — its
    // series is synthesized instead. Keyed on the requested identity, not on a
    // resolved asset_id, because 0139 has ids serving more than one identity.
    // ADR 0011 §6: canonical USDC is only ever a quote leg, so a normal query
    // for it matches zero rows in EITHER denomination and no amount of
    // filter-dropping helps — its series is synthesized instead.
    //
    // Matched on the natural identity (case-insensitively: `AssetIdentifier`
    // preserves the code's case verbatim) OR on the resolved id. The id arm is
    // what catches USDC addressed by its SAC contract address, which is a
    // different identity for the same asset.
    // ⚠️ [[0139]] means an `asset_id` can serve more than one identity, so the
    // id arm could in principle route a colliding asset here. Accepted: the
    // alternative is that a legitimate USDC identity silently falls back to the
    // known-empty path, which is the defect this task exists to remove.
    let (peg_usdc, peg_xlm) = match &args.denomination {
        queries_ch::Denomination::Usd(refs) => (Some(refs.usdc), refs.pivots.first().copied()),
        queries_ch::Denomination::QuoteLeg(x) => {
            match queries_ch::resolve_asset_id(state.ch(), &usdc_identifier()).await {
                Ok(Some(u)) => (Some(u), Some(*x)),
                Ok(None) => (None, Some(*x)),
                Err(e) => return errors::db_error(&e, "quote lookup"),
            }
        }
    };
    let is_peg_asset = peg_usdc.is_some_and(|u| {
        u == asset_id
            || id
                .to_canonical()
                .eq_ignore_ascii_case(&usdc_identifier().to_canonical())
    });

    let data = if is_peg_asset {
        // Both references anchor the series: USDC is the rate's identity, XLM is
        // the market the buckets come from (and, in XLM mode, the denominator).
        let (Some(usdc), Some(xlm)) = (peg_usdc, peg_xlm) else {
            tracing::error!("ohlcv peg series needs both USDC and native XLM tracked");
            return errors::service_unavailable(
                errors::QUOTE_UNAVAILABLE,
                "pricing in the requested base_currency is unavailable",
            );
        };
        let in_xlm = matches!(base_currency, BaseCurrency::Xlm);
        match queries_ch::ohlcv_peg_series(
            state.ch(),
            &args,
            usdc,
            xlm,
            prices_clickhouse::USDC_ISSUER,
            in_xlm,
        )
        .await
        {
            Ok(d) => d,
            Err(e) => return errors::db_error(&e, "ohlcv peg series"),
        }
    } else {
        match queries_ch::ohlcv(state.ch(), args).await {
            Ok(d) => d,
            Err(e) => return errors::db_error(&e, "ohlcv lookup"),
        }
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

/// Canonical USDC's natural identity — the same `(code, issuer)` pair the
/// enrichment peg tier and `views.sql` key on, so the three cannot drift.
fn usdc_identifier() -> AssetIdentifier {
    AssetIdentifier::Classic {
        code: "USDC".to_string(),
        issuer: prices_clickhouse::USDC_ISSUER.to_string(),
    }
}

/// Canonical Stellar USDT's natural identity.
///
/// ⚠️ This is a *reference* leg, not a peg. It depegged in June 2022 and trades
/// at ~$0.13 (task 0172); candles quoted in it are priced by measurement through
/// the pivot, exactly like XLM. Naming it here is only how those rows get
/// classified `traded`.
fn usdt_identifier() -> AssetIdentifier {
    AssetIdentifier::Classic {
        code: "USDT".to_string(),
        issuer: prices_clickhouse::USDT_ISSUER.to_string(),
    }
}

/// Resolve one reference leg, mapping "not tracked" to a 503.
///
/// A missing reference is a server-side data gap. Returning an empty 200 would
/// render it as "this asset has no history", which is the exact confusion this
/// endpoint's whole fix is about.
async fn resolve_reference(
    ch: &clickhouse::Client,
    ident: AssetIdentifier,
    base_currency: BaseCurrency,
) -> Result<u32, Response> {
    match queries_ch::resolve_asset_id(ch, &ident).await {
        Ok(Some(id)) => Ok(id),
        Ok(None) => {
            tracing::error!(
                base_currency = base_currency.as_str(),
                asset = %ident.to_canonical(),
                "ohlcv reference asset not tracked"
            );
            Err(errors::service_unavailable(
                errors::QUOTE_UNAVAILABLE,
                "pricing in the requested base_currency is unavailable",
            ))
        }
        Err(e) => Err(errors::db_error(&e, "quote lookup")),
    }
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
        // Datetime forms. Two query-string realities to undo before parsing
        // (PR #217 review): the documented space separator sits exactly at
        // byte 10 (fix it positionally — replacing the FIRST space would
        // corrupt the next case), and a literal `+` percent-decodes to a
        // space, so a trailing " HH:MM" / " HHMM" after the time is a
        // `+HH:MM` offset that lost its sign in transit.
        let mut norm: Cow<str> = if s.len() > 10 && s.as_bytes()[10] == b' ' {
            Cow::Owned(format!("{}T{}", &s[..10], &s[11..]))
        } else {
            Cow::Borrowed(s)
        };
        if let Some(i) = norm.rfind(' ')
            && looks_like_utc_offset(&norm[i + 1..])
        {
            let mut owned = norm.into_owned();
            owned.replace_range(i..=i, "+");
            norm = Cow::Owned(owned);
        }
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

/// `HH:MM` or `HHMM` — the tail of a `+HH:MM` offset whose `+` was
/// percent-decoded to a space in the query string.
fn looks_like_utc_offset(t: &str) -> bool {
    match t.len() {
        4 => t.bytes().all(|b| b.is_ascii_digit()),
        5 => {
            t.as_bytes()[2] == b':'
                && t.bytes()
                    .enumerate()
                    .all(|(i, b)| i == 2 || b.is_ascii_digit())
        }
        _ => false,
    }
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
            // What the handler actually receives when a client sends `+HH:MM`
            // raw in a query string: `+` percent-decodes to a space
            // (PR #217 review).
            "2026-06-15T11:30:00 02:00",
            "2026-06-15T11:30 0200",
            "2026-06-15 11:30:00 02:00",
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
        // Offsets shift correctly — including with the `+` lost in transit.
        assert_eq!(parse_time("2026-06-15T13:13:20+02:00"), Some(epoch));
        assert_eq!(parse_time("2026-06-15T13:13:20 02:00"), Some(epoch));
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
