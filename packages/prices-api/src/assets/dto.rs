//! Response DTOs for the `/v1/assets` resource.

use serde::Serialize;
use utoipa::ToSchema;

/// `GET /assets/{id}/price` response (overview §4.2). All numeric fields are
/// decimal strings to preserve precision.
#[derive(Debug, Serialize, ToSchema)]
pub struct PriceResponse {
    /// Echoed natural identity (`native`, `CODE:ISSUER`, or a C… contract).
    pub asset: String,
    /// Current USD price.
    pub price_usd: String,
    /// XLM-quoted price (task 0072).
    pub price_xlm: String,
    /// 24h USD volume-weighted average price. Weighted across sources with the
    /// general-overview §5.5 inter-source median-outlier filter applied.
    pub vwap_24h: String,
    /// Trailing-24h USD volume across **all** sources — a traded total, never
    /// outlier-filtered.
    pub volume_24h_usd: String,
    /// 24h percentage change (task 0072).
    pub change_24h_pct: String,
    /// Per-source (DEX) price/volume breakdown. Sources excluded by the §5.5
    /// outlier filter are **absent** from the object (general-overview §3.3).
    /// `{}` means no source carried a USD-priceable close in the window — the
    /// exotic-quote case, not an error.
    #[schema(value_type = Object)]
    pub sources: serde_json::Value,
    /// Timestamp of the snapshot (ISO-8601 UTC).
    pub updated_at: String,
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
    pub price_usd: String,
    pub change_24h_pct: String,
    pub change_7d_pct: String,
    pub volume_24h_usd: String,
    pub vwap_24h: String,
    /// Per-source breakdown; outlier-excluded sources are absent (§3.3).
    #[schema(value_type = Object)]
    pub sources: serde_json::Value,
    pub updated_at: String,
}

/// `GET /assets` paginated response.
#[derive(Debug, Serialize, ToSchema)]
pub struct AssetListResponse {
    pub data: Vec<AssetListItem>,
    /// Opaque cursor for the next page (`null` on the last page).
    pub cursor: Option<String>,
    pub has_more: bool,
}

/// One OHLCV candle (overview §4.2). O/H/L/C are in the `base_currency` quote
/// asset, **as stored** (no conversion). Doubles as the CH row.
#[derive(Debug, Serialize, serde::Deserialize, clickhouse::Row, ToSchema)]
pub struct Candle {
    /// Bucket start (ISO-8601 UTC).
    pub timestamp: String,
    pub open: String,
    pub high: String,
    pub low: String,
    pub close: String,
    /// Base-asset volume.
    pub volume_base: String,
    /// USD-denominated quote volume (the one USD figure on the candle).
    pub volume_quote_usd: String,
    pub vwap: String,
    /// Trades in the bucket. The ceiling is `2^53 - 1`, the largest integer a
    /// JSON number carries without loss — not a domain limit (no protocol bound
    /// exists on a trade count) but a transport one, so a client knows the value
    /// always survives parsing. Stellar's actual volumes are ~10 orders of
    /// magnitude below it, so it never binds in practice.
    #[schema(maximum = 9_007_199_254_740_991u64)]
    pub trade_count: u64,
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
    use super::parse_sources;

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
