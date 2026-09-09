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
    /// Price provenance (task 0178): `traded` / `oracle` / `""`. Decoded
    /// POSITIONALLY by `clickhouse::Row`, so this field's position must match
    /// the SELECT's — append to both together or the row silently misparses.
    pub method: String,
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
    /// Price provenance (task 0178). Positional — see [`CurrentPriceRow::method`].
    pub method: String,
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
           if(a.asset_code != '', a.asset_code, sym.symbol) AS asset_code, \
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
           c.method AS method, \
           {sort_key_expr} AS sort_key \
         FROM current_prices AS c FINAL \
         INNER JOIN assets AS a FINAL ON a.asset_id = c.asset_id \
         LEFT JOIN asset_metadata AS m FINAL ON m.asset_id = a.asset_id \
         LEFT JOIN asset_symbol AS sym FINAL ON sym.contract_address = a.contract_address \
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
           formatDateTime(c.updated_at, '%Y-%m-%dT%H:%i:%SZ') AS updated_at, \
           c.method AS method \
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
    /// Price provenance (task 0178). Positional — see [`CurrentPriceRow::method`].
    pub method: String,
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
           formatDateTime(c.updated_at, '%Y-%m-%dT%H:%i:%SZ') AS updated_at, \
           c.method AS method \
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
        "SELECT if(a.asset_code != '', a.asset_code, sym.symbol) AS asset_code, \
           a.issuer_address, a.contract_address, m.home_domain, a.is_active \
         FROM assets AS a FINAL \
         LEFT JOIN asset_metadata AS m FINAL ON m.asset_id = a.asset_id \
         LEFT JOIN asset_symbol AS sym FINAL ON sym.contract_address = a.contract_address \
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

/// Floor on the peg series' staleness window, in seconds — task 0246.
///
/// 🔑 **A one-bucket window is only safe while the bucket is wider than the
/// oracle's poll interval.** `oracleWatcher` runs `rate(5 minutes)`
/// (`infra/envs/production.json`) and `usd_rate` carries one row per poll, so a
/// 1-minute bucket contains an observation only about one time in five. Scoping
/// the rate strictly to the bucket — which is what `price_usd_series` does, and
/// what it can safely do because it exists only at `1d` and `1h` — would make
/// `GET /ohlcv?granularity=1m` alternate between a measured rate and the `$1`
/// fallback every minute, flipping `method` with it. That is worse than the
/// unbounded forward-fill task 0246 removed: a three-minute-old measurement is
/// strictly better evidence than a literal `$1`.
///
/// 300 s is not a new number — it is enrichment's `FORWARD_FILL_WINDOW_S`
/// default, the window the write path already forward-fills an oracle reading
/// across at 1-minute candles (`ch_enrich.rs`). Duplicated rather than imported
/// because `prices-api` does not depend on `enrichment-worker`; if that default
/// moves, this moves with it.
pub const ORACLE_POLL_FLOOR_S: u64 = 300;

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

    /// The `INTERVAL` literal for ONE bucket of this grain — task 0246.
    ///
    /// Used two ways and they must agree: `bkt + INTERVAL x` is the bucket's
    /// end, and the same width is the staleness window wherever it is at least
    /// [`ORACLE_POLL_FLOOR_S`] wide.
    ///
    /// ⚠️ These must stay identical to the intervals `rollups.sql` uses to BUILD
    /// the candle buckets. `bkt + INTERVAL x` has to land exactly on the next
    /// bucket's start, or the window is off by the difference on every row —
    /// silently, since nothing fails.
    pub fn interval_sql(self) -> &'static str {
        match self {
            Granularity::M1 => "1 MINUTE",
            Granularity::M15 => "15 MINUTE",
            Granularity::H1 => "1 HOUR",
            Granularity::H4 => "4 HOUR",
            Granularity::D1 => "1 DAY",
            Granularity::W1 => "1 WEEK",
            Granularity::Mo1 => "1 MONTH",
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

/// Reference `asset_id`s the USD classification keys on, resolved by **natural
/// identity** (never a bare `asset_id` — task 0139 has 3,281 ids serving 6,568
/// identities). Mirrors the enrichment worker's own `resolve_reference_ids`, so
/// the read side and the write side cannot disagree about what "USDC" means.
#[derive(Debug, Clone)]
pub struct UsdRefs {
    /// Canonical USDC. **Required** — the peg-vs-oracle split keys on it, and
    /// without it a genuine peg row (`close_usd = close` on a USDC leg) is
    /// indistinguishable from the anomalous same-signature rows on other legs
    /// and would be dropped. Its absence is a real server-side data gap.
    pub usdc: u32,
    /// XLM and canonical USDT, the two pivot references. **Optional**: they only
    /// select the `traded` label. An untracked reference cannot be any candle's
    /// quote leg, so the branch simply never matches — refusing to serve the
    /// endpoint over a missing label would turn a cosmetic dependency into an
    /// outage.
    pub pivots: Vec<u32>,
}

/// What `base_currency` asks for. Per [ADR 0011] it **denominates**; it does not
/// select a quote leg.
#[derive(Debug, Clone)]
pub enum Denomination {
    /// Express every candle in USD, whatever leg it traded against. No quote
    /// filter — that filter was the defect: an asset trading only against XLM
    /// matched the base conjunct and was emptied by the quote one, returning a
    /// `200` with no data for 20,481 assets.
    Usd(UsdRefs),
    /// Filter to one quote leg and return the candles as stored.
    ///
    /// ⚠️ This is the **pre-ADR-0011 behaviour**, still in place for
    /// `base_currency=XLM`. Converting that mode needs XLM's own USD rate per
    /// bucket, which is not on the candle row — ADR 0011 §6's degenerate cases.
    /// Tracked in [`0170`]; not a decision, just not done yet.
    QuoteLeg(u32),
}

/// Validated OHLCV query inputs. `start`/`end` are **validated epochs**
/// (task 0119): binding the handler's parse result instead of the raw string
/// leaves exactly one interpretation of the window — no divergence between our
/// point-count check and what ClickHouse would have made of the raw value.
pub struct OhlcvArgs {
    pub asset_id: u32,
    /// How the candles are denominated (ADR 0011 §1).
    pub denomination: Denomination,
    pub granularity: Granularity,
    /// Window lower bound (epoch seconds) — always set by the handler.
    pub start: Option<i64>,
    /// Window upper bound (epoch seconds) — only when the client supplied one;
    /// an open top costs nothing (future buckets don't exist).
    pub end: Option<i64>,
    pub limit: u64,
}

/// The exact close, as one expression, because three output columns must agree
/// on it. `c_x` is `close_usd` **as stored** — full `Decimal(38, 14)` — while
/// `o_x`/`h_x`/`l_x`/`w_x` are derived through `toFloat64`, whose 53-bit mantissa
/// holds only ~15-16 significant digits.
///
/// 🔑 **That is the whole of task 0229, and the mechanism is float precision, not
/// decimal rounding.** A five-figure price carries 19 significant digits at
/// `Decimal(38, 14)`, so the float product lands up to one ulp away — measured on
/// prod at BTC scale as **1.343e-11 against a 1.455e-11 ulp (0.92 of one)**. A
/// 14-decimal half-tick is 5e-15, roughly **2,700× too small** to explain it, so
/// re-rounding `close` to 14 decimals cannot fix this: it is already there.
///
/// 🔴 **`least`/`greatest` IGNORE null arguments — they do not propagate them.**
/// Verified on 26.3.10.60: `greatest(CAST(NULL AS Nullable(Decimal(38,14))), 2.5)`
/// returns `2.5`, where `NULL + 2.5` returns `NULL`. This is the opposite of
/// ClickHouse's usual behaviour and it matters here: `h_x`/`l_x` are
/// `toDecimal128OrNull` and go NULL on `Decimal128(38, 14)` overflow, which is
/// reachable because `rate` is unbounded above (a dust `close` at
/// [`PRECISION_FLOOR`] against a large `close_usd` gives `rate = 1e22`). Left
/// unguarded, an extreme the query **could not compute** would be reported as
/// the close — asserting a value rather than admitting the gap. The `isNull`
/// guards below keep the honest `null`, and
/// `ohlcv_an_unrepresentable_extreme_stays_null_rather_than_becoming_the_close`
/// pins it.
///
/// `least`/`greatest` therefore clamp the derived extremes over the exact close.
/// This preserves `close`'s exactness, which ADR 0011 §3 keeps deliberately, and
/// moves an extreme by at most the ulp the rounding already implied. `open` needs
/// no clamp: it is derived through the same `rate` as `h_x`/`l_x`, and scaling by
/// one positive factor is monotonic, so `l_x <= o_x <= h_x` holds within a row and
/// survives `min`/`max` across rows. Only the exact/derived boundary breaks.
const CLOSE_EXACT: &str = "argMaxIf(c_x, (volume_base, quote_asset_id), valid)";

/// The published `high` / `low`, before rendering. Named because `vw` clamps
/// against these rather than against the raw aggregates — the response has to be
/// self-consistent, so vwap is bounded by the values the caller actually sees.
///
/// The `isNull` arms carry the guard described above: an aggregate that is NULL
/// (every valid row overflowed) must publish as `null`, and `least`/`greatest`
/// would otherwise swallow it into the close.
///
/// 🔑 That same NULL-swallowing is then *relied upon* one level up, and the
/// asymmetry is deliberate rather than an accident of nesting. When a bound is
/// NULL the whole expression is NULL, so `greatest(vwap, NULL)` leaves vwap
/// unclamped on that side — which is correct: a bound we could not compute must
/// not be used to move a value we could.
const HIGH_PUBLISHED: &str = "if(isNull(maxIf(h_x, valid)), NULL, greatest(maxIf(h_x, valid), argMaxIf(c_x, (volume_base, quote_asset_id), valid)))";
const LOW_PUBLISHED: &str = "if(isNull(minIf(l_x, valid)), NULL, least(minIf(l_x, valid), argMaxIf(c_x, (volume_base, quote_asset_id), valid)))";

/// The volume-weighted mean before clamping — task 0229's review, finding 1.
///
/// 🔴 **`vwap` escapes `[low, high]` far more readily than the extremes do**, and
/// by a compounding of the same cause. `w_x` is already a rounded float product,
/// and this adds a *second* round-trip on top: `sum(w_x * volume) / sum(volume)`.
/// `(x*v)/v != x` in IEEE754, and at 14-decimal scale that ulp is not absorbed.
/// Measured on 26.3.10.60 over 300,000 single-trade candles at BTC scale
/// (`open = high = low = close = vwap`, so the true vwap sits exactly on the
/// bound): **26,395 rows returned `vwap > high` and 26,387 returned
/// `vwap < low` — ~8.8% each**, against a far rarer close/extreme crossing.
///
/// ⚠️ Nothing was going to surface this on its own. [[0120]]'s conformance
/// assertion is `low <= open,close <= high`; `vwap` appears there only in the
/// "is a decimal string" check. It was found by reviewing this fix, not by the
/// suite that found the defect this fix is for.
///
/// A volume-weighted mean of prices within a bucket must lie within that
/// bucket's range, so clamping is a restatement of what vwap *is* rather than a
/// correction applied to it.
const VWAP_RAW: &str = "toDecimal128OrNull(toString( \
                     sumIf(toFloat64(w_x) * toFloat64(volume_base), valid) \
                     / nullIf(sumIf(toFloat64(volume_base), valid), 0)), 14)";

/// Smallest `close` / `close_usd` a USD rate may be derived from — a
/// **precision precondition**, not a plausibility band.
///
/// The columns are `Decimal(38, 14)`, so one tick is `1e-14`. A row measured on
/// prod carries `close = 5e-14`, `close_usd = 4e-14` — five ticks over four. The
/// implied rate is 1.25, which looks perfectly ordinary, so **no check on the
/// derived rate can reject it**: the value is not what is wrong, the inputs are.
/// Both operands are single-digit multiples of the quantisation step, and their
/// ratio is quantisation noise wearing a plausible number.
///
/// `1e-12` is 100 ticks, so a value at the threshold still carries ~2 significant
/// digits. ⚠️ The exact figure is a judgement — the measurement establishes that
/// a floor is needed and roughly where the noise lives, not that 100 ticks is
/// the uniquely right line. Rows below it are treated as unpriced (§5): the
/// bucket returns, without price fields.
const PRECISION_FLOOR: &str = "toDecimal128('0.000000000001', 14)";

/// Synthesize a USD series for a **peg asset** — one that is only ever stored as
/// a quote leg, never as a base (ADR 0011 §6).
///
/// ## Why this cannot be a normal query
///
/// Canonical USDC never appears as `asset_id` in any candle: the quote-preference
/// design makes it the quote, always. So `GET /assets/{USDC}/ohlcv` asks for a
/// USDC/USDC self-pair and matches **zero rows** — and, unlike the wide defect
/// this endpoint's main path fixes, dropping the quote filter does not help. The
/// series has to be built rather than read.
///
/// ## Buckets from real trading, rate from `usd_rate`
///
/// The bucket timestamps come from candles where USDC is the **quote**, so the
/// series spans the whole backfilled range and every bucket corresponds to a
/// period the market was actually open — not a synthetic calendar.
///
/// The value per bucket is the newest `prices.usd_rate` observation **inside**
/// that bucket — its closing rate. That is 0167's stated rule for a
/// bucket-grained consumer (*"T is the BUCKET'S END"*), never an average, and it
/// is the same rule `price_usd_series{,_1h}` applies, so the two surfaces cannot
/// answer differently for the same request.
///
/// ⚠️ **It resolved at the bucket's START until task 0246**, with no staleness
/// bound — which made this surface disagree with the view on every bucket and
/// forward-fill a dead oracle's last reading indefinitely. See the comment at
/// the join for the full account; do not "simplify" it back to an ASOF.
///
/// ## The fallback is the peg, and it is labelled as such
///
/// `usd_rate` starts 2026-03-11; `timeframe=all` reads back to 2021. Buckets
/// holding no observation of their own fall back to $1 and are labelled
/// `method = 'peg'`, which 0165 defines as *"no measured rate was available"* —
/// never as an assertion that $1 is correct.
///
/// Since task 0246 that covers two cases with one rule: deep history before the
/// feed existed, and **any later gap in it**. A bucket the oracle sat out reads
/// `peg` rather than inheriting the previous bucket's measurement, so the label
/// stays true no matter how long the outage runs.
///
/// ⚠️ **This is the one place a literal `1.0` is right.** ADR 0011 §6 forbids a
/// hardcoded peg *where a measurement exists* — our own enrichment prices a
/// `TF/USDC` candle at `close × 0.9993`, so a flat $1 would contradict our data.
/// Where no measurement exists the peg IS the fallback, and the `method` field is
/// what keeps the two distinguishable. A response that silently rendered both as
/// the same number would be [`0212`]'s hardcoded-peg defect in a new place.
///
/// ## What is deliberately zero
///
/// `volume_base` and `trade_count` are `0`: USDC is not traded as a base, so
/// there is no base volume to report. Reporting its volume as a *quote* here
/// would answer a different question than the one asked.
///
/// ## Denominating in XLM — derived, deliberately not inverted
///
/// ADR 0011 §6 says *derive rather than invert*, and the difference matters. The
/// market is stored one way round only: base XLM, quote USDC. Flipping that
/// candle into a USDC/XLM one is a minefield — O/H/L invert with **high↔low
/// swapping**, `volume_base` becomes the *quote* volume rather than a
/// reciprocal, `volume_quote_usd` re-bases onto the other leg, and `vwap` has to
/// be re-weighted rather than flipped.
///
/// So the series is **built from two USD rates** instead:
///
/// ```text
/// USDC in XLM = USDC's USD rate / XLM's USD price in that bucket
/// ```
///
/// The numerator is the same `usd_rate` observation the USD path uses; the
/// denominator is `close_usd` on the XLM/USDC candle, which is XLM's USD price
/// as already computed by enrichment. No inversion, no volume re-basing, and the
/// pitfalls above never arise — the volumes are `0` here for the same reason
/// they are in USD mode.
///
/// ⚠️ The denominator is guarded by the same [`PRECISION_FLOOR`]: an unpriced or
/// dust-valued XLM bucket yields no price rather than a division blow-up.
/// The `Denomination::Usd` aggregate list, extracted so a ClickHouse-free test
/// can read it — see [`OUTER_ALIASES`] for why the column tail is not something
/// to maintain by hand in three places.
fn usd_aggregates() -> String {
    format!(
        "if(countIf(valid) = 0, NULL, toString(argMaxIf(o_x, (volume_base, quote_asset_id), valid))) AS o, \
                 if(countIf(valid) = 0, NULL, toString({HIGH_PUBLISHED})) AS h, \
                 if(countIf(valid) = 0, NULL, toString({LOW_PUBLISHED})) AS l, \
                 if(countIf(valid) = 0, NULL, toString({CLOSE_EXACT})) AS c, \
                 toString(sum(volume_base)) AS vb, \
                 toString(sum(volume_quote_usd)) AS vqu, \
                 if(countIf(valid) = 0 OR isNull({VWAP_RAW}), NULL, \
                    toString(least(greatest({VWAP_RAW}, {LOW_PUBLISHED}), {HIGH_PUBLISHED}))) AS vw, \
                 toUInt64(sum(trade_count)) AS tc, \
                 nullIf(if(countIf(valid) = 0, NULL, argMaxIf(meth, (volume_base, quote_asset_id), valid)), '') AS meth, \
                 if(countIf(valid) = 0, NULL, toUInt8(1)) AS drv, \
                 {PROVENANCE_NULL_TAIL}"
    )
}

/// The `Denomination::QuoteLeg` aggregate list. Same extraction, same reason.
fn quote_leg_aggregates() -> String {
    format!(
        "toNullable(toString(argMax(open, volume_base))) AS o, \
             toNullable(toString(max(high))) AS h, \
             toNullable(toString(min(low))) AS l, \
             toNullable(toString(argMax(close, volume_base))) AS c, \
             toString(sum(volume_base)) AS vb, \
             toString(sum(volume_quote_usd)) AS vqu, \
             toNullable(toString(if(isNull(toDecimal128OrNull(toString( \
                 sum(toFloat64(vwap) * toFloat64(volume_base)) \
                 / nullIf(sum(toFloat64(volume_base)), 0)), 14)), toDecimal128(0, 14), \
                 least(greatest(toDecimal128OrNull(toString( \
                     sum(toFloat64(vwap) * toFloat64(volume_base)) \
                     / nullIf(sum(toFloat64(volume_base)), 0)), 14), min(low)), max(high))))) AS vw, \
             toUInt64(sum(trade_count)) AS tc, \
             CAST(NULL AS Nullable(String)) AS meth, \
             CAST(NULL AS Nullable(UInt8)) AS drv, \
             {PROVENANCE_NULL_TAIL}"
    )
}

/// The peg-series SQL, as a pure string — extracted for the same reason
/// [`usd_method_expr`] is: the invariants below are unobservable from the
/// outside and a ClickHouse-free unit test needs a string it can read.
///
/// Pinned by `peg_series_sql_*` in this module's tests.
fn peg_series_sql(args: &OhlcvArgs, in_xlm: bool, table: &str, conds: &[String]) -> String {
    // The staleness window: `[floor, bucket end)`.
    //
    // For every grain at least ORACLE_POLL_FLOOR_S wide the floor is the
    // bucket's own start, so the window IS the bucket and this is exactly
    // `price_usd_series`'s rule — `toStartOfInterval(t, g) = bkt` and
    // `bkt <= t < bkt + g` are the same predicate, which is what keeps the two
    // surfaces in agreement (task 0246 AC 1).
    //
    // `1m` is the one grain narrower than the oracle's 5-minute cadence, so its
    // floor widens to ORACLE_POLL_FLOOR_S — see that constant for why a strict
    // one-bucket window would be a regression there rather than a fix.
    //
    // ⚠️ The floor is applied in the OUTER projection, where the bucket row is
    // `bo` (see below), so it is spelled against `bo`, not `b`.
    let floor = if args.granularity.seconds() >= ORACLE_POLL_FLOOR_S {
        "bo.bkt".to_string()
    } else {
        format!("bo.bend - INTERVAL {ORACLE_POLL_FLOOR_S} SECOND")
    };
    // The no-match sentinels, in one place because every expression below must
    // agree on what "no usable observation for this bucket" means. Two ways to
    // fail: nothing matched at all, or what matched is older than the window.
    //
    // ⚠️ The window test is not redundant under `join_use_nulls = 0`, it is the
    // belt: an unmatched ASOF yields the DEFAULT, and for a DateTime that is
    // `1970-01-01` — which fails the floor too. Under `join_use_nulls = 1` the
    // matched-row test carries it, since `NULL >= x` is NULL, and `false AND
    // NULL` is false.
    //
    // ⚠️ Task 0267, review round 1 (WR-05 / WR-06). The measured rate is read
    // through TWO method-specific ASOF joins rather than one `IN ('oracle',
    // 'external')` subquery, and the split is the whole preference rule:
    //
    //   * `ro` — the newest `oracle` reading before the bucket end, valid only
    //     inside the bucket's own window `[floor, bend)`. Unchanged from 0246.
    //   * `re` — the newest `external` row before the bucket end, valid for
    //     the WHOLE UTC DAY it is stamped on: `[toStartOfDay(bkt, 'UTC'), bend)`.
    //
    //     ⚠️ This day-wide window is a SAFETY NET, not the loading plan. Since
    //     the 2026-09-09 hourly decision the loader runs at BOTH grains
    //     (`--grain daily|hourly`) and production carries an external row for
    //     every hour of every covered day, so the window is only ever exercised
    //     by a bucket whose own hour has a row anyway. It exists for the case
    //     where someone loads the DAILY file alone: task 0268's external tier
    //     prices every candle of that day — at every grain — from that one row,
    //     and a one-bucket window here would publish `0.96812`/`external` for
    //     the 00:00 hour of 2023-03-11 and `1`/`peg` for the other twenty-three,
    //     while the same day's hourly XLM/USDC candles carried the measured rate
    //     throughout.
    //
    //     ⚠️ `price_usd_series_1h` buckets the rate strictly by the HOUR and has
    //     NO such net (views.sql). With hourly rows loaded, and with the net
    //     bounded by the epoch below, the two surfaces agree everywhere — which
    //     is what task 0246's cross-surface criterion asserts. Two cases used to
    //     break that: a daily-only load (stated in the views' own comment), and
    //     the oracle epoch's own day, where the import ends at 13:00 and the
    //     epoch is 14:00 — an hour at or after 14:00 with no poll in its window
    //     took the 13:00 import through the day-wide net and published it as
    //     `external` while the view published `1`/`peg`. The `bo.bkt` bound
    //     closes the second.
    //
    //     ⚠️ `'UTC'` is NAMED (review round 2, CR-02). `usd_rate.timestamp` and
    //     `price_ohlcv_*.timestamp` are bare `DateTime`, so an unzoned
    //     `toStartOfDay` resolves in the SERVER timezone and the window slides
    //     with its UTC offset — at UTC+2 the last two hours of every imported
    //     day would drop back to `peg`. `ch_enrich.rs` enforces the same rule
    //     for the external enrichment tier with a unit test; this is the other
    //     half of the same semantic.
    //
    // A valid oracle reading wins the bucket OUTRIGHT, regardless of which row
    // is newer — the same rule as `views.sql`'s rank-first `argMax` tuple, so
    // the two surfaces agree on any bucket that holds both, and the ONE such
    // bucket in production (the 1d bucket of the epoch day, 2026-03-11, which
    // holds the import at 00:00 and the polls from 14:00) reads `oracle`. The
    // previous shape ranked oracle only WITHIN an instant and let the ASOF's
    // recency decide across instants, which is a different rule and would have
    // published a later import over a poll.
    //
    // Within one method the sorting key admits at most one row per instant, so
    // neither side needs a collapse before the join.
    //
    // Both joins are NESTED (`bo` is `b` already joined to `ro`) rather than
    // chained in one FROM: a nested subquery needs nothing from the multi-JOIN
    // rewrite and reads the same under both analyzers. Column names on the two
    // right sides are DISTINCT (`orts`/`erts`, …) for the same reason.
    //
    // The two booleans below are the only place "valid" is defined; every
    // projection reads them, and "neither" is every `multiIf`'s else-branch —
    // the labelled $1 peg. `ifNull(…, '') != ''` is the matched-row test that
    // holds under both `join_use_nulls` settings (see the sentinel note in
    // `ohlcv_peg_series`), and the timestamp test is the window.
    let o_ok = format!("(ifNull(bo.om, '') != '' AND bo.orts >= {floor})");
    // ⚠️ The external side is bounded by the BUCKET, not by the imported row.
    //
    // The imported series stops below `USDC_ORACLE_EPOCH_S` — the loader and
    // `promote_statement` both refuse a row at or above it — so a bound on
    // `re.erts` would be a no-op against real data. The leak is on the other
    // side: `toStartOfDay(bo.bkt, 'UTC')` is a DAY-wide net, and on the epoch
    // day itself the import ends at 13:00 while the epoch is 14:00. A bucket at
    // or after 14:00 that day whose own oracle window found nothing therefore
    // reached back to the 13:00 import and published it as `external`, while
    // `price_usd_series_1h` published the $1 peg for the same hour: the two read
    // surfaces disagreed, and the candle path was the one claiming a measurement
    // for an hour the imported series does not cover. It is the same post-epoch
    // mis-attribution `ch_enrich::external_sql` bounds itself against.
    //
    // Bounding `bo.bkt` closes it at every grain and for all time: a bucket
    // wholly below the epoch keeps the day-wide net, the epoch day's 1d bucket
    // (whose `bkt` is 00:00) keeps it and still loses to the oracle rank, and no
    // bucket at or after the epoch can take an import at all.
    let e_ok = format!(
        "(ifNull(re.em, '') != '' AND re.erts >= toStartOfDay(bo.bkt, 'UTC') \
          AND bo.bkt < toDateTime({epoch}))",
        epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S
    );
    let rate = format!("multiIf({o_ok}, bo.orate, {e_ok}, re.erate, toDecimal128(1, 14))");

    let val = if in_xlm {
        format!(
            "toNullable(toString(toDecimal128OrNull(toString( \
             toFloat64({rate}) \
             / nullIf(toFloat64(bo.den), 0)), 14)))"
        )
    } else {
        format!("toNullable(toString({rate}))")
    };
    // The label follows the row that WON, never a hard-coded word: once the
    // rows say `external`, the API says `external` with no further change.
    let meth =
        format!("if(o IS NULL, NULL, toNullable(multiIf({o_ok}, bo.om, {e_ok}, re.em, 'peg')))");

    // Task 0267. The rate's provenance reaches the wire ONLY when an imported
    // row supplied the rate: NULL when the bucket has no price at all, NULL when
    // the oracle won (a poll has no outside series and no observation quality to
    // report — its `reference_asset` is `''` and its `quality` the column
    // DEFAULT), and NULL on the $1 fallback, which consulted nothing. Naming a
    // source for a value nobody imported is the exact mistake the
    // `peg`/`oracle`/`external` split exists to prevent.
    //
    // ⚠️ `nullIf(…, '')`, not `toNullable(…)` (review CR-01): `toNullable('')`
    // is `''`, not NULL, so the DEFAULT would have reached the wire as
    // `"source": ""` on every oracle-priced bucket — every live USDC bucket in
    // production — against the OpenAPI text. The same collapse `meth` gets on
    // the candle path.
    let src = format!("if(o IS NULL OR {o_ok} OR NOT {e_ok}, NULL, nullIf(re.esource, ''))");
    let qual = format!("if(o IS NULL OR {o_ok} OR NOT {e_ok}, NULL, nullIf(re.equality, ''))");
    format!(
        "SELECT {OUTER_ALIASES} FROM ( \
           SELECT \
             formatDateTime(bo.bkt, '%Y-%m-%dT%H:%i:%SZ') AS ts, \
             {val} AS o, \
             o AS h, o AS l, o AS c, o AS vw, \
             '0' AS vb, \
             '0' AS vqu, \
             toUInt64(0) AS tc, \
             {meth} AS meth, \
             if(o IS NULL, NULL, toNullable(toUInt8(1))) AS drv, \
             {src} AS src, \
             {qual} AS qual, \
             bo.bkt AS bkt \
           FROM ( \
             SELECT b.bkt AS bkt, b.bend AS bend, b.k AS k, b.den AS den, \
                    ro.orts AS orts, ro.orate AS orate, ro.om AS om \
             FROM ( SELECT timestamp AS bkt, timestamp + INTERVAL {interval} AS bend, \
                           1 AS k, {denom} AS den \
                    FROM {table} FINAL WHERE {conds} \
                    GROUP BY timestamp \
                    ORDER BY bkt DESC LIMIT {limit} ) AS b \
             ASOF LEFT JOIN ( \
                    SELECT 1 AS ok, timestamp AS orts, usd_rate AS orate, \
                           CAST(method AS String) AS om \
                    FROM usd_rate FINAL \
                    WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
                      AND issuer_address = ? AND contract_address = '' \
                      AND method = 'oracle' ) AS ro \
               ON b.k = ro.ok AND ro.orts < b.bend \
           ) AS bo \
           ASOF LEFT JOIN ( \
                  SELECT 1 AS ek, timestamp AS erts, usd_rate AS erate, \
                         CAST(method AS String) AS em, \
                         reference_asset AS esource, CAST(quality AS String) AS equality \
                  FROM usd_rate FINAL \
                  WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
                    AND issuer_address = ? AND contract_address = '' \
                    AND method = 'external' ) AS re \
             ON bo.k = re.ek AND re.erts < bo.bend \
         ) ORDER BY bkt ASC",
        conds = conds.join(" AND "),
        limit = args.limit,
        interval = args.granularity.interval_sql(),
        denom = if in_xlm {
            // XLM's USD price for the bucket, from the highest-volume source.
            format!("argMaxIf(close_usd, volume_base, close_usd >= {PRECISION_FLOOR})")
        } else {
            "toDecimal128(1, 14)".to_string()
        },
    )
}

/// The outer projection's alias list, shared by BOTH `/ohlcv` query shapes.
///
/// ⚠️ ONE source of truth on purpose. `Candle` derives `clickhouse::Row` and
/// RowBinary is POSITIONAL and carries no types, so a projection that gains or
/// loses a column relative to the struct either errors with `InvalidTagEncoding`
/// or — for lengths 0 and 1 — SILENTLY MIS-FRAMES the rest of the row. Two
/// hand-maintained copies of this list is exactly the drift that produces a
/// plausible wrong row on a public endpoint with nothing failing anywhere.
/// The alias count must equal `Candle`'s field count, and the ORDER must match
/// the struct's field order.
const OUTER_ALIASES: &str = "ts, o, h, l, c, vb, vqu, vw, tc, meth, drv, src, qual";

/// Task 0267's two provenance columns as the candle path emits them: NULL.
///
/// Neither `ohlcv` arm can populate them — the candle TABLES carry no
/// provenance column (task 0268's Issue 9), so `source`/`quality` are non-null
/// only on USDC's own synthesized series. They are still emitted, because
/// positional RowBinary counts columns, not names.
const PROVENANCE_NULL_TAIL: &str =
    "CAST(NULL AS Nullable(String)) AS src, CAST(NULL AS Nullable(String)) AS qual";

pub async fn ohlcv_peg_series(
    ch: &Client,
    args: &OhlcvArgs,
    usdc_id: u32,
    xlm_id: u32,
    usdc_issuer: &str,
    // Denominate in XLM instead of USD — ADR 0011 §6's second degenerate case.
    in_xlm: bool,
) -> Result<Vec<Candle>, clickhouse::error::Error> {
    let table = format!("price_ohlcv_{}", args.granularity.as_str());

    // ⚠️ `asset_id` FIRST, always — including in USD mode, where the id is not
    // otherwise needed. `price_ohlcv_*` is ORDER BY (asset_id, quote_asset_id,
    // source, timestamp), so filtering on `quote_asset_id` alone is NOT a key
    // prefix: no granule pruning applies and the query degenerates into a FINAL
    // scan of every asset's candles in the covered partitions (~24.9 M rows in
    // `price_ohlcv_1d` alone, far more at finer grains). `views.sql:370` flags
    // exactly this shape. Anchoring on the XLM/USDC market restores the prefix.
    let mut conds = vec!["asset_id = ?".to_string(), "quote_asset_id = ?".to_string()];
    if args.start.is_some() {
        conds.push("timestamp >= toDateTime(?)".to_string());
    }
    if args.end.is_some() {
        conds.push("timestamp <= toDateTime(?)".to_string());
    }

    // ⚠️ **Task 0246 replaced an unbounded ASOF with a bucket-scoped equi-join.**
    // This query used to read `ASOF LEFT JOIN … ON b.k = r.k AND r.rts <= b.bkt`
    // — the newest observation at or before the bucket's START, with no
    // staleness bound at all. Two defects followed, and they are independent:
    //
    //   1. A bucket's value was the PREVIOUS bucket's last reading, so this
    //      surface and `price_usd_series` (`views.sql`, task 0168) published
    //      different numbers for the same identity in the same bucket — they
    //      differed by the intraday drift, ~1e-4, on every row.
    //   2. After an oracle outage the last known rate forward-filled
    //      INDEFINITELY, still labelled `method = 'oracle'`. A dead oracle's
    //      final reading served as a measurement for the length of the outage.
    //
    // `init.sql`'s 0167 block names the rule for a bucket-grained consumer:
    // *T is the BUCKET'S END* — the bucket's closing rate. It is also the only
    // resolution under which a daily close equals the last hourly close of that
    // day, i.e. the only one that composes across the six grains.
    //
    // Written, as in `views.sql`, as an `argMax` INSIDE the bucket joined on the
    // bucket rather than as an ASOF, which is the same value and makes the
    // staleness window exactly one bucket width for free: an observation either
    // falls in the bucket or the bucket falls back to the labelled peg. There is
    // no window over which a stale reading can be presented as a measurement.
    //
    // ⚠️ **Only MEASURED rows are accepted.** The old form ranked
    // `oracle > pivot > pivot2 > …` with `argMin(rate, pref)` and rendered a
    // pivot row as `'traded'`. `price_usd_series` and `current.sql`'s tip
    // surface both take measurements or nothing, so this surface was the only
    // one that would have answered from a task 0154 pivot — a second way for the
    // same two surfaces to disagree, on a bucket that HAS observations. If 0154
    // ever wants pivots on a read surface, it must add them to ALL of them in
    // one change, not inherit one silently here.
    //
    // ⚠️ Task 0267 widened "measured" from `= 'oracle'` alone to `oracle` OR
    // `external` — two method-specific subqueries, each an EQUALITY — and that
    // is not a relaxation of the rule above: an `external` row is a reading an
    // OUTSIDE series observed, which is evidence of the same standing as a poll
    // and merely of different provenance. A `pivot` is COMPUTED from another
    // asset's price and is still refused, as is the pre-promotion
    // `external-candidate` staging word, which no read predicate anywhere names
    // (and which an equality, unlike a `LIKE 'external%'`, cannot reach).
    // Provenance is what separates them, not authorship. `views.sql`'s two
    // grains took the identical widening in the same commit — the two surfaces
    // must move together or they disagree about the same rate.
    //
    // ⚠️ An unmatched joined row does NOT yield NULL. By default
    // (`join_use_nulls = 0`, which is what production runs) it yields the
    // column's DEFAULT — so `r.rate` is `0` for every pre-observation bucket,
    // and a NULL test never fires, rendering USDC at $0.00 instead of falling
    // back to the peg. Caught by
    // `ohlcv_usdc_before_any_observation_falls_back_to_a_labelled_peg`.
    //
    // ⚠️ **And the setting cannot simply be asked for.** This query used to end
    // `SETTINGS join_use_nulls = 1`. `prices_reader` runs read-only in
    // production and a read-only user may not modify a setting, so ClickHouse
    // refused the whole query with `Code: 164 … (READONLY)` at
    // `ExceptionBeforeStart` — 40 ms, no rows read, and the endpoint answered
    // `500` for canonical USDC on the deployed API (2026-08-27). Every local
    // test passed throughout, because the local user is not read-only. Do not
    // reintroduce a `SETTINGS` clause here; it is the one query in this service
    // that ever carried one.
    //
    // So the no-match test is a SENTINEL rather than a NULL. `usd_rate.method`
    // is `LowCardinality(String)` (`init.sql:299`), so an unmatched row defaults
    // it to the empty string, and no real row can carry one — every writer sets
    // it and it sits in the table's ORDER BY key. The `ifNull(…, '')` wrapper
    // makes the test hold under `join_use_nulls = 1` too, so the answer no
    // longer depends on a server default in either direction.
    //
    // ⚠️ Each right side is filtered to ONE method before its join. `usd_rate`
    // is ORDER BY (…, timestamp, method) with `method` in the key
    // *deliberately*, so a measured `oracle` and a fallback `peg` — or, since
    // task 0267, an `oracle` and an `external` — can coexist at the same
    // instant and "the consumer chooses" (`init.sql:280`). Filtering each side
    // to a single method in its WHERE clause means no instant holds two rows on
    // either side, so no tie is ever broken by part read order — which it could
    // be when the raw table was joined directly. Which SIDE wins a bucket is
    // decided in the projection, and the note beside the two ASOF joins in
    // [`peg_series_sql`] is where that reasoning lives.
    let sql = peg_series_sql(args, in_xlm, &table, &conds);

    let mut q = ch.query(&sql).bind(xlm_id).bind(usdc_id);
    if let Some(st) = args.start {
        q = q.bind(st);
    }
    if let Some(e) = args.end {
        q = q.bind(e);
    }
    // ⚠️ The issuer binds TWICE: once for the `oracle` subquery and once for
    // the `external` one, in that textual order. `peg_series_sql_binds_the_
    // issuer_once_per_method_subquery` pins the count, because a positional
    // `?` that goes unbound fails at query time, not at compile time.
    q.bind(usdc_issuer)
        .bind(usdc_issuer)
        .fetch_all::<Candle>()
        .await
}

/// The candle-path USD provenance classification, as a whole
/// `multiIf(...) AS meth` fragment.
///
/// Extracted from [`ohlcv`]'s projection for one reason: its arm ORDER is
/// load-bearing and unobservable from the outside. `multiIf` takes the FIRST
/// matching arm, every arm here is valid SQL in any order, and a reordering
/// relabels whole populations on the wire without failing at compile time, at
/// query time or at render time. Pinning the order needs a string a unit test
/// can read, and that needs a function.
///
/// ## The arms, in order, and why each sits where it does
///
/// ⚠️ **A value signature is not provenance.** Until task 0268 this arm list
/// opened with `close_usd = close -> assumed-par`, which reads the OUTCOME and
/// calls it the input. USDC sits at exactly par most days: 174 of the 2049 days
/// in task 0267's imported series close at exactly `1.00000000`, so ~8.5% of
/// the re-enriched population carries a MEASURED rate whose product is
/// bit-identical to an assumed one. Those candles were labelled `assumed-par`
/// while `dto.rs` and the published OpenAPI text promise the opposite in as many
/// words: *"a bucket reading exactly 1.0 under `external` or `oracle` is a
/// measurement that happened to be at par, which is precisely what `assumed-par`
/// is not."* The doc was right and the SQL was wrong.
///
/// The fix is to consult the rate table instead of the stored bytes: a
/// pre-epoch USDC-quoted candle is `external` when an imported rate actually
/// covers its UTC day, whatever the arithmetic came out to. That is the same
/// uncorrelated day-set the enrichment worker resets on
/// (`ch_enrich::external_rate_day_pred`), so the read label names the tier that
/// actually wrote the value.
///
/// 1. `quote_asset_id = usdc AND timestamp < USDC_ORACLE_EPOCH_S AND <the
///    bucket's UTC day carries an imported rate>` -> **`external`**. Task 0267's
///    series priced it, via task 0268's external tier.
///
///    ⚠️ **One residual ambiguity, stated rather than hidden.** Day membership
///    is not bucket membership: the external tier resolves a rate at the BUCKET
///    END within a staleness window, so a bucket on a covered day whose own
///    window found nothing falls to the peg tier and is still reported
///    `external` here. The candle tables carry no provenance column, so no
///    read-side expression can separate those two cases — that is task 0268's
///    Issue 9, and closing it properly means storing the tier on the row.
///    Between the two available errors this is the rarer one: the day-set is
///    exactly what the reset zeroed, and a covered day with an unrefillable
///    bucket also trips the campaign's own `rows_reset ~ rows_enriched` abort.
///
///    ⚠️ **Still sound only because `prices.usd_rate` holds no `oracle` row for
///    canonical USDC before the epoch.** That claim is in-repo prose, NOT a live
///    measurement. The 0268 runbook's Appendix B precondition 3 confirms it on
///    prod before the pass runs, and `assert_no_pre_epoch_oracle_rows` refuses
///    the pass if it is false.
/// 2. `quote_asset_id = usdc AND close_usd = close` -> **`assumed-par`**. No
///    imported rate covers the day, so the peg tier multiplied by a literal
///    $1.00 and the equality is exact integer arithmetic on the stored decimals.
///    The word names the INPUT. Task 0268 retired the older spelling here;
///    `ohlcv_peg_series` keeps it, because on USDC's OWN series it means 0165's
///    "no measured rate was available" and that is still true.
/// 3. `quote_asset_id = usdc AND timestamp >= USDC_ORACLE_EPOCH_S` ->
///    **`oracle`**: at or after the epoch a scaled USDC-quoted candle was priced
///    from a measured Reflector reading.
/// 4. `quote_asset_id = usdc` -> **`''`**. Pre-epoch, not at par, and no
///    imported rate covers the day — a state no tier can produce. It is spelled
///    out rather than folded into `oracle`, because the old bare-USDC arm
///    reported a poll that provably did not exist, and a null is the honest
///    answer to "which input priced this". It also keeps USDC off arm 5 if it is
///    ever tracked as a pivot.
/// 5. `quote_asset_id IN (pivots)` -> **`traded`** (ADR 0011 §4; see [`ohlcv`]).
///    Omitted ENTIRELY when no pivot reference is tracked — an empty `IN ()` is
///    a ClickHouse syntax error, and the label is optional.
/// 6. `''` -> the fallback, which [`ohlcv`]'s `nullIf` turns into a JSON `null`.
///    A `multiIf` with no else is an error, so this arm always closes the list.
///
/// ⚠️ The bare column names below (`close_usd`, `close`, `timestamp`) resolve to
/// TABLE columns only because the projection this is spliced into aliases none
/// of them (`valid`, `rate`, `o_x`, `c_x`, ...). ClickHouse resolves aliases
/// BEFORE columns, so an alias added there with one of these names would
/// silently change what this expression tests — the defect task 0268 shipped in
/// `ch_enrich::reset_sql`.
/// The `Candle.method` labels the candle path can emit, in arm order.
///
/// Extracted because the rendered SQL now carries incidental literals of its own
/// (`'UTC'`, `'credit'`, `'USDC'`, the issuer) once the `external` arm consults
/// `usd_rate`, and a test that scrapes every quoted literal out of the statement
/// would demand the OpenAPI text describe those too. The vocabulary is the
/// contract; the SQL is one emitter of it.
#[cfg(test)]
pub(crate) const CANDLE_METHOD_LABELS: [&str; 4] = ["external", "assumed-par", "oracle", "traded"];

pub(crate) fn usd_method_expr(usdc: u32, pivots: &[u32], granularity: Granularity) -> String {
    let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
    let traded_arm = if pivots.is_empty() {
        String::new()
    } else {
        let ids: Vec<String> = pivots.iter().map(|i| i.to_string()).collect();
        format!("quote_asset_id IN ({}), 'traded', ", ids.join(", "))
    };
    let issuer = prices_clickhouse::USDC_ISSUER;
    let interval = granularity.interval_sql();
    // The imported days, as ONE uncorrelated set built per query (ClickHouse
    // renders this as a single `CreatingSet` node, not per row).
    let imported_days = format!(
        "(SELECT groupArray(DISTINCT toDate(timestamp, 'UTC')) FROM usd_rate FINAL \
           WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
             AND issuer_address = '{issuer}' AND contract_address = '' \
             AND method = 'external' AND usd_rate > 0)"
    );
    // ⚠️ The tier resolves a rate at the BUCKET END within a staleness window of
    // `max(bucket_width, 1 day)`, so the read side must ask about the same span
    // — not about the bucket's first day. A `toDate(timestamp) IN (days)` test
    // reports null for every WEEKLY and MONTHLY USDC candle priced from a
    // mid-period rate, which `ch_enrich::external_rate_day_pred` documents as
    // normal. The range below is that window, rounded out to whole days:
    // one day before the bucket start (the hourly grain's window reaches back a
    // day) through the bucket's end day.
    let covered = format!(
        "arrayExists(d -> d >= toDate(timestamp, 'UTC') - 1 \
                     AND d <= toDate(timestamp + INTERVAL {interval}, 'UTC'), {imported_days})"
    );
    format!(
        "multiIf(quote_asset_id = {usdc} AND timestamp < toDateTime({epoch}) \
           AND {covered}, 'external', \
         quote_asset_id = {usdc} AND timestamp < toDateTime({epoch}) \
           AND close_usd = close, 'assumed-par', \
         quote_asset_id = {usdc} AND timestamp >= toDateTime({epoch}), 'oracle', \
         quote_asset_id = {usdc}, '', \
         {traded_arm}\
         '') AS meth"
    )
}

/// Read merged candles for one asset at the chosen grain, denominated per
/// [ADR 0011].
///
/// Per-source rows are collapsed (`FINAL`) then merged per bucket: `high=max`,
/// `low=min`, volumes + `trade_count` summed, `vwap` volume-weighted, and
/// `open`/`close` from the highest-volume source (`argMax(.., volume_base)`).
/// Ascending by timestamp.
///
/// ## 🔑 In USD mode the conversion happens BEFORE the merge, and the order is
/// load-bearing
///
/// Dropping the quote filter means one bucket can hold candles from several
/// quote legs at once — AUD against XLM and against USDC in the same day. The
/// merge takes `max(high)` across those rows. Convert *after* merging and that
/// `max` compares an XLM-denominated high with a USDC-denominated one: different
/// units, silently, with a plausible-looking number falling out.
///
/// So every row is scaled to USD in the inner SELECT and only then aggregated.
/// ADR 0011 §1 forces this — a denomination whose meaning varies with the data
/// available is the `close_usd = 0` defect class in a new place — but the ADR
/// does not state the ordering, so it is stated here.
///
/// ## Provenance is derived, because the candle tables do not store it
///
/// `close_usd` is a bare `Decimal(38,14)` with no companion `method` column, so
/// there is nothing to propagate. It is reconstructed from the quote leg and the
/// rate signature instead. Measured on prod 2026-08-26 over `price_ohlcv_1d`:
///
/// | quote leg | signature | n | method |
/// |---|---|---|---|
/// | USDC, pre-oracle | `close_usd = close` | 522,321 (100%) | `assumed-par` |
/// | USDC, oracle window | `close_usd = close` | 134,193 | `assumed-par` |
/// | USDC, oracle window | scaled | 121,474 | `oracle` |
/// | XLM / USDT | scaled | 11,038,372 | `traded` |
/// | anything else | `close_usd = 0` | 13,114,668 (100%) | — no USD fields |
///
/// The peg tier multiplies by exactly $1, so `close_usd = close` is an exact
/// integer comparison on the stored decimals — no division, no float error, and
/// no dividing by a near-zero `close`. Pre-oracle USDC came back 100% pegged with
/// zero scaled rows, which is what makes this a classification rather than a
/// guess.
///
/// ⚠️ **The measurement is dated, and its first row is the population task 0268
/// removes.** Those 522,321 pre-oracle USDC-quoted candles are `close x $1.00`
/// with nothing measured behind them, and USDC closed at **0.9681** on
/// 2023-03-11. After 0268's prod pass the ones whose bucket the imported
/// USDC/USD series covers carry a SCALED `close_usd`, so they stop matching arm
/// 1 and classify through [`usd_method_expr`]'s `external` arm instead. Rows the
/// series does not cover keep the par signature and keep saying `assumed-par` —
/// deliberately, since nothing measured them. Re-measure this table after that
/// pass rather than assuming these counts still hold.
///
/// The two words that changed here changed on the WIRE too, in the same commit
/// as the OpenAPI text, so the schema and the response can never disagree about
/// what they mean.
///
/// ⚠️ **`traded` covers the pivot.** ADR 0011 §4 forbids coining a fourth word,
/// and 0165 defines `traded` as a volume-weighted aggregate of candles a venue
/// actually traded — which is exactly what the pivot's reference rate is (the
/// reference asset's own close against USDC). Note this leaves an XLM-quoted
/// candle labelled `traded` resting on the USDC peg one hop back; that
/// dependency is [`0228`], not this function.
///
/// ## The 5,921 rows this deliberately drops
///
/// A candle on an XLM or USDT leg with `close_usd = close` claims its reference
/// asset was worth exactly $1.00000000000000. XLM has never been near a dollar,
/// and canonical Stellar USDT trades at ~$0.13 since its 2022 depeg (task 0172).
/// Measured: 2,139 XLM-quoted and 3,782 USDT-quoted such rows.
///
/// They are excluded from the USD aggregation rather than labelled, because
/// every available label would be a false claim — `assumed-par` asserts a $1
/// assumption that was never applied to that leg, and `external` asserts a
/// measured USDC/USD rate that has nothing to do with it. A bucket left with no valid row still returns, with
/// its price fields absent (§5); it does not vanish. The underlying rows are
/// [`0227`]/[`0182`] territory.
pub async fn ohlcv(ch: &Client, args: OhlcvArgs) -> Result<Vec<Candle>, clickhouse::error::Error> {
    let table = format!("price_ohlcv_{}", args.granularity.as_str());

    let mut conds = vec!["asset_id = ?".to_string()];
    if let Denomination::QuoteLeg(_) = args.denomination {
        conds.push("quote_asset_id = ?".to_string());
    }
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
    let (projection, aggregates) = match args.denomination {
        Denomination::Usd(ref refs) => {
            let usdc = refs.usdc;
            let floor = PRECISION_FLOOR;
            // The whole classification, extracted so its arm ORDER is testable
            // without a ClickHouse — see [`usd_method_expr`].
            let meth_arm = usd_method_expr(usdc, &refs.pivots, args.granularity);
            (
                // Per-row scaling — see the ordering note above. `valid` gates
                // both the arithmetic and the classification, so a row that
                // cannot be priced contributes to volume and trade_count but
                // never to a price or a method.
                format!(
                    "timestamp, volume_base, volume_quote_usd, trade_count, quote_asset_id, \
                     (close >= {floor} AND close_usd >= {floor} \
                       AND (quote_asset_id = {usdc} OR close_usd != close)) AS valid, \
                     toFloat64(close_usd) / nullIf(toFloat64(close), 0) AS rate, \
                     toDecimal128OrNull(toString(toFloat64(open) * rate), 14) AS o_x, \
                     toDecimal128OrNull(toString(toFloat64(high) * rate), 14) AS h_x, \
                     toDecimal128OrNull(toString(toFloat64(low)  * rate), 14) AS l_x, \
                     close_usd AS c_x, \
                     toDecimal128OrNull(toString(toFloat64(vwap) * rate), 14) AS w_x, \
                     {meth_arm}"
                ),
                // `countIf(valid) = 0` is what produces §5's price-less bucket:
                // NULL across every price field, while the volume columns below
                // still aggregate over all rows.
                // `c` is EXACT (`close_usd` as stored) while `h`/`l` are derived
                // through `toFloat64`, so the two are on different scales and can
                // cross — task 0229. `least`/`greatest` pull the derived extremes
                // back over the exact close; see the CLOSE_EXACT note above.
                usd_aggregates(),
            )
        }
        // As stored: no conversion, so nothing is derived and there is no USD
        // rate to attribute. Both provenance fields are NULL rather than
        // guessed — see Denomination::QuoteLeg.
        Denomination::QuoteLeg(_) => (
            "timestamp, open, high, low, close, volume_base, volume_quote_usd, vwap, trade_count"
                .to_string(),
            // ⚠️ `vw` is clamped into `[min(low), max(high)]` here too — task 0229's
            // review, finding 1. This arm applies no rate, so `o`/`h`/`l`/`c` are
            // the stored decimals and cannot cross; the merged vwap still can,
            // because it is a float weighted mean and `(x*v)/v != x`.
            //
            // 🔴 A single-source bucket reads CLEAN and that is a false negative
            // — measured 0 violations in 200,000. With TWO sources at equal
            // prices, the boundary case, the same expression gave **12,017 above
            // `high` and 12,026 below `low` in 200,000 buckets**. The merge is
            // the whole point of this aggregate, so a one-row probe tests the
            // path that does not exist in production.
            //
            // The `isNull` arm preserves the pre-existing zero sentinel: no
            // volume means no weighted mean, and that must stay `0` rather than
            // being clamped up to `low`, which would assert a vwap the bucket
            // does not have.
            //
            // ⚠️ The price columns MUST be Nullable to match `Candle`'s
            // `Option<String>` fields. RowBinary is positional and carries no
            // types (the client does not use WithNamesAndTypes), so the
            // deserializer reads one byte as the Option tag: handed a plain
            // String it reads the LEB128 length instead and either errors
            // (`InvalidTagEncoding`) or, for lengths 0/1, silently mis-frames
            // the rest of the row. Pinned by
            // `ohlcv_xlm_denomination_decodes_rows` — the pre-existing XLM test
            // asserts an EMPTY series, so no row is ever decoded and it cannot
            // catch this.
            quote_leg_aggregates(),
        ),
    };

    // ⚠️ Task 0267's two provenance columns are emitted by BOTH arms even though
    // NEITHER can populate them: the candle tables carry no provenance column
    // (task 0268's Issue 9), so `src`/`qual` are non-null only on USDC's own
    // synthesized series in `ohlcv_peg_series`.
    //
    // They cannot simply be omitted here. `Candle` derives `clickhouse::Row` and
    // RowBinary is POSITIONAL and carries no types — see the QuoteLeg arm's note
    // above: the deserializer reads one byte as the Option tag, and a column
    // count that disagrees with the struct either errors with
    // `InvalidTagEncoding` or, for lengths 0 and 1, SILENTLY MIS-FRAMES the rest
    // of the row. Three aggregate strings and two outer projections have to move
    // together; miss one and the failure is a plausible wrong row on a public
    // endpoint, not a compile error.
    // `ohlcv_and_peg_series_project_the_same_alias_tail` is the guard.
    let sql = format!(
        "SELECT {OUTER_ALIASES} FROM ( \
           SELECT \
             formatDateTime(timestamp, '%Y-%m-%dT%H:%i:%SZ') AS ts, \
             {aggregates} \
           FROM ( SELECT {projection} FROM {table} FINAL WHERE {conds} ) \
           GROUP BY timestamp \
           ORDER BY timestamp DESC \
           LIMIT {limit} \
         ) ORDER BY ts ASC",
        conds = conds.join(" AND "),
        limit = args.limit
    );

    let mut q = ch.query(&sql).bind(args.asset_id);
    if let Denomination::QuoteLeg(quote) = args.denomination {
        q = q.bind(quote);
    }
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

    /// The word this task RETIRES from the candle path, built from characters so
    /// no test, plan or grep-able string in this file carries the quoted literal
    /// and accidentally satisfies its own search.
    fn retired_label() -> String {
        ['\'', 'p', 'e', 'g', '\''].iter().collect()
    }

    /// The par arm names the INPUT (the literal 1.0) rather than the outcome,
    /// and since the round-4 fix it is arm TWO — the measured day-set is tested
    /// first, so a rate that happened to read exactly 1.0 is not reported as an
    /// assumption. The retired word must not survive anywhere in the
    /// candle-path fragment — `ohlcv_peg_series` keeps it, this does not.
    #[test]
    fn usd_method_expr_first_arm_is_the_par_signature_and_retires_the_old_word() {
        let sql = usd_method_expr(2, &[], Granularity::H1);
        assert!(
            sql.contains("close_usd = close"),
            "the exact-equality signature is the par arm's whole condition: {sql}"
        );
        assert!(
            sql.contains("'assumed-par'"),
            "the par arm must name the assumption: {sql}"
        );
        assert!(
            !sql.contains(&retired_label()),
            "the retired candle-path label must not survive: {sql}"
        );
    }

    /// The epoch is what separates an imported rate from a poll, and it appears
    /// on both sides: the `external` arm is bounded above by it and the `oracle`
    /// arm below by it. A USDC-quoted candle stamped before the first measured
    /// oracle row was priced by the IMPORTED series, never by a poll.
    #[test]
    fn usd_method_expr_second_arm_keys_on_the_usdc_oracle_epoch() {
        let sql = usd_method_expr(7, &[], Granularity::H1);
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        assert!(
            sql.contains(&format!(
                "quote_asset_id = 7 AND timestamp < toDateTime({epoch})"
            )),
            "the external arm pins the USDC quote id and the named epoch: {sql}"
        );
        assert!(sql.contains("'external'"), "{sql}");
    }

    /// ⚠️ Arm ORDER is load-bearing and `multiIf` takes the FIRST match. Put the
    /// par signature back above the measured day-set and every bucket whose
    /// imported rate read exactly 1.0 — 174 of 2049 days — relabels
    /// `assumed-par`, reporting a measurement as an assumption. Drop the epoch
    /// bound off the par arm and a post-epoch poll that read 1.0 does the same.
    /// Neither errors — the SQL stays valid and the wire quietly lies. Byte
    /// offsets are the only cheap way to pin it.
    #[test]
    fn usd_method_expr_arm_order_is_external_then_par_then_oracle() {
        let sql = usd_method_expr(2, &[], Granularity::H1);
        let ext = sql.find("'external'").expect("external arm present");
        let par = sql.find("'assumed-par'").expect("par arm present");
        let orc = sql.find("'oracle'").expect("oracle arm present");
        assert!(
            ext < par,
            "the measured day-set must be tested BEFORE the par signature, or a \
             rate that measured exactly 1.0 is reported as an assumption: {sql}"
        );
        assert!(par < orc, "par must be tested before oracle: {sql}");
    }

    /// The `external` arm must consult the rate table, not the stored bytes.
    /// A candle whose measured rate came out at exactly 1.0 is bit-identical to
    /// a pegged one, so any test of `close_usd` alone cannot tell them apart —
    /// 174 of task 0267's 2049 imported days close at exactly 1.00000000.
    #[test]
    fn usd_method_expr_external_arm_reads_provenance_not_the_value() {
        let sql = usd_method_expr(2, &[], Granularity::H1);
        assert!(
            sql.contains("FROM usd_rate FINAL"),
            "the external arm must consult the rate table: {sql}"
        );
        assert!(
            sql.contains("AND method = 'external' AND usd_rate > 0"),
            "only a positive imported rate may claim a day: {sql}"
        );
        assert!(
            sql.contains(&format!(
                "issuer_address = '{}'",
                prices_clickhouse::USDC_ISSUER
            )),
            "the day-set is pinned to canonical USDC: {sql}"
        );
        assert!(
            sql.contains("toDate(timestamp, 'UTC')"),
            "the day-set must name its timezone, not inherit the server's: {sql}"
        );
        // The external arm's condition must not be reachable by value alone.
        let ext = sql.find("'external'").unwrap();
        let par_sig = sql.find("close_usd = close").unwrap();
        assert!(
            ext < par_sig,
            "the value signature must not gate the measured label: {sql}"
        );
    }

    /// The par arm is bounded ABOVE by the epoch, and that bound is not
    /// decoration. `peg_sql` carries no epoch bound, so the peg tier still
    /// writes `close_usd = close` after the epoch whenever the oracle tier
    /// missed a bucket — and a poll that read exactly 1.0 leaves the same bytes.
    /// Without the bound, a post-epoch measurement at par reports `assumed-par`,
    /// which is the pre-epoch defect this round removed, one side over.
    #[test]
    fn usd_method_expr_does_not_call_a_post_epoch_bucket_an_assumption() {
        let sql = usd_method_expr(2, &[], Granularity::H1);
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        assert!(
            sql.contains(&format!(
                "quote_asset_id = 2 AND timestamp < toDateTime({epoch}) \
           AND close_usd = close, 'assumed-par'"
            )),
            "the par arm must be pre-epoch only: {sql}"
        );
    }

    /// ⚠️ The day-set must span the BUCKET, not its first day. The external tier
    /// resolves a rate at the bucket END within `max(bucket_width, 1 day)`, so a
    /// weekly or monthly candle is routinely priced from a mid-period rate. A
    /// test on the bucket's start day alone reports null for every one of them.
    #[test]
    fn usd_method_expr_covers_the_whole_bucket_at_every_granularity() {
        for (g, interval) in [
            (Granularity::H1, "1 HOUR"),
            (Granularity::D1, "1 DAY"),
            (Granularity::W1, "1 WEEK"),
            (Granularity::Mo1, "1 MONTH"),
        ] {
            let sql = usd_method_expr(2, &[], g);
            assert!(
                sql.contains(&format!("toDate(timestamp + INTERVAL {interval}, 'UTC')")),
                "the coverage test must reach the bucket's end day at {interval}: {sql}"
            );
            assert!(
                sql.contains("d >= toDate(timestamp, 'UTC') - 1"),
                "and back one day, the hourly grain's own window: {sql}"
            );
        }
    }

    /// Pre-epoch, not at par, and no imported rate for the day is a state no
    /// tier produces. It reports null rather than `oracle`: the old bare-USDC
    /// arm claimed a poll that provably did not exist before the epoch.
    #[test]
    fn usd_method_expr_reports_null_rather_than_claim_a_pre_epoch_poll() {
        let sql = usd_method_expr(2, &[], Granularity::H1);
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        assert!(
            sql.contains(&format!(
                "quote_asset_id = 2 AND timestamp >= toDateTime({epoch}), 'oracle'"
            )),
            "the oracle arm is bounded BELOW by the epoch: {sql}"
        );
        assert!(
            sql.contains("quote_asset_id = 2, '', "),
            "the unexplained USDC state reports null: {sql}"
        );
    }

    /// An empty `IN ()` is a ClickHouse syntax error, so the traded arm is
    /// omitted whole rather than emitted empty — and the fallback still closes
    /// the `multiIf`, because a `multiIf` without an else is also an error.
    #[test]
    fn usd_method_expr_omits_the_traded_arm_when_no_pivot_is_tracked() {
        let sql = usd_method_expr(2, &[], Granularity::H1);
        assert!(
            !sql.contains("IN ()"),
            "no empty IN list may be emitted: {sql}"
        );
        assert!(
            !sql.contains("quote_asset_id IN ("),
            "the pivot arm is omitted whole when no pivot is tracked: {sql}"
        );
        assert!(!sql.contains("'traded'"), "{sql}");
        assert!(
            sql.contains("'') AS meth"),
            "the fallback arm closes it: {sql}"
        );
    }

    #[test]
    fn usd_method_expr_joins_pivot_ids_when_present() {
        let sql = usd_method_expr(2, &[4, 9], Granularity::H1);
        assert!(
            sql.contains("quote_asset_id IN (4, 9), 'traded'"),
            "pivot ids are joined by ', ': {sql}"
        );
        let ext = sql.find("'external'").unwrap();
        let traded = sql.find("'traded'").unwrap();
        assert!(
            ext < traded,
            "the USDC arms are tested before traded: {sql}"
        );
    }

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
    // ------------------------------------------------------------------
    // Task 0267 — the read-path widening and the two new wire columns.
    //
    // CI has no ClickHouse, so the behavioural proof is `#[ignore]` in
    // tests/ohlcv_it.rs. These pin the SHAPE that test depends on, and one of
    // them guards a failure mode no behavioural test would ever surface as a
    // failure: a mis-framed RowBinary row is a plausible WRONG row, not an
    // error.
    // ------------------------------------------------------------------

    /// The output aliases of an aggregate list, in order.
    ///
    /// A plain identifier after ` AS ` is an output alias; a parenthesised one
    /// (`CAST(NULL AS Nullable(String))`) is a type and is skipped. Crude on
    /// purpose — a real SQL parser here would be a dependency and a second thing
    /// to trust.
    fn aliases_of(sql: &str) -> Vec<&str> {
        sql.split(" AS ")
            .skip(1)
            .filter_map(|tail| {
                let word = tail.split([',', ' ']).next()?;
                (!word.is_empty()
                    && word
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
                .then_some(word)
            })
            .collect()
    }

    fn peg_args() -> OhlcvArgs {
        OhlcvArgs {
            asset_id: 2,
            denomination: Denomination::QuoteLeg(2),
            granularity: Granularity::D1,
            start: None,
            end: None,
            limit: 100,
        }
    }

    fn peg_sql() -> String {
        peg_series_sql(
            &peg_args(),
            false,
            "price_ohlcv_1d",
            &["asset_id = ?".to_string()],
        )
    }

    /// The USDC self-series must read an IMPORTED measurement, not only a
    /// polled one — otherwise task 0267's loaded history is written and never
    /// served, and every pre-2026 bucket keeps publishing a literal 1.0.
    ///
    /// Each method is read through its OWN subquery and its own EQUALITY: an
    /// `IN` list would be fine too, but a `LIKE`/`startsWith` would reach the
    /// `external-candidate` staging rows, which are unverified by definition.
    #[test]
    fn peg_series_sql_admits_both_measured_methods() {
        let sql = peg_sql();
        assert_eq!(
            sql.matches("AND method = 'oracle' )").count(),
            1,
            "exactly one oracle subquery: {sql}"
        );
        assert_eq!(
            sql.matches("AND method = 'external' )").count(),
            1,
            "exactly one external subquery — without it the imported rows are \
             written and never served: {sql}"
        );
        // The pre-promotion staging word must stay unreadable. Staged rows are
        // unverified by definition, and this is the surface that would publish
        // them. Equality predicates only — no prefix idiom.
        assert!(!sql.contains("external-candidate"), "{sql}");
        for idiom in ["LIKE", "startsWith", "external%"] {
            assert!(
                !sql.contains(idiom),
                "`{idiom}` would select the staged rows too: {sql}"
            );
        }
    }

    /// ⚠️ Review WR-05: a valid ORACLE reading wins the WHOLE bucket, the same
    /// rule as `views.sql`'s rank-first `argMax` tuple — not merely a tie at
    /// one instant with the ASOF's recency deciding across instants. The
    /// oracle test comes FIRST in every `multiIf`, and the external branch is
    /// reached only when it fails, so a later import can never outrank a poll.
    #[test]
    fn peg_series_sql_ranks_a_valid_oracle_reading_over_the_whole_bucket() {
        let sql = peg_sql();
        let o_ok = "(ifNull(bo.om, '') != '' AND bo.orts >= bo.bkt)";
        let e_ok = &format!(
            "(ifNull(re.em, '') != '' AND re.erts >= toStartOfDay(bo.bkt, 'UTC') \
          AND bo.bkt < toDateTime({}))",
            prices_clickhouse::USDC_ORACLE_EPOCH_S
        );
        assert!(
            sql.contains(&format!(
                "multiIf({o_ok}, bo.orate, {e_ok}, re.erate, toDecimal128(1, 14))"
            )),
            "the rate must be oracle-first, then external, then the $1 peg: {sql}"
        );
        assert!(
            sql.contains(&format!("multiIf({o_ok}, bo.om, {e_ok}, re.em, 'peg')")),
            "the label must follow the same order and the row that won: {sql}"
        );
        // Two method-specific ASOFs, each bounded by the bucket END — recency
        // decides WITHIN a method only.
        assert!(sql.contains("ON b.k = ro.ok AND ro.orts < b.bend"), "{sql}");
        assert!(
            sql.contains("ON bo.k = re.ek AND re.erts < bo.bend"),
            "{sql}"
        );
        // The old shape — a single subquery ranking inside the instant — is gone.
        assert!(!sql.contains("AS pref"), "{sql}");
        assert!(!sql.contains("argMax(rate, rts)"), "{sql}");
        // The oracle window is the bucket's own start for a grain at least as
        // wide as the poll cadence — unchanged from 0246.
        assert!(sql.contains("bo.orts >= bo.bkt)"), "{sql}");
    }

    /// ⚠️ Review WR-06: an `external` row is DAILY and stamped at the UTC day
    /// start, so it is valid for the WHOLE day at every grain — otherwise, at
    /// `granularity=1h`, the 00:00 bucket of 2023-03-11 reads `0.96812` and
    /// the other twenty-three read the $1 peg, while task 0268's external tier
    /// prices every hourly candle of that day from the same row.
    #[test]
    fn peg_series_sql_admits_an_external_row_for_its_whole_utc_day() {
        for (grain, oracle_floor) in [
            (Granularity::M1, "bo.bend - INTERVAL 300 SECOND"),
            (Granularity::H1, "bo.bkt"),
            (Granularity::D1, "bo.bkt"),
        ] {
            let sql = peg_series_sql(
                &OhlcvArgs {
                    granularity: grain,
                    ..peg_args()
                },
                false,
                "price_ohlcv_x",
                &["asset_id = ?".to_string()],
            );
            assert!(
                sql.contains("re.erts >= toStartOfDay(bo.bkt, 'UTC')"),
                "{grain:?}: the external floor is the DAY start: {sql}"
            );
            assert!(
                sql.contains(&format!("bo.orts >= {oracle_floor})")),
                "{grain:?}: the oracle floor is unchanged from 0246: {sql}"
            );
            assert!(
                !sql.contains("re.erts >= bo.bkt"),
                "{grain:?}: a one-bucket window on the import is the 23-of-24 \
                 `peg` defect: {sql}"
            );
        }
    }

    /// The label comes from the row that WON, not from a hard-coded word. Once
    /// the rows say `external`, the API says `external` with no further change.
    #[test]
    fn peg_series_sql_reports_the_method_the_row_carries() {
        let sql = peg_sql();
        assert!(
            sql.contains("bo.om, ") && sql.contains("re.em, 'peg')"),
            "the non-peg branches must forward the row's own method: {sql}"
        );
        assert!(
            !sql.contains(", 'oracle', "),
            "a hard-coded oracle branch reports every imported rate as a poll: {sql}"
        );
        assert!(
            !sql.contains(", 'external', "),
            "a hard-coded external branch is the same mistake the other way: {sql}"
        );
        // The matched-row sentinel is untouched by this task: an unmatched ASOF
        // yields the column DEFAULT ('' for the method) under join_use_nulls = 0
        // and NULL under = 1, and `ifNull(…, '') != ''` reads both. Four
        // readers — the rate, the label, and the two provenance columns.
        assert_eq!(sql.matches("ifNull(bo.om, '') != ''").count(), 4, "{sql}");
        assert_eq!(sql.matches("ifNull(re.em, '') != ''").count(), 4, "{sql}");
    }

    /// `source` and `quality` are NULL on the peg fallback, on a price-less
    /// bucket, AND on an oracle-priced bucket. A fallback consulted no series
    /// and observed nothing; a poll has no outside series. Naming a source for
    /// either would make an assumption indistinguishable from a measurement —
    /// the exact conflation this whole subsystem exists to prevent.
    ///
    /// 🔴 Review CR-01: the DEFAULT is collapsed with `nullIf(…, '')`, not
    /// `toNullable(…)`. `toNullable('')` is `''`, so the previous shape put
    /// `"source": "", "quality": ""` on the wire for every oracle-priced USDC
    /// bucket — every live bucket in production — against the OpenAPI text,
    /// and the `#[ignore]` epoch-seam test's `Value::Null` assertion could
    /// never have held.
    #[test]
    fn peg_series_sql_nulls_the_provenance_outside_an_imported_rate() {
        let sql = peg_sql();
        let o_ok = "(ifNull(bo.om, '') != '' AND bo.orts >= bo.bkt)";
        let e_ok = &format!(
            "(ifNull(re.em, '') != '' AND re.erts >= toStartOfDay(bo.bkt, 'UTC') \
          AND bo.bkt < toDateTime({}))",
            prices_clickhouse::USDC_ORACLE_EPOCH_S
        );
        for (alias, col) in [("src", "esource"), ("qual", "equality")] {
            let want = format!(
                "if(o IS NULL OR {o_ok} OR NOT {e_ok}, NULL, nullIf(re.{col}, '')) AS {alias},"
            );
            assert!(
                sql.contains(&want),
                "`{alias}` must be NULL unless the IMPORT won, and must collapse \
                 the '' DEFAULT to NULL: {sql}"
            );
        }
        assert!(!sql.contains("toNullable(re.esource)"), "{sql}");
        assert!(!sql.contains("toNullable(re.equality)"), "{sql}");
        assert!(sql.contains("reference_asset AS esource"), "{sql}");
        assert!(sql.contains("CAST(quality AS String) AS equality"), "{sql}");
    }

    /// The issuer is bound positionally and appears once per method subquery,
    /// in the order `oracle` then `external` — exactly the two trailing binds
    /// in `ohlcv_peg_series`. An unbound `?` fails at query time only.
    #[test]
    fn peg_series_sql_binds_the_issuer_once_per_method_subquery() {
        let sql = peg_sql();
        assert_eq!(
            sql.matches("issuer_address = ?").count(),
            2,
            "one issuer bind per method subquery: {sql}"
        );
        let oracle_at = sql.find("AND method = 'oracle' )").unwrap();
        let external_at = sql.find("AND method = 'external' )").unwrap();
        assert!(
            oracle_at < external_at,
            "the oracle subquery (and its bind) precede the external one"
        );
        // The bucket conds come first, so their binds precede both issuers.
        assert!(sql.find("asset_id = ?").unwrap() < oracle_at, "{sql}");
    }

    /// 🔴 Review round 2, CR-02 — every timezone-sensitive expression in the
    /// peg series NAMES its zone.
    ///
    /// `usd_rate.timestamp` and `price_ohlcv_*.timestamp` are bare `DateTime`
    /// (`grep 'DateTime(' init.sql` finds no `'UTC'` anywhere in the schema), so
    /// an unzoned `toStartOfDay` resolves in the SERVER timezone. Nothing in
    /// this repo pins that: `docker-compose.yml` sets no `TZ`, so local and CI
    /// runs are UTC and every other test passes — while `ch-prod-01` is a
    /// Hetzner box whose zone is undocumented. At UTC+2 the external floor for
    /// the 22:00 and 23:00 buckets of every imported day lands ON THE NEXT
    /// day's start, so a row stamped 00:00 UTC fails it and those hours drop
    /// back to `1`/`peg` while task 0268's (zone-pinned) external tier has
    /// already written the measured rate into the very same hours' candles.
    ///
    /// The shape mirrors `ch_enrich::every_timezone_sensitive_expression_pins_utc`
    /// deliberately: this is the read half of the semantic that file enforces on
    /// the write half, and one rule should be spelled one way.
    #[test]
    fn the_peg_series_pins_utc_on_every_timezone_sensitive_expression() {
        for grain in [Granularity::M1, Granularity::H1, Granularity::D1] {
            let sql = peg_series_sql(
                &OhlcvArgs {
                    granularity: grain,
                    ..peg_args()
                },
                false,
                "price_ohlcv_x",
                &["asset_id = ?".to_string()],
            );
            let mut seen = 0usize;
            for f in ["toStartOfDay(", "toDate(", "toStartOfInterval("] {
                for (i, _) in sql.match_indices(f) {
                    let tail = &sql[i..];
                    let close = tail.find(')').unwrap();
                    assert!(
                        tail[..close].ends_with("'UTC'"),
                        "{grain:?}: `{f}` without an explicit zone — the window \
                         slides with the server's UTC offset: {}",
                        &tail[..close + 1]
                    );
                    seen += 1;
                }
            }
            // Not vacuous: the external floor is the only such expression, and
            // it is spelled FOUR times — the rate, the label and the two
            // provenance columns all read it. A rewrite that dropped it would
            // otherwise pass here silently.
            assert_eq!(
                seen, 4,
                "{grain:?}: expected the external day floor, four readers: {sql}"
            );
            assert!(!sql.contains("toStartOfDay(bo.bkt)"), "{grain:?}: {sql}");
        }
    }

    /// 🔴 Review round 2, WR-09 — the two surfaces that serve an imported rate
    /// must SPELL the external predicate the same way, checked without a
    /// ClickHouse.
    ///
    /// They drifted once already: `/ohlcv` gained the day-wide safety net and
    /// `price_usd_series_1h` did not, and nothing failed — the cross-surface
    /// integration test could not see it because its fixture seeded oracle rows
    /// only, and it is `#[ignore]` regardless. This is the cheap half of the
    /// fix: the words themselves, in CI, on every push.
    #[test]
    fn the_views_and_the_peg_series_admit_the_same_external_rows() {
        let views = prices_clickhouse::VIEWS_SQL;
        let sql = peg_sql();

        // Both rate surfaces of views.sql admit exactly the two measured words,
        // spelled identically, and the peg series names the same two.
        assert_eq!(
            views.matches("method IN ('oracle', 'external')").count(),
            2,
            "price_usd_series and price_usd_series_1h must both admit the \
             import, spelled the same way"
        );
        assert!(sql.contains("AND method = 'external' )"), "{sql}");
        assert!(sql.contains("AND method = 'oracle' )"), "{sql}");

        // Neither surface may REACH the pre-promotion staging word, by any
        // idiom — `external-candidate` has `external` as a prefix, so a
        // `LIKE`/`startsWith` form would select unverified rows.
        //
        // ⚠️ Checked as "no prefix idiom", not as "the word is absent": the
        // views' comment block names the staging word in prose, deliberately,
        // to say that nothing reads it. `external_rate.rs`'s
        // `no_shipped_view_reads_the_staging_method` is the test that strips
        // comments and proves the executable text is clean; this one must not
        // duplicate it badly.
        assert!(!sql.contains("external-candidate"), "{sql}");
        for s in [views, sql.as_str()] {
            for idiom in ["LIKE", "startsWith", "external%"] {
                assert!(!s.contains(idiom), "`{idiom}` selects staged rows: {s}");
            }
        }

        // Both surfaces rank oracle FIRST and by rank rather than by recency.
        assert_eq!(
            views
                .matches("(if(method = 'oracle', 1, 0), timestamp)")
                .count(),
            4,
            "two argMax tuples per view, rank-first"
        );
        let o_ok = sql.find("bo.orts >=").unwrap();
        let e_ok = sql.find("re.erts >=").unwrap();
        assert!(
            o_ok < e_ok,
            "the oracle test precedes the external one: {sql}"
        );
    }
    /// 🔴 The mis-framing guard, and the reason it exists rather than a
    /// behavioural test: `Candle` derives `clickhouse::Row`, RowBinary is
    /// POSITIONAL and carries no types, and a projection that disagrees with the
    /// struct either errors with `InvalidTagEncoding` or — for lengths 0 and 1 —
    /// SILENTLY MIS-FRAMES the rest of the row. The failure is a plausible wrong
    /// row on a public endpoint. Two outer projections and three aggregate
    /// strings have to move together; this is what says so.
    ///
    /// The expected tail is built ONCE and every site is compared against it.
    #[test]
    fn every_projection_and_aggregate_ends_with_the_same_provenance_tail() {
        // The outer projection is a single shared constant, so the two query
        // shapes cannot disagree by construction. Pin its content anyway — the
        // constant is only a single source of truth if it is the RIGHT list.
        let aliases: Vec<&str> = OUTER_ALIASES.split(", ").collect();
        assert_eq!(
            aliases.len(),
            13,
            "one alias per `Candle` field, in the struct's order: {OUTER_ALIASES}"
        );
        assert_eq!(&aliases[11..], &["src", "qual"], "{OUTER_ALIASES}");

        assert_eq!(
            PROVENANCE_NULL_TAIL,
            "CAST(NULL AS Nullable(String)) AS src, CAST(NULL AS Nullable(String)) AS qual",
            "both new columns must be explicitly typed Nullable — the \
             deserializer reads one byte as the Option tag"
        );

        // Both `ohlcv` arms end with that exact tail and nothing after it, so a
        // thirteenth column appended to one arm alone fails here.
        //
        // The alias sequence is compared against `OUTER_ALIASES` itself, minus
        // `ts` (which the outer projection adds), so the aggregates and the
        // projection cannot drift apart in COUNT or in ORDER — and order is what
        // matters, because positional RowBinary reads by position, not by name.
        let want: Vec<&str> = OUTER_ALIASES.split(", ").skip(1).collect();
        for (name, agg) in [
            ("Denomination::Usd", usd_aggregates()),
            ("Denomination::QuoteLeg", quote_leg_aggregates()),
        ] {
            assert!(
                agg.ends_with(PROVENANCE_NULL_TAIL),
                "{name}'s aggregate list must END with the provenance tail: {agg}"
            );
            assert_eq!(
                aliases_of(&agg),
                want,
                "{name} must project exactly `Candle`'s fields, in order: {agg}"
            );
        }

        // The peg series populates them for real, so it cannot share the tail —
        // but it must still project the same two aliases last, in the same
        // order, and wrap them in the same outer list.
        let sql = peg_sql();
        assert!(
            sql.starts_with(&format!("SELECT {OUTER_ALIASES} FROM (")),
            "{sql}"
        );
        let src_at = sql.find(" AS src,").expect("peg series projects src");
        let qual_at = sql.find(" AS qual,").expect("peg series projects qual");
        assert!(
            src_at < qual_at,
            "`source` precedes `quality` in `Candle`, so it must precede it here"
        );
    }
}
