//! prices ClickHouse — schema + connection layer for the `prices` database.
//!
//! Mirrors BE's `crates/db-clickhouse` layout: `schema/init.sql` is the single
//! source of truth, embedded at compile time and applied by the
//! `prices-clickhouse-init` binary. Self-contained — the backfill / extractor
//! crates own their own row structs and writers; this crate only stands up the
//! schema and hands out a configured client.

use clickhouse::Client;

/// Optional env-var helpers (`env_or` / `env_parse_or`) shared by the worker
/// Lambdas. Companion to `mtls::require_env` (the must-be-set case).
pub mod env;

/// Shared observability setup for the worker Lambdas (`init_tracing`).
pub mod observability;

/// mTLS transport for the remote Hetzner CH endpoint (Caddy:443). Gated behind
/// the `aws-mtls` feature so the plaintext local-dev / init-CLI path does not
/// pull the rustls / hyper-util / reqwest stack. Ported from BE (task 0052).
#[cfg(feature = "aws-mtls")]
pub mod mtls;

/// Table schema embedded at compile time (DATABASE + all `prices.*` tables).
pub const INIT_SQL: &str = include_str!("../schema/init.sql");

/// Production refreshable-MV rollup chain. Applied separately from `INIT_SQL`
/// (needs ClickHouse ≥ 23.12); not part of the default init flow.
pub const ROLLUPS_SQL: &str = include_str!("../schema/rollups.sql");

/// Deterministic full-range coarse-granularity pre-roll for the backfill /
/// sizing measurement (re-aggregates `_1m FINAL` into `_15m … _1M`).
pub const PREROLL_SQL: &str = include_str!("../schema/preroll.sql");

/// Incremental, non-truncating pre-roll bounded to the pre-Soroban SDEX tail
/// `[genesis, activation)` (task 0088).
pub const PREROLL_INCREMENTAL_SQL: &str = include_str!("../schema/preroll-incremental.sql");

/// Incremental pre-roll for the live-era coarse gap, all sources (task 0090 —
/// the six rollup MVs are dropped on production, so nothing else rolls up).
pub const PREROLL_LIVE_GAP_SQL: &str = include_str!("../schema/preroll-live-gap.sql");

/// Incremental pre-roll scoped to the Soroban-era AMM sources corrected by the
/// events-sourced reprice (task 0097).
pub const PREROLL_AMM_REPRICE_SQL: &str = include_str!("../schema/preroll-amm-reprice.sql");

/// Every pre-roll script, for guards that must hold across all of them.
/// Operator-run scripts are embedded here only so the test suite can see them —
/// `apply_sql` is never pointed at the incremental three.
pub const ALL_PREROLL_SQL: [(&str, &str); 4] = [
    ("preroll.sql", PREROLL_SQL),
    ("preroll-incremental.sql", PREROLL_INCREMENTAL_SQL),
    ("preroll-live-gap.sql", PREROLL_LIVE_GAP_SQL),
    ("preroll-amm-reprice.sql", PREROLL_AMM_REPRICE_SQL),
];

/// Refreshable MV maintaining `prices.current_prices` (task 0039 — replaces the
/// price-updater Lambda). Applied separately like [`ROLLUPS_SQL`] (needs
/// ClickHouse ≥ 23.12); not part of the default init flow.
pub const CURRENT_SQL: &str = include_str!("../schema/current.sql");

/// Read-surface views (task 0061 Step 5): `prices.price_usd_series` (USD close
/// per natural-identity asset/bucket) + `prices.usd_reference` (per-bucket USD
/// reference availability). Plain views — applied after the init tables.
///
/// Every statement here is `CREATE OR REPLACE VIEW`, NOT `CREATE … IF NOT
/// EXISTS` (task 0134): the latter does not redefine a view that already exists,
/// so an edit to a body would silently no-op against a provisioned target.
/// Re-applying therefore always re-lands the definitions.
///
/// ⚠️ Consequence: this file needs a privileged applier. `CREATE OR REPLACE
/// VIEW` requires a `DROP VIEW` grant unconditionally, which the scoped
/// production users do not have (and cannot be granted by us — they are
/// XML-managed by BE). Applying it is an operator action; see the header of
/// `schema/views.sql`.
pub const VIEWS_SQL: &str = include_str!("../schema/views.sql");

/// Canonical `backfill_progress` seed (task 0051 / §3.5): the `sdex_archive` and
/// `soroban_amm` streams. Idempotent — only inserts task_names not already
/// present, so a re-run never resets live progress. Applied after the init
/// tables (the `backfill_progress` table must exist first).
pub const SEED_SQL: &str = include_str!("../schema/seed.sql");

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

/// Apply [`SEED_SQL`] to the given client. Idempotent — the guarded `INSERT`
/// only adds canonical `backfill_progress` rows that are not already present,
/// so re-running never clobbers live progress. Requires the `backfill_progress`
/// table to exist (run [`apply_init_sql`] first).
pub async fn apply_seed(client: &Client) -> Result<(), SchemaError> {
    apply_sql(client, SEED_SQL).await
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
        // 1 CREATE DATABASE + 18 CREATE TABLE (assets, asset_metadata, _1m,
        // _15m, _1h, _4h, _1d, _1w, _1M, current_prices, asset_supply,
        // oracle_prices, backfill_sdex_ledgers, backfill_progress,
        // discovery_state, unresolved_pools, pool_registry, ingest_cursor) + 7
        // close_usd ALTERs (one per OHLCV grain) + 1 assets.sac_address ALTER
        // (task 0061) + 2 backfill_progress ALTERs (earliest_data_available
        // [0073 half → 0053] + newest_data_available [0053]) = 29 statements.
        // (+discovery_state task 0054, +asset_supply task 0039, +unresolved_pools
        // + pool_registry task 0053, +asset_metadata task 0067, +ingest_cursor
        // task 0064.)
        let stmts = split_statements(INIT_SQL);
        assert_eq!(stmts.len(), 29, "got {}", stmts.len());
    }

    #[test]
    fn rollups_and_preroll_each_have_six_statements() {
        assert_eq!(split_statements(ROLLUPS_SQL).len(), 6);
        assert_eq!(split_statements(PREROLL_SQL).len(), 6);
    }

    #[test]
    fn current_sql_is_a_drop_then_create_of_one_materialized_view() {
        // Was `stmts.len() == 1` (CREATE only) until task 0072. A refreshable
        // MV's definition is FIXED AT CREATE TIME, so changing the SELECT needs
        // DROP + re-CREATE — an ALTER does not take. The DROP is therefore part
        // of the deploy contract, not a stray statement, and the ORDER is
        // load-bearing: a CREATE-then-DROP file would leave no view at all.
        let stmts = split_statements(CURRENT_SQL);
        assert_eq!(stmts.len(), 2, "got {}", stmts.len());
        assert!(
            stmts[0].contains("DROP VIEW") && stmts[0].contains("prices.mv_current_prices"),
            "first statement must drop the MV, got: {}",
            stmts[0]
        );
        assert!(
            stmts[1].contains("CREATE MATERIALIZED VIEW")
                && stmts[1].contains("prices.mv_current_prices"),
            "second statement must re-create the MV"
        );
    }

    /// The `TO prices.current_prices (...)` column list must name every column
    /// the SELECT projects: a materialised view inserts POSITIONALLY, so an
    /// omitted column silently shifts every value one slot left (0039 review).
    #[test]
    fn current_sql_to_clause_names_all_ten_written_columns() {
        for col in [
            "asset_id",
            "price_usd",
            "price_xlm",
            "change_24h_pct",
            "change_7d_pct",
            "volume_24h_usd",
            "market_cap_usd",
            "vwap_24h",
            "sources",
            "updated_at",
        ] {
            assert!(
                CURRENT_SQL.contains(col),
                "current.sql must write column `{col}`"
            );
        }
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

    /// Task 0134 — no view in `views.sql` may be declared `IF NOT EXISTS`.
    ///
    /// `CREATE VIEW IF NOT EXISTS` does not redefine a view that already exists,
    /// so on a provisioned target (ch-prod-01) an edit to a view body silently
    /// no-ops: the apply reports success and the definition never changes. Task
    /// 0072 hit exactly that on `current_price_usd`. This test exists so a view
    /// added later in the wrong form fails the build instead of shipping the
    /// footgun to prod, where it is invisible.
    ///
    /// Views only. `init.sql` stays `IF NOT EXISTS` — tables must NOT be
    /// recreated — and the refreshable MVs (`current.sql`, `rollups.sql`) cannot
    /// use OR REPLACE at all; they require DROP + re-CREATE.
    #[test]
    fn views_sql_uses_create_or_replace_for_every_view() {
        let stmts = split_statements(VIEWS_SQL);
        assert_eq!(stmts.len(), 6, "guard is vacuous if the file is empty");

        for stmt in &stmts {
            let head: String = stmt.chars().take(80).collect();
            assert!(
                stmt.contains("CREATE OR REPLACE VIEW"),
                "every view must use CREATE OR REPLACE VIEW (task 0134); got: {head}"
            );
            assert!(
                !stmt.contains("IF NOT EXISTS"),
                "CREATE VIEW IF NOT EXISTS silently fails to redefine an existing \
                 view — the edit would never land on ch-prod-01; got: {head}"
            );
        }
    }

    /// `close_usd` is baked by a separate, lagging enrichment pass onto a
    /// non-nullable `Decimal(38,14) DEFAULT 0` column, so an unguarded
    /// `argMax(close_usd, t.timestamp)` hands a coarse bucket a fabricated zero
    /// whenever its newest sub-bucket is not yet enriched — throwing away every
    /// priced sub-bucket underneath it (task 0145, from BE's 0199 report via
    /// 0144). The pre-rolls are where this is most damaging: they run over
    /// historical spans where enrichment is incomplete *by definition*, at
    /// backfill scale, and the rows they zero then age out of the MV
    /// re-aggregation windows where only the 0114 sweep can still reach them.
    ///
    /// Asserted over `split_statements`, which strips comments — so the header
    /// disclosure block in each file cannot make this guard pass or fail.
    #[test]
    fn no_preroll_script_uses_an_unguarded_argmax_on_close_usd() {
        for (name, sql) in ALL_PREROLL_SQL {
            let stmts = split_statements(sql);
            assert!(
                !stmts.is_empty(),
                "{name}: guard is vacuous if the file yields no statements"
            );

            let mut guarded = 0usize;
            for stmt in &stmts {
                if let Some(offset) = stmt.find("argMax(close_usd") {
                    let head: String = stmt[offset..].chars().take(90).collect();
                    panic!(
                        "{name}: unguarded argMax on close_usd — use \
                         argMaxIf(close_usd, t.timestamp, close_usd > 0) so the bucket \
                         carries its latest *priced* close instead of inheriting the \
                         un-enriched sentinel 0 (task 0145); got: {head}"
                    );
                }
                guarded += stmt
                    .matches("argMaxIf(close_usd, t.timestamp, close_usd > 0)")
                    .count();
            }

            // Non-vacuity: the file must still be projecting close_usd at all.
            // Without this, deleting every close_usd projection would "pass".
            assert!(
                guarded > 0,
                "{name}: no guarded close_usd projection found — either the file \
                 stopped projecting close_usd, or the guard expression was reworded \
                 and this test has gone blind"
            );
        }
    }

    /// The 121 sites are the whole point: 6 + 14 + 6 + 95, matching scope
    /// correction C1 in task 0144. A count regression here means a pre-roll
    /// projection was added without the guard, or one was silently dropped.
    #[test]
    fn preroll_guarded_close_usd_site_counts_match_the_0144_audit() {
        let expected = [
            ("preroll.sql", 6usize),
            ("preroll-incremental.sql", 14),
            ("preroll-live-gap.sql", 6),
            ("preroll-amm-reprice.sql", 95),
        ];

        let mut total = 0usize;
        for ((name, sql), (expected_name, want)) in ALL_PREROLL_SQL.iter().zip(expected) {
            assert_eq!(*name, expected_name, "ALL_PREROLL_SQL order changed");
            // Count over statements, not raw text: each file's header disclosure
            // block quotes the guard expression verbatim, which would otherwise
            // inflate every count by one.
            let got: usize = split_statements(sql)
                .iter()
                .map(|s| {
                    s.matches("argMaxIf(close_usd, t.timestamp, close_usd > 0)")
                        .count()
                })
                .sum();
            assert_eq!(got, want, "{name}: guarded close_usd site count");
            total += got;
        }
        assert_eq!(total, 121, "total guarded pre-roll sites (task 0144 C1)");
    }

    /// The live-spot view must forward every column `mv_current_prices` writes
    /// (task 0072). BE consumes this view IN-CLUSTER — named views, no HTTP —
    /// so a column the view omits is unreachable to that consumer no matter how
    /// well the MV populates it.
    #[test]
    fn views_sql_current_price_usd_forwards_every_current_prices_column() {
        let stmt = split_statements(VIEWS_SQL)
            .into_iter()
            .find(|s| s.contains("prices.current_price_usd"))
            .expect("current_price_usd view statement");

        for col in [
            "price_usd",
            "price_xlm",
            "change_24h_pct",
            "change_7d_pct",
            "volume_24h_usd",
            "market_cap_usd",
            "vwap_24h",
            "sources",
            "updated_at",
        ] {
            assert!(
                stmt.contains(&format!("c.{col}")),
                "current_price_usd must forward `{col}` from current_prices"
            );
        }

        // `CREATE VIEW IF NOT EXISTS` does NOT redefine a view that already
        // exists — the apply silently no-ops and the new columns never land on
        // a target that already has the old definition. A plain view supports
        // atomic OR REPLACE (the refreshable MV in current.sql cannot, hence
        // its DROP + re-CREATE).
        assert!(
            stmt.contains("CREATE OR REPLACE VIEW"),
            "current_price_usd must REPLACE rather than IF NOT EXISTS"
        );
    }
}
