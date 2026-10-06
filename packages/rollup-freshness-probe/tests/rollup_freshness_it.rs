//! Integration test for the rollup freshness probe's query against a local
//! Docker ClickHouse with the `prices` schema applied (task 0137).
//!
//! The unit tests in `lib.rs` exercise `lag_metrics` (pure Rust) and the query
//! *string* shape — they cannot catch a query that fails to **execute or
//! deserialize**, which is exactly the class of regression that shipped a broken
//! `backfill-freshness-probe` in PR #97 (a `Nullable(Int64)` column that would
//! not deserialize into a non-`Option` field, so the probe errored on every run).
//!
//! This IT also pins the two ClickHouse behaviours the query design rests on,
//! both of which were measured on 26.3.10.60 and neither of which is obvious
//! from reading the SQL:
//!
//! - an **empty** tier must produce **no row** — ungated, `max()` over zero rows
//!   returns `1970-01-01`, i.e. a ~56-year lag that breaches every threshold;
//! - a **stalled** tier must produce a lag over its bound — the 0136 scenario.
//!
//! ```text
//! tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//! cargo test -p rollup-freshness-probe --test rollup_freshness_it -- --ignored --nocapture --test-threads=1
//! ```
//!
//! ⚠️ **Destructive, and to more than the candles.** These tests `TRUNCATE`
//! `prices.price_ohlcv_*` **and `prices.assets`** (the asset registry), and the
//! gap-3 drift tests `CREATE`/`DROP` a real materialized view, its target table
//! and a throwaway database inside the server they connect to. `ch_url()` honours
//! `CLICKHOUSE_URL`, so a mis-set environment variable points all of that at
//! whatever cluster it names. **Never run against a shared or production
//! cluster.**

use clickhouse::Client;
use prices_clickhouse::asset_id::fixture::{AssetFixture, assets_insert};
use rollup_freshness_probe::mv_drift::{
    DriftMetric, MV_DRIFT_CRITICAL_METRIC, MV_DRIFT_METRIC, MV_DRIFT_UNREADABLE_METRIC, describe,
    drift_metrics, visible_objects_query,
};
use rollup_freshness_probe::usd_sanity::{
    PEG_TABLE, PegCounts, STRANDED_TABLE, SanityRefusal, StrandedCounts, peg_metric, peg_query,
    stranded_metric, stranded_query,
};
use rollup_freshness_probe::{ROLLUP_TIERS, TableLag, freshness_query, lag_metrics};
use std::fmt::Display;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// The probe binds the client to the `prices` database (`client_from_lambda_env
/// ("prices")` in `main.rs`), which is why the query references tables
/// unqualified. The IT must do the same so the exact production query resolves.
///
/// ⚠️ **It also STOPs the reconciliation MVs in the shared `prices` database**
/// (review WR-05) — see [`stop_shared_reconcile`]. Every test here that uses
/// the shared database comes through this function, so none can run while an
/// hourly reconcile pass is free to rewrite the rows it seeded.
async fn client() -> Client {
    let c = Client::default().with_url(ch_url()).with_database("prices");
    stop_shared_reconcile(&c).await;
    c
}

/// `SYSTEM STOP VIEW` every `prices.mv_reconcile_*` that is not already
/// stopped (review WR-05).
///
/// CI applies `schema/rollups.sql` to the shared `prices` database
/// (`prices-clickhouse-init --rollups`), so it holds the six hourly
/// reconciliation MVs on the REAL clock, reaching seven days back. These tests
/// seed `_1m` and `_1h` independently, with deliberately different values, 3 h
/// to 5 days old — rows no fast MV window reaches. A reconcile pass firing on a
/// `:00` crossing mid-suite would roll the `_1m` seed up through every tier and
/// overwrite or add the `_1h`/coarse rows the stranded, peg, freshness and
/// zero-invariant assertions count: an hourly flake window.
///
/// Found LIVE, so a server without the reconcile MVs (a schema applied before
/// task 0203) needs nothing. The views stay stopped afterwards: that is the
/// state the whole shared-database suite wants, and STOP is lost on a server
/// restart anyway (a fresh CI server, or `SYSTEM START VIEW` by hand locally).
async fn stop_shared_reconcile(c: &Client) {
    let views: Vec<String> = c
        .query(
            "SELECT view FROM system.view_refreshes \
             WHERE database = 'prices' AND startsWith(view, 'mv_reconcile_') \
               AND status != 'Disabled'",
        )
        .fetch_all()
        .await
        .expect("list the shared reconcile MVs");
    for view in views {
        exec(c, &format!("SYSTEM STOP VIEW prices.{view}")).await;
    }
}

async fn exec(c: &Client, sql: &str) {
    c.query(sql).execute().await.expect(sql);
}

/// Insert one OHLCV row via `INSERT … SELECT` so `now()` / `INTERVAL`
/// expressions evaluate server-side. `ts_sql` is a ClickHouse expression for
/// `timestamp` (e.g. `now() - INTERVAL 20 DAY`).
async fn insert_bucket(c: &Client, table: &str, ts_sql: &str) {
    exec(
        c,
        &format!(
            "INSERT INTO prices.{table} \
               (timestamp, asset_id, quote_asset_id, source, open, high, low, close, vwap, version) \
             SELECT {ts_sql}, 1, 2, 'sdex', 1, 1, 1, 1, 1, 1"
        ),
    )
    .await;
}

fn bound(table: &str) -> i64 {
    ROLLUP_TIERS
        .iter()
        .find(|t| t.table == table)
        .expect("tier present in ROLLUP_TIERS")
        .lag_bound_seconds
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn freshness_query_executes_deserializes_and_gates_empty_tiers() {
    let c = client().await;
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    for tier in ROLLUP_TIERS {
        exec(&c, &format!("TRUNCATE TABLE prices.{}", tier.table)).await;
    }

    // --- Two tiers populated, five deliberately left empty -------------------
    // Fresh: 2 minutes old, well inside the 15-minute bound.
    insert_bucket(&c, "price_ohlcv_1m", "now() - INTERVAL 2 MINUTE").await;
    // Stalled: the 0136 shape — a coarse tier frozen while 1m keeps flowing.
    insert_bucket(&c, "price_ohlcv_1h", "now() - INTERVAL 20 DAY").await;

    let query = freshness_query();

    // --- Run the EXACT production query -------------------------------------
    // This line is the regression guard: a query that will not execute or whose
    // lag_seconds column is not a plain non-nullable Int64 fails here.
    let rows =
        c.query(&query).fetch_all::<TableLag>().await.expect(
            "freshness query must execute and deserialize into TableLag (non-nullable i64)",
        );

    // Only the two populated tiers survive the `HAVING count() > 0` gate. This
    // is the assertion that matters most: ungated, the five empty tiers would
    // each report ~1.79e9 seconds and breach on a freshly-provisioned env.
    assert_eq!(
        rows.len(),
        2,
        "expected exactly the 2 populated tiers, got {rows:?} — empty tiers must be gated out"
    );
    let names: Vec<&str> = rows.iter().map(|r| r.table_name.as_str()).collect();
    assert_eq!(
        names,
        vec!["price_ohlcv_1h", "price_ohlcv_1m"],
        "results must be sorted by table_name (the union has to be wrapped for ORDER BY to apply)"
    );

    let lag = |t: &str| {
        rows.iter()
            .find(|r| r.table_name == t)
            .unwrap_or_else(|| panic!("{t} present"))
            .lag_seconds
    };

    // Fresh tier: ~2 min, under its 15-min bound → would not page.
    let fresh = lag("price_ohlcv_1m");
    assert!(
        (60..=600).contains(&fresh),
        "price_ohlcv_1m lag {fresh}s should be ~120s"
    );
    assert!(
        fresh < bound("price_ohlcv_1m"),
        "a fresh 1m tier must stay under its bound"
    );

    // Stalled tier: ~20 days, far over its 3-hour bound → pages. This is 0136.
    let stalled = lag("price_ohlcv_1h");
    let twenty_days = 20 * 86_400;
    assert!(
        (twenty_days - 600..=twenty_days + 600).contains(&stalled),
        "price_ohlcv_1h lag {stalled}s should be ~20 days ({twenty_days})"
    );
    assert!(
        stalled > bound("price_ohlcv_1h"),
        "a 20-day-stalled 1h tier must exceed its {}s bound",
        bound("price_ohlcv_1h")
    );

    // The shaped metrics the probe publishes. `1m` and `1h` are measured; `15m`
    // sits BETWEEN them and is empty while a coarser tier (`1h`) holds data, so
    // it is synthesised as breaching rather than silently skipped — otherwise a
    // tier emptied by retention mid-freeze would read as recovered. `4h`/`1d`/
    // `1w`/`1M` are coarser than everything populated, so they stay absent.
    let metrics = lag_metrics(&rows);
    let published: Vec<&str> = metrics.iter().map(|m| m.table.as_str()).collect();
    assert_eq!(
        published,
        vec!["price_ohlcv_15m", "price_ohlcv_1h", "price_ohlcv_1m"],
        "expected the two measured tiers plus a synthesised 15m"
    );
    let by = |t: &str| {
        metrics
            .iter()
            .find(|m| m.table == t)
            .unwrap_or_else(|| panic!("{t} published"))
            .value
    };
    assert_eq!(by("price_ohlcv_1h"), stalled as f64);
    assert_eq!(by("price_ohlcv_1m"), fresh as f64);
    assert_eq!(
        by("price_ohlcv_15m"),
        rollup_freshness_probe::EMPTY_TIER_SENTINEL_SECONDS as f64
    );

    // --- Fresh-environment end state: every tier empty → zero rows -----------
    // No datum at all, so the NOT_BREACHING alarms stay OK rather than all seven
    // firing at once on a newly provisioned environment.
    for tier in ROLLUP_TIERS {
        exec(&c, &format!("TRUNCATE TABLE prices.{}", tier.table)).await;
    }
    let rows = c
        .query(&query)
        .fetch_all::<TableLag>()
        .await
        .expect("freshness query must still execute with zero matching rows");
    assert!(
        rows.is_empty(),
        "all tiers empty ⇒ no rows from the query, got {rows:?}"
    );
    assert!(
        lag_metrics(&rows).is_empty(),
        "all tiers empty ⇒ nothing published at all, not even sentinels — a fresh \
         environment must not page on seven alarms at once"
    );
}

/// The empty-tier gate is the single most load-bearing clause in the query, and
/// its justification is a ClickHouse behaviour rather than anything visible in
/// the SQL. Pin that behaviour directly, so if a future ClickHouse release ever
/// makes `max()` over zero rows return NULL (or an empty result), this test
/// fails and tells the next reader the gate's rationale has changed — rather
/// than the gate silently becoming cargo cult.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn ungated_max_over_empty_tier_yields_the_epoch_not_null() {
    let c = client().await;
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    exec(&c, "TRUNCATE TABLE prices.price_ohlcv_1w").await;

    #[derive(Debug, clickhouse::Row, serde::Deserialize)]
    struct Ungated {
        lag_seconds: i64,
    }

    let rows = c
        .query(
            "SELECT toInt64(toUnixTimestamp(now()) - toUnixTimestamp(max(timestamp))) \
               AS lag_seconds FROM price_ohlcv_1w",
        )
        .fetch_all::<Ungated>()
        .await
        .expect("ungated max() must execute");

    assert_eq!(
        rows.len(),
        1,
        "ungated max() over an empty table returns one row, not zero — this is why HAVING is needed"
    );
    // ~56 years of seconds: max() returned the DateTime zero value, 1970-01-01.
    assert!(
        rows[0].lag_seconds > 50 * 365 * 86_400,
        "expected an epoch-derived lag (~1.79e9s), got {}s — if this changed, revisit the \
         HAVING count() > 0 gate in freshness_query()",
        rows[0].lag_seconds
    );
    // And it would breach every single tier's bound.
    for tier in ROLLUP_TIERS {
        assert!(
            rows[0].lag_seconds > tier.lag_bound_seconds,
            "{} would false-fire without the gate",
            tier.table
        );
    }
}

/// The disk-headroom query executes and deserializes (task 0204, gap 1).
///
/// Same regression class as the freshness IT above: the unit tests pin the
/// query *string*, which cannot catch a query that fails to execute or whose
/// columns will not deserialize into `DiskUsage` — the bug that shipped a
/// broken `backfill-freshness-probe` in PR #97.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn disk_query_executes_and_deserializes() {
    use rollup_freshness_probe::disk::{
        DISK_FREE_PERCENT_METRIC, DiskUsage, disk_metrics, disk_query, free_percent,
    };

    let c = client().await;
    let usage =
        c.query(disk_query()).fetch_one::<DiskUsage>().await.expect(
            "disk query must execute and deserialize into DiskUsage (two non-nullable u64)",
        );

    assert!(
        usage.capacity_bytes > 0,
        "filesystemCapacity() must report a real filesystem, got {usage:?}"
    );
    assert!(
        usage.available_bytes <= usage.capacity_bytes,
        "available must not exceed capacity: {usage:?}"
    );

    let pct = free_percent(&usage).expect("a real filesystem has non-zero capacity");
    assert!(
        (0.0..=100.0).contains(&pct),
        "free percent {pct} out of range for {usage:?}"
    );

    let metrics = disk_metrics(&usage).expect("readable capacity publishes metrics");
    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].name, DISK_FREE_PERCENT_METRIC);
    assert_eq!(metrics[0].value, pct);
}

/// ⚠️ The privilege finding the whole design rests on — pinned so a future
/// "simplification" back to `system.disks` fails here instead of on prod.
///
/// The probe connects as the `ingestion` mTLS identity (`prices_writer`), which
/// holds `GRANT SELECT ON prices.*` and nothing more. Against a user of that
/// exact shape:
///
/// - `system.disks` is **ACCESS_DENIED**, and the grant cannot be added —
///   `prices_writer` is XML-defined and that access storage is read-only
///   (`ACCESS_STORAGE_READONLY`, the same wall task 0182 hit on `ALTER FREEZE`);
/// - `filesystemAvailable()` / `filesystemCapacity()` are functions, carry no
///   table grant, and answer fine.
///
/// Creates and drops its own least-privileged user, so it asserts the real
/// privilege behaviour rather than a mock of it.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn restricted_user_can_read_disk_headroom_but_not_system_disks() {
    use rollup_freshness_probe::disk::{DiskUsage, disk_query};

    let admin = client().await;
    exec(&admin, "DROP USER IF EXISTS rollup_probe_it").await;
    exec(
        &admin,
        "CREATE USER rollup_probe_it IDENTIFIED WITH no_password",
    )
    .await;
    exec(&admin, "GRANT SELECT ON prices.* TO rollup_probe_it").await;

    let restricted = Client::default()
        .with_url(ch_url())
        .with_database("prices")
        .with_user("rollup_probe_it");

    // ⚠️ BOTH reads are collected BEFORE anything can panic, and the user is
    // dropped BEFORE the assertions run. A failing `.expect()` mid-test would
    // otherwise unwind past the cleanup and leave a passwordless account holding
    // SELECT on the whole `prices` database behind on the server — on a
    // developer machine that is untidy, and `ch_url()` honours `CLICKHOUSE_URL`.
    // The probe's own query: must work with no system grant whatsoever.
    let usage = restricted
        .query(disk_query())
        .fetch_one::<DiskUsage>()
        .await;

    // The obvious alternative: must NOT work. If this ever starts succeeding,
    // the constraint has changed and the module docs need revisiting — but
    // until then, switching to system.disks would deploy green and then fail on
    // every invocation against prod.
    let denied = restricted
        .query("SELECT free_space, total_space FROM system.disks")
        .fetch_all::<DiskUsage>()
        .await;

    exec(&admin, "DROP USER IF EXISTS rollup_probe_it").await;

    let usage = usage.expect(
        "disk_query() must run for a user holding only SELECT ON prices.* — if this fails, \
         the probe cannot read disk headroom on prod at all",
    );
    assert!(usage.capacity_bytes > 0);

    let err = denied
        .expect_err("system.disks must be denied to a prices-only user")
        .to_string();
    assert!(
        err.contains("ACCESS_DENIED") || err.contains("Not enough privileges"),
        "expected an access-denied error from system.disks, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Task 0204, gap 4 — USD-value correctness on the USDT quote leg.
// ---------------------------------------------------------------------------

/// The canonical USDT identity. Displays as its derived id, so a candle quoted
/// `{USDT}` sits on the leg the probe resolves.
const USDT: AssetFixture = AssetFixture::new(
    "USDT",
    "credit_alphanum4",
    prices_clickhouse::USDT_ISSUER,
    "",
);

/// Seed [`USDT`] into `prices.assets`. The probe resolves the leg by code +
/// issuer rather than a hard-coded id (task 0139), so the IT has to make that
/// resolution succeed.
async fn seed_usdt_identity(c: &Client) {
    exec(c, &assets_insert("prices", &[USDT])).await;
}

/// Insert one USDT-quoted candle with an explicit `close` / `close_usd` into a
/// named tier.
///
/// ⚠️ **The table is a parameter since task 0213.** The two directions read
/// different tiers, and the defect that task exists to close was invisible
/// precisely because a test could not tell them apart. `_1h` is created as
/// `AS price_ohlcv_1m`, so one statement shape serves both.
///
/// `ts_sql` is a server-side expression so `now()` arithmetic matches the
/// probe's own window and grace bounds exactly.
async fn insert_candle_into(
    c: &Client,
    table: &str,
    usdt_id: impl Display,
    asset_id: u32,
    ts_sql: &str,
    close: &str,
    close_usd: &str,
) {
    exec(
        c,
        &format!(
            "INSERT INTO prices.{table} \
               (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             SELECT {ts_sql}, {asset_id}, {usdt_id}, 'sdex', {close}, {close}, {close}, {close}, \
                    1, 1, 0, {close_usd}, {close}, 1, 1"
        ),
    )
    .await;
}

/// Insert into the **stranded** tier (`price_ohlcv_1h`).
async fn insert_usdt_candle(
    c: &Client,
    usdt_id: impl Display,
    asset_id: u32,
    ts_sql: &str,
    close: &str,
    close_usd: &str,
) {
    insert_candle_into(
        c,
        STRANDED_TABLE,
        usdt_id,
        asset_id,
        ts_sql,
        close,
        close_usd,
    )
    .await;
}

/// Insert into the **peg** tier — the tier enrichment writes.
///
/// ⚠️ **The table is spelled literally, not as `PEG_TABLE`.** A fixture written
/// in terms of the constant under test follows it wherever it points, so the
/// tier assertions below would hold for `_1h` just as happily and would prove
/// nothing. Found by reverting `PEG_TABLE` to `price_ohlcv_1h` and watching
/// tests that should have failed keep passing.
async fn insert_usdt_minute_candle(
    c: &Client,
    usdt_id: impl Display,
    asset_id: u32,
    ts_sql: &str,
    close: &str,
    close_usd: &str,
) {
    insert_candle_into(
        c,
        "price_ohlcv_1m",
        usdt_id,
        asset_id,
        ts_sql,
        close,
        close_usd,
    )
    .await;
}

/// Clear both tiers and the registry. Both, always — a test that truncated only
/// the tier it was about would inherit the other's rows and read a count nobody
/// wrote.
async fn reset_sanity_tables(c: &Client) {
    exec(c, "TRUNCATE TABLE prices.price_ohlcv_1h").await;
    exec(c, "TRUNCATE TABLE prices.price_ohlcv_1m").await;
    exec(c, "TRUNCATE TABLE prices.assets").await;
}

async fn read_stranded(c: &Client) -> StrandedCounts {
    c.query(&stranded_query())
        .fetch_one::<StrandedCounts>()
        .await
        .expect("stranded query executes and deserializes")
}

async fn read_peg(c: &Client) -> PegCounts {
    c.query(&peg_query())
        .fetch_one::<PegCounts>()
        .await
        .expect("peg query executes and deserializes")
}

/// The query must **execute and deserialize** against a real ClickHouse — the
/// class of regression the unit tests structurally cannot catch, and the one
/// that shipped a broken `backfill-freshness-probe` in PR #97. It also pins the
/// arithmetic: a healthy leg reads zero on both directions rather than being
/// unable to tell.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn usd_sanity_query_executes_and_reads_a_healthy_leg_as_zero() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    // A correctly-priced USDT-quoted candle on each tier: USDT at its measured
    // ~0.15, so close_usd is nowhere near close.
    insert_usdt_candle(&c, USDT, 5, "now() - INTERVAL 3 DAY", "100", "15").await;
    insert_usdt_minute_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "15").await;

    let stranded = read_stranded(&c).await;
    assert_eq!(stranded.resolved_legs, 1, "the USDT identity must resolve");
    assert_eq!(stranded.stranded, 0, "a priced candle is not stranded");
    assert_eq!(stranded.scanned, 1);

    let peg = read_peg(&c).await;
    assert_eq!(peg.resolved_legs, 1, "the USDT identity must resolve");
    assert_eq!(peg.peg_applied, 0, "a 0.15 rate is not the peg");
    assert_eq!(peg.scanned, 1);
}

/// ⚠️ **Induce the condition, do not read the CDK.** This is task 0137's lesson
/// applied to gap 4: write the exact two defects into the table and assert each
/// one is counted. Without this the alarm is only proven to *exist*.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn usd_sanity_counts_both_induced_defects() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    // Defect 1 — the peg re-applied, on the tier enrichment WRITES: close_usd
    // == close (task 0172 / 0212).
    insert_usdt_minute_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    // Defect 2 — stranded past the grace period, on the tier the consumer
    // reads: zero on a representable close (what task 0182's own reset produced
    // on 2026-08-19).
    insert_usdt_candle(&c, USDT, 6, "now() - INTERVAL 3 DAY", "100", "0").await;

    assert_eq!(
        read_peg(&c).await.peg_applied,
        1,
        "close_usd == close must be counted"
    );
    assert_eq!(
        read_stranded(&c).await.stranded,
        1,
        "an aged zero must be counted"
    );
}

/// The grace period is what makes the stranded metric usable at all: enrichment
/// fills `close_usd` asynchronously, so the newest candles are *legitimately*
/// zero on every single run. Without this the alarm would breach permanently
/// and get muted — the state task 0204 exists to end.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_freshly_written_zero_is_not_yet_stranded() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    // Inside the 48 h grace — awaiting enrichment, not damaged.
    insert_usdt_candle(&c, USDT, 5, "now() - INTERVAL 1 HOUR", "100", "0").await;
    assert_eq!(read_stranded(&c).await.stranded, 0, "still within grace");

    // The same row, aged past the grace, is the defect.
    exec(&c, "TRUNCATE TABLE prices.price_ohlcv_1h").await;
    insert_usdt_candle(&c, USDT, 5, "now() - INTERVAL 3 DAY", "100", "0").await;
    assert_eq!(read_stranded(&c).await.stranded, 1, "past grace = stranded");
}

/// Dust is not damage. A `close` below the `Decimal(38, 14)` underflow bound
/// cannot produce a non-zero `close_usd` at any plausible rate, so counting it
/// would put the alarm permanently in ALARM over rows with nothing to lose.
/// Task 0182 hit exactly this and its first bound (`1e-11`) was three orders of
/// magnitude too generous.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn dust_below_the_underflow_bound_is_not_counted_as_stranded() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    insert_usdt_candle(
        &c,
        USDT,
        5,
        "now() - INTERVAL 3 DAY",
        "0.00000000000001",
        "0",
    )
    .await;
    assert_eq!(read_stranded(&c).await.stranded, 0, "1e-14 close is dust");
}

/// ⚠️ The trap that makes this check scoped rather than global: exotic-quoted
/// candles sit at `close_usd = 0` **by design** — no USD reference exists and no
/// enrichment tier can price them (~74M rows on `_1h` alone, task 0182). If the
/// quote-leg filter were dropped, the alarm would breach forever on healthy
/// data.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_exotic_quoted_zero_is_ignored_because_it_is_by_design() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    // quote_asset_id 999 is not the USDT leg — an unpriceable exotic pair.
    insert_usdt_candle(&c, 999, 5, "now() - INTERVAL 3 DAY", "100", "0").await;

    let counts = read_stranded(&c).await;
    assert_eq!(counts.scanned, 0, "the exotic leg is out of scope entirely");
    assert_eq!(counts.stranded, 0);
}

/// `ReplacingMergeTree` + a repair that re-inserts at a higher `version`.
/// Without `FINAL` the query reads the superseded row and alarms on a defect
/// that has already been corrected — an alarm firing on history rather than on
/// state.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_repaired_candle_stops_counting_once_a_higher_version_supersedes_it() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    insert_usdt_minute_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    assert_eq!(read_peg(&c).await.peg_applied, 1, "the defect is present");

    // The repair: same primary key, corrected value, version + 1.
    exec(
        &c,
        &format!(
            "INSERT INTO prices.price_ohlcv_1m \
               (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             SELECT timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                    volume_base, volume_quote, volume_quote, 15, vwap, trade_count, version + 1 \
             FROM prices.price_ohlcv_1m FINAL \
             WHERE quote_asset_id = {USDT} AND close_usd = close"
        ),
    )
    .await;

    assert_eq!(
        read_peg(&c).await.peg_applied,
        0,
        "FINAL must collapse to the repaired row"
    );
}

/// The silent all-clear. With no USDT identity in the registry the quote-leg
/// filter matches nothing, both counts read zero, and a `NOT_BREACHING` alarm
/// would score a check that never ran as perfectly healthy. `resolved_legs`
/// exists so `peg_metric` can refuse it, and `main.rs` fails the invocation.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_unresolvable_usdt_leg_reads_as_zero_and_is_therefore_refused() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    // Deliberately no USDT identity seeded.
    insert_usdt_minute_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;

    let counts = read_peg(&c).await;
    assert_eq!(counts.resolved_legs, 0);
    assert_eq!(
        counts.peg_applied, 0,
        "a real defect is invisible without the identity — hence the refusal"
    );
    assert_eq!(
        peg_metric(&counts),
        Err(SanityRefusal::UnresolvableLeg {
            resolved_legs: 0,
            table: PEG_TABLE
        }),
        "this reading must never be published as healthy"
    );
}

/// 🔴 **The defect task 0213 exists to close, induced rather than reasoned
/// about.**
///
/// Reproduces the exact production state measured on 2026-08-20: `price_ohlcv_1m`
/// carries peg-valued rows (1,564,045 of them) while every coarse tier reads
/// clean, because task 0182's repair wrote the coarse tables **directly** and
/// never touched the tier they roll from (task 0212).
///
/// Before this task the peg direction read `_1h` and would have published a
/// confident **0** over that population. The assertion that matters is the
/// second one: the tier the check used to read shows nothing wrong.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_peg_row_only_in_1m_is_counted_although_every_coarse_tier_reads_clean() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    // The tier enrichment writes: the peg re-applied.
    insert_usdt_minute_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    // The repaired coarse tier: the SAME candle, correctly valued at ~0.15 —
    // which is precisely what 0182's repair left behind.
    insert_usdt_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "15").await;

    assert_eq!(
        read_peg(&c).await.peg_applied,
        1,
        "the peg direction must see the tier enrichment writes"
    );

    // ⚠️ The regression this pins. A check reading the repaired tier sees a
    // healthy leg and publishes zero — the silent all-clear, from a scan that
    // really did run and really did examine rows.
    let stranded = read_stranded(&c).await;
    assert_eq!(
        stranded.scanned, 1,
        "the coarse tier was genuinely examined — this is not an empty scan"
    );
    assert_eq!(
        stranded.stranded, 0,
        "and it reads perfectly healthy, which is exactly why the peg direction \
         cannot live here"
    );
}

/// The two directions must not be able to see each other's rows. Pinned because
/// the tempting simplification — one query over one tier — is what made the peg
/// direction blind, and a future "let's just union them" would restore it.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn each_direction_only_scans_its_own_tier() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    // Rows in `_1m` only.
    insert_usdt_minute_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    assert_eq!(read_peg(&c).await.scanned, 1);
    assert_eq!(
        read_stranded(&c).await.scanned,
        0,
        "the stranded direction must not see _1m rows"
    );

    // Rows in `_1h` only.
    //
    // ⚠️ Seeded at 3 HOURS, not 3 days, and that is the whole point of the
    // assertion. A 3-day-old row falls outside the 48 h peg window whichever
    // table the peg direction reads, so it would pass with `PEG_TABLE` reverted
    // to `price_ohlcv_1h` — testing the window instead of the tier. Inside the
    // peg window, only the tier can explain a zero.
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;
    insert_usdt_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "0").await;
    assert_eq!(read_stranded(&c).await.scanned, 1);
    assert_eq!(
        read_peg(&c).await.scanned,
        0,
        "the peg direction must not see _1h rows"
    );
}

/// ⚠️ **The window bound the peg direction does not inherit.** `_1m` is
/// retention-managed at 7 days, so the peg scan keeps a wide margin below that
/// frontier (48 h) rather than reusing the stranded direction's 7 days. A row
/// older than the peg window is out of scope even though it is still in the
/// table — which is the property that makes a cleanup run unable to move the
/// count.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_peg_window_excludes_rows_a_cleanup_run_could_delete() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    // Inside the 48 h peg window — counted.
    insert_usdt_minute_candle(&c, USDT, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    // Older than the peg window but still well inside `_1m`'s 7-day retention,
    // i.e. exactly the band a widened window would have picked up and a cleanup
    // run could then remove underneath it.
    insert_usdt_minute_candle(&c, USDT, 6, "now() - INTERVAL 5 DAY", "100", "100").await;

    let peg = read_peg(&c).await;
    assert_eq!(peg.scanned, 1, "only the in-window row is examined");
    assert_eq!(
        peg.peg_applied, 1,
        "a defect outside the window is task 0212's population, not this alarm's"
    );
}

/// ⚠️ A `_1m` scan that matched nothing must not suppress a working `_1h`
/// reading. Before task 0213 one refusal killed both metrics — harmless while
/// they came from one query, and the muting failure from the other side once
/// they read different tiers.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_empty_peg_scan_does_not_suppress_the_stranded_metric() {
    let c = client().await;
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c).await;

    // `_1m` is empty; `_1h` carries a real stranded candle.
    insert_usdt_candle(&c, USDT, 5, "now() - INTERVAL 3 DAY", "100", "0").await;

    let peg = read_peg(&c).await;
    assert_eq!(
        peg_metric(&peg),
        Err(SanityRefusal::EmptyScan {
            table: PEG_TABLE,
            lookback_seconds: rollup_freshness_probe::usd_sanity::PEG_LOOKBACK_SECONDS,
        }),
        "an unexamined tier must be refused, not published as zero"
    );

    let stranded = read_stranded(&c).await;
    assert_eq!(
        stranded_metric(&stranded)
            .expect("published on its own evidence")
            .value,
        1.0,
        "the working direction must still publish"
    );
}

// ---------------------------------------------------------------------------
// Task 0204, gap 3 — materialized-view drift on a schedule.
// ---------------------------------------------------------------------------

fn drift_value(metrics: &[DriftMetric], name: &str) -> f64 {
    metrics
        .iter()
        .find(|m| m.name == name)
        .unwrap_or_else(|| panic!("{name} published"))
        .value
}

/// The control, and the one that matters most in practice: a schema that really
/// is in sync must read as clean. An alarm that fires on a healthy chain gets
/// muted, and a muted alarm is the state task 0204 exists to end.
///
/// This also exercises `check_rollup_drift` against a live server — the unit
/// tests shape a report that is handed to them, and cannot catch a query that
/// fails to execute or a fingerprint parser that no longer matches what
/// ClickHouse renders.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_freshly_applied_schema_reports_no_drift() {
    let c = client().await;
    let visible: u64 = c
        .query(&visible_objects_query("prices"))
        .fetch_one()
        .await
        .expect("system.tables is grant-filtered, not denied");
    assert!(visible > 0, "the probe must be able to see its own schema");

    let reports = prices_clickhouse::drift::check_rollup_drift(&c, "prices")
        .await
        .expect("drift check executes");
    let m = drift_metrics(&reports, visible);

    assert_eq!(
        drift_value(&m, MV_DRIFT_CRITICAL_METRIC),
        0.0,
        "in-sync schema: {}",
        describe(&reports)
    );
    assert_eq!(
        drift_value(&m, MV_DRIFT_METRIC),
        0.0,
        "in-sync schema: {}",
        describe(&reports)
    );
    assert_eq!(drift_value(&m, MV_DRIFT_UNREADABLE_METRIC), 0.0);
}

/// ⚠️ **Induce the condition.** Feed the checker a *modified* copy of
/// `rollups.sql` so the live definitions genuinely disagree with the declared
/// ones, and assert the ordinary-drift count moves. Non-destructive: the live
/// MVs are untouched, only the file side is edited in memory.
///
/// Without this the alarm is proven to exist but not to detect anything —
/// exactly the "verified by reading the CDK" failure AC 4 names.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_edited_declaration_is_detected_as_drift() {
    let c = client().await;
    let visible: u64 = c
        .query(&visible_objects_query("prices"))
        .fetch_one()
        .await
        .expect("visible");

    // Change the declared SELECT body of one MV. The live object is unchanged,
    // so the check must report exactly this one as drifted.
    let edited = prices_clickhouse::ROLLUPS_SQL.replace(
        "toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp",
        "toStartOfInterval(t.timestamp, INTERVAL 16 MINUTE) AS timestamp",
    );
    assert_ne!(
        edited,
        prices_clickhouse::ROLLUPS_SQL,
        "the edit must actually apply, or this test proves nothing"
    );

    let reports = prices_clickhouse::drift::check_mv_drift(&c, "prices", &edited)
        .await
        .expect("drift check executes");
    let m = drift_metrics(&reports, visible);

    assert!(
        drift_value(&m, MV_DRIFT_METRIC) >= 1.0,
        "an edited declaration must surface as drift, got: {}",
        describe(&reports)
    );
    assert_eq!(
        drift_value(&m, MV_DRIFT_CRITICAL_METRIC),
        0.0,
        "a body edit is not history destruction — it must not page as critical"
    );
}

/// ⚠️ **Induce the critical condition**: an MV that is live *without* `APPEND`.
/// This is the task 0090/0095 data loss — replace mode overwrites the whole
/// target table on every refresh — and it must land on its own metric rather
/// than being counted as ordinary drift.
///
/// Creates a throwaway MV and target rather than touching the real rollup chain,
/// and drops both afterwards.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_live_mv_without_append_is_detected_as_critical() {
    let c = client().await;
    exec(&c, "DROP VIEW IF EXISTS prices.mv_gap3_probe").await;
    exec(&c, "DROP TABLE IF EXISTS prices.gap3_probe_target").await;
    exec(
        &c,
        "CREATE TABLE prices.gap3_probe_target (timestamp DateTime, n UInt64) \
         ENGINE = MergeTree ORDER BY timestamp",
    )
    .await;
    // Deliberately NO `APPEND` — replace mode, the destructive shape.
    exec(
        &c,
        "CREATE MATERIALIZED VIEW prices.mv_gap3_probe \
         REFRESH EVERY 1 HOUR \
         TO prices.gap3_probe_target AS \
         SELECT timestamp, count() AS n FROM prices.price_ohlcv_1d GROUP BY timestamp",
    )
    .await;

    // Declare it WITH append, so file and live disagree on exactly that.
    let declared = "CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_gap3_probe \
                    REFRESH EVERY 1 HOUR APPEND \
                    TO prices.gap3_probe_target AS \
                    SELECT timestamp, count() AS n FROM prices.price_ohlcv_1d GROUP BY timestamp;";

    let reports = prices_clickhouse::drift::check_mv_drift(&c, "prices", declared)
        .await
        .expect("drift check executes");
    let m = drift_metrics(&reports, 32);

    exec(&c, "DROP VIEW IF EXISTS prices.mv_gap3_probe").await;
    exec(&c, "DROP TABLE IF EXISTS prices.gap3_probe_target").await;

    assert_eq!(
        drift_value(&m, MV_DRIFT_CRITICAL_METRIC),
        1.0,
        "a live MV without APPEND must be critical, got: {}",
        describe(&reports)
    );
    assert_eq!(
        drift_value(&m, MV_DRIFT_METRIC),
        0.0,
        "and must NOT also inflate the ordinary count — one object, one alarm"
    );
}

/// The grant-gap discriminator, end to end. A database the probe cannot see
/// yields no visible objects, and the counts must be suppressed rather than
/// published as "every MV is missing" — which would page as if the whole rollup
/// chain had been deleted.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_invisible_database_suppresses_the_counts_instead_of_paging() {
    let c = client().await;
    let visible: u64 = c
        .query(&visible_objects_query("no_such_database"))
        .fetch_one()
        .await
        .expect("counting an absent database is not an error");
    assert_eq!(visible, 0);

    // Every MV reports Missing against a database that holds none of them.
    let reports = prices_clickhouse::drift::check_rollup_drift(&c, "no_such_database")
        .await
        .expect("drift check executes");
    assert!(
        reports.iter().all(|r| r.needs_attention()),
        "all MVs should look missing here — that is the ambiguity being handled"
    );

    let m = drift_metrics(&reports, visible);
    assert_eq!(drift_value(&m, MV_DRIFT_UNREADABLE_METRIC), 1.0);
    assert_eq!(
        drift_value(&m, MV_DRIFT_METRIC),
        0.0,
        "must not page as if the rollup chain were deleted"
    );
}

// ---- current_prices writer liveness (task 0243) ------------------------------
//
// Both tests run in a scratch database built from the real schema, so they never
// TRUNCATE anything in `prices.*`, cannot race each other, and are immune to
// whatever a local `prices.mv_current_prices` happens to be doing.

fn scratch_rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// A fresh scratch database holding the full `init.sql` schema, and a client
/// bound to it — so the exact production query (unqualified table name) resolves.
async fn scratch_db(db: &str) -> Client {
    let admin = Client::default().with_url(ch_url());
    exec(&admin, &format!("DROP DATABASE IF EXISTS {db}")).await;
    exec(&admin, &format!("CREATE DATABASE {db}")).await;
    prices_clickhouse::apply_sql(&admin, &scratch_rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("init schema");
    Client::default().with_url(ch_url()).with_database(db)
}

async fn drop_scratch_db(db: &str) {
    let admin = Client::default().with_url(ch_url());
    let _ = admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await;
}

async fn current_prices_age(
    c: &Client,
) -> rollup_freshness_probe::current_prices::CurrentPricesAge {
    c.query(rollup_freshness_probe::current_prices::current_prices_age_query())
        .fetch_one()
        .await
        .expect("the production current_prices query executes and deserializes")
}

/// Task 0243: the exact production query against a real `current_prices` schema
/// — empty, stale, and holding an unmerged newer version — and the FINAL
/// correction to the task sketch, pinned against the engine rather than argued.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn current_prices_age_query_executes_and_breaches_when_stale_or_empty() {
    use rollup_freshness_probe::EMPTY_TIER_SENTINEL_SECONDS;
    use rollup_freshness_probe::current_prices::{AGE_BOUND_SECONDS, current_prices_metric};

    let db = "it_current_prices_age_0243";
    let c = scratch_db(db).await;

    // Empty: one row comes back even over zero rows (there is no HAVING), and
    // what gets published is the sentinel, not the ~56-year epoch age.
    let empty = current_prices_age(&c).await;
    assert_eq!(empty.row_count, 0);
    let m = current_prices_metric(&empty);
    assert_eq!(m.table, "current_prices");
    assert_eq!(m.value, EMPTY_TIER_SENTINEL_SECONDS as f64);

    // Stale: last rewritten 20 minutes ago, so over the bound and published as-is.
    exec(
        &c,
        "INSERT INTO current_prices (asset_id, updated_at) SELECT 1, now() - INTERVAL 20 MINUTE",
    )
    .await;
    let stale = current_prices_age(&c).await;
    assert_eq!(stale.row_count, 1);
    assert!(
        (1190..=1260).contains(&stale.age_seconds),
        "a row written 20 min ago must read ~1200 s, got {}",
        stale.age_seconds
    );
    assert!(stale.age_seconds > AGE_BOUND_SECONDS);
    assert_eq!(
        current_prices_metric(&stale).value,
        stale.age_seconds as f64
    );

    // FINAL invariance: a newer, still unmerged version of the same asset must be
    // the one measured, with or without FINAL — the version column IS updated_at.
    exec(
        &c,
        "INSERT INTO current_prices (asset_id, updated_at) SELECT 1, now() - INTERVAL 30 SECOND",
    )
    .await;
    let plain = current_prices_age(&c).await.age_seconds;
    let with_final: i64 = c
        .query(
            "SELECT toInt64(toUnixTimestamp(now()) - toUnixTimestamp(max(updated_at))) \
             FROM current_prices FINAL",
        )
        .fetch_one()
        .await
        .expect("FINAL reading");
    assert!(
        (plain - with_final).abs() <= 1,
        "without FINAL {plain} s, with FINAL {with_final} s"
    );
    assert!(
        (25..=45).contains(&plain),
        "the newer version must be the one measured, got {plain} s"
    );

    drop_scratch_db(db).await;
}

/// Task 0243: the link the alarm rests on, end to end on the pinned engine.
/// While `mv_current_prices` runs, the age stays low; once it is STOPPED the age
/// grows one-for-one with the clock and the rows stay put; START + REFRESH bring
/// it back. It doubles as a rehearsal of the production commands in task 0283.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_stopped_mv_current_prices_freezes_updated_at_and_its_age_grows() {
    let db = "it_current_prices_mv_0243";
    let c = scratch_db(db).await;

    // The real MV, with its 1-minute schedule shortened so the test runs in seconds.
    let original = scratch_rewrite(prices_clickhouse::CURRENT_SQL, db);
    let mv_sql = original.replace("REFRESH EVERY 1 MINUTE", "REFRESH EVERY 2 SECOND");
    assert_ne!(mv_sql, original, "the schedule swap must apply");
    let mv_client = Client::default()
        .with_url(ch_url())
        .with_option("allow_experimental_refreshable_materialized_view", "1");
    prices_clickhouse::apply_sql(&mv_client, &mv_sql)
        .await
        .expect("create mv_current_prices");

    // One priced candle, so the MV has a row to write.
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES (now(), 1, 2, 'sdex', 2, 2, 2, 2, 50, 100, 100, 2, 2, 1, 1)"
        ),
    )
    .await;
    exec(&c, &format!("SYSTEM REFRESH VIEW {db}.mv_current_prices")).await;

    // Running: the writer keeps rewriting, so the age stays within a few seconds.
    let mut running = None;
    for _ in 0..40 {
        let a = current_prices_age(&c).await;
        if a.row_count > 0 {
            running = Some(a);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    let running = running.expect("mv_current_prices did not populate current_prices in time");
    assert!(
        running.age_seconds <= 4,
        "a running writer keeps the age low, got {} s",
        running.age_seconds
    );

    // Stopped: the table keeps its rows and the age climbs with the clock.
    exec(&c, &format!("SYSTEM STOP VIEW {db}.mv_current_prices")).await;
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let a0 = current_prices_age(&c).await;
    tokio::time::sleep(std::time::Duration::from_secs(6)).await;
    let a1 = current_prices_age(&c).await;
    assert_eq!(
        a1.row_count, a0.row_count,
        "a stopped writer leaves its last rows in place"
    );
    assert!(
        a1.age_seconds >= a0.age_seconds + 5,
        "the age must grow once the writer stops: {} s -> {} s",
        a0.age_seconds,
        a1.age_seconds
    );

    // Restarted: START + REFRESH bring the age back down.
    exec(&c, &format!("SYSTEM START VIEW {db}.mv_current_prices")).await;
    exec(&c, &format!("SYSTEM REFRESH VIEW {db}.mv_current_prices")).await;
    let mut recovered = false;
    for _ in 0..40 {
        if current_prices_age(&c).await.age_seconds <= 4 {
            recovered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(recovered, "START + REFRESH did not bring the age back down");

    drop_scratch_db(db).await;
}

// ---- ADR 0292 / task 0151: the stored-data invariants of the zero sentinel ----

/// One `price_ohlcv_1m` row with every column the invariants read spelled out —
/// the pf column above all: left to its DEFAULT (`trade_count`) it would turn
/// the dust-only fixture into a healthy row.
///
/// `ts` is a unix timestamp fixed by the caller, NOT `now()` evaluated per
/// insert: the table's key is `(asset_id, quote_asset_id, source, timestamp)`,
/// so a repair only supersedes the row it repairs if it lands on the same one.
async fn insert_invariant_row(
    c: &Client,
    ts: u32,
    asset_id: u32,
    (close, close_usd, pf): (&str, &str, u32),
    version: u32,
) {
    exec(
        c,
        &format!(
            "INSERT INTO prices.price_ohlcv_1m \
               (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
                version, pf_trade_count, pf_volume, pf_price_volume) \
             SELECT toDateTime({ts}), {asset_id}, 2, 'sdex', \
                    {close}, {close}, {close}, {close}, 1, 1, 0, {close_usd}, 1, 3, {version}, \
                    {pf}, {pf}, {pf}"
        ),
    )
    .await;
}

async fn read_zero_invariants(
    c: &Client,
) -> rollup_freshness_probe::zero_invariants::ZeroInvariantCounts {
    c.query(&rollup_freshness_probe::zero_invariants::zero_invariant_query())
        .fetch_one()
        .await
        .expect("the invariant query executes and deserializes")
}

/// The assertion must **execute and deserialize** on the production build, and
/// count exactly the rows that break an invariant — no healthy shape among them.
/// The two healthy rows are the ones a careless predicate would flag: a priced
/// candle not yet enriched (`close_usd = 0` is meaning 1, not a violation) and
/// a dust-only candle (`close = 0` is CORRECT when `pf_trade_count = 0`).
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_zero_invariant_scan_counts_only_rows_that_break_an_invariant() {
    use rollup_freshness_probe::zero_invariants::{ZeroInvariantCounts, zero_invariant_metric};

    let c = client().await;
    reset_sanity_tables(&c).await;
    let ts: u32 = c
        .query("SELECT toUnixTimestamp(now() - INTERVAL 2 MINUTE)")
        .fetch_one()
        .await
        .unwrap();

    insert_invariant_row(&c, ts, 10, ("5", "0", 3), 1).await; // priced, pending enrichment
    insert_invariant_row(&c, ts, 11, ("0", "0", 0), 1).await; // dust-only: no price, correctly
    insert_invariant_row(&c, ts, 12, ("5", "0", 0), 1).await; // ⛔ no price-forming fill, yet a close
    insert_invariant_row(&c, ts, 13, ("0", "3", 3), 1).await; // ⛔ a USD close without a close

    let counts = read_zero_invariants(&c).await;
    assert_eq!(
        counts,
        ZeroInvariantCounts {
            violations: 2,
            scanned: 4
        }
    );
    assert_eq!(zero_invariant_metric(&counts).unwrap().value, 2.0);

    reset_sanity_tables(&c).await;
}

/// The writer defect the alarm names as its usual cause, reproduced as a writer
/// would commit it: a statement that OMITS `pf_trade_count`. The column then
/// takes its DEFAULT (`trade_count`), so a dust-only minute — `close = 0` — is
/// stored claiming five price-forming fills (ADR 0287's trap). Neither of the
/// first two invariants can see it: one needs `pf_trade_count = 0`, the other
/// `close_usd > 0`. RED without the third, `pf_trade_count > 0 ⇒ close > 0`.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_dust_minute_written_without_its_pf_column_is_a_zero_invariant_violation() {
    use rollup_freshness_probe::zero_invariants::ZeroInvariantCounts;

    let c = client().await;
    reset_sanity_tables(&c).await;

    // No pf_trade_count, pf_volume or pf_price_volume in the column list.
    exec(
        &c,
        "INSERT INTO prices.price_ohlcv_1m \
           (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
            volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
         SELECT now() - INTERVAL 2 MINUTE, 14, 2, 'sdex', 0, 0, 0, 0, 5, 5, 0, 0, 0, 5, 1",
    )
    .await;

    assert_eq!(
        read_zero_invariants(&c).await,
        ZeroInvariantCounts {
            violations: 1,
            scanned: 1
        },
        "a candle that claims price-forming fills must carry a price"
    );

    reset_sanity_tables(&c).await;
}

/// `FINAL` is the alarm's whole recovery path: an operator repairs a violating
/// candle by re-inserting it at a higher `version`, and the count must DROP.
/// RED without `FINAL`: the superseded row is still read, so the repair adds a
/// scanned row and clears nothing — a page that latches after the data is fixed.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_repaired_candle_stops_counting_as_a_zero_invariant_violation() {
    use rollup_freshness_probe::zero_invariants::ZeroInvariantCounts;

    let c = client().await;
    reset_sanity_tables(&c).await;
    let ts: u32 = c
        .query("SELECT toUnixTimestamp(now() - INTERVAL 2 MINUTE)")
        .fetch_one()
        .await
        .unwrap();

    insert_invariant_row(&c, ts, 12, ("5", "0", 0), 1).await; // ⛔ the violation
    insert_invariant_row(&c, ts, 13, ("0", "3", 3), 1).await; // ⛔ a second, left unrepaired
    insert_invariant_row(&c, ts, 12, ("0", "0", 0), 2).await; // the repair of the first

    assert_eq!(
        read_zero_invariants(&c).await,
        ZeroInvariantCounts {
            violations: 1,
            scanned: 2
        },
        "a repair at a higher version must clear its violation, not add a row to the scan"
    );

    reset_sanity_tables(&c).await;
}

/// The window is a claim about scope, not only about cost: a legacy row written
/// before task 0286 is out of scope until its phase 3 re-ingests the history
/// (ADR 0292), and must neither page nor pad `scanned`. RED without the `WHERE`.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_violation_older_than_the_window_is_out_of_the_zero_invariant_scan() {
    use rollup_freshness_probe::zero_invariants::{
        ZERO_INVARIANT_LOOKBACK_SECONDS, ZeroInvariantCounts,
    };

    let c = client().await;
    reset_sanity_tables(&c).await;
    let now: u32 = c
        .query("SELECT toUnixTimestamp(now())")
        .fetch_one()
        .await
        .unwrap();
    let inside = now - 120;
    let outside = now - ZERO_INVARIANT_LOOKBACK_SECONDS as u32 - 3_600;

    insert_invariant_row(&c, inside, 10, ("5", "0", 3), 1).await; // healthy, in the window
    insert_invariant_row(&c, outside, 12, ("5", "0", 0), 1).await; // ⛔ but an hour past it

    assert_eq!(
        read_zero_invariants(&c).await,
        ZeroInvariantCounts {
            violations: 0,
            scanned: 1
        }
    );

    reset_sanity_tables(&c).await;
}

// ---- Rollup MVs stuck behind a dependency (task 0203 / 0143) ----------------
//
// Since task 0143 the rollup MVs are chained with `DEPENDS ON`. A stopped,
// failing or missing dependency leaves its dependents `WaitingForDependencies`
// forever, with no error (BRIEF §2). These pin the probe's read of that state
// against a live 26.3.10.60 scheduler, and its refusal to publish a 0 when the
// table is denied.

/// One declared view's `(status, next_refresh_time, last_success_time)` as
/// epoch seconds, straight from `system.view_refreshes`.
async fn view_state(c: &Client, db: &str, view: &str) -> (String, i64, i64) {
    c.query(&format!(
        "SELECT toString(status), \
                toInt64(toUnixTimestamp(ifNull(next_refresh_time, toDateTime(0)))), \
                toInt64(toUnixTimestamp(ifNull(last_success_time, toDateTime(0)))) \
         FROM system.view_refreshes WHERE database = '{db}' AND view = '{view}'"
    ))
    .fetch_one::<(String, i64, i64)>()
    .await
    .unwrap_or_else(|e| panic!("{view} is listed in system.view_refreshes: {e}"))
}

/// ⚠️ **Induce the condition.** STOP `mv_ohlcv_1h_to_4h`, then move
/// `mv_ohlcv_4h_to_1d`'s scheduler just past its next 4-hour slot with
/// `SYSTEM TEST VIEW … SET FAKE TIME`. The dependent fires for that slot, finds
/// its dependency has not refreshed for it, and waits — forever, with no error.
/// The probe's own query must see it `WaitingForDependencies` and its
/// dependency `Disabled`.
///
/// ⚠️ The classifier is handed the FAKE clock (`slot + period + 1`), not the
/// row's `db_now_unix`: fake time moves only the view's scheduler, not `now()`,
/// so the wait has not yet aged in wall time. A real stall reaches the same
/// state by waiting out one period of wall time — which is what the probe's
/// server clock measures in production.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_stopped_dependency_is_reported_as_waiting_and_disabled() {
    use rollup_freshness_probe::refresh_waits::{
        MV_REFRESH_DISABLED_METRIC, MV_REFRESH_FAILING_METRIC, MV_REFRESH_UNREADABLE_METRIC,
        MV_REFRESH_WAITING_METRIC, ViewRefreshRow, refresh_wait_metrics, refresh_waits_query,
    };

    const DEPENDENT: &str = "mv_ohlcv_4h_to_1d";
    const DEPENDENCY: &str = "mv_ohlcv_1h_to_4h";
    const PERIOD: i64 = 14_400; // mv_ohlcv_4h_to_1d REFRESH EVERY 4 HOUR

    let db = "it_refresh_waits_0203";
    let c = scratch_db(db).await;
    let rmv = Client::default()
        .with_url(ch_url())
        .with_option("allow_experimental_refreshable_materialized_view", "1");
    prices_clickhouse::apply_sql(&rmv, &scratch_rewrite(prices_clickhouse::ROLLUPS_SQL, db))
        .await
        .expect("apply the generated rollup chain");

    // Let the CREATE-time refreshes settle, then switch the dependency off.
    for view in [DEPENDENCY, DEPENDENT] {
        exec(&rmv, &format!("SYSTEM WAIT VIEW {db}.{view}")).await;
    }
    exec(&rmv, &format!("SYSTEM STOP VIEW {db}.{DEPENDENCY}")).await;

    // The dependent's next slot on the real clock, and a fake clock 30 s past
    // it (FAKE TIME takes a string literal; the server renders it in its TZ).
    let (_, slot, _) = view_state(&c, db, DEPENDENT).await;
    assert!(slot > 0, "{DEPENDENT} has a scheduled next refresh");
    let fake: String = c
        .query(&format!("SELECT toString(toDateTime({}))", slot + 30))
        .fetch_one()
        .await
        .expect("render the fake time");
    exec(
        &rmv,
        &format!("SYSTEM TEST VIEW {db}.{DEPENDENT} SET FAKE TIME '{fake}'"),
    )
    .await;

    // Poll until the dependent either waits or (without DEPENDS ON) runs.
    let mut state = view_state(&c, db, DEPENDENT).await;
    for _ in 0..80 {
        if state.0 == "WaitingForDependencies" || state.2 >= slot {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        state = view_state(&c, db, DEPENDENT).await;
    }

    // Collect everything before asserting, and drop the scratch DB (its MVs
    // keep firing) before any assertion can unwind past the cleanup.
    let rows = c
        .query(&refresh_waits_query(db))
        .fetch_all::<ViewRefreshRow>()
        .await;
    drop_scratch_db(db).await;
    let rows = rows.expect("the probe's refresh-waits query executes and deserializes");

    let find = |view: &str| {
        rows.iter()
            .find(|r| r.view == view)
            .unwrap_or_else(|| panic!("{view} is among the probe's rows: {rows:?}"))
    };
    let dependent = find(DEPENDENT);
    assert_eq!(
        dependent.status, "WaitingForDependencies",
        "with its dependency STOPped, {DEPENDENT} must wait for slot {slot} instead of \
         running (last_success_time {}); rows: {rows:?}",
        state.2
    );
    assert_eq!(
        dependent.next_refresh_unix, slot,
        "a waiting view keeps the slot it waits for as next_refresh_time"
    );
    assert_eq!(find(DEPENDENCY).status, "Disabled");
    assert_eq!(rows.len(), 12, "all twelve declared views are listed");

    let value_of = |m: &[rollup_freshness_probe::mv_drift::DriftMetric], name: &str| {
        m.iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("{name} published"))
            .value
    };

    // A whole period later on the scheduler's clock: stuck, and the STOP shows.
    let m = refresh_wait_metrics(&rows, slot + PERIOD + 1);
    assert!(value_of(&m, MV_REFRESH_WAITING_METRIC) >= 1.0, "{m:?}");
    assert!(value_of(&m, MV_REFRESH_DISABLED_METRIC) >= 1.0, "{m:?}");
    assert_eq!(value_of(&m, MV_REFRESH_UNREADABLE_METRIC), 0.0);

    // The threshold, on the two views this test controls. The other ten run
    // on the REAL clock, so judging them against a fake "now" hours ahead could
    // count an ordinary momentary wait of theirs.
    let pair: Vec<ViewRefreshRow> = rows
        .iter()
        .filter(|r| r.view == DEPENDENT || r.view == DEPENDENCY)
        .cloned()
        .collect();
    assert_eq!(
        value_of(
            &refresh_wait_metrics(&pair, slot + PERIOD + 1),
            MV_REFRESH_WAITING_METRIC
        ),
        1.0
    );
    // Review WR-07: one stall is counted once — the waiting view's and the
    // STOPped view's stale successes belong to their own counts, not failing.
    // Judged where both successes ARE stale (more than two own periods old),
    // so only the waiting/disabled exclusion keeps them out of the count.
    let last_success = |view: &str| {
        find(view)
            .last_success_unix
            .unwrap_or_else(|| panic!("{view} succeeded at CREATE: {rows:?}"))
    };
    let stale_now = (slot + PERIOD + 1)
        .max(last_success(DEPENDENT) + 2 * PERIOD + 1)
        .max(last_success(DEPENDENCY) + 2 * 3_600 + 1);
    let m = refresh_wait_metrics(&pair, stale_now);
    assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), 1.0, "{m:?}");
    assert_eq!(value_of(&m, MV_REFRESH_DISABLED_METRIC), 1.0, "{m:?}");
    assert_eq!(
        value_of(&m, MV_REFRESH_FAILING_METRIC),
        0.0,
        "a waiting or STOPped view is never also failing, however old its last success: {m:?}"
    );
    for not_yet in [slot + 1, slot + PERIOD] {
        assert_eq!(
            value_of(
                &refresh_wait_metrics(&pair, not_yet),
                MV_REFRESH_WAITING_METRIC
            ),
            0.0,
            "a wait of at most one period (now = slot + {}) is an ordinary slot",
            not_yet - slot
        );
    }
}

/// ⚠️ **Induce the condition** (review WR-07). The two monthly LEAVES —
/// `mv_ohlcv_1d_to_1M` and `mv_reconcile_1d_to_1M` — fail on every pass: the
/// column `vwap` of their shared target is renamed away, so the `INSERT` each
/// pass makes into it fails at analysis, whatever the data and whatever the
/// clock (a data-driven failure would need a CLOSED month inside the 7-day
/// reconcile window, which exists only in a month's first week). Nothing
/// `DEPENDS ON` a leaf, so neither makes anything wait: after its retries
/// ClickHouse puts it back to `Scheduled` with the error in `exception`. The
/// waiting and disabled counts therefore read 0 — the gap — and only the
/// failing count, from the probe's exact read and classification, sees them.
///
/// Real clock throughout (no fake time), so the row's own `db_now_unix` is
/// the probe's `now`, exactly as in production.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_failing_leaf_is_reported_as_failing_though_nothing_waits_on_it() {
    use rollup_freshness_probe::refresh_waits::{
        MV_REFRESH_DISABLED_METRIC, MV_REFRESH_FAILING_METRIC, MV_REFRESH_UNREADABLE_METRIC,
        MV_REFRESH_WAITING_METRIC, ViewRefreshRow, describe_failing, metrics_for_read,
        refresh_waits_query,
    };

    const LEAVES: [&str; 2] = ["mv_ohlcv_1d_to_1M", "mv_reconcile_1d_to_1M"];

    let db = "it_refresh_failing_0203";
    let c = scratch_db(db).await;
    let rmv = Client::default()
        .with_url(ch_url())
        .with_option("allow_experimental_refreshable_materialized_view", "1");
    prices_clickhouse::apply_sql(&rmv, &scratch_rewrite(prices_clickhouse::ROLLUPS_SQL, db))
        .await
        .expect("apply the generated rollup chain");
    for (view, _) in prices_clickhouse::rollup_sql::rollup_views() {
        exec(&rmv, &format!("SYSTEM WAIT VIEW {db}.{view}")).await;
    }

    exec(
        &c,
        &format!("ALTER TABLE {db}.price_ohlcv_1M RENAME COLUMN vwap TO vwap_renamed_by_it"),
    )
    .await;
    for view in LEAVES {
        // SYSTEM REFRESH VIEW ignores DEPENDS ON, so each leaf runs now.
        exec(&rmv, &format!("SYSTEM REFRESH VIEW {db}.{view}")).await;
    }

    // Poll until both leaves are back to Scheduled with their error (retries
    // spent), or give up after ~20 s and let the assertions say what is left.
    let mut rows: Result<Vec<ViewRefreshRow>, clickhouse::error::Error> = Ok(vec![]);
    for _ in 0..80 {
        rows = c
            .query(&refresh_waits_query(db))
            .fetch_all::<ViewRefreshRow>()
            .await;
        let settled = rows.as_ref().is_ok_and(|rows| {
            LEAVES.iter().all(|leaf| {
                rows.iter()
                    .any(|r| r.view == *leaf && r.status == "Scheduled" && !r.exception.is_empty())
            })
        });
        if settled {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }

    // Everything collected; drop the scratch DB (its MVs keep firing) before
    // any assertion can unwind past the cleanup.
    drop_scratch_db(db).await;
    let rows = rows.expect("the probe's refresh-waits query executes and deserializes");
    assert_eq!(
        rows.len(),
        12,
        "all twelve declared views are listed: {rows:?}"
    );

    for leaf in LEAVES {
        let row = rows
            .iter()
            .find(|r| r.view == leaf)
            .unwrap_or_else(|| panic!("{leaf} is among the probe's rows"));
        assert_eq!(
            row.status, "Scheduled",
            "{leaf}: a failing leaf is Scheduled, not waiting — the state no \
             other count can see; row: {row:?}"
        );
        assert!(
            row.exception.contains("vwap"),
            "{leaf}: the induced failure is the one recorded: {row:?}"
        );
    }

    let value_of = |m: &[DriftMetric], name: &str| {
        m.iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("{name} published: {m:?}"))
            .value
    };
    let detail = describe_failing(&rows, rows[0].db_now_unix);
    let m =
        metrics_for_read::<clickhouse::error::Error>(Ok(rows)).expect("a readable table publishes");
    assert_eq!(
        value_of(&m, MV_REFRESH_FAILING_METRIC),
        2.0,
        "exactly the two failing leaves: {detail}"
    );
    assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), 0.0, "{m:?}");
    assert_eq!(value_of(&m, MV_REFRESH_DISABLED_METRIC), 0.0, "{m:?}");
    assert_eq!(value_of(&m, MV_REFRESH_UNREADABLE_METRIC), 0.0, "{m:?}");
    for leaf in LEAVES {
        assert!(
            detail.contains(&format!("{leaf}: Code: ")),
            "the log detail names {leaf} and its error: {detail}"
        );
    }
}

/// `system.view_refreshes` is DENIED — not grant-filtered like `system.tables`
/// — to a user holding only `SELECT ON prices.*`, the shape of the probe's
/// `prices_writer` identity (measured on 26.3.10.60, RESEARCH §3). The probe's
/// exact read must come back as the unreadable flag ALONE: a waiting/disabled/failing
/// 0 would read as a healthy chain the probe cannot see.
///
/// Creates and drops its own least-privileged user; both results are
/// collected and the user dropped before anything can panic.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn view_refreshes_is_denied_to_a_prices_only_user_and_reads_as_unreadable() {
    use rollup_freshness_probe::refresh_waits::{
        ViewRefreshRow, is_access_denied, metrics_for_read, refresh_waits_query, unreadable_metrics,
    };

    let admin = client().await;
    exec(&admin, "DROP USER IF EXISTS rollup_probe_waits_it").await;
    exec(
        &admin,
        "CREATE USER rollup_probe_waits_it IDENTIFIED WITH no_password",
    )
    .await;
    exec(&admin, "GRANT SELECT ON prices.* TO rollup_probe_waits_it").await;

    let restricted = Client::default()
        .with_url(ch_url())
        .with_database("prices")
        .with_user("rollup_probe_waits_it");

    let read = restricted
        .query(&refresh_waits_query("prices"))
        .fetch_all::<ViewRefreshRow>()
        .await
        .map_err(|e| e.to_string());
    // The control: the same user CAN read the grant-filtered system.tables.
    let tables: Result<u64, _> = restricted
        .query("SELECT count() FROM system.tables WHERE database = 'prices'")
        .fetch_one()
        .await;

    exec(&admin, "DROP USER IF EXISTS rollup_probe_waits_it").await;

    assert!(
        tables.expect("system.tables is filtered, not denied") > 0,
        "the control: a prices-only user sees its own schema"
    );
    let err = read
        .clone()
        .expect_err("system.view_refreshes must be denied to a prices-only user");
    assert!(
        is_access_denied(&err),
        "the refusal must be recognised as a grant gap, got: {err}"
    );
    assert_eq!(
        metrics_for_read(read),
        Ok(unreadable_metrics()),
        "a denied read publishes the unreadable flag and no count"
    );
}

// ---- Task 0139: asset-id uniqueness and orphan candles -----------------------
//
// Scratch databases built from the real schema, so the probe's unqualified
// queries resolve exactly as on prod and nothing in `prices.*` is touched.

const FOO: AssetFixture = AssetFixture::new("FOO", "classic", "GFOO", "");
const USDC: AssetFixture = AssetFixture::new("USDC", "classic", prices_clickhouse::USDC_ISSUER, "");

async fn read_asset_ids(c: &Client) -> rollup_freshness_probe::asset_id_uniqueness::AssetIdCounts {
    c.query(&rollup_freshness_probe::asset_id_uniqueness::collisions_query())
        .fetch_one()
        .await
        .expect("the collision query executes and deserializes")
}

async fn read_orphans(
    c: &Client,
) -> rollup_freshness_probe::asset_id_uniqueness::OrphanCandleCounts {
    c.query(&rollup_freshness_probe::asset_id_uniqueness::orphan_candles_query())
        .fetch_one()
        .await
        .expect("the orphan query executes and deserializes")
}

/// One `_1m` candle `mins_ago` minutes old. `base` / `quote` are SQL id
/// expressions: a fixture asset (`{FOO}`) or a bare number no row carries.
async fn insert_id_candle(c: &Client, base: impl Display, quote: impl Display, mins_ago: u32) {
    exec(
        c,
        &format!(
            "INSERT INTO price_ohlcv_1m \
               (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             SELECT now() - INTERVAL {mins_ago} MINUTE, {base}, {quote}, 'sdex', \
                    1, 1, 1, 1, 1, 1, 0, 0, 1, 1, 1"
        ),
    )
    .await;
}

/// A registry whose ids are derived from the identity reads 0 collisions, and
/// candles on those ids read 0 orphans.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_derived_registry_reads_no_collision_and_no_orphan() {
    use rollup_freshness_probe::asset_id_uniqueness::{
        AssetIdCounts, OrphanCandleCounts, collisions_metric, orphan_candles_metric,
    };

    let db = "it_probe_0139_clean";
    let c = scratch_db(db).await;
    exec(&c, &assets_insert(db, &[FOO, USDC])).await;
    insert_id_candle(&c, FOO, USDC, 2).await;

    let ids = read_asset_ids(&c).await;
    assert_eq!(
        ids,
        AssetIdCounts {
            identities: 2,
            ids: 2
        }
    );
    assert_eq!(collisions_metric(&ids).unwrap().value, 0.0);
    let orphans = read_orphans(&c).await;
    assert_eq!(
        orphans,
        OrphanCandleCounts {
            orphans: 0,
            scanned: 1
        }
    );
    assert_eq!(orphan_candles_metric(&orphans).unwrap().value, 0.0);

    drop_scratch_db(db).await;
}

/// Two identities on one id read as one collision. The registry here has the
/// pre-0139 shape (a plain id column, as prod's `assets` holds until the
/// window), because the derived schema cannot store a shared id at all.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn one_id_shared_by_two_identities_is_one_collision() {
    use rollup_freshness_probe::asset_id_uniqueness::collisions_metric;

    let db = "it_probe_0139_collision";
    let c = scratch_db(db).await;
    exec(&c, "DROP TABLE assets").await;
    exec(
        &c,
        "CREATE TABLE assets ( \
             asset_id UInt32, asset_code String, issuer_address String, \
             contract_address String, updated_at DateTime DEFAULT now()) \
         ENGINE = ReplacingMergeTree(updated_at) \
         ORDER BY (asset_code, issuer_address, contract_address)",
    )
    .await;
    // 4194's shape on prod: STW and ARBRIDGE on one id, beside a clean one.
    exec(
        &c,
        "INSERT INTO assets (asset_id, asset_code, issuer_address, contract_address) VALUES \
         (4194, 'STW', 'GA2L', ''), (4194, 'ARBRIDGE', 'GBAC', ''), (3, 'USDC', 'GA5Z', '')",
    )
    .await;

    let ids = read_asset_ids(&c).await;
    assert_eq!((ids.identities, ids.ids), (3, 2));
    assert_eq!(collisions_metric(&ids).unwrap().value, 1.0);

    drop_scratch_db(db).await;
}

/// A candle naming an id with no `assets` row is an orphan, on either leg; one
/// older than the window is not counted.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_candle_on_an_unregistered_id_is_an_orphan_on_either_leg() {
    use rollup_freshness_probe::asset_id_uniqueness::orphan_candles_metric;

    let db = "it_probe_0139_orphan";
    let c = scratch_db(db).await;
    exec(&c, &assets_insert(db, &[FOO, USDC])).await;
    insert_id_candle(&c, FOO, USDC, 2).await;
    insert_id_candle(&c, 999, USDC, 3 * 60).await; // outside the 2 h window

    insert_id_candle(&c, 999, USDC, 2).await;
    let orphans = read_orphans(&c).await;
    assert_eq!((orphans.orphans, orphans.scanned), (1, 2), "base leg");
    assert_eq!(orphan_candles_metric(&orphans).unwrap().value, 1.0);

    insert_id_candle(&c, FOO, 998, 2).await;
    let orphans = read_orphans(&c).await;
    assert_eq!((orphans.orphans, orphans.scanned), (2, 3), "quote leg");

    drop_scratch_db(db).await;
}

/// An empty registry and an empty window are unreadable: both reads succeed,
/// and both readings are refused rather than published as a healthy 0.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_empty_registry_or_window_is_refused_as_unreadable() {
    use rollup_freshness_probe::asset_id_uniqueness::{
        UniquenessRefusal, collisions_metric, orphan_candles_metric,
    };

    let db = "it_probe_0139_empty";
    let c = scratch_db(db).await;

    assert_eq!(
        collisions_metric(&read_asset_ids(&c).await),
        Err(UniquenessRefusal::EmptyRegistry)
    );
    assert_eq!(
        orphan_candles_metric(&read_orphans(&c).await),
        Err(UniquenessRefusal::EmptyWindow)
    );

    // Candles only outside the window: still nothing measured.
    exec(&c, &assets_insert(db, &[FOO, USDC])).await;
    insert_id_candle(&c, FOO, USDC, 3 * 60).await;
    assert_eq!(
        orphan_candles_metric(&read_orphans(&c).await),
        Err(UniquenessRefusal::EmptyWindow)
    );

    drop_scratch_db(db).await;
}

// ---- task 0236: stored candles outside the OHLC band, on all seven tiers ----

/// Clear every tier of `OHLC_BAND_TIERS`, in its order: `_1m` FIRST, then the
/// coarse tiers finest to coarsest. The rollup MVs run in this database:
/// clearing a source before its target stops a refresh landing between the two
/// from re-feeding the tier just cleared. Its own helper — [`reset_sanity_tables`]
/// is shared and clears only two tiers.
async fn reset_ohlc_band_tables(c: &Client) {
    for (table, _) in rollup_freshness_probe::ohlc_band::OHLC_BAND_TIERS {
        exec(c, &format!("TRUNCATE TABLE prices.{table}")).await;
    }
}

/// One candle on any tier with its four prices spelled out, each as an exact
/// `Decimal` literal — no Float64 path, so a fixture one ulp off the band is
/// stored as written. Every other column as in [`insert_invariant_row`]:
/// `pf_trade_count` explicit, never left to its DEFAULT.
///
/// `ts` is a unix timestamp fixed by the caller, so a repair at a higher
/// `version` lands on the same key as the row it supersedes.
async fn insert_ohlc_row(
    c: &Client,
    table: &str,
    ts: u32,
    asset_id: u32,
    (open, high, low, close): (&str, &str, &str, &str),
    pf: u32,
    version: u32,
) {
    exec(
        c,
        &format!(
            "INSERT INTO prices.{table} \
               (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
                version, pf_trade_count, pf_volume, pf_price_volume) \
             SELECT toDateTime({ts}), {asset_id}, 2, 'sdex', \
                    toDecimal128('{open}', 14), toDecimal128('{high}', 14), \
                    toDecimal128('{low}', 14), toDecimal128('{close}', 14), \
                    1, 1, 0, 0, 1, 3, {version}, {pf}, {pf}, {pf}"
        ),
    )
    .await;
}

/// Run every production query — `ohlc_band_queries()` itself, never a copy.
async fn read_ohlc_band(
    c: &Client,
) -> Vec<(
    &'static str,
    rollup_freshness_probe::ohlc_band::OhlcBandCounts,
)> {
    let mut readings = Vec::new();
    for (table, sql) in rollup_freshness_probe::ohlc_band::ohlc_band_queries() {
        let counts = c
            .query(&sql)
            .fetch_one()
            .await
            .unwrap_or_else(|e| panic!("the ohlc-band query for {table} executes: {e}"));
        readings.push((table, counts));
    }
    readings
}

/// A server-side unix timestamp for `now() - <interval>`, computed ONCE so every
/// fixture of a test shares it.
async fn ts_ago(c: &Client, interval: &str) -> u32 {
    c.query(&format!("SELECT toUnixTimestamp(now() - {interval})"))
        .fetch_one()
        .await
        .unwrap()
}

fn reading(
    readings: &[(
        &'static str,
        rollup_freshness_probe::ohlc_band::OhlcBandCounts,
    )],
    table: &str,
) -> rollup_freshness_probe::ohlc_band::OhlcBandCounts {
    readings
        .iter()
        .find(|(t, _)| *t == table)
        .unwrap_or_else(|| panic!("{table} was read"))
        .1
}

/// The seven queries must **execute and deserialize** on the production build,
/// and count exactly the rows outside the band — none of the healthy shapes a
/// careless predicate flags: a single-trade candle (O=H=L=C), a dust-only
/// candle (all four 0 with `pf_trade_count = 0`, correct per ADR 0287), and a
/// priced candle still pending enrichment (`close_usd = 0`). The row breaking
/// BOTH shapes (`open = 0` with a positive low) counts ONCE — RED if the band
/// class loses its four-positive-prices gate.
///
/// `_1m` fixtures sit at `now() - 3 HOUR`: inside the probe's 2-day window,
/// outside the 1m→15m MV's 2-hour window, so no refresh copies them upward
/// mid-test and every coarse tier reads exactly zero.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_ohlc_band_scan_counts_only_rows_outside_the_band_on_every_tier() {
    use rollup_freshness_probe::ohlc_band::{OHLC_BAND_TIERS, OhlcBandCounts, ohlc_band_metric};

    let c = client().await;
    reset_ohlc_band_tables(&c).await;
    let ts = ts_ago(&c, "INTERVAL 3 HOUR").await;
    let m = "price_ohlcv_1m";

    insert_ohlc_row(&c, m, ts, 20, ("5", "5", "5", "5"), 1, 1).await; // single trade
    insert_ohlc_row(&c, m, ts, 21, ("0", "0", "0", "0"), 0, 1).await; // dust only
    insert_ohlc_row(&c, m, ts, 22, ("4", "6", "3", "5"), 3, 1).await; // pending enrichment
    insert_ohlc_row(&c, m, ts, 23, ("5", "4.5", "3", "5"), 2, 1).await; // ⛔ high < close
    insert_ohlc_row(&c, m, ts, 24, ("4", "6", "0", "5"), 2, 1).await; // ⛔ low = 0
    insert_ohlc_row(&c, m, ts, 25, ("0", "6", "3", "5"), 2, 1).await; // ⛔ open = 0, low > open

    let readings = read_ohlc_band(&c).await;
    assert_eq!(readings.len(), OHLC_BAND_TIERS.len());
    assert_eq!(
        reading(&readings, m),
        OhlcBandCounts::new(1, 2, 6),
        "_1m: one band violation, two non-positive rows (the double-shaped one counted once)"
    );
    for (table, counts) in &readings[1..] {
        assert_eq!(
            *counts,
            OhlcBandCounts::new(0, 0, 0),
            "{table} holds nothing"
        );
    }
    assert_eq!(ohlc_band_metric(&readings).unwrap().value, 3.0);

    reset_ohlc_band_tables(&c).await;
}

/// Seed the one healthy `_1m` row every arm test needs so the scan is not
/// refused as empty, and return the shared `now() - 3 HOUR` timestamp.
async fn seed_healthy_1m(c: &Client) -> u32 {
    let ts = ts_ago(c, "INTERVAL 3 HOUR").await;
    insert_ohlc_row(c, "price_ohlcv_1m", ts, 30, ("5", "5", "5", "5"), 1, 1).await;
    ts
}

/// The arm `high < greatest(open, close)`, and ONLY that arm: `o4 h4.5 l3 c5`
/// has `low (3) <= least (4)` and `low <= high`. RED without the arm.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_ohlc_band_scan_catches_a_high_below_the_close() {
    let c = client().await;
    reset_ohlc_band_tables(&c).await;
    let ts = seed_healthy_1m(&c).await;
    insert_ohlc_row(&c, "price_ohlcv_1m", ts, 31, ("4", "4.5", "3", "5"), 2, 1).await;

    let readings = read_ohlc_band(&c).await;
    assert_eq!(
        reading(&readings, "price_ohlcv_1m"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(1, 0, 2),
        "high 4.5 below close 5 is outside the band"
    );

    reset_ohlc_band_tables(&c).await;
}

/// The arm `low > least(open, close)`, and ONLY that arm: `o3 h6 l4 c5` has
/// `high (6) >= greatest (5)` and `low <= high`. RED without the arm.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_ohlc_band_scan_catches_a_low_above_the_open() {
    let c = client().await;
    reset_ohlc_band_tables(&c).await;
    let ts = seed_healthy_1m(&c).await;
    insert_ohlc_row(&c, "price_ohlcv_1m", ts, 32, ("3", "6", "4", "5"), 2, 1).await;

    let readings = read_ohlc_band(&c).await;
    assert_eq!(
        reading(&readings, "price_ohlcv_1m"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(1, 0, 2),
        "low 4 above open 3 is outside the band"
    );

    reset_ohlc_band_tables(&c).await;
}

/// `low > high` — `o5 h4 l6 c5` — counted ONCE although it fires all three band
/// arms (countIf counts rows). The arm is implied by the other two (if
/// `low > high`, either `low > least(o, c)`, or `least(o, c) >= low > high` and
/// then `greatest(o, c) > high`), so this goes RED only with all three band arms
/// removed; it pins that the literal invariant is caught and counted once.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_ohlc_band_scan_catches_a_low_above_the_high_once() {
    let c = client().await;
    reset_ohlc_band_tables(&c).await;
    let ts = seed_healthy_1m(&c).await;
    insert_ohlc_row(&c, "price_ohlcv_1m", ts, 33, ("5", "4", "6", "5"), 2, 1).await;

    let readings = read_ohlc_band(&c).await;
    assert_eq!(
        reading(&readings, "price_ohlcv_1m"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(1, 0, 2),
        "a low above the high is one bad row"
    );

    reset_ohlc_band_tables(&c).await;
}

/// `low = 0` beside a positive close — the shape most of the prod baseline was
/// made of, and the one `zero_invariants` cannot see (its invariant 3 reads
/// `close = 0`). Non-positive, not band: the classes are disjoint. RED without
/// `low <= 0`.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_ohlc_band_scan_catches_a_zero_low_beside_a_positive_close() {
    let c = client().await;
    reset_ohlc_band_tables(&c).await;
    let ts = seed_healthy_1m(&c).await;
    insert_ohlc_row(&c, "price_ohlcv_1m", ts, 34, ("4", "6", "0", "5"), 2, 1).await;

    let readings = read_ohlc_band(&c).await;
    assert_eq!(
        reading(&readings, "price_ohlcv_1m"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(0, 1, 2),
        "a priced low of 0 is a non-positive price"
    );

    reset_ohlc_band_tables(&c).await;
}

/// BRIEF point 4, the regression it exists for: a `_1w` bucket starting 5 days
/// ago and a `_1M` bucket starting 12 days ago both overlap the last two days
/// once each is widened by its own length, and a FLAT 2-day window never sees
/// them. RED with the widening removed. Only the terminal sinks `_1w`/`_1M`
/// are used (fed daily from `_1d`, which is empty here), and both sit days away
/// from either window edge.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_ohlc_band_scan_reads_each_coarse_tier_over_its_own_bucket_window() {
    use rollup_freshness_probe::ohlc_band::ohlc_band_metric;

    let c = client().await;
    reset_ohlc_band_tables(&c).await;
    seed_healthy_1m(&c).await;
    let week = ts_ago(&c, "INTERVAL 5 DAY").await;
    let month = ts_ago(&c, "INTERVAL 12 DAY").await;
    insert_ohlc_row(&c, "price_ohlcv_1w", week, 35, ("5", "4.5", "3", "5"), 2, 1).await;
    insert_ohlc_row(&c, "price_ohlcv_1M", month, 36, ("3", "6", "4", "5"), 2, 1).await;

    let readings = read_ohlc_band(&c).await;
    assert_eq!(
        reading(&readings, "price_ohlcv_1w"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(1, 0, 1),
        "a week bucket older than 2 days but inside its widened window"
    );
    assert_eq!(
        reading(&readings, "price_ohlcv_1M"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(1, 0, 1),
        "a month bucket older than 2 days but inside its widened window"
    );
    assert_eq!(
        reading(&readings, "price_ohlcv_1m"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(0, 0, 1)
    );
    assert_eq!(ohlc_band_metric(&readings).unwrap().value, 2.0);

    reset_ohlc_band_tables(&c).await;
}

/// The window has an edge. A `_1m` violation an hour beyond 2 days and a `_1w`
/// violation an hour beyond `2 days + 1 week` are NOT counted — nor scanned.
/// RED with the WHERE removed (the legacy pre-0286 residue would page forever).
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_ohlc_band_scan_ignores_violations_outside_the_window() {
    let c = client().await;
    reset_ohlc_band_tables(&c).await;
    seed_healthy_1m(&c).await;
    let old_minute = ts_ago(&c, "INTERVAL 2 DAY - INTERVAL 1 HOUR").await;
    let old_week = ts_ago(&c, "INTERVAL 2 DAY - INTERVAL 1 WEEK - INTERVAL 1 HOUR").await;
    insert_ohlc_row(
        &c,
        "price_ohlcv_1m",
        old_minute,
        37,
        ("5", "4.5", "3", "5"),
        2,
        1,
    )
    .await;
    insert_ohlc_row(
        &c,
        "price_ohlcv_1w",
        old_week,
        38,
        ("4", "6", "0", "5"),
        2,
        1,
    )
    .await;

    let readings = read_ohlc_band(&c).await;
    assert_eq!(
        reading(&readings, "price_ohlcv_1m"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(0, 0, 1),
        "only the healthy seed is inside the 1m window"
    );
    assert_eq!(
        reading(&readings, "price_ohlcv_1w"),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(0, 0, 0),
        "a week bucket starting before now - 2d - 1w is outside"
    );
    for (table, counts) in &readings {
        assert_eq!(counts.violations(), 0, "{table}");
    }

    reset_ohlc_band_tables(&c).await;
}

/// The alarm must stop once the data is fixed: a bad row re-inserted good at a
/// higher `version` on the same key is superseded, so `FINAL` neither counts
/// nor scans the bad version. The unrepaired neighbour still counts. RED
/// without `FINAL` (the superseded version is read as a second row).
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_repaired_candle_stops_counting_in_the_ohlc_band_scan() {
    let c = client().await;
    reset_ohlc_band_tables(&c).await;
    let ts = seed_healthy_1m(&c).await;
    let m = "price_ohlcv_1m";
    insert_ohlc_row(&c, m, ts, 40, ("5", "4.5", "3", "5"), 2, 1).await; // bad
    insert_ohlc_row(&c, m, ts, 40, ("4", "6", "3", "5"), 2, 2).await; // its repair
    insert_ohlc_row(&c, m, ts, 41, ("5", "4.5", "3", "5"), 2, 1).await; // left bad

    let readings = read_ohlc_band(&c).await;
    assert_eq!(
        reading(&readings, m),
        rollup_freshness_probe::ohlc_band::OhlcBandCounts::new(1, 0, 3),
        "the superseded version is neither counted nor scanned"
    );

    reset_ohlc_band_tables(&c).await;
}
