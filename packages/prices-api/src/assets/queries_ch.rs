//! ClickHouse query layer for the `/v1/assets` resource.
//!
//! Decimal columns are returned as **strings** (full precision preserved via
//! `toString`), matching the §4.2 string-typed JSON contract and sidestepping
//! Decimal↔Rust mapping. Rows deserialize positionally (RowBinary), so struct
//! field order MUST match the `SELECT` column order.

use clickhouse::Client;

use crate::common::cursor::Cursor;
use crate::identity::AssetIdentifier;

/// One current-price row, all numeric fields as decimal strings.
#[derive(Debug, clickhouse::Row, serde::Deserialize)]
pub struct CurrentPriceRow {
    pub price_usd: String,
    pub vwap_24h: String,
    pub volume_24h_usd: String,
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
    pub updated_at: String,
    /// String form of the sort-column value for this row (cursor payload).
    pub sort_key: String,
}

/// Sortable columns for the listing (validated from the `?sort` param).
#[derive(Debug, Clone, Copy)]
pub enum SortCol {
    Price,
    Volume24h,
    Change24h,
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

    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s.unwrap_or("volume_24h") {
            "price" => Some(SortCol::Price),
            "volume_24h" => Some(SortCol::Volume24h),
            "change_24h" => Some(SortCol::Change24h),
            "code" => Some(SortCol::Code),
            _ => None,
        }
    }
}

/// Sort direction.
#[derive(Debug, Clone, Copy)]
pub enum Order {
    Asc,
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

    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s.unwrap_or("desc") {
            "asc" => Some(Order::Asc),
            "desc" => Some(Order::Desc),
            _ => None,
        }
    }
}

/// `?type` filter.
#[derive(Debug, Clone, Copy)]
pub enum TypeFilter {
    Classic,
    Soroban,
    All,
}

impl TypeFilter {
    pub fn parse(s: Option<&str>) -> Option<Self> {
        match s.unwrap_or("all") {
            "classic" => Some(TypeFilter::Classic),
            "soroban" => Some(TypeFilter::Soroban),
            "all" => Some(TypeFilter::All),
            _ => None,
        }
    }
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
           a.home_domain AS home_domain, \
           toString(c.price_usd) AS price_usd, \
           toString(c.change_24h_pct) AS change_24h_pct, \
           toString(c.change_7d_pct) AS change_7d_pct, \
           toString(c.volume_24h_usd) AS volume_24h_usd, \
           toString(c.vwap_24h) AS vwap_24h, \
           formatDateTime(c.updated_at, '%Y-%m-%dT%H:%i:%SZ') AS updated_at, \
           {sort_key_expr} AS sort_key \
         FROM current_prices AS c FINAL \
         INNER JOIN assets AS a FINAL ON a.asset_id = c.asset_id \
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
           toString(c.vwap_24h) AS vwap_24h, \
           toString(c.volume_24h_usd) AS volume_24h_usd, \
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

/// Fetch the `assets` row for `id` (for the detail endpoint).
pub async fn asset_detail(
    ch: &Client,
    id: &AssetIdentifier,
) -> Result<Option<AssetRow>, clickhouse::error::Error> {
    let (where_sql, binds) = identity_where(id);
    let sql = format!(
        "SELECT a.asset_code, a.issuer_address, a.contract_address, a.home_domain, a.is_active \
         FROM assets AS a FINAL \
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
}
