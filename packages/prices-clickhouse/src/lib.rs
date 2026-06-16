//! prices ClickHouse — schema + connection layer for the `prices` database.
//!
//! Mirrors BE's `crates/db-clickhouse` layout: `schema/init.sql` is the single
//! source of truth, embedded at compile time and applied by the
//! `prices-clickhouse-init` binary. Self-contained — the backfill / extractor
//! crates own their own row structs and writers; this crate only stands up the
//! schema and hands out a configured client.

use clickhouse::Client;

/// Table schema embedded at compile time (DATABASE + all `prices.*` tables).
pub const INIT_SQL: &str = include_str!("../schema/init.sql");

/// Production refreshable-MV rollup chain. Applied separately from `INIT_SQL`
/// (needs ClickHouse ≥ 23.12); not part of the default init flow.
pub const ROLLUPS_SQL: &str = include_str!("../schema/rollups.sql");

/// Deterministic full-range coarse-granularity pre-roll for the backfill /
/// sizing measurement (re-aggregates `_1m FINAL` into `_15m … _1M`).
pub const PREROLL_SQL: &str = include_str!("../schema/preroll.sql");

/// Read-surface views (task 0061 Step 5): `prices.price_usd_series` (USD close
/// per natural-identity asset/bucket) + `prices.usd_reference` (per-bucket USD
/// reference availability). Plain views — applied after the init tables.
pub const VIEWS_SQL: &str = include_str!("../schema/views.sql");

/// Default ClickHouse HTTP endpoint when `CLICKHOUSE_URL` is not set.
pub const DEFAULT_URL: &str = "http://localhost:8123";

/// Default ClickHouse user when `CLICKHOUSE_USER` is not set.
pub const DEFAULT_USER: &str = "default";

/// The `prices` logical store. Every table in `schema/init.sql` lives here.
pub const PROD_DATABASE: &str = "prices";

/// Canonical mainnet issuer of USDC (Circle). **Load-bearing join key** for the
/// USD-close path: the backfill interns the USDC identity under this issuer, the
/// enrichment peg tier and `resolve_reference_ids` match `prices.assets` on it,
/// and `schema/views.sql` embeds the same literal. Single Rust source of truth —
/// re-exported by `sdex-backfill` and `enrichment-worker` so the address can
/// never drift between the writer and the reader. (The `views.sql` copy is a SQL
/// literal that cannot reference a Rust const; keep it in sync with this value.)
pub const USDC_ISSUER: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

/// Canonical mainnet issuer of USDT. Companion to [`USDC_ISSUER`]; same
/// single-source-of-truth contract.
pub const USDT_ISSUER: &str = "GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V";

/// ClickHouse client configuration, sourced from environment with local-dev
/// defaults.
#[derive(Debug, Clone)]
pub struct Config {
    pub url: String,
    pub user: String,
    pub password: String,
    pub database: String,
}

impl Config {
    /// Read from `CLICKHOUSE_URL`, `CLICKHOUSE_USER`, `CLICKHOUSE_PASSWORD`,
    /// `CLICKHOUSE_DATABASE`. Each falls back to a `DEFAULT_*` constant;
    /// database defaults to [`PROD_DATABASE`] (`prices`).
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| DEFAULT_URL.to_string()),
            user: std::env::var("CLICKHOUSE_USER").unwrap_or_else(|_| DEFAULT_USER.to_string()),
            password: std::env::var("CLICKHOUSE_PASSWORD").unwrap_or_default(),
            database: std::env::var("CLICKHOUSE_DATABASE")
                .unwrap_or_else(|_| PROD_DATABASE.to_string()),
        }
    }
}

/// Build a `clickhouse::Client` from a `Config`. Clients are cheap to clone —
/// clone instead of rebuilding so the hyper connection pool is reused.
///
/// Note: `init.sql` issues `CREATE DATABASE IF NOT EXISTS prices` and fully
/// qualifies every object as `prices.*`, so applying the schema does not depend
/// on `database` already existing.
pub fn client(cfg: &Config) -> Client {
    Client::default()
        .with_url(&cfg.url)
        .with_user(&cfg.user)
        .with_password(&cfg.password)
        .with_database(&cfg.database)
}

/// Errors raised while applying schema SQL.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("clickhouse query failed: {0}")]
    Query(#[from] clickhouse::error::Error),
}

/// Apply [`INIT_SQL`] to the given client. Idempotent — every statement is a
/// `CREATE … IF NOT EXISTS`.
pub async fn apply_init_sql(client: &Client) -> Result<(), SchemaError> {
    apply_sql(client, INIT_SQL).await
}

/// Apply an arbitrary multi-statement SQL string (used for `ROLLUPS_SQL` /
/// `PREROLL_SQL`). The HTTP query endpoint takes one statement per request, so
/// we split on `;` and submit each individually.
pub async fn apply_sql(client: &Client, sql: &str) -> Result<(), SchemaError> {
    for stmt in split_statements(sql) {
        client.query(&stmt).execute().await?;
    }
    Ok(())
}

/// Split a multi-statement SQL string into individual statements. Strips `-- …`
/// line comments and empty statements. Does not handle quoted `;` or block
/// comments — keep the schema files free of both.
fn split_statements(sql: &str) -> Vec<String> {
    let stripped: String = sql
        .lines()
        .map(|line| match line.find("--") {
            Some(idx) => &line[..idx],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n");

    stripped
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_statements_drops_line_comments_and_empty_chunks() {
        let sql = "-- top\n\
                   CREATE TABLE a (x Int64) ENGINE = MergeTree ORDER BY x;\n\
                   -- mid\n\
                   CREATE TABLE b (y Int64) ENGINE = MergeTree ORDER BY y;\n\
                   ;\n";
        let stmts = split_statements(sql);
        assert_eq!(stmts.len(), 2);
        assert!(stmts[0].starts_with("CREATE TABLE a"));
    }

    #[test]
    fn init_sql_parses_into_statements() {
        // 1 CREATE DATABASE + 12 CREATE TABLE (assets, _1m, _15m, _1h, _4h,
        // _1d, _1w, _1M, current_prices, oracle_prices, backfill_sdex_ledgers,
        // backfill_progress) + 7 close_usd ALTERs (one per OHLCV grain) + 1
        // assets.sac_address ALTER (task 0061) = 21 statements.
        let stmts = split_statements(INIT_SQL);
        assert_eq!(stmts.len(), 21, "got {}", stmts.len());
    }

    #[test]
    fn rollups_and_preroll_each_have_six_statements() {
        assert_eq!(split_statements(ROLLUPS_SQL).len(), 6);
        assert_eq!(split_statements(PREROLL_SQL).len(), 6);
    }

    #[test]
    fn views_sql_has_six_create_view_statements() {
        // series + reference at 1d and 1h, the SAC read-seam resolver, and the
        // live-spot view.
        let stmts = split_statements(VIEWS_SQL);
        assert_eq!(stmts.len(), 6, "got {}", stmts.len());
        for v in [
            "prices.usd_reference AS",
            "prices.price_usd_series AS",
            "prices.usd_reference_1h",
            "prices.price_usd_series_1h",
            "prices.identity_by_contract",
            "prices.current_price_usd",
        ] {
            assert!(stmts.iter().any(|s| s.contains(v)), "missing {v}");
        }
    }
}
