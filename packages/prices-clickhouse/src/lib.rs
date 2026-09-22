//! prices ClickHouse — schema + connection layer for the `prices` database.
//!
//! Mirrors BE's `crates/db-clickhouse` layout: `schema/init.sql` is the single
//! source of truth, embedded at compile time and applied by the
//! `prices-clickhouse-init` binary. Self-contained — the backfill / extractor
//! crates own their own row structs and writers; this crate only stands up the
//! schema and hands out a configured client.

use clickhouse::{Client, Compression};

/// Optional env-var helpers (`env_or` / `env_parse_or`) shared by the worker
/// Lambdas. Companion to `mtls::require_env` (the must-be-set case).
pub mod env;

/// Shared observability setup for the worker Lambdas (`init_tracing`).
pub mod observability;

/// Read-only drift detection between `schema/rollups.sql` and the live
/// refreshable-MV definitions on a target (task 0142). The rollup MVs are
/// `IF NOT EXISTS`, so an edit to one silently no-ops against a provisioned
/// cluster; this makes that divergence visible without touching any object.
pub mod drift;

/// The single source of the coarse rollup SQL (task 0286 / ADR 0287 §2–§5):
/// the six refreshable MVs of `schema/rollups.sql` and the two maintained
/// pre-roll scripts are all rendered from one generator, so a tier cannot be
/// pf-gated in one file and ungated in another. Unit tests below assert each
/// shipped file equals its rendering, whitespace-normalised.
pub mod rollup_sql;

/// mTLS transport for the remote Hetzner CH endpoint (Caddy:443). Gated behind
/// the `aws-mtls` feature so the plaintext local-dev / init-CLI path does not
/// pull the rustls / hyper-util / reqwest stack. Ported from BE (task 0052).
#[cfg(feature = "aws-mtls")]
pub mod mtls;

/// Table schema embedded at compile time (DATABASE + all `prices.*` tables).
pub const INIT_SQL: &str = include_str!("../schema/init.sql");

/// The canonical candle column list, in DDL order, for every `price_ohlcv_*`
/// table (task 0286 / ADR 0287). The first fifteen are the pre-0286 shape; the
/// last three are the price-forming aggregates ADR 0287 §1 introduces.
///
/// This exists because the ingest writer is **name-routed**, not positional:
/// `clickhouse` 0.13 emits `INSERT INTO t(<struct field names>) FORMAT
/// RowBinary`, so a candle column the row struct omits silently takes its
/// column DEFAULT instead of erroring. On the pf columns that failure mode is
/// invisible and wrong — `pf_trade_count DEFAULT trade_count` would report a
/// dust-only minute as fully price-forming. So the DDL here and the field names
/// of `OhlcvRow` in `packages/prices-ingest-core/src/writer.rs` are both pinned
/// to this list by unit tests; change one and the other fails.
pub const CANDLE_COLUMNS: [&str; 18] = [
    "timestamp",
    "asset_id",
    "quote_asset_id",
    "source",
    "open",
    "high",
    "low",
    "close",
    "volume_base",
    "volume_quote",
    "volume_quote_usd",
    "close_usd",
    "vwap",
    "trade_count",
    "version",
    "pf_trade_count",
    "pf_volume",
    "pf_price_volume",
];

/// The smallest price this system treats as a price, as a decimal literal
/// (task 0286, review WR-03).
///
/// Candle price columns are `Decimal(38, 14)`, so one tick is `1e-14` and this
/// floor is 100 of them — a value at the threshold still carries ~2 significant
/// digits. Below it, a stored price is quantisation noise: a row measured on
/// prod carries `close = 5e-14` beside `close_usd = 4e-14`, five ticks over
/// four, whose ratio (1.25) looks perfectly ordinary. No check on a derived
/// value can reject that; the inputs are what is wrong.
///
/// ⚠️ The exact figure is a judgement — the measurement establishes that a floor
/// is needed and roughly where the noise lives, not that 100 ticks is the
/// uniquely right line. What is NOT a judgement is that it must be ONE line:
/// the rollups used to gate coarse prices at `close > 0` while `/ohlcv` refused
/// anything under this floor, and 24 `1d` rows on the verification database
/// published `low = 9e-14` beside a floor-clearing close as a result.
///
/// Lives here because `prices-clickhouse` is the one crate the ingest
/// (`prices-ingest-core`), the read path (`prices-api`) and the enrichment
/// worker all depend on.
pub const PRICE_FLOOR_LITERAL: &str = "0.000000000001";

/// [`PRICE_FLOOR_LITERAL`] as the SQL expression every gate compares against.
///
/// The explicit `toDecimal128(…, 14)` is not decoration: compared against a
/// `Decimal(38, 14)` column, a bare float literal would make ClickHouse pick a
/// float comparison and re-introduce the rounding the floor exists to exclude.
pub const PRICE_FLOOR_SQL: &str = "toDecimal128('0.000000000001', 14)";

/// Every `price_ohlcv_*` grain suffix, in rollup order. The `CREATE … AS` copies
/// do NOT inherit a post-hoc base-table `ALTER`, so every migration in
/// `init.sql` has to be applied per table — this is the list it iterates, and
/// the list the DDL-contract tests iterate.
pub const CANDLE_GRAINS: [&str; 7] = ["1m", "15m", "1h", "4h", "1d", "1w", "1M"];

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

/// The first instant `prices.usd_rate` holds a **measured** `oracle` row for
/// canonical USDC on prod: **2026-03-11 14:00:00 UTC**.
///
/// Load-bearing boundary, and a single Rust source of truth in the same spirit
/// as [`USDC_ISSUER`] — both `prices-api` and `enrichment-worker` already depend
/// on this crate, so the two consumers read ONE value and cannot drift:
///
/// - **`prices-api` (read side)** — `/ohlcv` derives a candle's provenance from
///   its rate signature rather than a stored column, so a scaled USDC-quoted
///   candle stamped *before* this instant was priced by the imported series
///   (task 0267's `method = 'external'` rows) and one stamped *after* it was
///   priced by a polled Reflector reading. The label arm keys on this constant.
/// - **`enrichment-worker` (write side)** — the default upper bound of task
///   0268's USD reset (`--reset-not-after`). Scoping the reset below this
///   instant is what makes the oracle-shadow guard's premise false there.
///
/// ⚠️ The read side's soundness rests on prod holding **no** `oracle` row for
/// USDC before this instant. That is in-repo prose (`queries_ch.rs`,
/// `views.sql`), not a live measurement; the 0268 runbook's Appendix B
/// precondition 3 is the check that confirms it before the pass runs.
pub const USDC_ORACLE_EPOCH_S: u32 = 1_773_237_600;

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

/// Bound how long a **single statement** may run on `client`, in seconds
/// (ClickHouse's `max_execution_time`), returning the bounded client.
///
/// Task 0215 half 2. The value is in the FAILURE MODE, not in the ceiling.
/// ClickHouse enforces this itself and — `timeout_overflow_mode` being `throw`
/// on production — raises a real `TIMEOUT_EXCEEDED` exception carrying an error
/// code, which the caller logs. Without it a statement that outruns its caller
/// produces something indistinguishable from a network blip: during the 0215
/// outage the client saw an empty body, ClickHouse completed the statement
/// anyway, the rows landed, and every signal read healthy for 26 days.
///
/// **It must live on the client, not in a settings profile.** The scheduled
/// Lambdas and the operator CLIs share the single `prices_writer` user, whose
/// `prices_write_ddl` profile carries no execution bound and does not inherit
/// `default`. There is no server-side place to give those callers different
/// ceilings, and they need different ones — a CLI legitimately runs statements
/// far longer than a Lambda ever should.
///
/// `secs == 0` is ClickHouse's spelling of *unlimited*, so it sets no option at
/// all and [`execution_bound`] reports it as `None`; a caller that offers this
/// as a knob should say so in its logs rather than let the unbounded state
/// return quietly.
pub fn with_execution_bound(client: Client, secs: u64) -> Client {
    match execution_bound(secs) {
        Some(secs) => client.with_option("max_execution_time", secs.to_string()),
        None => client,
    }
}

/// Configure `client` so a failed statement reaches the caller carrying
/// ClickHouse's own message (task 0281).
///
/// ⚠️ **Not a performance setting.** With the crate's default
/// `Compression::Lz4`, EVERY ClickHouse error arrives as `BadResponse("")` —
/// an empty string. Not only timeouts: unknown table, syntax error, quota
/// exceeded and disk-full all lose their code and message, becoming
/// indistinguishable from a network blip. That is the signature that hid task
/// 0215's outage for 26 days.
///
/// The mechanism is a fallback that fails to fire. `collect_bad_response`
/// LZ4-decodes the error body and falls back to the raw bytes on failure:
///
/// ```text
/// let bytes = collect_bytes(stream).await.unwrap_or(raw_bytes);
/// ```
///
/// Straight to ClickHouse the decode fails, the fallback fires, the message
/// survives. Through a proxy the chunk reframing makes the decode *succeed*
/// with zero bytes, so `unwrap_or` never runs. Every production client reaches
/// ClickHouse through Caddy.
///
/// Every client built in this crate must go through here, so the guard cannot
/// be lost by someone constructing a client a different way.
pub fn with_readable_errors(client: Client) -> Client {
    client.with_compression(Compression::None)
}

/// The effective per-statement bound for a configured value: `None` when there
/// is none.
///
/// Split out from [`with_execution_bound`] because `0` is the trap. It reads
/// like "no delay" and means "no limit" — a mistyped or deliberately-zeroed
/// knob restores exactly the unbounded state this guard exists to end, and does
/// it silently. Callers branch on this to log which of the two they got.
pub fn execution_bound(secs: u64) -> Option<u64> {
    (secs > 0).then_some(secs)
}

/// Errors raised while applying schema SQL.
#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("clickhouse query failed: {0}")]
    Query(#[from] clickhouse::error::Error),

    /// A `CREATE MATERIALIZED VIEW` rendering could not be fingerprinted (task
    /// 0142). Raised rather than skipped: a statement the drift check cannot
    /// parse must not drop silently out of the report, because a shorter report
    /// reads exactly like a clean one.
    #[error("could not parse a materialized-view definition: {rendering}")]
    UnparsableDdl { rendering: String },
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
pub(crate) fn split_statements(sql: &str) -> Vec<String> {
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
    use extractors_core::Venue;

    /// Task 0215: `max_execution_time = 0` is ClickHouse's spelling of
    /// UNLIMITED, not "no delay". A knob zeroed by a typo, or by an operator
    /// who read it as a disable, silently restores the unbounded state the
    /// bound exists to end — and the 0215 outage is the record of how long an
    /// unbounded statement can fail while every signal reads healthy.
    ///
    /// Asserted on [`execution_bound`] rather than the client, because
    /// `clickhouse::Client` exposes no way to read an option back: a test
    /// against the client could only re-state what the call passed in.
    #[test]
    fn a_zero_execution_bound_is_unlimited_not_instant() {
        assert_eq!(execution_bound(0), None, "0 means unlimited to ClickHouse");
        assert_eq!(execution_bound(1), Some(1));
        assert_eq!(execution_bound(120), Some(120));

        // And the client builder agrees — a zero sets no option, so it cannot
        // send `max_execution_time=0` and pin the server to unlimited either.
        let base = Client::default();
        let _bounded = with_execution_bound(base.clone(), 120);
        let _unbounded = with_execution_bound(base, 0);
    }

    /// Days from 1970-01-01 to a civil (y, m, d), Howard Hinnant's `days_from_civil`.
    ///
    /// Written out rather than pulled from a date crate on purpose: the point of
    /// the assertion below is to derive the epoch INDEPENDENTLY of the literal in
    /// the constant. Re-typing the same digits either side of an `assert_eq!`
    /// would pass whatever was mistyped.
    fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = if y >= 0 { y } else { y - 399 } / 400;
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// [`USDC_ORACLE_EPOCH_S`] is 2026-03-11T14:00:00Z, and a mistyped digit must
    /// not be able to ship. The read-side label arm and the 0268 reset's upper
    /// bound both key on this instant, and both fail SILENTLY if it drifts — a
    /// too-early epoch relabels genuinely-oracle-priced candles `external`, a
    /// too-late one relabels imported ones `oracle`. Nothing errors either way.
    #[test]
    fn usdc_oracle_epoch_is_2026_03_11t14_00_00z() {
        let expected = days_from_civil(2026, 3, 11) * 86_400 + 14 * 3_600;
        assert_eq!(
            i64::from(USDC_ORACLE_EPOCH_S),
            expected,
            "USDC_ORACLE_EPOCH_S must be 2026-03-11T14:00:00Z (the first measured \
             oracle row for canonical USDC in prices.usd_rate on prod)"
        );
    }

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
        // 1 CREATE DATABASE + 21 CREATE TABLE (assets, asset_metadata, _1m,
        // _15m, _1h, _4h, _1d, _1w, _1M, current_prices, asset_supply,
        // asset_symbol, oracle_prices, usd_rate, backfill_sdex_ledgers,
        // backfill_progress, discovery_state, unresolved_pools, pool_registry,
        // ingest_cursor, enrichment_frontier) + 7 close_usd ALTERs (one per
        // OHLCV grain) + 1 assets.sac_address ALTER (task 0061) + 2
        // backfill_progress ALTERs (earliest_data_available [0073 half → 0053]
        // + newest_data_available [0053]) + 1 current_prices.method ALTER
        // (task 0178) = 33 statements.
        // (+discovery_state task 0054, +asset_supply task 0039, +unresolved_pools
        // + pool_registry task 0053, +asset_metadata task 0067, +ingest_cursor
        // task 0064, +usd_rate task 0167, +enrichment_frontier task 0111,
        // +asset_symbol task 0210.)
        // (+1 = 34: task 0267's idempotent `ALTER TABLE prices.usd_rate ADD
        // COLUMN IF NOT EXISTS quality`, the same shape as the current_prices
        // ALTERs already counted above.)
        // (+7 = 41: task 0286's per-table pf ALTERs — pf_trade_count, pf_volume
        // and pf_price_volume added as three clauses of one ALTER per OHLCV
        // grain, because the `CREATE … AS` copies do not inherit them.)
        let stmts = split_statements(INIT_SQL);
        assert_eq!(stmts.len(), 41, "got {}", stmts.len());
    }

    /// The single `CREATE TABLE … IF NOT EXISTS <table> (` statement of `sql`.
    fn create_statement(sql: &str, table: &str) -> String {
        let needle = format!("CREATE TABLE IF NOT EXISTS {table} (");
        split_statements(sql)
            .into_iter()
            .find(|s| s.contains(&needle))
            .unwrap_or_else(|| panic!("no CREATE TABLE for {table}"))
    }

    /// Column names of a `CREATE TABLE` body, in DDL order: everything between
    /// the opening `(` line and the closing `)` line, first identifier per line.
    fn create_column_names(stmt: &str) -> Vec<String> {
        stmt.lines()
            .skip_while(|l| !l.trim_end().ends_with('('))
            .skip(1)
            .take_while(|l| !l.trim_start().starts_with(')'))
            .filter_map(|l| l.split_whitespace().next())
            .map(|t| t.trim_end_matches(',').to_string())
            .collect()
    }

    /// Task 0286: the fresh `_1m` CREATE and [`CANDLE_COLUMNS`] are one contract.
    /// The writer routes its INSERT by name against this DDL, so a column that
    /// exists in only one of the two is a silent DEFAULT, not an error (F5/F6b).
    #[test]
    fn init_sql_1m_create_lists_every_candle_column_in_order() {
        let stmt = create_statement(INIT_SQL, "prices.price_ohlcv_1m");
        let cols = create_column_names(&stmt);
        assert_eq!(
            cols,
            CANDLE_COLUMNS.to_vec(),
            "the _1m CREATE must list exactly CANDLE_COLUMNS, in order"
        );
    }

    /// Task 0286: a `CREATE … AS` copy does not inherit a post-hoc base-table
    /// ALTER, so each of the seven grains needs its own idempotent ALTER — 21
    /// assertions, the same shape as the `close_usd` block above them.
    #[test]
    fn every_candle_table_gains_the_three_pf_columns() {
        let stmts = split_statements(INIT_SQL);
        for grain in CANDLE_GRAINS {
            let table = format!("prices.price_ohlcv_{grain}");
            let alters: Vec<&String> = stmts
                .iter()
                .filter(|s| {
                    // Tokenised, not `starts_with`: the pf ALTERs put their
                    // clauses on continuation lines, so the table name is
                    // followed by a newline rather than a space.
                    let head: Vec<&str> = s.split_whitespace().take(3).collect();
                    head == ["ALTER", "TABLE", table.as_str()]
                })
                .collect();
            for col in ["pf_trade_count", "pf_volume", "pf_price_volume"] {
                let needle = format!("ADD COLUMN IF NOT EXISTS {col} ");
                assert!(
                    alters.iter().any(|s| s.contains(&needle)),
                    "{table} has no idempotent ALTER adding {col}"
                );
            }
        }
    }

    /// Task 0286: a pre-0061 database must still get `close_usd` in its
    /// mid-table position first. The pf ALTERs append `AFTER version`, so they
    /// must run after — otherwise `AFTER volume_quote_usd` lands `close_usd`
    /// between columns the pf ALTERs have already appended past.
    #[test]
    fn pf_alters_come_after_the_close_usd_alters() {
        let stmts = split_statements(INIT_SQL);
        let last_close_usd = stmts
            .iter()
            .rposition(|s| s.contains("ADD COLUMN IF NOT EXISTS close_usd"))
            .expect("close_usd ALTERs");
        let first_pf = stmts
            .iter()
            .position(|s| s.contains("ADD COLUMN IF NOT EXISTS pf_trade_count"))
            .expect("pf ALTERs");
        assert!(
            first_pf > last_close_usd,
            "pf ALTERs at {first_pf} must come after the last close_usd ALTER at {last_close_usd}"
        );
    }

    /// Task 0286: widening the candle tables to eighteen columns turns a
    /// positional `INSERT … SELECT <15 columns>` into a Code 20 failure (F6d),
    /// so the two MAINTAINED pre-rolls name their columns. Since S2 they name
    /// all EIGHTEEN: a named list that omits one is NOT an error — ClickHouse
    /// fills it from its DEFAULT, and `pf_trade_count DEFAULT trade_count`
    /// would declare a dust-only bucket fully price-forming (F6b).
    #[test]
    fn maintained_prerolls_insert_by_explicit_column_list() {
        let want: Vec<String> = CANDLE_COLUMNS.iter().map(|c| c.to_string()).collect();
        let mut checked = 0;
        for (name, sql) in [
            ("preroll.sql", PREROLL_SQL),
            ("preroll-live-gap.sql", PREROLL_LIVE_GAP_SQL),
        ] {
            let mut per_file = 0;
            for stmt in split_statements(sql) {
                if !stmt.starts_with("INSERT INTO prices.price_ohlcv_") {
                    continue;
                }
                let head = stmt.split("SELECT").next().expect("INSERT head");
                let open = head
                    .find('(')
                    .unwrap_or_else(|| panic!("{name}: INSERT with no column list: {head}"));
                let close = head.rfind(')').expect("closing paren");
                let got: Vec<String> = head[open + 1..close]
                    .split(',')
                    .map(|c| c.trim().to_string())
                    .collect();
                assert_eq!(got, want, "{name}: column list of `{head}`");
                per_file += 1;
            }
            assert_eq!(per_file, 6, "{name}: expected six candle INSERTs");
            checked += per_file;
        }
        assert_eq!(checked, 12);
    }

    /// Task 0286 WR-06. The widening that lands in this slice breaks the two
    /// HISTORICAL pre-rolls: their ~110 bare positional
    /// `INSERT INTO prices.price_ohlcv_* SELECT` statements project fifteen
    /// columns into eighteen-column tables and fail with Code 20. Their bodies
    /// belong to task 0286's rollup generator (S2), but the SCHEMA that breaks
    /// them ships now, and `docs/runbooks/preroll-incremental-presoroban.md`
    /// still points an operator at one of them — so the warning has to be in
    /// the file from the same commit as the widening.
    #[test]
    fn historical_prerolls_warn_that_0286_superseded_them() {
        for (name, sql) in [
            ("preroll-incremental.sql", PREROLL_INCREMENTAL_SQL),
            ("preroll-amm-reprice.sql", PREROLL_AMM_REPRICE_SQL),
        ] {
            // Header, not a footnote: it must be readable before the operator
            // has scrolled to the first statement.
            let header: String = sql.lines().take(12).collect::<Vec<_>>().join("\n");
            assert!(
                header.contains("HISTORICAL"),
                "{name} must be marked HISTORICAL in its header"
            );
            assert!(
                header.contains("0286"),
                "{name}'s header must name the task that superseded it"
            );
            assert!(
                header.contains("DO NOT RUN"),
                "{name}'s header must tell the operator not to run it"
            );
        }
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

    /// Task 0142 — the rollup MVs must stay `IF NOT EXISTS`, and the file must
    /// keep pointing at the procedure for changing one.
    ///
    /// This asserts the OPPOSITE of the 0134 guard above, and deliberately so. A
    /// refreshable `TO`-table MV has no `CREATE OR REPLACE` form, so the escape
    /// 0134 used on the plain views is unavailable; the only route is `DROP` +
    /// re-`CREATE`, and that is not free. While an MV is dropped its tier stops
    /// rolling up (task 0136 is the precedent — nine days unnoticed), and a
    /// re-`CREATE` that loses `APPEND`, `sum(version)` or the aligned window
    /// silently reintroduces the task 0090/0095 production data loss. So the
    /// `DROP` stays an operator action under a runbook rather than something an
    /// apply does implicitly, and this file must never acquire one.
    ///
    /// The consequence — that editing a body here does NOT land on a target that
    /// already holds the MV — is what `drift::check_rollup_drift` exists to make
    /// visible.
    #[test]
    fn rollups_sql_keeps_if_not_exists_and_references_the_reapply_runbook() {
        let stmts = split_statements(ROLLUPS_SQL);
        assert_eq!(stmts.len(), 6, "guard is vacuous if the file is empty");

        for stmt in &stmts {
            let head: String = stmt.chars().take(80).collect();
            assert!(
                stmt.contains("CREATE MATERIALIZED VIEW IF NOT EXISTS"),
                "every rollup MV must stay IF NOT EXISTS (task 0142); got: {head}"
            );
            assert!(
                !stmt.contains("OR REPLACE"),
                "a refreshable TO-table MV has no OR REPLACE form — ClickHouse \
                 rejects it; changing one is DROP + re-CREATE under the runbook; \
                 got: {head}"
            );
            assert!(
                !stmt.contains("DROP"),
                "the DROP belongs in the re-apply runbook, not in the apply path: \
                 applying this file must never take a rollup tier offline; got: {head}"
            );
        }

        // Asserted on the raw text, not on `split_statements`, because the
        // pointer lives in the header comment block — which is exactly where an
        // operator about to edit a body will be looking.
        assert!(
            ROLLUPS_SQL.contains("docs/runbooks/0142-rollup-mv-reapply.md"),
            "rollups.sql must point at the re-apply procedure: an edit to a body \
             here does not land on a provisioned target, and the file is the only \
             place that warning is guaranteed to be read"
        );
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

            // Task 0286: the two MAINTAINED pre-rolls no longer carry the 0145
            // guard at all — the generator's rate form supersedes it (`close ×
            // the latest priced child's close_usd / close`), which is a
            // STRONGER contract: it carries the priced value forward AND keeps
            // `close_usd` on the same bucket as `close`. So the non-vacuity
            // check accepts either spelling. Without it, a file that stopped
            // projecting close_usd altogether would still "pass".
            let rate_form = stmts
                .iter()
                .map(|s| s.matches("toFloat64(close) * argMaxIf(").count())
                .sum::<usize>();
            assert!(
                guarded + rate_form > 0,
                "{name}: no close_usd projection found in either form — either \
                 the file stopped projecting close_usd, or both expressions were \
                 reworded and this test has gone blind"
            );
        }
    }

    /// Task 0135 extends the 0145 guard to the current-prices MV: every
    /// close_usd aggregate in current.sql must be the If-guarded form. The
    /// pre-roll matcher above is alias-specific (`t.timestamp`), so this one
    /// matches the unaliased spelling current.sql uses. A revert of the 0135
    /// contract (argMaxIf → argMax) is otherwise invisible to CI — the
    /// behavioural tests in current_mv_it.rs need a local ClickHouse and are
    /// `#[ignore]`d.
    #[test]
    fn current_sql_uses_no_unguarded_argmax_on_close_usd() {
        let stmts = split_statements(CURRENT_SQL);
        assert!(!stmts.is_empty(), "current.sql yields no statements");

        let mut guarded = 0usize;
        for stmt in &stmts {
            if let Some(offset) = stmt.find("argMax(close_usd") {
                let head: String = stmt[offset..].chars().take(90).collect();
                panic!(
                    "current.sql: unguarded argMax on close_usd — use \
                     argMaxIf(close_usd, timestamp, close_usd > 0) so the tip \
                     carries its latest *priced* close instead of the \
                     un-enriched sentinel 0 (task 0135); got: {head}"
                );
            }
            // Prefix match, not the full expression: per_source's guard also
            // carries the carry-bound predicate, so pinning the exact text
            // would silently stop counting it (and the count below would
            // "pass" one short).
            guarded += stmt.matches("argMaxIf(close_usd, timestamp,").count();
            guarded += stmt.matches("argMinIf(close_usd, timestamp,").count();
        }

        // Non-vacuity: 3 argMaxIf (xlm_usd scalar, per_source's src_price,
        // unfiltered) + 2 argMinIf (ref_7d, open_24h). A drop means a
        // projection lost its guard or the expression was reworded and this
        // test has gone blind.
        //
        // Was 6 until the PR #241 review: per_source's src_price_fresh was a
        // second argMaxIf carrying the bound in its predicate. Liveness is now
        // `max(timestamp) >= now() - INTERVAL 2 HOUR`, which is not a
        // close_usd aggregate at all — it must NOT be, since counting
        // close_usd there is exactly the finding-1 defect.
        assert_eq!(
            guarded, 5,
            "current.sql guarded close_usd aggregate count changed — verify \
             every site still skips un-enriched rows, then update this count"
        );

        // Task 0135, review finding #5: the carry bound must exist at EXACTLY
        // one site. It was briefly duplicated across per_source and unfiltered,
        // and a bound tuned in one place but not the other reproduces the very
        // contradiction the review caught — a zero headline price beside a
        // populated `sources`. The guard above cannot see that; this can.
        let bounds = CURRENT_SQL.matches("INTERVAL 2 HOUR").count();
        assert_eq!(
            bounds, 1,
            "the carry bound must appear exactly once (per_source). Found \
             {bounds}: a second site means the two can drift apart"
        );
    }

    /// The 0144 audit counted 121 guarded sites: 6 + 14 + 6 + 95. Task 0286
    /// moved the twelve MAINTAINED ones (preroll.sql and preroll-live-gap.sql)
    /// to the rate form, which supersedes the 0145 guard rather than sitting
    /// beside it — so those two files are now 0 and the legacy total is 109.
    /// A count regression here still means a projection was added without a
    /// guard, or one was silently dropped; the two HISTORICAL files keep their
    /// counts because their bodies are untouched by design.
    #[test]
    fn preroll_guarded_close_usd_site_counts_match_the_0144_audit() {
        let expected = [
            ("preroll.sql", 0usize),
            ("preroll-incremental.sql", 14),
            ("preroll-live-gap.sql", 0),
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
        assert_eq!(
            total, 109,
            "total guarded pre-roll sites — 121 in the 0144 audit, less the \
             twelve maintained ones task 0286 moved to the rate form"
        );
    }

    /// Every `source IN (...)` in the AMM reprice pre-roll must name EVERY AMM
    /// venue.
    ///
    /// This is the task 0290 defect: `sushiswap` became a candle `source`, the
    /// pre-roll's three-venue allowlist did not move with it, and the backfill
    /// would have written history into `price_ohlcv_1m` that never reached the
    /// coarse tables — invisible to consumers, which read AMM history from
    /// `_1d`/`_1h`, while `_1m` looked correct. Pinning the filter against
    /// `Venue` means the next venue breaks this test instead of a backfill.
    #[test]
    fn the_amm_preroll_source_filter_names_every_amm_venue() {
        // Adding a `Venue` variant makes this match non-exhaustive — a compile
        // error here is the reminder to widen the pre-roll filter as well.
        let venues = [
            Venue::Aquarius,
            Venue::Phoenix,
            Venue::Soroswap,
            Venue::Sushiswap,
        ];
        for v in &venues {
            match v {
                Venue::Aquarius | Venue::Phoenix | Venue::Soroswap | Venue::Sushiswap => {}
            }
        }

        let filters: Vec<&str> = PREROLL_AMM_REPRICE_SQL
            .match_indices("source IN (")
            .map(|(i, _)| {
                let rest = &PREROLL_AMM_REPRICE_SQL[i..];
                let end = rest.find(')').expect("unterminated source IN (...)");
                &rest[..=end]
            })
            .collect();

        assert!(
            !filters.is_empty(),
            "no `source IN (...)` found — the scoping this test guards is gone"
        );

        for filter in &filters {
            for venue in &venues {
                let name = venue.as_source();
                assert!(
                    filter.contains(&format!("'{name}'")),
                    "AMM venue '{name}' missing from a pre-roll filter: {filter}"
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // Task 0286 / ADR 0287 §2–§5 — the three generated SQL files.
    //
    // `rollup_sql.rs` is the source; these files are its output, kept in the
    // repo because an operator pastes them and `drift.rs` compares them to the
    // live definitions. The comparison is whitespace-normalised (the files are
    // laid out for reading; the generator is not), and the failure message
    // prints the rendering — which is how the files are regenerated.
    // ------------------------------------------------------------------

    /// Statements of a shipped file, comments stripped and whitespace collapsed.
    fn normalised(sql: &str) -> Vec<String> {
        split_statements(sql).iter().map(|s| squash(s)).collect()
    }

    /// Compare a shipped file to a generator rendering, statement by statement.
    fn assert_generated(name: &str, file: &str, rendered: Vec<String>) {
        let shipped = normalised(file);
        assert_eq!(
            shipped.len(),
            rendered.len(),
            "{name}: expected {} statements, got {}",
            rendered.len(),
            shipped.len()
        );
        for (i, (got, want)) in shipped.iter().zip(rendered.iter()).enumerate() {
            let want = squash(want);
            assert_eq!(
                *got, want,
                "{name}: statement {i} is not the generator's output.\n\
                 Regenerate the file from `rollup_sql.rs`. Expected:\n{want}"
            );
        }
    }

    #[test]
    fn rollups_sql_is_exactly_the_generators_six_mv_ddls() {
        assert_generated(
            "rollups.sql",
            ROLLUPS_SQL,
            rollup_sql::TIERS
                .iter()
                .map(|t| rollup_sql::mv_ddl(t, PROD_DATABASE).expect("a checked rendering"))
                .collect(),
        );
    }

    #[test]
    fn preroll_sql_is_exactly_the_generators_full_range_inserts() {
        assert_generated(
            "preroll.sql",
            PREROLL_SQL,
            rollup_sql::TIERS
                .iter()
                .map(|t| {
                    rollup_sql::rollup_insert(t, PROD_DATABASE, &rollup_sql::Bounds::Full, None)
                        .expect("a checked rendering")
                })
                .collect(),
        );
    }

    /// The live-gap pre-roll's range is a pair of ClickHouse BOUND PARAMETERS,
    /// not interpolated operator text — the generator only ever sees the
    /// placeholder (ASVS V5).
    #[test]
    fn preroll_live_gap_sql_is_exactly_the_generators_bounded_inserts() {
        assert_generated(
            "preroll-live-gap.sql",
            PREROLL_LIVE_GAP_SQL,
            rollup_sql::TIERS
                .iter()
                .map(|t| {
                    rollup_sql::rollup_insert(
                        t,
                        PROD_DATABASE,
                        &rollup_sql::Bounds::Range {
                            from: rollup_sql::Bound::Param("start_ts"),
                            to: rollup_sql::Bound::Param("end_ts"),
                        },
                        Some("max_threads = 4"),
                    )
                    .expect("a checked rendering")
                })
                .collect(),
        );
    }

    /// The drift detector's constraints on the FILE (BRIEF F10), which no
    /// amount of generator correctness enforces: `apply_sql` splits on `;`, so
    /// a `;` inside a comment or a top-level `WITH` would break statement
    /// splitting, and a header that names an MV makes
    /// `rollup_drift_it`'s statement lookups match the comment block instead of
    /// the statement.
    #[test]
    fn rollups_sql_respects_the_drift_detectors_file_constraints() {
        let stmts = split_statements(ROLLUPS_SQL);
        assert_eq!(stmts.len(), 6);
        for stmt in &stmts {
            assert!(
                stmt.starts_with("CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_"),
                "unexpected statement head: {}",
                &stmt[..80.min(stmt.len())]
            );
            assert!(stmt.contains("\nTO prices.price_ohlcv_"));
            assert!(stmt.contains(" AS\nSELECT"));
            assert!(!stmt.contains("WITH "), "no top-level WITH: {stmt}");
            assert!(!stmt.contains("DROP"), "no DROP in the apply path: {stmt}");
        }

        // `apply_sql` strips `--` comments before splitting, but
        // `rollup_drift_it.rs` and the freshness probe look a statement up with
        // a RAW `split(';')` over the whole file — so a `;` in the header would
        // become its own chunk, and an MV named in the header would make those
        // lookups match the comment block instead of the statement.
        let header = ROLLUPS_SQL
            .split("CREATE MATERIALIZED VIEW")
            .next()
            .expect("header");
        for tier in rollup_sql::TIERS {
            assert!(
                !header.contains(tier.mv),
                "the header must not name {} — the per-statement lookups in \
                 rollup_drift_it.rs would match the comment block",
                tier.mv
            );
        }
        assert!(
            !header.contains(';'),
            "no `;` in the header — a raw split would make it its own chunk"
        );
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
    // ------------------------------------------------------------------
    // Task 0267 — the read-path widening, asserted on the SQL TEXT.
    //
    // CI has no ClickHouse, so the behavioural proof (an oracle row and an
    // imported row in one bucket, oracle wins) is `#[ignore]` in
    // tests/views_it.rs. These tests are the half that runs on every push:
    // they pin the SHAPE the behavioural test depends on, so a regression is
    // caught by CI rather than by whoever next remembers to start docker.
    // ------------------------------------------------------------------

    /// Collapse whitespace runs so an assertion pins the SQL rather than its
    /// indentation. `split_statements` strips comments but preserves layout.
    fn squash(sql: &str) -> String {
        sql.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// The two `price_usd_series` grain statements, squashed, as
    /// `(name, sql)` — every task 0267 assertion runs over BOTH.
    ///
    /// ⚠️ Keeping the two grains in step is the point. The pre-0165 hourly
    /// variant carried a defect the daily one did not, and a fix applied to one
    /// grain only is the exact shape of that bug.
    fn series_grains() -> Vec<(&'static str, String)> {
        let stmts = split_statements(VIEWS_SQL);
        let find = |needle: &str| -> String {
            squash(
                stmts
                    .iter()
                    .find(|s| s.contains(needle))
                    .unwrap_or_else(|| panic!("no view statement containing `{needle}`")),
            )
        };
        vec![
            ("price_usd_series", find("prices.price_usd_series AS")),
            ("price_usd_series_1h", find("prices.price_usd_series_1h AS")),
        ]
    }

    /// The two `usd_reference` grain statements, squashed — the series'
    /// companion surfaces, which carry the same weighted-average shape.
    fn reference_grains() -> Vec<(&'static str, String)> {
        let stmts = split_statements(VIEWS_SQL);
        let find = |needle: &str| -> String {
            squash(
                stmts
                    .iter()
                    .find(|s| s.contains(needle))
                    .unwrap_or_else(|| panic!("no view statement containing `{needle}`")),
            )
        };
        vec![
            ("usd_reference", find("prices.usd_reference AS")),
            ("usd_reference_1h", find("prices.usd_reference_1h AS")),
        ]
    }

    /// Tasks 0171 / 0198. Every weighted-average surface admits a candle only
    /// with `volume_base > 0`, so `sum(w)` can never be 0 for a group that
    /// exists and `CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14))` can
    /// never see NULL — which on 26.3.10.60 it silently turns into
    /// Decimal128::MIN (≈ -1.7e24) flagged `traded`. The behavioural proof is
    /// `#[ignore]` in tests/views_it.rs; this pins the predicate in CI, on
    /// all four statements, so a fix that reaches only one grain fails here.
    #[test]
    fn views_sql_every_weighted_surface_admits_only_candles_with_volume() {
        for (name, stmt) in series_grains() {
            assert!(
                stmt.contains("WHERE p.close_usd > 0 AND p.volume_base > 0"),
                "{name}: arm A must require volume_base > 0 beside close_usd > 0 \
                 (tasks 0171/0198), got no such predicate"
            );
        }
        for (name, stmt) in reference_grains() {
            assert!(
                stmt.contains("AND p.close > 0 AND p.volume_base > 0"),
                "{name}: the reference must require volume_base > 0 beside close > 0 \
                 (task 0171 audit), got no such predicate"
            );
        }
    }

    /// Task 0267 decision C. An IMPORTED measurement (`method = 'external'`)
    /// is evidence of the same standing as a polled one and must be readable
    /// by both grains — otherwise the loaded USDC history is written and never
    /// served, and `/ohlcv` keeps publishing a literal 1.0 labelled `peg`.
    #[test]
    fn views_sql_rate_subquery_admits_oracle_and_external_at_both_grains() {
        for (name, stmt) in series_grains() {
            assert!(
                stmt.contains("WHERE method IN ('oracle', 'external')"),
                "{name}: the usd_rate subquery must admit both measured methods"
            );
            assert!(
                !stmt.contains("WHERE method = 'oracle'"),
                "{name}: the single-method equality predicate must be gone — it \
                 is what hid task 0267's imported rows from this surface"
            );
            // The staging word must NOT be readable. Rows land under it before
            // an operator has verified them, and the whole safety of the
            // shadow/promote split is that no read predicate names it.
            assert!(
                !stmt.contains("external-candidate"),
                "{name}: the pre-promotion staging method must never be read"
            );
        }
    }

    /// The preference rule, and the reason it is a TUPLE. `argMax` over
    /// `(rank, timestamp)` compares element by element, so an oracle row beats
    /// every imported row in the bucket regardless of when each was observed.
    /// A bare `timestamp` key — the shape this had before the widening — would
    /// let a backfilled import outrank a measured reading purely by landing
    /// later in the day. This test fails if anyone reverts to that key.
    #[test]
    fn views_sql_rate_preference_is_a_rank_first_tuple_at_both_grains() {
        for (name, stmt) in series_grains() {
            for col in ["usd_rate", "method"] {
                assert!(
                    stmt.contains(&format!(
                        "argMax({col}, (if(method = 'oracle', 1, 0), timestamp))"
                    )),
                    "{name}: argMax over `{col}` must key on the rank-first tuple"
                );
            }
            assert!(
                !stmt.contains("argMax(usd_rate, timestamp)"),
                "{name}: a bare timestamp key lets a later import outrank an \
                 oracle reading — the rank must come first"
            );
            assert!(
                stmt.contains("AS rate_method"),
                "{name}: the winning row's method must be carried up, or the \
                 label arm has nothing to rank on"
            );
        }
    }

    /// The label is three-way since task 0267, and the ORDER of its arms is
    /// load-bearing: `max(peg_rate) <= 0` stays the first discriminator because
    /// it is the only test that reads identically under both `join_use_nulls`
    /// settings (arm B's own comment explains why). Ordering the rank test
    /// first would work by accident rather than by rule.
    #[test]
    fn views_sql_method_label_is_three_way_with_the_peg_test_first() {
        for (name, stmt) in series_grains() {
            assert!(
                stmt.contains(
                    "multiIf(max(peg_rate) <= 0, 'peg', max(rate_rank) = 2, 'oracle', 'external')"
                ),
                "{name}: the label must be a three-way multiIf with the peg \
                 discriminator first"
            );
            assert!(
                !stmt.contains("if(max(peg_rate) > 0, 'oracle', 'peg')"),
                "{name}: the two-way label cannot survive — it reports every \
                 imported rate as an oracle reading"
            );
            assert!(
                stmt.contains("'traded'"),
                "{name}: the traded arm is unchanged by task 0267"
            );
        }
    }

    /// `UNION ALL` matches its arms POSITIONALLY and requires an identical
    /// column count, so `rate_rank` has to appear in BOTH arms even though arm
    /// A can never carry a rate. Two arm sites plus one outer aggregate.
    #[test]
    fn views_sql_rate_rank_reaches_both_union_arms_and_the_outer_aggregate() {
        for (name, stmt) in series_grains() {
            assert_eq!(
                stmt.matches("AS rate_rank").count(),
                2,
                "{name}: rate_rank must be produced by BOTH union arms"
            );
            assert!(
                stmt.contains("toUInt8(0) AS rate_rank"),
                "{name}: arm A must emit the structural placeholder"
            );
            assert!(
                stmt.contains("multiIf(ifNull(r.rate_method, '') = 'oracle', 2, ifNull(r.rate_method, '') = 'external', 1, 0) AS rate_rank"),
                "{name}: arm B's rank must be NUMERIC — a max() over the method \
                 string would rank by the lexicographic accident that 'oracle' \
                 sorts above 'external'"
            );
            assert_eq!(
                stmt.matches("max(rate_rank)").count(),
                1,
                "{name}: the outer aggregate must consume the rank exactly once"
            );
        }
    }

    /// The single-grain-edit guard. Every token task 0267 introduced must occur
    /// the SAME number of times in both grain statements — so a fix applied to
    /// the daily view and forgotten on the hourly one fails here rather than
    /// silently shipping two surfaces that disagree about the same rate.
    #[test]
    fn views_sql_both_series_grains_agree_on_every_task_0267_token() {
        let grains = series_grains();
        let (daily_name, daily) = &grains[0];
        let (hourly_name, hourly) = &grains[1];
        for token in [
            "WHERE method IN ('oracle', 'external')",
            "argMax(usd_rate, (if(method = 'oracle', 1, 0), timestamp))",
            "argMax(method, (if(method = 'oracle', 1, 0), timestamp))",
            "AS rate_method",
            "AS rate_rank",
            "toUInt8(0) AS rate_rank",
            "max(rate_rank) = 2",
            "multiIf(max(peg_rate) <= 0, 'peg', max(rate_rank) = 2, 'oracle', 'external')",
        ] {
            assert_eq!(
                daily.matches(token).count(),
                hourly.matches(token).count(),
                "`{token}` occurs a different number of times in {daily_name} \
                 than in {hourly_name} — the grains have drifted apart"
            );
        }
    }
}
