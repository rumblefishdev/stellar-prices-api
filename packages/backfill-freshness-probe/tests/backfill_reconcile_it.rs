//! Runs the production `RECONCILE_QUERY` (task 0272) against a local
//! ClickHouse, each case on its own scratch database: other targets leave
//! `_1h` rows in the shared `prices` database.
//!
//!     cargo test -p backfill-freshness-probe --test backfill_reconcile_it -- --ignored --test-threads=1

use backfill_freshness_probe::reconcile::{RECONCILE_QUERY, StreamOverclaim};
use backfill_freshness_probe::{SDEX_ARCHIVE_STREAM as SDEX, SOROBAN_AMM_STREAM as AMM};
use clickhouse::Client;

const H: i64 = 3_600;

fn client(db: &str) -> Client {
    let url = std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into());
    Client::default().with_url(url).with_database(db)
}

async fn exec(c: &Client, sql: &str) {
    c.query(sql).execute().await.expect(sql);
}

struct Case {
    name: &'static str,
    /// (table, timestamp, source)
    candles: &'static [(&'static str, &'static str, &'static str)],
    /// (task, status, claim as a SQL literal); one INSERT (= one part) each,
    /// later rows get a later `updated_at`.
    progress: &'static [(&'static str, &'static str, &'static str)],
    want: &'static [(&'static str, i64)],
}

/// Scratch database with AS-copies of the tables the query reads, seeded.
async fn seed(
    name: &str,
    candles: &[(&str, &str, &str)],
    progress: &[(&str, &str, &str)],
) -> Client {
    let p = client("prices");
    prices_clickhouse::apply_sql(&p, prices_clickhouse::INIT_SQL)
        .await
        .expect("init schema");
    let db = format!("bf_reconcile_it_{name}");
    exec(&p, &format!("DROP DATABASE IF EXISTS {db}")).await;
    exec(&p, &format!("CREATE DATABASE {db}")).await;
    for t in ["price_ohlcv_1h", "price_ohlcv_1m", "backfill_progress"] {
        exec(&p, &format!("CREATE TABLE {db}.{t} AS prices.{t}")).await;
    }
    // Keep every progress version in its own part, so FINAL is exercised.
    exec(&p, &format!("SYSTEM STOP MERGES {db}.backfill_progress")).await;
    for (table, ts, source) in candles {
        exec(&p, &format!(
            "INSERT INTO {db}.{table} (timestamp, asset_id, quote_asset_id, source, open, high, low, close, vwap, version) \
             VALUES ('{ts}', 1, 2, '{source}', 1, 1, 1, 1, 1, 1)"
        )).await;
    }
    for (i, (task, status, claim)) in progress.iter().enumerate() {
        exec(&p, &format!(
            "INSERT INTO {db}.backfill_progress \
               (task_name, start_ledger, target_ledger, current_ledger, status, earliest_data_available, updated_at) \
             VALUES ('{task}', 1, 100, 50, '{status}', {claim}, toDateTime('2026-01-01 00:00:00') + {i})"
        )).await;
    }
    client(&db)
}

async fn reconcile(c: &Client) -> Vec<(String, i64)> {
    let rows = c
        .query(RECONCILE_QUERY)
        .fetch_all::<StreamOverclaim>()
        .await
        .expect("reconcile");
    rows.into_iter()
        .map(|r| (r.task_name, r.overclaim_seconds))
        .collect()
}

const CASES: &[Case] = &[
    // Prod-shaped correct claims: SDEX 03:47 vs bucket 03:00 is slack, AMM exact.
    Case {
        name: "correct_claim",
        candles: &[
            ("price_ohlcv_1h", "2015-11-18 03:00:00", "sdex"),
            ("price_ohlcv_1h", "2024-03-08 19:00:00", "aquarius"),
        ],
        progress: &[
            (SDEX, "completed", "'2015-11-18 03:47:00'"),
            (AMM, "completed", "'2024-03-08 19:00:00'"),
        ],
        want: &[(SDEX, -2820), (AMM, 0)],
    },
    Case {
        name: "overclaim_exact",
        candles: &[("price_ohlcv_1h", "2024-03-08 19:00:00", "phoenix")],
        progress: &[(AMM, "running", "'2024-03-08 14:00:00'")],
        want: &[(AMM, 5 * H)],
    },
    // NULL claim, missing row and unknown task: no output.
    Case {
        name: "null_claim_and_absent_row",
        candles: &[("price_ohlcv_1h", "2015-11-18 03:00:00", "sdex")],
        progress: &[
            (SDEX, "running", "NULL"),
            ("gapfill_2019", "running", "'2010-01-01 00:00:00'"),
        ],
        want: &[],
    },
    // Frozen claims are the drift path: no status filter.
    Case {
        name: "status_ignored",
        candles: &[
            ("price_ohlcv_1h", "2015-11-18 03:00:00", "sdex"),
            ("price_ohlcv_1h", "2024-03-08 19:00:00", "aquarius"),
        ],
        progress: &[
            (SDEX, "paused", "'2015-11-18 01:00:00'"),
            (AMM, "error", "'2024-03-08 18:00:00'"),
        ],
        want: &[(SDEX, 2 * H), (AMM, H)],
    },
    // An earlier AMM row only in `_1m` must not hide the overclaim.
    Case {
        name: "reads_1h_not_1m",
        candles: &[
            ("price_ohlcv_1m", "2024-03-08 10:00:00", "aquarius"),
            ("price_ohlcv_1h", "2024-03-08 21:00:00", "aquarius"),
        ],
        progress: &[(AMM, "completed", "'2024-03-08 19:00:00'")],
        want: &[(AMM, 2 * H)],
    },
    // A venue nobody listed still counts as AMM.
    Case {
        name: "unenumerated_venue",
        candles: &[
            ("price_ohlcv_1h", "2024-03-08 19:00:00", "newvenue"),
            ("price_ohlcv_1h", "2024-03-09 04:00:00", "aquarius"),
        ],
        progress: &[(AMM, "completed", "'2024-03-08 19:00:00'")],
        want: &[(AMM, 0)],
    },
    // FINAL: the newer version (3 h too early) wins over the older correct one.
    Case {
        name: "final_latest_claim",
        candles: &[("price_ohlcv_1h", "2024-03-08 19:00:00", "aquarius")],
        progress: &[
            (AMM, "completed", "'2024-03-08 19:00:00'"),
            (AMM, "completed", "'2024-03-08 16:00:00'"),
        ],
        want: &[(AMM, 3 * H)],
    },
    // FINAL before the NULL filter: the newer NULL version wins, no row.
    Case {
        name: "final_latest_claim_is_null",
        candles: &[("price_ohlcv_1h", "2024-03-08 19:00:00", "aquarius")],
        progress: &[
            (AMM, "completed", "'2024-03-08 16:00:00'"),
            (AMM, "completed", "NULL"),
        ],
        want: &[],
    },
];

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn reconcile_cases() {
    for case in CASES {
        let c = seed(case.name, case.candles, case.progress).await;
        let want: Vec<_> = case.want.iter().map(|(s, v)| (s.to_string(), *v)).collect();
        assert_eq!(reconcile(&c).await, want, "case {}", case.name);
    }
}

/// A claim with no `_1h` rows at all reads `now() − claim` (LEFT JOIN +
/// `join_use_nulls`), not a dropped row or a 1970 default.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn claim_without_rows() {
    let c = seed(
        "claim_without_rows",
        &[],
        &[(AMM, "completed", "'2024-03-08 19:00:00'")],
    )
    .await;
    let since = "SELECT toInt64(toUnixTimestamp(now()) - toUnixTimestamp(toDateTime('2024-03-08 19:00:00')))";
    let before = c.query(since).fetch_one::<i64>().await.unwrap();
    let rows = reconcile(&c).await;
    let after = c.query(since).fetch_one::<i64>().await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(
        (before..=after).contains(&rows[0].1),
        "{rows:?} not in [{before}, {after}]"
    );
}
