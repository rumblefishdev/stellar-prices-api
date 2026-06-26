//! Full-chain rollup version-propagation integration test (task 0059).
//!
//!     docker compose up -d clickhouse
//!     cargo test -p prices-clickhouse --test rollup_chain_it -- --ignored
//!
//! Exercises the REAL shipped refreshable-MV chain (`schema/rollups.sql`,
//! landed by task 0051) end-to-end across every granularity `_1m → _15m → _1h
//! → _4h → _1d → _1w → _1M`, and proves the two correctness properties task
//! 0059 was opened to verify against the production DDL:
//!
//!   AC#1/AC#2 — an enrichment re-INSERT into `_1m` (`volume_quote_usd` filled,
//!   `version` bumped) propagates to EVERY rolled granularity after a refresh,
//!   with NO under-count and NO double-count of the summed volumes, and the
//!   re-aggregated row WINS at every grain.
//!
//! The shipped chain is a TRUE refreshable MV in *replace* mode (atomic target
//! swap) re-aggregating from the previous grain `FINAL` — so `max(version)` is
//! a sufficient projection (the swap discards the stale row; there is no
//! ReplacingMergeTree version tie to lose). This test pins that behaviour.
//!
//! Owns an isolated scratch database and drops it at the end. The 0059 G-note
//! proof (`lore/.../proof/`) only covered the `_1m → _15m` hop by hand; this is
//! the full-chain automation that closes AC#3.

use clickhouse::Client;
use std::time::Duration;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Rewrite the `prices.*` schema onto an isolated scratch database name
/// (same trick as `views_it.rs`), so the test never touches the real `prices`
/// tables and can be dropped wholesale at the end.
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// One 15-minute bucket of three per-minute `_1m` rows for a single
/// `(asset 1, quote 2, sdex)` series, all anchored to the PREVIOUS completed
/// 15-minute bucket (12–14 min before the current boundary) so they:
///   - share one `_15m`/`_1h`/.../`_1M` bucket at every grain, and
///   - sit comfortably inside every rollup `WHERE timestamp >= now() - …`
///     window (the tightest is `_15m`'s 2 HOUR).
///
/// `vqusd` is the per-row `volume_quote_usd` (0 = un-enriched, >0 = enriched);
/// `version` is the `ReplacingMergeTree(version)` discriminator.
fn insert_bucket(db: &str, vqusd: &str, version: u64) -> String {
    let b = "toStartOfInterval(now(), INTERVAL 15 MINUTE)";
    format!(
        "INSERT INTO {db}.price_ohlcv_1m \
         (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
          volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
         ({b} - INTERVAL 14 MINUTE, 1,2,'sdex', 1.00,1.50,0.90,1.10, 10,50,{vqusd},0,1.00,1,{version}), \
         ({b} - INTERVAL 13 MINUTE, 1,2,'sdex', 1.10,1.60,1.00,1.20, 10,50,{vqusd},0,1.10,1,{version}), \
         ({b} - INTERVAL 12 MINUTE, 1,2,'sdex', 1.20,1.40,0.80,1.30, 10,50,{vqusd},0,1.20,1,{version})"
    )
}

/// The chain in dependency order: each MV re-aggregates the previous grain's
/// target `FINAL`, so they must be refreshed front-to-back.
const CHAIN: &[(&str, &str)] = &[
    ("mv_ohlcv_1m_to_15m", "price_ohlcv_15m"),
    ("mv_ohlcv_15m_to_1h", "price_ohlcv_1h"),
    ("mv_ohlcv_1h_to_4h", "price_ohlcv_4h"),
    ("mv_ohlcv_4h_to_1d", "price_ohlcv_1d"),
    ("mv_ohlcv_1d_to_1w", "price_ohlcv_1w"),
    ("mv_ohlcv_1w_to_1M", "price_ohlcv_1M"),
];

/// Trigger an immediate refresh of one MV and block until its target reflects
/// the expected value of `metric_expr` (an aggregate over the target `FINAL`).
/// Deterministic stand-in for waiting on the `REFRESH EVERY` schedule.
async fn refresh_until(
    client: &Client,
    db: &str,
    mv: &str,
    target: &str,
    metric_expr: &str,
    want: f64,
) {
    client
        .query(&format!("SYSTEM REFRESH VIEW {db}.{mv}"))
        .execute()
        .await
        .unwrap_or_else(|e| panic!("refresh {mv}: {e}"));

    for _ in 0..40 {
        let got: f64 = client
            .query(&format!(
                "SELECT toFloat64({metric_expr}) FROM {db}.{target} FINAL"
            ))
            .fetch_one()
            .await
            .unwrap_or_else(|e| panic!("poll {target}: {e}"));
        if (got - want).abs() < 1e-6 {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("{target}: {metric_expr} never reached {want} after refresh of {mv}");
}

/// Drive the whole chain front-to-back, waiting on `metric_expr == want` at
/// each grain before refreshing the next (the coarser MV reads this grain
/// `FINAL`, so it must be settled first).
async fn drive_chain(client: &Client, db: &str, metric_expr: &str, want: f64) {
    for (mv, target) in CHAIN {
        refresh_until(client, db, mv, target, metric_expr, want).await;
    }
}

/// Assert the single rolled bucket at `target`. The bucket is fixed across the
/// test (3 minutes of `volume_base = 10`, `volume_quote = 50`, `trade_count =
/// 1`), so `volume_base = 30` / `volume_quote = 150` / `trade_count = 3` and the
/// OHLC corners are invariants — only `volume_quote_usd` and the projected
/// `version` change with enrichment, so those are the parameters. One row per
/// grain (FINAL), so every property is checked against the post-dedup winner.
async fn assert_bucket(
    client: &Client,
    db: &str,
    target: &str,
    want_vqusd: f64,
    want_version: u64,
) {
    let n: u64 = client
        .query(&format!("SELECT count() FROM {db}.{target} FINAL"))
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("count {target}: {e}"));
    assert_eq!(
        n, 1,
        "{target}: expected exactly one rolled bucket, got {n}"
    );

    let (vbase, vquote, vqusd, open, high, low, close, tc, version): (
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        f64,
        u32,
        u64,
    ) = client
        .query(&format!(
            "SELECT toFloat64(volume_base), toFloat64(volume_quote), toFloat64(volume_quote_usd), \
             toFloat64(open), toFloat64(high), toFloat64(low), toFloat64(close), \
             trade_count, version \
             FROM {db}.{target} FINAL"
        ))
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("row {target}: {e}"));

    let approx = |a: f64, b: f64, what: &str| {
        assert!(
            (a - b).abs() < 1e-6,
            "{target}: {what} expected {b}, got {a}"
        );
    };
    // Volumes: sum of the whole bucket — proves no multi-row under-count and,
    // post-enrichment, no double-count.
    approx(vbase, 30.0, "volume_base");
    approx(vquote, 150.0, "volume_quote");
    approx(vqusd, want_vqusd, "volume_quote_usd");
    // OHLC: argMin(open)/max(high)/min(low)/argMax(close) over the 3 minutes,
    // identical at every grain because they share one bucket.
    approx(open, 1.00, "open (argMin by ts)");
    approx(high, 1.60, "high (max)");
    approx(low, 0.80, "low (min)");
    approx(close, 1.30, "close (argMax by ts)");
    assert_eq!(tc, 3, "{target}: trade_count");
    assert_eq!(version, want_version, "{target}: projected version");
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn enrichment_propagates_through_full_rollup_chain() {
    let db = "it_rollup_chain";
    let admin = Client::default().with_url(ch_url());

    // Fresh scratch DB with the real init schema (base tables) ...
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

    // ... and the REAL production rollup chain (needs the experimental flag on
    // builds where refreshable MVs are still gated, matching current_mv_it.rs).
    let mv_client = admin
        .clone()
        .with_option("allow_experimental_refreshable_materialized_view", "1");
    prices_clickhouse::apply_sql(&mv_client, &rewrite(prices_clickhouse::ROLLUPS_SQL, db))
        .await
        .expect("create rollup MV chain");

    // ---- Phase 1: un-enriched (volume_quote_usd = 0), roll up the full chain.
    admin
        .query(&insert_bucket(db, "0", 1))
        .execute()
        .await
        .expect("insert un-enriched _1m bucket");

    // Sanity: _1m FINAL already holds the full bucket (3 rows summed).
    let one_min_vbase: f64 = admin
        .query(&format!(
            "SELECT toFloat64(sum(volume_base)) FROM {db}.price_ohlcv_1m FINAL"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert!(
        (one_min_vbase - 30.0).abs() < 1e-6,
        "_1m FINAL volume_base should be 30, got {one_min_vbase}"
    );

    drive_chain(&admin, db, "sum(volume_base)", 30.0).await;

    // Every grain reflects the full summed bucket; USD volume still 0 pre-enrich.
    for (_, target) in CHAIN {
        assert_bucket(&admin, db, target, 0.0, 1).await;
    }

    // ---- Phase 2: enrichment re-INSERT — fill volume_quote_usd, bump version.
    admin
        .query(&insert_bucket(db, "100", 2))
        .execute()
        .await
        .expect("insert enriched _1m bucket");

    // _1m FINAL dedups to the enriched 3 rows: USD volume = 300, base STILL 30.
    let (m_vbase, m_vqusd): (f64, f64) = admin
        .query(&format!(
            "SELECT toFloat64(sum(volume_base)), toFloat64(sum(volume_quote_usd)) \
             FROM {db}.price_ohlcv_1m FINAL"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert!(
        (m_vbase - 30.0).abs() < 1e-6,
        "_1m FINAL volume_base must stay 30 after enrichment (no double-count), got {m_vbase}"
    );
    assert!(
        (m_vqusd - 300.0).abs() < 1e-6,
        "_1m FINAL volume_quote_usd should be 300 after enrichment, got {m_vqusd}"
    );

    drive_chain(&admin, db, "sum(volume_quote_usd)", 300.0).await;

    // The enriched value wins at EVERY grain, volumes are NOT double-counted
    // (volume_base still 30, not 60), and the projected version advanced 1 → 2.
    for (_, target) in CHAIN {
        assert_bucket(&admin, db, target, 300.0, 2).await;
    }

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The full-range `preroll.sql` path (backfill / sizing — task 0060) shares the
/// rollup SELECT and therefore the same `argMin/argMax` correctness contract.
/// One deterministic pass over a multi-row bucket must produce the true
/// first-open / last-close at EVERY grain (the bug task 0059's full-chain test
/// surfaced: the `AS timestamp` bucket alias shadowing the source column).
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn preroll_reaggregates_full_chain_ohlc_correctly() {
    let db = "it_preroll_chain";
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

    admin
        .query(&insert_bucket(db, "100", 1))
        .execute()
        .await
        .expect("insert _1m bucket");

    // preroll is a plain front-to-back INSERT…SELECT chain (no refresh): one
    // apply populates _15m … _1M from _1m FINAL.
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::PREROLL_SQL, db))
        .await
        .expect("run preroll chain");

    for (_, target) in CHAIN {
        // open=1.00 (first minute), close=1.30 (last) — the argMin/argMax-by-time
        // properties; volumes summed across the 3 minutes, version carried.
        assert_bucket(&admin, db, target, 300.0, 1).await;
    }

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
