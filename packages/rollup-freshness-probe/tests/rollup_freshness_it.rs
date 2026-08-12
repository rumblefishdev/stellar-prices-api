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
//! Destructive to the local `prices.price_ohlcv_*` tables (truncates them);
//! never run against a shared/prod cluster.

use clickhouse::Client;
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

    // The shaped metrics the probe publishes.
    let metrics = lag_metrics(&rows);
    assert_eq!(metrics.len(), 2);
    assert_eq!(metrics[0].table, "price_ohlcv_1h");
    assert_eq!(metrics[0].value, stalled as f64);

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
        "all tiers empty ⇒ no metric published (fresh env stays OK), got {rows:?}"
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
