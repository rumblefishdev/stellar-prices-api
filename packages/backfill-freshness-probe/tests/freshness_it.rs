//! Integration test for the freshness probe's [`AGE_QUERY`] against a local
//! Docker ClickHouse with the `prices` schema applied (task 0056 finding A).
//!
//! The unit tests in `lib.rs` only exercise `age_metrics` (pure Rust) and the
//! query *string* shape — they cannot catch a query that fails to **execute or
//! deserialize**. This IT closes exactly the gap that let the PR #97 regression
//! land: dropping `coalesce` made `age_seconds` a `Nullable(Int64)` column that
//! will not deserialize into the non-`Option` `i64` field, so the real probe
//! errored on every run. It also proves the `status='running' AND last_push_at
//! IS NOT NULL` gate (finding A: no live-only false-fire) and that `FINAL`
//! returns the latest version, not a stale pre-merge `running` row.
//!
//!     docker compose up -d clickhouse
//!     cargo test -p backfill-freshness-probe --test freshness_it -- --ignored --nocapture
//!
//! Destructive to the local `prices.backfill_progress` table (truncates it);
//! never run against a shared/prod cluster.

use backfill_freshness_probe::{AGE_QUERY, SDEX_ARCHIVE_STREAM, StreamAge, age_metrics};
use clickhouse::Client;

const SEVEN_DAYS: i64 = 7 * 86_400;
const EIGHT_DAYS: i64 = 8 * 86_400;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// The probe binds the client to the `prices` database (`client_from_lambda_env
/// ("prices")` in `main.rs`), which is why `AGE_QUERY` references the table
/// unqualified. The IT must do the same so the exact production query resolves.
fn client() -> Client {
    Client::default().with_url(ch_url()).with_database("prices")
}

async fn exec(c: &Client, sql: &str) {
    c.query(sql).execute().await.expect(sql);
}

/// Insert one `backfill_progress` row via `INSERT … SELECT` so `now()` /
/// `INTERVAL` expressions evaluate. `last_push_sql` is a ClickHouse expression
/// for `last_push_at` (e.g. `now() - INTERVAL 8 DAY`, or `CAST(NULL AS
/// Nullable(DateTime))` for a never-pushed seed row).
async fn insert_row(c: &Client, task: &str, status: &str, last_push_sql: &str, updated_sql: &str) {
    exec(
        c,
        &format!(
            "INSERT INTO prices.backfill_progress \
               (task_name, start_ledger, target_ledger, current_ledger, status, last_push_at, updated_at) \
             SELECT '{task}', 1, 100, 50, '{status}', {last_push_sql}, {updated_sql}"
        ),
    )
    .await;
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn age_query_executes_deserializes_and_gates() {
    let c = client();
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    exec(&c, "TRUNCATE TABLE prices.backfill_progress").await;

    // --- Rows covering every branch of the WHERE gate ----------------------
    // Included: running + pushed 8 days ago → age > 7-day default threshold.
    insert_row(
        &c,
        SDEX_ARCHIVE_STREAM,
        "running",
        "now() - INTERVAL 8 DAY",
        "now()",
    )
    .await;
    // Excluded: the live-only false-fire case — running but never pushed
    // (seed row, NULL last_push_at). Before finding A this aged from started_at
    // and sat the alarm permanently in ALARM.
    insert_row(
        &c,
        "seed_never_pushed",
        "running",
        "CAST(NULL AS Nullable(DateTime))",
        "now()",
    )
    .await;
    // Excluded: paused (the between-runs seam) even though it has a stale push.
    insert_row(
        &c,
        "stream_paused",
        "paused",
        "now() - INTERVAL 8 DAY",
        "now()",
    )
    .await;
    // Excluded: completed backfill legitimately stops pushing.
    insert_row(
        &c,
        "stream_completed",
        "completed",
        "now() - INTERVAL 8 DAY",
        "now()",
    )
    .await;
    // FINAL correctness: a stream whose latest version is 'completed' but which
    // still has an older 'running' part must be EXCLUDED — proves FINAL returns
    // the merged latest version and the status predicate is not moved to
    // PREWHERE (evaluated pre-merge on the stale 'running' row).
    insert_row(
        &c,
        "stream_flipped",
        "running",
        "now() - INTERVAL 8 DAY",
        "now() - INTERVAL 1 HOUR",
    )
    .await;
    insert_row(
        &c,
        "stream_flipped",
        "completed",
        "now() - INTERVAL 8 DAY",
        "now()",
    )
    .await;

    // --- Run the EXACT production query ------------------------------------
    // This line is the regression guard: with a Nullable(Int64) age_seconds
    // column and a plain-i64 field, fetch_all::<StreamAge> errors here.
    let rows = c
        .query(AGE_QUERY)
        .fetch_all::<StreamAge>()
        .await
        .expect("AGE_QUERY must execute and deserialize into StreamAge (non-nullable i64)");

    // Only the running + already-pushed sdex_archive stream survives the gate.
    assert_eq!(
        rows.len(),
        1,
        "expected exactly 1 gated row, got {rows:?} — the seed/paused/completed/flipped streams must all be excluded"
    );
    let row = &rows[0];
    assert_eq!(row.task_name, SDEX_ARCHIVE_STREAM);

    // Age ~8 days (two now() reads a few seconds apart): comfortably over the
    // 7-day default threshold, so this stream would breach and page (AC #5).
    assert!(
        row.age_seconds >= EIGHT_DAYS - 300 && row.age_seconds <= EIGHT_DAYS + 300,
        "age_seconds {} should be ~8 days ({EIGHT_DAYS})",
        row.age_seconds
    );
    assert!(
        row.age_seconds > SEVEN_DAYS,
        "age_seconds {} must exceed the 7-day default freshness threshold",
        row.age_seconds
    );

    // The shaped metric the probe publishes.
    let metrics = age_metrics(&rows);
    assert_eq!(metrics.len(), 1);
    assert_eq!(metrics[0].stream, SDEX_ARCHIVE_STREAM);
    assert_eq!(metrics[0].value, row.age_seconds as f64);

    // --- Live-only end state: no running+pushed stream → zero rows ----------
    // Flip sdex_archive to completed; now every stream is excluded and the
    // probe publishes nothing → NOT_BREACHING alarm stays OK (no false-fire).
    insert_row(
        &c,
        SDEX_ARCHIVE_STREAM,
        "completed",
        "now() - INTERVAL 8 DAY",
        "now() + INTERVAL 1 HOUR",
    )
    .await;
    let rows = c
        .query(AGE_QUERY)
        .fetch_all::<StreamAge>()
        .await
        .expect("AGE_QUERY must still execute with zero matching rows");
    assert!(
        rows.is_empty(),
        "no running+pushed stream ⇒ no metric published (live-only stays OK), got {rows:?}"
    );
}
