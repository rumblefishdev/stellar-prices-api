//! Response DTOs for the `/v1/assets` resource.

use serde::Serialize;
use utoipa::ToSchema;

/// `GET /assets/{id}/price` response (overview §4.2). All numeric fields are
/// decimal strings to preserve precision.
#[derive(Debug, Serialize, ToSchema)]
pub struct PriceResponse {
    /// Echoed natural identity (`native`, `CODE:ISSUER`, or a C… contract).
    pub asset: String,
    /// Latest **priced** USD close (task 0135): a candle whose USD value is
    /// not yet computed is skipped rather than reported as `"0"`.
    ///
    /// **This value can be older than it looks, and is not age-bounded.** For
    /// an asset that has stopped trading it is simply the last priced close,
    /// up to the 24 h aggregation window old. `updated_at` is the snapshot
    /// time, **not** the price's age, and no field currently carries that
    /// age. `"0"` means no priced close exists in the window at all.
    ///
    /// Note the deliberate asymmetry with `sources` / `vwap_24h`: those drop a
    /// venue whose last quote is stale, because a per-venue price asserts
    /// "quoting now". This field makes no such claim.
    pub price_usd: String,
    /// XLM-quoted price (task 0072): `price_usd / XLM-USD close`. Shares
    /// `price_usd`'s `"0"` sentinel — and note it is a quotient of **two
    /// independently dated** closes, the asset's and XLM's, so it is not a
    /// price "as of" any single instant and never was.
    pub price_xlm: String,
    /// 24h USD volume-weighted average price. Weighted across sources with the
    /// general-overview §5.5 `min_volume_usd` threshold (task 0118; the $100
    /// system default applies *conditionally* — a below-threshold source is
    /// kept when no source on the asset clears the threshold — and is
    /// overridable per-request via `?min_volume_usd=`, which always filters
    /// strictly) and the inter-source median-outlier filter applied.
    pub vwap_24h: String,
    /// Trailing-24h USD volume across **all** sources — a traded total, never
    /// outlier- or threshold-filtered.
    pub volume_24h_usd: String,
    /// 24h percentage change (task 0072).
    pub change_24h_pct: String,
    /// Per-source (DEX) price/volume breakdown. A source is **absent** when
    /// the §5.5 `min_volume_usd` threshold or outlier filter excluded it OR it
    /// has no USD-priced close within the 0135 carry bound (general-overview
    /// §3.3 / §4.2). `{}` means no source qualified — the exotic-quote and
    /// all-below-threshold cases, not an error.
    #[schema(value_type = Object)]
    pub sources: serde_json::Value,
    /// Timestamp of the snapshot (ISO-8601 UTC).
    pub updated_at: String,
    /// How `price_usd` was arrived at (task 0178) — the same vocabulary
    /// `price_usd_series.method` uses, so the tip and the series read alike:
    ///
    /// * `"traded"` — a real aggregate of candles some pricing tier priced.
    /// * `"oracle"` — a measured depeg-aware rate from `prices.usd_rate`,
    ///   used for assets that never trade as a base leg and so have no candle
    ///   of their own. Canonical USDC is currently the only one.
    /// * `""` — the "unavailable" sentinel, shared with `price_usd`'s `"0"`:
    ///   no priced candle in the window, so no method applies.
    ///
    /// ⚠️ Never read `"oracle"` as "more accurate than traded" — it means the
    /// price came from a rate rather than from this asset's own trades.
    pub method: String,
}

/// Parse the MV's `sources` JSON string into a value for the response.
///
/// Degrades to `{}` rather than failing the request. The column is `String` with
/// a table DEFAULT of `''`, so an empty value is the normal pre-0072 / no-refresh
/// state, not corruption — and a read endpoint should not 500 because a producer
/// wrote something unexpected. Anything that is not a JSON *object* is also
/// rejected, so the response shape stays `{ … }` for every asset.
pub(crate) fn parse_sources(raw: &str) -> serde_json::Value {
    if raw.is_empty() {
        return serde_json::json!({});
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(v) if v.is_object() => v,
        _ => serde_json::json!({}),
    }
}

/// Upper bound on `?min_volume_usd=` — far above any real 24h venue volume,
/// present so a nonsense magnitude is a 400 instead of a silent all-excluded
/// response.
pub(crate) const MAX_MIN_VOLUME_USD: f64 = 1e15;

/// Re-apply the §5.5 weighting rule at a caller-supplied threshold (task 0118,
/// request-level override — option (a): recompute from the `sources` JSON the
/// row already carries, no second ClickHouse round-trip).
///
/// **An explicit threshold always filters strictly**, at whatever value the
/// caller sent, and it can empty `sources`. There is deliberately no
/// pass-through band around the producer's $100 default: that default is
/// applied *conditionally* by the MV (an all-dust asset keeps its sources —
/// measured on prod, the unconditional form would have blanked 96.5% of priced
/// assets), so on such an asset a below-$100 venue IS present in the JSON and
/// a caller asking for `min_volume_usd=100` must not be handed it back.
///
/// Behaviour:
/// - drop every source whose `volume_24h` is not strictly above the threshold
///   (the MV's own comparison is strict too);
/// - if nothing was dropped, the row passes through **untouched** — the MV's
///   Decimal strings are never reformatted. On an asset with a funded venue
///   the producer has already applied the $100 cut, so `min_volume_usd=100`
///   is byte-identical to omitting the param;
/// - otherwise recompute `vwap_24h = Σ(p×v)/Σ(v)` over the survivors **in
///   f64** — deliberately the MV's own numeric strategy (it computes the vwap
///   over Float64 arrays before the Decimal cast, see current.sql's
///   numeric-strategy note), so the override can never claim more precision
///   than the value it overrides. Nothing left → `"0"` + `{}`, the MV's own
///   sentinels for that shape.
///
/// The outlier mask is NOT re-run here: it already excluded its venues
/// producer-side (they are absent from the JSON), and §5.5 orders the volume
/// threshold *before* the median — raising it can only shrink the mask's
/// input population, never re-admit an outlier.
pub(crate) fn apply_min_volume(
    sources: &mut serde_json::Value,
    vwap_24h: &mut String,
    threshold: f64,
) {
    let Some(obj) = sources.as_object_mut() else {
        return;
    };
    let vol = |v: &serde_json::Value| -> f64 {
        v.get("volume_24h")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0)
    };
    let before = obj.len();
    obj.retain(|_, v| vol(v) > threshold);
    if obj.len() == before {
        // Nothing dropped: leave the producer's Decimal strings exactly as
        // they came out of ClickHouse rather than round-tripping them.
        return;
    }

    let (mut pv, mut vsum) = (0.0_f64, 0.0_f64);
    for v in obj.values() {
        let p = v
            .get("price")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.0);
        let w = vol(v);
        pv += p * w;
        vsum += w;
    }
    *vwap_24h = if vsum > 0.0 {
        format_decimal(pv / vsum)
    } else {
        "0".to_string()
    };
}

/// Format an f64 the way ClickHouse serialises `Decimal(38,14)` into the
/// `sources` JSON: up to 14 fractional digits, trailing zeros trimmed.
fn format_decimal(x: f64) -> String {
    let s = format!("{x:.14}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

impl PriceResponse {
    /// Build from a current-price row. `/price` and `/prices/batch` both go
    /// through here, so the two endpoints cannot drift.
    ///
    /// Task 0072 removed the `price_xlm` / `change_24h_pct` / `sources` stubs:
    /// all three are now materialized producer-side by `mv_current_prices`
    /// (`schema/current.sql`) and pass straight through, keeping this endpoint a
    /// cheap point lookup — the reason 0040 stubbed them rather than deriving
    /// them per request (it would have added a 24h scan + GROUP BY to the
    /// hottest route and undermined the p95 SLO).
    pub fn from_row(asset: String, row: crate::assets::queries_ch::CurrentPriceRow) -> Self {
        PriceResponse {
            asset,
            price_usd: row.price_usd,
            price_xlm: row.price_xlm,
            vwap_24h: row.vwap_24h,
            volume_24h_usd: row.volume_24h_usd,
            change_24h_pct: row.change_24h_pct,
            sources: parse_sources(&row.sources),
            updated_at: row.updated_at,
            method: row.method,
        }
    }
}

/// `GET /assets/{id}` response (overview §4.1). The doc fixes only the request
/// forms; this is the chosen detail shape, resolved from `prices.assets`.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetDetail {
    /// Echoed natural identity.
    pub asset: String,
    /// Normalized kind: `native`, `credit`, or `contract`.
    pub asset_kind: String,
    /// Classic asset code (`""` for native/contract).
    pub code: String,
    /// Classic issuer G-strkey (`""` otherwise).
    pub issuer: String,
    /// Soroban contract C-strkey (`""` otherwise).
    pub contract: String,
    /// SEP-1 home domain, if known.
    pub home_domain: String,
    /// Whether the asset is currently tracked as active.
    pub is_active: bool,
}

/// One item in the `GET /assets` listing (overview §4.1).
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetListItem {
    pub asset_code: String,
    /// `classic` or `soroban` (matches the `?type` filter vocabulary).
    pub asset_type: String,
    pub issuer_address: String,
    pub contract_address: String,
    pub home_domain: String,
    /// Latest priced USD close — same 0135 semantics and `"0"` sentinel as
    /// `PriceResponse.price_usd`.
    pub price_usd: String,
    pub change_24h_pct: String,
    pub change_7d_pct: String,
    pub volume_24h_usd: String,
    pub vwap_24h: String,
    /// Per-source breakdown; sources excluded by the §5.5 `min_volume_usd`
    /// threshold or by outlier detection are absent (§3.3). Same semantics as
    /// `PriceResponse::sources`, including the `?min_volume_usd=` override.
    #[schema(value_type = Object)]
    pub sources: serde_json::Value,
    pub updated_at: String,
    /// Price provenance; same vocabulary and caveats as
    /// [`PriceResponse::method`].
    pub method: String,
}

/// `GET /assets` paginated response.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetListResponse {
    pub data: Vec<AssetListItem>,
    /// Opaque cursor for the next page (`null` on the last page).
    pub cursor: Option<String>,
    pub has_more: bool,
}

/// One OHLCV candle (overview §4.2), denominated in `base_currency` per
/// [ADR 0011]. Doubles as the CH row.
///
/// ## Why the price fields are nullable (ADR 0011 §5)
///
/// `close_usd` is a cached product, not a stored fact, and it is absent for two
/// distinct populations: a bucket enrichment has not reached yet (~1.8% at the
/// right-hand edge), and a candle quoted in a leg we hold no reference for. Both
/// are returned **with the price fields absent, never dropped** — omitting the
/// bucket would put a hole at the end of every chart and make "not yet priced"
/// indistinguishable from "did not trade".
///
/// `volume_base`, `volume_quote_usd` and `trade_count` are always present: they
/// do not depend on the USD rate (`volume_quote_usd` is already USD whatever the
/// quote leg), so a price-less bucket still carries real activity.
#[derive(Debug, Serialize, serde::Deserialize, clickhouse::Row, ToSchema)]
pub struct Candle {
    /// Bucket start (ISO-8601 UTC).
    pub timestamp: String,
    pub open: Option<String>,
    pub high: Option<String>,
    pub low: Option<String>,
    pub close: Option<String>,
    /// Base-asset volume.
    pub volume_base: String,
    /// USD-denominated quote volume, summed over **every** row in the bucket.
    ///
    /// ⚠️ **Not the same population as `volume_base`.** The column is `0` on a
    /// row enrichment has not priced, so an unpriced leg contributes its base
    /// volume and trade count but nothing here. Since [ADR 0011] dropped the
    /// quote filter a bucket can hold several legs, so the two can now disagree
    /// where before the fix both came from the same USDC-legged rows.
    ///
    /// Read it as "the USD volume we can account for", not as the bucket's total
    /// restated in USD. It is strictly more complete than before the fix — more
    /// legs are counted, not fewer — but it is a subtotal.
    pub volume_quote_usd: String,
    pub vwap: Option<String>,
    /// Trades in the bucket. The ceiling is `2^53 - 1`, the largest integer a
    /// JSON number carries without loss — not a domain limit (no protocol bound
    /// exists on a trade count) but a transport one, so a client knows the value
    /// always survives parsing. Stellar's actual volumes are ~10 orders of
    /// magnitude below it, so it never binds in practice.
    #[schema(maximum = 9_007_199_254_740_991u64)]
    pub trade_count: u64,
    /// Where the USD rate behind this bucket came from — [`0165`]'s existing
    /// vocabulary, reused rather than re-coined (ADR 0011 §4):
    ///
    /// - `peg` — no measured rate was available; the $1 USDC assumption applied.
    /// - `oracle` — a measured Reflector reading.
    /// - `traded` — priced through a reference asset's own traded candles.
    ///
    /// Derived from the candle's quote leg and rate signature, not stored: the
    /// candle tables carry `close_usd` with no companion provenance column. See
    /// `queries_ch::ohlcv` for the classification and the prod measurement
    /// behind it.
    ///
    /// `None` when the price fields are absent, **and also for every
    /// `base_currency=XLM` response** — that mode returns candles as stored, so
    /// there is no USD rate to attribute. A null `method` therefore means "no
    /// USD provenance to report", not "this bucket has no price".
    pub method: Option<String>,
    /// Whether `open`/`high`/`low`/`vwap` were **derived** rather than measured
    /// (ADR 0011 §3).
    ///
    /// On the normal path `close` is exact — it is `close_usd` as stored — while
    /// the extremes are reconstructed with one rate per bucket, so the true USD
    /// high may have fallen at a different instant than the quote-denominated
    /// high.
    ///
    /// ⚠️ **On the synthesized peg-asset path (§6) nothing is measured, `close`
    /// included.** Canonical USDC has no candles of its own, so every field is
    /// the `usd_rate` observation for the bucket — or the $1 fallback when none
    /// precedes it, which [`Candle::method`] reports as `peg`. Do not read
    /// `derived: true` as "only the extremes are reconstructed"; read it as "not
    /// measured on this market".
    ///
    /// A separate axis from [`Candle::method`], deliberately: a bucket can be
    /// `traded` *and* derived, so one field cannot carry both (ADR 0011 §4,
    /// settled 2026-08-26).
    ///
    /// `None` when the price fields are absent, and for `base_currency=XLM`,
    /// where nothing is converted and so nothing is derived.
    pub derived: Option<bool>,
}

/// `GET /assets/{id}/ohlcv` response.
#[derive(Debug, Serialize, ToSchema)]
pub struct OhlcvResponse {
    /// Echoed natural identity.
    pub asset: String,
    /// Effective granularity (auto-selected from `timeframe` unless overridden).
    pub granularity: String,
    /// `USD` or `XLM` — the quote the candles are denominated in.
    pub base_currency: String,
    /// Present only when `timeframe=all` and the backfill is still running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backfill_note: Option<String>,
    pub data: Vec<Candle>,
}

#[cfg(test)]
mod tests {
    use super::{apply_min_volume, parse_sources};

    fn three_sources() -> serde_json::Value {
        serde_json::json!({
            "sdex":     {"price": "1",    "volume_24h": "100000"},
            "aquarius": {"price": "1.02", "volume_24h": "20000"},
            "soroswap": {"price": "5",    "volume_24h": "150"},
        })
    }

    #[test]
    fn on_a_funded_asset_the_system_default_is_byte_identical() {
        // AC: a funded asset's response must not change whether the param is
        // omitted, equal to the $100 default, or below it — not by a
        // pass-through band, but because the MV already applied that cut, so
        // the strict filter finds nothing left to drop.
        for t in [0.0, 50.0, 100.0] {
            let mut s = three_sources();
            let mut vwap = "1.00505050505051".to_string();
            apply_min_volume(&mut s, &mut vwap, t);
            assert_eq!(s, three_sources(), "threshold {t}");
            assert_eq!(vwap, "1.00505050505051", "threshold {t}");
        }
    }

    #[test]
    fn an_explicit_default_still_cuts_an_all_dust_asset() {
        // The MV's system default is CONDITIONAL, so an asset with no funded
        // venue keeps its dust in `sources`. An explicit min_volume_usd=100
        // must still apply the cut the caller asked for — there is no
        // pass-through band that would hand a $50 venue back.
        let mut s = serde_json::json!({
            "sdex":     {"price": "3", "volume_24h": "50"},
            "soroswap": {"price": "4", "volume_24h": "30"},
        });
        let mut vwap = "3.375".to_string();
        apply_min_volume(&mut s, &mut vwap, 100.0);
        assert_eq!(s, serde_json::json!({}), "got {s}");
        assert_eq!(vwap, "0");
    }

    #[test]
    fn a_higher_threshold_narrows_the_set_and_reweights() {
        let mut s = three_sources();
        let mut vwap = "1.00505050505051".to_string();
        apply_min_volume(&mut s, &mut vwap, 1000.0);
        assert!(s.get("soroswap").is_none(), "got {s}");
        assert!(s.get("sdex").is_some() && s.get("aquarius").is_some());
        // (1*100000 + 1.02*20000) / 120000
        assert_eq!(vwap, "1.00333333333333");
    }

    #[test]
    fn a_threshold_that_bites_nothing_leaves_the_row_untouched() {
        // Above the default but below every venue: the MV's Decimal string
        // must survive verbatim, not be reformatted through f64.
        let mut s = three_sources();
        let mut vwap = "1.00505050505051".to_string();
        apply_min_volume(&mut s, &mut vwap, 120.0);
        assert_eq!(s, three_sources());
        assert_eq!(vwap, "1.00505050505051");
    }

    #[test]
    fn excluding_everything_lands_on_the_mv_sentinels() {
        let mut s = three_sources();
        let mut vwap = "1.00505050505051".to_string();
        apply_min_volume(&mut s, &mut vwap, 1e9);
        assert_eq!(s, serde_json::json!({}));
        assert_eq!(vwap, "0");
    }

    #[test]
    fn the_comparison_is_strict_like_the_mv() {
        // volume == threshold must be excluded: current.sql uses `>` per
        // §5.5's "volume_24h > threshold".
        let mut s = serde_json::json!({
            "sdex":     {"price": "2", "volume_24h": "500"},
            "soroswap": {"price": "9", "volume_24h": "150"},
        });
        let mut vwap = "x".to_string();
        apply_min_volume(&mut s, &mut vwap, 150.0);
        assert!(s.get("soroswap").is_none());
        assert_eq!(vwap, "2");
    }

    #[test]
    fn an_unparseable_volume_counts_as_zero_and_is_excluded() {
        let mut s = serde_json::json!({
            "sdex": {"price": "2", "volume_24h": "500"},
            "odd":  {"price": "3", "volume_24h": "not a number"},
        });
        let mut vwap = "x".to_string();
        apply_min_volume(&mut s, &mut vwap, 200.0);
        assert!(s.get("odd").is_none(), "got {s}");
        assert_eq!(vwap, "2");
    }

    #[test]
    fn parses_a_well_formed_sources_object() {
        let v = parse_sources(r#"{"sdex":{"price":"1.02","volume_24h":"50000"}}"#);
        assert_eq!(v["sdex"]["price"], "1.02");
        // Precision is carried as a STRING end-to-end (general-overview §3.3):
        // Decimal(38,14) values must never round-trip through a float.
        assert!(v["sdex"]["price"].is_string());
    }

    #[test]
    fn empty_column_becomes_an_empty_object() {
        // The table DEFAULT is '' — the normal state before the MV first
        // refreshes, and for an exotic-quote asset with no USD-priceable source.
        assert_eq!(parse_sources(""), serde_json::json!({}));
    }

    #[test]
    fn malformed_or_non_object_json_degrades_instead_of_failing() {
        // A read endpoint must not 500 because a producer wrote something odd.
        for raw in ["not json", "[1,2,3]", "\"a string\"", "42", "null", "{"] {
            let v = parse_sources(raw);
            assert_eq!(v, serde_json::json!({}), "input {raw:?} should degrade");
            assert!(v.is_object(), "input {raw:?} must stay an object");
        }
    }
}
