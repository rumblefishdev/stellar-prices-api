//! ClickHouse query layer for the `/v1/assets` resource.
//!
//! Decimal columns are returned as **strings** (full precision preserved via
//! `toString`), matching the §4.2 string-typed JSON contract and sidestepping
//! Decimal↔Rust mapping. Rows deserialize positionally (RowBinary), so struct
//! field order MUST match the `SELECT` column order.

use clickhouse::Client;

use crate::assets::dto::Candle;
use crate::common::cursor::Cursor;
use crate::identity::AssetIdentifier;

/// One current-price row, all numeric fields as decimal strings.
#[derive(Debug, clickhouse::Row, serde::Deserialize)]
pub struct CurrentPriceRow {
    pub price_usd: String,
    pub price_xlm: String,
    pub vwap_24h: String,
    pub volume_24h_usd: String,
    pub change_24h_pct: String,
    /// Per-source breakdown, carried as the raw JSON **string** the MV wrote.
    /// Parsed into a `serde_json::Value` at the DTO boundary, not here.
    pub sources: String,
    pub updated_at: String,
}

/// One `assets` row, for the detail endpoint.
#[derive(Debug, clickhouse::Row, serde::Deserialize)]
pub struct AssetRow {
    pub asset_code: String,
    pub issuer_address: String,
    pub contract_address: String,
    pub home_domain: String,
    pub is_active: u8,
}

#[derive(Debug, clickhouse::Row, serde::Deserialize)]
struct IdRow {
    asset_id: u32,
}

/// One row of the `GET /assets` listing (with the sort key for cursoring).
#[derive(Debug, clickhouse::Row, serde::Deserialize)]
pub struct AssetListRow {
    pub asset_id: u32,
    pub asset_code: String,
    pub issuer_address: String,
    pub contract_address: String,
    pub home_domain: String,
    pub price_usd: String,
    pub change_24h_pct: String,
    pub change_7d_pct: String,
    pub volume_24h_usd: String,
    pub vwap_24h: String,
    /// Raw JSON string from the MV; parsed at the DTO boundary.
    pub sources: String,
    pub updated_at: String,
    /// String form of the sort-column value for this row (cursor payload).
    pub sort_key: String,
}

/// Sortable columns for the listing (the `?sort` param). Deserializes straight
/// from the documented tokens (case-sensitive) — an unknown value fails serde
/// with a message enumerating the valid ones, surfaced as a 400 by
/// `ValidatedQuery`. `ToSchema` publishes the same enum in the OpenAPI doc.
#[derive(Debug, Clone, Copy, serde::Deserialize, utoipa::ToSchema)]
pub enum SortCol {
    #[serde(rename = "price")]
    Price,
    #[serde(rename = "volume_24h")]
    Volume24h,
    #[serde(rename = "change_24h")]
    Change24h,
    #[serde(rename = "code")]
    Code,
}

impl SortCol {
    /// (column expression, is-numeric).
    fn sql(self) -> (&'static str, bool) {
        match self {
            SortCol::Price => ("c.price_usd", true),
            SortCol::Volume24h => ("c.volume_24h_usd", true),
            SortCol::Change24h => ("c.change_24h_pct", true),
            SortCol::Code => ("a.asset_code", false),
        }
    }

    /// True for sorts whose cursor payload must be numeric (`toFloat64` bind).
    pub fn is_numeric(self) -> bool {
        self.sql().1
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy, serde::Deserialize, utoipa::ToSchema)]
pub enum Order {
    #[serde(rename = "asc")]
    Asc,
    #[serde(rename = "desc")]
    Desc,
}

impl Order {
    /// (ORDER BY keyword, keyset comparison operator).
    fn sql(self) -> (&'static str, &'static str) {
        match self {
            Order::Asc => ("ASC", ">"),
            Order::Desc => ("DESC", "<"),
        }
    }
}

/// `?type` filter.
#[derive(Debug, Clone, Copy, serde::Deserialize, utoipa::ToSchema)]
pub enum TypeFilter {
    #[serde(rename = "classic")]
    Classic,
    #[serde(rename = "soroban")]
    Soroban,
    #[serde(rename = "all")]
    All,
}

/// Validated inputs for [`list_assets`].
pub struct ListArgs {
    pub sort: SortCol,
    pub order: Order,
    pub type_filter: TypeFilter,
    pub search: Option<String>,
    pub cursor: Option<Cursor>,
    /// Rows to fetch (caller passes `limit + 1` to detect `has_more`).
    pub fetch_limit: u64,
}

/// Listing query (overview §4.1 / §3.3 CH idiom: `ORDER BY` + `LIMIT` on the
/// merged `current_prices`, keyset cursor on `(sort, asset_id)`). Numeric sorts
/// compare via `toFloat64` (asset_id breaks ties); `code` sorts lexically.
pub async fn list_assets(
    ch: &Client,
    args: ListArgs,
) -> Result<Vec<AssetListRow>, clickhouse::error::Error> {
    let (col, numeric) = args.sort.sql();
    let (dir, cmp) = args.order.sql();
    let sort_expr = if numeric {
        format!("toFloat64({col})")
    } else {
        col.to_string()
    };
    let sort_key_expr = if numeric {
        format!("toString({col})")
    } else {
        col.to_string()
    };

    let mut where_parts: Vec<String> = Vec::new();
    match args.type_filter {
        TypeFilter::Classic => where_parts.push("a.contract_address = ''".to_string()),
        TypeFilter::Soroban => where_parts.push("a.contract_address != ''".to_string()),
        TypeFilter::All => {}
    }
    if args.search.is_some() {
        where_parts.push("startsWith(a.asset_code, ?)".to_string());
    }
    if args.cursor.is_some() {
        let rhs = if numeric {
            "(toFloat64(?), ?)"
        } else {
            "(?, ?)"
        };
        where_parts.push(format!("({sort_expr}, a.asset_id) {cmp} {rhs}"));
    }
    let where_clause = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };

    let sql = format!(
        "SELECT \
           a.asset_id AS asset_id, \
           a.asset_code AS asset_code, \
           a.issuer_address AS issuer_address, \
           a.contract_address AS contract_address, \
           m.home_domain AS home_domain, \
           toString(c.price_usd) AS price_usd, \
           toString(c.change_24h_pct) AS change_24h_pct, \
           toString(c.change_7d_pct) AS change_7d_pct, \
           toString(c.volume_24h_usd) AS volume_24h_usd, \
           toString(c.vwap_24h) AS vwap_24h, \
           c.sources AS sources, \
           formatDateTime(c.updated_at, '%Y-%m-%dT%H:%i:%SZ') AS updated_at, \
           {sort_key_expr} AS sort_key \
         FROM current_prices AS c FINAL \
         INNER JOIN assets AS a FINAL ON a.asset_id = c.asset_id \
         LEFT JOIN asset_metadata AS m FINAL ON m.asset_id = a.asset_id \
         {where_clause} \
         ORDER BY {sort_expr} {dir}, a.asset_id {dir} \
         LIMIT {limit}",
        limit = args.fetch_limit
    );

    // Bind in the order placeholders appear: search, then cursor (value, id).
    let mut q = ch.query(&sql);
    if let Some(s) = args.search {
        q = q.bind(s);
    }
    if let Some(c) = args.cursor {
        q = q.bind(c.v);
        q = q.bind(c.id);
    }
    q.fetch_all::<AssetListRow>().await
}

/// Build the natural-identity `WHERE` fragment + ordered binds selecting the
/// `assets` row for `id`. Variable parts are parameterized (`?`); the native
/// case is fully literal (it has no variable component).
fn identity_where(id: &AssetIdentifier) -> (&'static str, Vec<String>) {
    match id {
        AssetIdentifier::Native => (
            "a.asset_code = 'XLM' AND a.issuer_address = '' AND a.contract_address = ''",
            vec![],
        ),
        AssetIdentifier::Classic { code, issuer } => (
            "a.asset_code = ? AND a.issuer_address = ? AND a.contract_address = ''",
            vec![code.clone(), issuer.clone()],
        ),
        AssetIdentifier::Contract(c) => ("a.contract_address = ?", vec![c.clone()]),
    }
}

/// Fetch the current price for `id` from `current_prices ⨝ assets`.
///
/// Returns `None` when the asset has no current-price row (unknown asset, or the
/// updater MV hasn't produced one yet). `FINAL` collapses both ReplacingMergeTree
/// tables to their latest rows.
pub async fn current_price(
    ch: &Client,
    id: &AssetIdentifier,
) -> Result<Option<CurrentPriceRow>, clickhouse::error::Error> {
    let (where_sql, binds) = identity_where(id);
    let sql = format!(
        "SELECT \
           toString(c.price_usd) AS price_usd, \
           toString(c.price_xlm) AS price_xlm, \
           toString(c.vwap_24h) AS vwap_24h, \
           toString(c.volume_24h_usd) AS volume_24h_usd, \
           toString(c.change_24h_pct) AS change_24h_pct, \
           c.sources AS sources, \
           formatDateTime(c.updated_at, '%Y-%m-%dT%H:%i:%SZ') AS updated_at \
         FROM current_prices AS c FINAL \
         INNER JOIN assets AS a FINAL ON a.asset_id = c.asset_id \
         WHERE {where_sql} \
         LIMIT 1"
    );
    let mut q = ch.query(&sql);
    for b in binds {
        q = q.bind(b);
    }
    q.fetch_optional::<CurrentPriceRow>().await
}

/// One current-price row plus its natural-identity columns, so a batch result
/// can be mapped back to the requested identifier ([`IdentKey`]).
#[derive(Debug, clickhouse::Row, serde::Deserialize)]
pub struct BatchPriceRow {
    pub asset_code: String,
    pub issuer_address: String,
    pub contract_address: String,
    pub price_usd: String,
    pub price_xlm: String,
    pub vwap_24h: String,
    pub volume_24h_usd: String,
    pub change_24h_pct: String,
    /// Raw JSON string from the MV; parsed at the DTO boundary. Kept in lockstep
    /// with [`CurrentPriceRow`] so `/price` and `/prices/batch` cannot drift.
    pub sources: String,
    pub updated_at: String,
}

/// A natural-identity lookup key shared by a requested [`AssetIdentifier`] and a
/// returned [`BatchPriceRow`]. Soroban assets key by contract; classic/native
/// key by `(code, issuer)` — matching how [`identity_where`] filters each.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IdentKey {
    ClassicLike(String, String),
    Contract(String),
}

impl IdentKey {
    pub fn of(id: &AssetIdentifier) -> Self {
        match id {
            AssetIdentifier::Native => IdentKey::ClassicLike("XLM".to_string(), String::new()),
            AssetIdentifier::Classic { code, issuer } => {
                IdentKey::ClassicLike(code.clone(), issuer.clone())
            }
            AssetIdentifier::Contract(c) => IdentKey::Contract(c.clone()),
        }
    }
}

impl BatchPriceRow {
    pub fn ident_key(&self) -> IdentKey {
        if self.contract_address.is_empty() {
            IdentKey::ClassicLike(self.asset_code.clone(), self.issuer_address.clone())
        } else {
            IdentKey::Contract(self.contract_address.clone())
        }
    }
}

/// Fetch current prices for many assets in ONE query (vs. a per-asset N+1 loop).
/// The identity predicates are OR-ed; positional binds are collected in clause
/// order. Returns one row per matched asset — callers map back via [`IdentKey`]
/// and treat absent identifiers as not-found.
pub async fn current_prices_batch(
    ch: &Client,
    ids: &[AssetIdentifier],
) -> Result<Vec<BatchPriceRow>, clickhouse::error::Error> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut clauses = Vec::with_capacity(ids.len());
    let mut binds: Vec<String> = Vec::new();
    for id in ids {
        let (where_sql, b) = identity_where(id);
        clauses.push(format!("({where_sql})"));
        binds.extend(b);
    }
    let sql = format!(
        "SELECT a.asset_code, a.issuer_address, a.contract_address, \
           toString(c.price_usd) AS price_usd, \
           toString(c.price_xlm) AS price_xlm, \
           toString(c.vwap_24h) AS vwap_24h, \
           toString(c.volume_24h_usd) AS volume_24h_usd, \
           toString(c.change_24h_pct) AS change_24h_pct, \
           c.sources AS sources, \
           formatDateTime(c.updated_at, '%Y-%m-%dT%H:%i:%SZ') AS updated_at \
         FROM current_prices AS c FINAL \
         INNER JOIN assets AS a FINAL ON a.asset_id = c.asset_id \
         WHERE {where_clause}",
        where_clause = clauses.join(" OR ")
    );
    let mut q = ch.query(&sql);
    for b in binds {
        q = q.bind(b);
    }
    q.fetch_all::<BatchPriceRow>().await
}

/// Fetch the `assets` row for `id` (for the detail endpoint).
pub async fn asset_detail(
    ch: &Client,
    id: &AssetIdentifier,
) -> Result<Option<AssetRow>, clickhouse::error::Error> {
    let (where_sql, binds) = identity_where(id);
    let sql = format!(
        "SELECT a.asset_code, a.issuer_address, a.contract_address, m.home_domain, a.is_active \
         FROM assets AS a FINAL \
         LEFT JOIN asset_metadata AS m FINAL ON m.asset_id = a.asset_id \
         WHERE {where_sql} \
         LIMIT 1"
    );
    let mut q = ch.query(&sql);
    for b in binds {
        q = q.bind(b);
    }
    q.fetch_optional::<AssetRow>().await
}

/// Resolve a natural identity to the internal `asset_id` surrogate, or `None` if
/// the asset is unknown. Used by endpoints keyed on `asset_id` (e.g. oracles).
pub async fn resolve_asset_id(
    ch: &Client,
    id: &AssetIdentifier,
) -> Result<Option<u32>, clickhouse::error::Error> {
    let (where_sql, binds) = identity_where(id);
    let sql = format!("SELECT a.asset_id FROM assets AS a FINAL WHERE {where_sql} LIMIT 1");
    let mut q = ch.query(&sql);
    for b in binds {
        q = q.bind(b);
    }
    Ok(q.fetch_optional::<IdRow>().await?.map(|r| r.asset_id))
}

// ----------------------------------------------------------------------------
// OHLCV (overview §4.2)
// ----------------------------------------------------------------------------

/// The quote leg the candles are denominated in (the `?base_currency` param).
/// The documented tokens are uppercase; the all-lowercase aliases preserve the
/// historically case-insensitive behavior of this one param (mixed case like
/// `uSd` is a 400 — task 0119's exact-token policy).
#[derive(Debug, Clone, Copy, serde::Deserialize, utoipa::ToSchema)]
pub enum BaseCurrency {
    #[serde(rename = "USD", alias = "usd")]
    Usd,
    #[serde(rename = "XLM", alias = "xlm")]
    Xlm,
}

impl BaseCurrency {
    pub fn as_str(self) -> &'static str {
        match self {
            BaseCurrency::Usd => "USD",
            BaseCurrency::Xlm => "XLM",
        }
    }
}

/// OHLCV granularity → per-grain table suffix. Tokens are case-sensitive by
/// necessity: `1m` (minute) and `1M` (month) differ only by case.
#[derive(Debug, Clone, Copy, serde::Deserialize, utoipa::ToSchema)]
pub enum Granularity {
    #[serde(rename = "1m")]
    M1,
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "1d")]
    D1,
    #[serde(rename = "1w")]
    W1,
    #[serde(rename = "1M")]
    Mo1,
}

impl Granularity {
    pub fn as_str(self) -> &'static str {
        match self {
            Granularity::M1 => "1m",
            Granularity::M15 => "15m",
            Granularity::H1 => "1h",
            Granularity::H4 => "4h",
            Granularity::D1 => "1d",
            Granularity::W1 => "1w",
            Granularity::Mo1 => "1M",
        }
    }

    /// Bucket width in seconds, for the window-vs-granularity point count
    /// (task 0119). `1M` uses 30 days — under-counting a month's seconds
    /// over-counts buckets, which only makes the check stricter.
    pub fn seconds(self) -> u64 {
        match self {
            Granularity::M1 => 60,
            Granularity::M15 => 15 * 60,
            Granularity::H1 => 3600,
            Granularity::H4 => 4 * 3600,
            Granularity::D1 => 86_400,
            Granularity::W1 => 7 * 86_400,
            Granularity::Mo1 => 30 * 86_400,
        }
    }

    /// Finest granularity whose inclusive point count for `span` seconds stays
    /// within `max_points` — the auto-granularity for explicit windows and for
    /// `timeframe=all` (PR #217 review): maximum resolution the cap allows,
    /// coarsening by itself as the window grows. Falls back to `1M` (a span
    /// would need >400 years to overflow even that).
    pub fn finest_for_span(span: u64, max_points: u64) -> Self {
        [
            Granularity::M1,
            Granularity::M15,
            Granularity::H1,
            Granularity::H4,
            Granularity::D1,
            Granularity::W1,
        ]
        .into_iter()
        .find(|g| span.div_ceil(g.seconds()) < max_points)
        .unwrap_or(Granularity::Mo1)
    }
}

/// Requested time window (overview §4.2 auto-granularity table).
#[derive(Debug, Clone, Copy, serde::Deserialize, utoipa::ToSchema)]
pub enum Timeframe {
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "24h")]
    H24,
    #[serde(rename = "7d")]
    D7,
    #[serde(rename = "30d")]
    D30,
    #[serde(rename = "1y")]
    Y1,
    #[serde(rename = "all")]
    All,
}

impl Timeframe {
    /// Auto-selected granularity when `?granularity` is omitted.
    pub fn default_granularity(self) -> Granularity {
        match self {
            Timeframe::H1 => Granularity::M1,
            Timeframe::H24 => Granularity::M15,
            Timeframe::D7 => Granularity::H1,
            Timeframe::D30 => Granularity::H4,
            Timeframe::Y1 => Granularity::D1,
            Timeframe::All => Granularity::D1,
        }
    }

    /// Window width in seconds, or `None` for `all` (whose start is
    /// [`STELLAR_GENESIS_EPOCH`], not a width). The single width table: the
    /// handler derives the SQL-bound epochs from it, so the validated window
    /// and the executed window cannot disagree.
    pub fn seconds(self) -> Option<u64> {
        match self {
            Timeframe::H1 => Some(3600),
            Timeframe::H24 => Some(86_400),
            Timeframe::D7 => Some(7 * 86_400),
            Timeframe::D30 => Some(30 * 86_400),
            Timeframe::Y1 => Some(365 * 86_400),
            Timeframe::All => None,
        }
    }

    pub fn is_all(self) -> bool {
        matches!(self, Timeframe::All)
    }
}

/// Earliest possible candle: Stellar genesis (2015-09-30 UTC). Lower bound for
/// `timeframe=all` window math — makes `all` computable without touching CH.
pub const STELLAR_GENESIS_EPOCH: i64 = 1_443_571_200;

/// Validated OHLCV query inputs. `start`/`end` are **validated epochs**
/// (task 0119): binding the handler's parse result instead of the raw string
/// leaves exactly one interpretation of the window — no divergence between our
/// point-count check and what ClickHouse would have made of the raw value.
pub struct OhlcvArgs {
    pub asset_id: u32,
    pub quote_asset_id: u32,
    pub granularity: Granularity,
    /// Window lower bound (epoch seconds) — always set by the handler.
    pub start: Option<i64>,
    /// Window upper bound (epoch seconds) — only when the client supplied one;
    /// an open top costs nothing (future buckets don't exist).
    pub end: Option<i64>,
    pub limit: u64,
}

/// Read merged candles for one asset against one quote leg, at the chosen grain.
///
/// Per-source rows are collapsed (`FINAL`) then merged across sources per
/// bucket: `high=max`, `low=min`, volumes + `trade_count` summed, `vwap`
/// volume-weighted, and `open`/`close` taken from the highest-volume source
/// (`argMax(.., volume_base)`). O/H/L/C are returned as stored (quote-asset
/// denominated). Ascending by timestamp.
pub async fn ohlcv(ch: &Client, args: OhlcvArgs) -> Result<Vec<Candle>, clickhouse::error::Error> {
    let table = format!("price_ohlcv_{}", args.granularity.as_str());

    let mut conds = vec!["asset_id = ?".to_string(), "quote_asset_id = ?".to_string()];
    if args.start.is_some() {
        conds.push("timestamp >= toDateTime(?)".to_string());
    }
    if args.end.is_some() {
        conds.push("timestamp <= toDateTime(?)".to_string());
    }

    // NB: output aliases must NOT collide with column names referenced inside
    // aggregates — e.g. aliasing `sum(volume_base) AS volume_base` shadows the
    // `volume_base` column so `argMax(open, volume_base)` would nest aggregates
    // (CH error 184). Deserialization is positional (RowBinary), so the alias
    // labels here are cosmetic and need only be distinct.
    // When the window × granularity yields more buckets than `limit`, keep the
    // MOST-RECENT ones (inner `ORDER BY timestamp DESC LIMIT`), then re-sort
    // ascending for output. An ASC+LIMIT would instead return the OLDEST N and
    // silently drop the recent candles a chart actually wants. `ts` is ISO-8601
    // (`%Y-%m-%dT%H:%i:%SZ`), so lexicographic `ts ASC` == chronological order.
    let sql = format!(
        "SELECT ts, o, h, l, c, vb, vqu, vw, tc FROM ( \
           SELECT \
             formatDateTime(timestamp, '%Y-%m-%dT%H:%i:%SZ') AS ts, \
             toString(argMax(open, volume_base)) AS o, \
             toString(max(high)) AS h, \
             toString(min(low)) AS l, \
             toString(argMax(close, volume_base)) AS c, \
             toString(sum(volume_base)) AS vb, \
             toString(sum(volume_quote_usd)) AS vqu, \
             toString(toDecimal128(ifNull(sum(toFloat64(vwap) * toFloat64(volume_base)) \
                 / nullIf(sum(toFloat64(volume_base)), 0), 0), 14)) AS vw, \
             toUInt64(sum(trade_count)) AS tc \
           FROM {table} FINAL \
           WHERE {conds} \
           GROUP BY timestamp \
           ORDER BY timestamp DESC \
           LIMIT {limit} \
         ) ORDER BY ts ASC",
        conds = conds.join(" AND "),
        limit = args.limit
    );

    let mut q = ch.query(&sql).bind(args.asset_id).bind(args.quote_asset_id);
    if let Some(s) = args.start {
        q = q.bind(s);
    }
    if let Some(e) = args.end {
        q = q.bind(e);
    }
    q.fetch_all::<Candle>().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_where_native_is_literal_no_binds() {
        let (sql, binds) = identity_where(&AssetIdentifier::Native);
        assert!(sql.contains("asset_code = 'XLM'"));
        assert!(sql.contains("contract_address = ''"));
        assert!(binds.is_empty());
    }

    #[test]
    fn identity_where_classic_binds_code_then_issuer() {
        let (sql, binds) = identity_where(&AssetIdentifier::Classic {
            code: "USDC".into(),
            issuer: "GISSUER".into(),
        });
        assert_eq!(binds, vec!["USDC".to_string(), "GISSUER".to_string()]);
        assert!(sql.contains("asset_code = ?"));
        assert!(sql.contains("contract_address = ''"));
    }

    #[test]
    fn identity_where_contract_binds_address() {
        let (sql, binds) = identity_where(&AssetIdentifier::Contract("CTOKEN".into()));
        assert_eq!(binds, vec!["CTOKEN".to_string()]);
        assert!(sql.contains("a.contract_address = ?"));
    }

    // The param enums deserialize from their documented tokens (task 0119).
    // `serde_json::from_str` on a quoted token exercises the same `Deserialize`
    // impl the axum `Query` extractor drives via serde_urlencoded.

    fn tok<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, serde_json::Error> {
        serde_json::from_str(&format!("\"{s}\""))
    }

    #[test]
    fn enum_params_accept_their_documented_tokens() {
        assert!(matches!(tok::<SortCol>("price"), Ok(SortCol::Price)));
        assert!(matches!(
            tok::<SortCol>("volume_24h"),
            Ok(SortCol::Volume24h)
        ));
        assert!(matches!(
            tok::<SortCol>("change_24h"),
            Ok(SortCol::Change24h)
        ));
        assert!(matches!(tok::<SortCol>("code"), Ok(SortCol::Code)));
        assert!(matches!(tok::<Order>("asc"), Ok(Order::Asc)));
        assert!(matches!(tok::<Order>("desc"), Ok(Order::Desc)));
        assert!(matches!(
            tok::<TypeFilter>("classic"),
            Ok(TypeFilter::Classic)
        ));
        assert!(matches!(
            tok::<TypeFilter>("soroban"),
            Ok(TypeFilter::Soroban)
        ));
        assert!(matches!(tok::<TypeFilter>("all"), Ok(TypeFilter::All)));
        for t in ["1h", "24h", "7d", "30d", "1y", "all"] {
            assert!(tok::<Timeframe>(t).is_ok(), "timeframe {t}");
        }
        for g in ["1m", "15m", "1h", "4h", "1d", "1w", "1M"] {
            assert!(tok::<Granularity>(g).is_ok(), "granularity {g}");
        }
    }

    #[test]
    fn enum_params_are_case_sensitive() {
        assert!(tok::<SortCol>("PRICE").is_err());
        assert!(tok::<Order>("DESC").is_err());
        assert!(tok::<TypeFilter>("Classic").is_err());
        assert!(tok::<Timeframe>("ALL").is_err());
    }

    #[test]
    fn granularity_case_distinguishes_minute_from_month() {
        assert!(matches!(tok::<Granularity>("1m"), Ok(Granularity::M1)));
        assert!(matches!(tok::<Granularity>("1M"), Ok(Granularity::Mo1)));
        assert!(tok::<Granularity>("1H").is_err());
    }

    #[test]
    fn base_currency_accepts_lowercase_alias_only() {
        assert!(matches!(tok::<BaseCurrency>("USD"), Ok(BaseCurrency::Usd)));
        assert!(matches!(tok::<BaseCurrency>("usd"), Ok(BaseCurrency::Usd)));
        assert!(matches!(tok::<BaseCurrency>("xlm"), Ok(BaseCurrency::Xlm)));
        assert!(tok::<BaseCurrency>("uSd").is_err());
        assert!(tok::<BaseCurrency>("EUR").is_err());
    }

    #[test]
    fn finest_for_span_picks_max_resolution_within_cap() {
        // 24h fits minute candles; 30d needs 15m; ~11y (genesis → 2026) needs
        // daily; the fallback for absurd spans is monthly.
        assert!(matches!(
            Granularity::finest_for_span(86_400, 5000),
            Granularity::M1
        ));
        assert!(matches!(
            Granularity::finest_for_span(30 * 86_400, 5000),
            Granularity::M15
        ));
        assert!(matches!(
            Granularity::finest_for_span(11 * 365 * 86_400, 5000),
            Granularity::D1
        ));
        assert!(matches!(
            Granularity::finest_for_span(500 * 365 * 86_400, 5000),
            Granularity::Mo1
        ));
    }

    #[test]
    fn sort_col_numeric_flag_matches_sql() {
        assert!(SortCol::Price.is_numeric());
        assert!(SortCol::Volume24h.is_numeric());
        assert!(SortCol::Change24h.is_numeric());
        assert!(!SortCol::Code.is_numeric());
    }
}
