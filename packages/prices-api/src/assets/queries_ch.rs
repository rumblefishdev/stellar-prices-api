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
    // ⚠️ **Only `method = 'oracle'` is accepted now.** The old form ranked
    // `oracle > pivot > pivot2 > …` with `argMin(rate, pref)` and rendered a
    // pivot row as `'traded'`. `price_usd_series` and `current.sql`'s tip
    // surface both take measurements or nothing, so this surface was the only
    // one that would have answered from a task 0154 pivot — a second way for the
    // same two surfaces to disagree, on a bucket that HAS observations. Today it
    // is a no-op: nothing writes a non-`oracle` row for canonical USDC. If 0154
    // ever wants pivots on a read surface, it must add them to ALL of them in
    // one change, not inherit one silently here.
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
    // ⚠️ The right side is collapsed to ONE row per BUCKET before the join.
    // `usd_rate` is ORDER BY (…, timestamp, method) with `method` in the key
    // *deliberately*, so a measured `oracle` and a fallback `peg` can coexist at
    // the same instant and "the consumer chooses" (`init.sql:280`). This
    // consumer chooses measured-or-nothing in the WHERE clause, so the tie
    // cannot be broken by part read order — which it could when the raw table
    // was joined directly.
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
    let floor = if args.granularity.seconds() >= ORACLE_POLL_FLOOR_S {
        "b.bkt".to_string()
    } else {
        format!("b.bend - INTERVAL {ORACLE_POLL_FLOOR_S} SECOND")
    };
    // The no-match sentinel, in one place because three expressions below must
    // agree on what "no usable observation for this bucket" means. Two ways to
    // fail: nothing matched at all, or what matched is older than the window.
    //
    // ⚠️ The second test is not redundant under `join_use_nulls = 0`, it is the
    // belt: an unmatched ASOF yields the DEFAULT, and for a DateTime that is
    // `1970-01-01` — which fails the floor too. Under `join_use_nulls = 1` the
    // first test carries it, since `NULL < x` is NULL rather than true.
    let no_rate = format!("(ifNull(r.meth, '') = '' OR r.rts < {floor})");

    let val = if in_xlm {
        format!(
            "toNullable(toString(toDecimal128OrNull(toString( \
             toFloat64(if({no_rate}, toDecimal128(1, 14), r.rate)) \
             / nullIf(toFloat64(b.den), 0)), 14)))"
        )
    } else {
        format!("toNullable(toString(if({no_rate}, toDecimal128(1, 14), r.rate)))")
    };
    let sql = format!(
        "SELECT ts, o, h, l, c, vb, vqu, vw, tc, meth, drv FROM ( \
           SELECT \
             formatDateTime(b.bkt, '%Y-%m-%dT%H:%i:%SZ') AS ts, \
             {val} AS o, \
             o AS h, o AS l, o AS c, o AS vw, \
             '0' AS vb, \
             '0' AS vqu, \
             toUInt64(0) AS tc, \
             if(o IS NULL, NULL, \
                toNullable(if({no_rate}, 'peg', 'oracle'))) AS meth, \
             if(o IS NULL, NULL, toNullable(toUInt8(1))) AS drv, \
             b.bkt AS bkt \
           FROM ( SELECT timestamp AS bkt, timestamp + INTERVAL {interval} AS bend, \
                         1 AS k, {denom} AS den \
                  FROM {table} FINAL WHERE {conds} \
                  GROUP BY timestamp \
                  ORDER BY bkt DESC LIMIT {limit} ) AS b \
           ASOF LEFT JOIN ( \
                  SELECT 1 AS k, rts, argMax(rate, rts) AS rate, \
                         CAST(argMax(m, rts) AS String) AS meth \
                  FROM ( SELECT timestamp AS rts, usd_rate AS rate, method AS m \
                         FROM usd_rate FINAL \
                         WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
                           AND issuer_address = ? AND contract_address = '' \
                           AND method = 'oracle' ) \
                  GROUP BY rts ) AS r \
             ON b.k = r.k AND r.rts < b.bend \
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
    );

    let mut q = ch.query(&sql).bind(xlm_id).bind(usdc_id);
    if let Some(st) = args.start {
        q = q.bind(st);
    }
    if let Some(e) = args.end {
        q = q.bind(e);
    }
    q.bind(usdc_issuer).fetch_all::<Candle>().await
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
/// | USDC, pre-oracle | `close_usd = close` | 522,321 (100%) | `peg` |
/// | USDC, oracle window | `close_usd = close` | 134,193 | `peg` |
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
/// every available label would be a false claim — `peg` asserts a peg that does
/// not exist on that leg. A bucket left with no valid row still returns, with
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
            // Omitted entirely when neither pivot reference is tracked — an
            // empty `IN ()` is a syntax error, and the label is optional.
            let traded_arm = if refs.pivots.is_empty() {
                String::new()
            } else {
                let ids: Vec<String> = refs.pivots.iter().map(|i| i.to_string()).collect();
                format!("quote_asset_id IN ({}), 'traded', ", ids.join(", "))
            };
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
                     multiIf(quote_asset_id = {usdc} AND close_usd = close, 'peg', \
                             quote_asset_id = {usdc}, 'oracle', \
                             {traded_arm}\
                             '') AS meth"
                ),
                // `countIf(valid) = 0` is what produces §5's price-less bucket:
                // NULL across every price field, while the volume columns below
                // still aggregate over all rows.
                // `c` is EXACT (`close_usd` as stored) while `h`/`l` are derived
                // through `toFloat64`, so the two are on different scales and can
                // cross — task 0229. `least`/`greatest` pull the derived extremes
                // back over the exact close; see the CLOSE_EXACT note above.
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
                 if(countIf(valid) = 0, NULL, toUInt8(1)) AS drv"
                ),
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
             CAST(NULL AS Nullable(UInt8)) AS drv"
                .to_string(),
        ),
    };

    let sql = format!(
        "SELECT ts, o, h, l, c, vb, vqu, vw, tc, meth, drv FROM ( \
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
