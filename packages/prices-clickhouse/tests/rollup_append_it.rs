//! APPEND-mode rollup durability test (task 0095).
//!
//!     docker compose up -d clickhouse
//!     cargo test -p prices-clickhouse --test rollup_append_it -- --ignored
//!
//! This is the test the 0059 full-chain test *structurally could not be*.
//! `rollup_chain_it.rs` deliberately anchors every row INSIDE the refresh window
//! (its own comment: rows "sit comfortably inside every rollup `WHERE timestamp
//! >= now() - …`"), so "replace the table with the last 2 h" and "replace the
//! table with everything" are the same operation — the replace-mode wipe that
//! deleted pre-rolled history in production (task 0090) is invisible to it.
//!
//! These tests place data OUTSIDE the window and exercise a real `SYSTEM REFRESH
//! VIEW` against the production `rollups.sql` DDL on ClickHouse pinned to the
//! prod version (26.3.10.60), proving the task 0095 fix end-to-end:
//!
//!   1. `append_preserves_pre_rolled_history_outside_window` — a coarse bucket
//!      older than the refresh window SURVIVES a refresh, and a fresh live bucket
//!      is ADDED (AC #2). Fails under replace mode (whole target swapped).
//!   2. `aligned_window_rebuilds_oldest_bucket_complete` — the oldest in-window
//!      bucket is re-aggregated from ALL its source rows, not the post-`now() -
//!      window` slice — the alignment that stops a partial bucket from being
//!      appended over pre-rolled history.
//!   3. `sum_version_wins_early_minute_correction` — correcting an EARLY minute
//!      bumps that row's version but leaves the bucket `max(version)` unchanged;
//!      the corrected rollup row must still win. Passes under `sum(version)`,
//!      ties (and non-deterministically loses) under `max(version)` — finding #5.

use clickhouse::Client;
use std::time::Duration;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Rewrite the `prices.*` schema onto an isolated scratch database (same trick
/// as `rollup_chain_it.rs` / `views_it.rs`) so the test never touches the real
/// `prices` tables and can be dropped wholesale at the end.
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// Fresh scratch DB with the real init schema + the real production rollup MV
/// chain. Returns an admin client and an MV-capable client (experimental flag
/// on, matching `current_mv_it.rs` for builds that still gate refreshable MVs).
async fn setup(db: &str) -> Client {
    let admin = Client::default().with_url(ch_url());
    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    admin
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("apply init schema");
    let mv_client = admin
        .clone()
        .with_option("allow_experimental_refreshable_materialized_view", "1");
    prices_clickhouse::apply_sql(&mv_client, &rewrite(prices_clickhouse::ROLLUPS_SQL, db))
        .await
        .expect("create rollup MV chain");
    admin
}

/// Insert one raw `_1m` row at `ts_expr` (a SQL DateTime expression) for the
/// single `(asset 1, quote 2, sdex)` series. `vqusd`/`version` vary per case;
/// OHLC/volumes are fixed so bucket sums are predictable.
async fn insert_1m(client: &Client, db: &str, ts_expr: &str, vqusd: u64, version: u64) {
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES ({ts_expr}, 1,2,'sdex', 1.0,1.5,0.9,1.1, 10,50,{vqusd},0,1.0,1,{version})"
        ))
        .execute()
        .await
        .unwrap_or_else(|e| panic!("insert _1m @ {ts_expr}: {e}"));
}

/// Insert one already-rolled `_15m` coarse row directly (stand-in for a
/// pre-rolled historical bucket), at `ts_expr`, with a chosen `version`.
async fn insert_15m(client: &Client, db: &str, ts_expr: &str, vqusd: u64, version: u64) {
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_15m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES ({ts_expr}, 1,2,'sdex', 7.0,7.0,7.0,7.0, 30,150,{vqusd},0,5.0,3,{version})"
        ))
        .execute()
        .await
        .unwrap_or_else(|e| panic!("insert _15m @ {ts_expr}: {e}"));
}

/// Trigger an immediate refresh of the `_1m -> _15m` MV and block until a scalar
/// probe over `_15m FINAL` reaches `want`. Deterministic stand-in for the
/// `REFRESH EVERY` schedule.
async fn refresh_15m_until(client: &Client, db: &str, probe: &str, want: f64) {
    client
        .query(&format!("SYSTEM REFRESH VIEW {db}.mv_ohlcv_1m_to_15m"))
        .execute()
        .await
        .unwrap_or_else(|e| panic!("refresh mv_ohlcv_1m_to_15m: {e}"));
    for _ in 0..40 {
        let got: f64 = client
            .query(&format!(
                "SELECT toFloat64({probe}) FROM {db}.price_ohlcv_15m FINAL"
            ))
            .fetch_one()
            .await
            .unwrap_or_else(|e| panic!("probe _15m: {e}"));
        if (got - want).abs() < 1e-6 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let got: f64 = client
        .query(&format!(
            "SELECT toFloat64({probe}) FROM {db}.price_ohlcv_15m FINAL"
        ))
        .fetch_one()
        .await
        .unwrap();
    panic!("_15m: probe `{probe}` never reached {want} (last {got})");
}

/// AC #2, headline: a coarse bucket OLDER than the refresh window survives a
/// refresh, and a fresh in-window live bucket is added alongside it. This is the
/// property replace mode violated in prod (task 0090): a replace-mode refresh
/// swaps the whole target for just the window, deleting the old row.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn append_preserves_pre_rolled_history_outside_window() {
    let db = "it_append_durability";
    let admin = setup(db).await;

    // A pre-rolled bucket 30 days back — far outside the _15m MV's 2 HOUR window.
    // version 9_000_000 stands in for a prior pre-roll's sum(version).
    let old_ts = "toStartOfInterval(now(), INTERVAL 15 MINUTE) - INTERVAL 30 DAY";
    insert_15m(&admin, db, old_ts, 42, 9_000_000).await;

    // A fresh live bucket: three _1m rows ~12–14 min ago (inside the 2 HOUR
    // window, in the PREVIOUS completed 15m bucket).
    let anchor = "toStartOfInterval(now(), INTERVAL 15 MINUTE)";
    for m in [12u32, 13, 14] {
        insert_1m(
            &admin,
            db,
            &format!("{anchor} - INTERVAL {m} MINUTE"),
            0,
            100,
        )
        .await;
    }

    // Refresh: wait until the fresh bucket has rolled up (volume_base 30 appears).
    refresh_15m_until(
        &admin,
        db,
        "sumIf(volume_base, timestamp >= now() - INTERVAL 1 DAY)",
        30.0,
    )
    .await;

    // The 30-day-old bucket is UNTOUCHED — still exactly one row, still its
    // pre-rolled values. Under replace mode this count would be 0.
    let (old_n, old_vqusd, old_ver): (u64, f64, u64) = admin
        .query(&format!(
            "SELECT count(), toFloat64(any(volume_quote_usd)), any(version) \
             FROM {db}.price_ohlcv_15m FINAL WHERE timestamp < now() - INTERVAL 1 DAY"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(
        old_n, 1,
        "pre-rolled bucket outside the window was wiped by refresh"
    );
    assert!(
        (old_vqusd - 42.0).abs() < 1e-6,
        "pre-rolled bucket value changed: {old_vqusd}"
    );
    assert_eq!(
        old_ver, 9_000_000,
        "pre-rolled bucket version changed: {old_ver}"
    );

    // And the fresh bucket is present with the full summed volume.
    let fresh_vbase: f64 = admin
        .query(&format!(
            "SELECT toFloat64(sum(volume_base)) FROM {db}.price_ohlcv_15m FINAL \
             WHERE timestamp >= now() - INTERVAL 1 DAY"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert!(
        (fresh_vbase - 30.0).abs() < 1e-6,
        "fresh live bucket missing/short: {fresh_vbase}"
    );

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The aligned lower bound (`toStartOfInterval(now() - 2 HOUR, INTERVAL 15
/// MINUTE)`) makes the OLDEST in-window bucket rebuild COMPLETE. A raw `now() -
/// 2 HOUR` bound would fall mid-bucket and re-aggregate that bucket from only its
/// post-bound minutes — a partial row that (with sum(version)) could still be
/// appended and, in prod, outrank a complete pre-rolled bucket at the boundary.
///
/// We seed a full 15-minute bucket straddling `now() - 2 HOUR` and assert the
/// rolled `_15m` bucket carries the FULL three-minute volume, not a truncated
/// slice.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn aligned_window_rebuilds_oldest_bucket_complete() {
    let db = "it_append_alignment";
    let admin = setup(db).await;

    // The 15-minute bucket whose START is <= now()-2h < its end. Its three
    // minutes span the raw `now() - 2 HOUR` instant: one before, one at, one
    // after — so a RAW (unaligned) bound would drop the earlier minute(s).
    let bstart = "toStartOfInterval(now() - INTERVAL 2 HOUR, INTERVAL 15 MINUTE)";
    // Place rows at bucket_start +1, +5, +9 minutes; now()-2h sits ~mid-bucket,
    // so a raw bound cuts the +1 (and possibly +5) minute.
    for off in [1u32, 5, 9] {
        insert_1m(
            &admin,
            db,
            &format!("{bstart} + INTERVAL {off} MINUTE"),
            0,
            100,
        )
        .await;
    }

    // Probe the straddling bucket specifically.
    let probe = format!("sumIf(volume_base, timestamp = {bstart})");
    refresh_15m_until(&admin, db, &probe, 30.0).await;

    // Complete: all three minutes rolled (30), not a post-bound slice (10 or 20).
    let vbase: f64 = admin
        .query(&format!(
            "SELECT toFloat64(sumIf(volume_base, timestamp = {bstart})) \
             FROM {db}.price_ohlcv_15m FINAL"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert!(
        (vbase - 30.0).abs() < 1e-6,
        "oldest in-window bucket rebuilt PARTIAL ({vbase}), expected complete 30 — window not aligned"
    );

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Finding #5: correcting an EARLY minute of a bucket bumps only that row's
/// version, leaving the bucket `max(version)` unchanged — so under
/// `max(version)` the corrected rollup row TIES the stale one and RMT's
/// tie-break is not contractual. `sum(version)` strictly increases (one addend
/// rises), so the corrected row wins deterministically.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn sum_version_wins_early_minute_correction() {
    let db = "it_append_version_tie";
    let admin = setup(db).await;

    // One 15m bucket, three minutes, versions 10/20/30 (max 30, sum 60), vqusd 0.
    // The EARLIEST minute carries the smallest version (10) — the one we correct.
    let anchor = "toStartOfInterval(now(), INTERVAL 15 MINUTE)";
    insert_1m(&admin, db, &format!("{anchor} - INTERVAL 14 MINUTE"), 0, 10).await; // earliest
    insert_1m(&admin, db, &format!("{anchor} - INTERVAL 13 MINUTE"), 0, 20).await;
    insert_1m(&admin, db, &format!("{anchor} - INTERVAL 12 MINUTE"), 0, 30).await;

    // Roll up: bucket appears with sum(version) = 60, vqusd = 0. Probe on
    // volume_base (30) — a sum over the still-empty table is 0 and would satisfy
    // a vqusd==0 probe before the refresh ever lands.
    refresh_15m_until(&admin, db, "sum(volume_base)", 30.0).await;
    let ver0: u64 = admin
        .query(&format!(
            "SELECT any(version) FROM {db}.price_ohlcv_15m FINAL"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(
        ver0, 60,
        "pre-correction rollup version should be sum(10,20,30)=60, got {ver0}"
    );

    // Correct the EARLIEST minute: same timestamp (RMT dedup in _1m), vqusd 100,
    // version 10 -> 11. Bucket versions become {11,20,30}: max STILL 30, sum 61.
    insert_1m(
        &admin,
        db,
        &format!("{anchor} - INTERVAL 14 MINUTE"),
        100,
        11,
    )
    .await;

    // The correction must propagate: the rolled bucket's vqusd becomes 100.
    // Under max(version) this ties at 30 and can silently keep the stale 0.
    refresh_15m_until(&admin, db, "sum(volume_quote_usd)", 100.0).await;
    let ver1: u64 = admin
        .query(&format!(
            "SELECT any(version) FROM {db}.price_ohlcv_15m FINAL"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(
        ver1, 61,
        "post-correction rollup version should be sum(11,20,30)=61, got {ver1}"
    );

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
