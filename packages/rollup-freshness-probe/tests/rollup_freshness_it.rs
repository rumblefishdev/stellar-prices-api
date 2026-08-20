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
//! docker compose up -d clickhouse
//! cargo test -p rollup-freshness-probe --test rollup_freshness_it -- --ignored --nocapture
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
use rollup_freshness_probe::mv_drift::{
    DriftMetric, MV_DRIFT_CRITICAL_METRIC, MV_DRIFT_METRIC, MV_DRIFT_UNREADABLE_METRIC, describe,
    drift_metrics, visible_objects_query,
};
use rollup_freshness_probe::usd_sanity::{
    PEG_TABLE, PegCounts, STRANDED_TABLE, SanityRefusal, StrandedCounts, peg_metric, peg_query,
    stranded_metric, stranded_query,
};
use rollup_freshness_probe::{ROLLUP_TIERS, TableLag, freshness_query, lag_metrics};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// The probe binds the client to the `prices` database (`client_from_lambda_env
/// ("prices")` in `main.rs`), which is why the query references tables
/// unqualified. The IT must do the same so the exact production query resolves.
fn client() -> Client {
    Client::default().with_url(ch_url()).with_database("prices")
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn freshness_query_executes_deserializes_and_gates_empty_tiers() {
    let c = client();
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn ungated_max_over_empty_tier_yields_the_epoch_not_null() {
    let c = client();
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn disk_query_executes_and_deserializes() {
    use rollup_freshness_probe::disk::{
        DISK_FREE_PERCENT_METRIC, DiskUsage, disk_metrics, disk_query, free_percent,
    };

    let c = client();
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn restricted_user_can_read_disk_headroom_but_not_system_disks() {
    use rollup_freshness_probe::disk::{DiskUsage, disk_query};

    let admin = client();
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

/// Seed the canonical USDT identity into `prices.assets` and return its
/// `asset_id`. The probe resolves the leg by code + issuer rather than a
/// hard-coded id (task 0139), so the IT has to make that resolution succeed.
async fn seed_usdt_identity(c: &Client, asset_id: u32) {
    exec(
        c,
        &format!(
            "INSERT INTO prices.assets \
               (asset_id, asset_code, asset_type, issuer_address, contract_address) \
             SELECT {asset_id}, 'USDT', 'credit_alphanum4', '{issuer}', ''",
            issuer = prices_clickhouse::USDT_ISSUER,
        ),
    )
    .await;
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
    usdt_id: u32,
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
    usdt_id: u32,
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
    usdt_id: u32,
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn usd_sanity_query_executes_and_reads_a_healthy_leg_as_zero() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    // A correctly-priced USDT-quoted candle on each tier: USDT at its measured
    // ~0.15, so close_usd is nowhere near close.
    insert_usdt_candle(&c, 111, 5, "now() - INTERVAL 3 DAY", "100", "15").await;
    insert_usdt_minute_candle(&c, 111, 5, "now() - INTERVAL 3 HOUR", "100", "15").await;

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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn usd_sanity_counts_both_induced_defects() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    // Defect 1 — the peg re-applied, on the tier enrichment WRITES: close_usd
    // == close (task 0172 / 0212).
    insert_usdt_minute_candle(&c, 111, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    // Defect 2 — stranded past the grace period, on the tier the consumer
    // reads: zero on a representable close (what task 0182's own reset produced
    // on 2026-08-19).
    insert_usdt_candle(&c, 111, 6, "now() - INTERVAL 3 DAY", "100", "0").await;

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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn a_freshly_written_zero_is_not_yet_stranded() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    // Inside the 48 h grace — awaiting enrichment, not damaged.
    insert_usdt_candle(&c, 111, 5, "now() - INTERVAL 1 HOUR", "100", "0").await;
    assert_eq!(read_stranded(&c).await.stranded, 0, "still within grace");

    // The same row, aged past the grace, is the defect.
    exec(&c, "TRUNCATE TABLE prices.price_ohlcv_1h").await;
    insert_usdt_candle(&c, 111, 5, "now() - INTERVAL 3 DAY", "100", "0").await;
    assert_eq!(read_stranded(&c).await.stranded, 1, "past grace = stranded");
}

/// Dust is not damage. A `close` below the `Decimal(38, 14)` underflow bound
/// cannot produce a non-zero `close_usd` at any plausible rate, so counting it
/// would put the alarm permanently in ALARM over rows with nothing to lose.
/// Task 0182 hit exactly this and its first bound (`1e-11`) was three orders of
/// magnitude too generous.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn dust_below_the_underflow_bound_is_not_counted_as_stranded() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    insert_usdt_candle(
        &c,
        111,
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn an_exotic_quoted_zero_is_ignored_because_it_is_by_design() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn a_repaired_candle_stops_counting_once_a_higher_version_supersedes_it() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    insert_usdt_minute_candle(&c, 111, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    assert_eq!(read_peg(&c).await.peg_applied, 1, "the defect is present");

    // The repair: same primary key, corrected value, version + 1.
    exec(
        &c,
        "INSERT INTO prices.price_ohlcv_1m \
           (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
            volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
         SELECT timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                volume_base, volume_quote, volume_quote, 15, vwap, trade_count, version + 1 \
         FROM prices.price_ohlcv_1m FINAL \
         WHERE quote_asset_id = 111 AND close_usd = close",
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn an_unresolvable_usdt_leg_reads_as_zero_and_is_therefore_refused() {
    let c = client();
    reset_sanity_tables(&c).await;
    // Deliberately no USDT identity seeded.
    insert_usdt_minute_candle(&c, 111, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;

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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn a_peg_row_only_in_1m_is_counted_although_every_coarse_tier_reads_clean() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    // The tier enrichment writes: the peg re-applied.
    insert_usdt_minute_candle(&c, 111, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    // The repaired coarse tier: the SAME candle, correctly valued at ~0.15 —
    // which is precisely what 0182's repair left behind.
    insert_usdt_candle(&c, 111, 5, "now() - INTERVAL 3 HOUR", "100", "15").await;

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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn each_direction_only_scans_its_own_tier() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    // Rows in `_1m` only.
    insert_usdt_minute_candle(&c, 111, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    assert_eq!(read_peg(&c).await.scanned, 1);
    assert_eq!(
        read_stranded(&c).await.scanned,
        0,
        "the stranded direction must not see _1m rows"
    );

    // Rows in `_1h` only.
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;
    insert_usdt_candle(&c, 111, 5, "now() - INTERVAL 3 DAY", "100", "0").await;
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn the_peg_window_excludes_rows_a_cleanup_run_could_delete() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    // Inside the 48 h peg window — counted.
    insert_usdt_minute_candle(&c, 111, 5, "now() - INTERVAL 3 HOUR", "100", "100").await;
    // Older than the peg window but still well inside `_1m`'s 7-day retention,
    // i.e. exactly the band a widened window would have picked up and a cleanup
    // run could then remove underneath it.
    insert_usdt_minute_candle(&c, 111, 6, "now() - INTERVAL 5 DAY", "100", "100").await;

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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn an_empty_peg_scan_does_not_suppress_the_stranded_metric() {
    let c = client();
    reset_sanity_tables(&c).await;
    seed_usdt_identity(&c, 111).await;

    // `_1m` is empty; `_1h` carries a real stranded candle.
    insert_usdt_candle(&c, 111, 5, "now() - INTERVAL 3 DAY", "100", "0").await;

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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn a_freshly_applied_schema_reports_no_drift() {
    let c = client();
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn an_edited_declaration_is_detected_as_drift() {
    let c = client();
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn a_live_mv_without_append_is_detected_as_critical() {
    let c = client();
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
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn an_invisible_database_suppresses_the_counts_instead_of_paging() {
    let c = client();
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
