//! Rollup MV ordering integration test (task 0143).
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-clickhouse --test rollup_order_it -- --ignored --test-threads=1
//!
//! Every refreshable MV fires on its own clock. At 00:00 `mv_ohlcv_4h_to_1d`
//! (EVERY 4 HOUR) and the two dailies that read `_1d` (`mv_ohlcv_1d_to_1w`,
//! `mv_ohlcv_1d_to_1M`, EVERY 1 DAY) share a slot. Without an order, a daily
//! can read `_1d` before the day it should include has been written, and
//! nothing re-runs it until the next day. `DEPENDS ON` makes a dependent's
//! slot wait until its dependency has refreshed for the same slot.
//!
//! The race is made deterministic rather than waited for: `mv_ohlcv_4h_to_1d`
//! is STOPPED while the whole fast chain is moved past the next daily slot
//! with `SYSTEM TEST VIEW … SET FAKE TIME`, so the dailies reach their slot
//! strictly before the day exists in `_1d`. With `DEPENDS ON` they wait; once
//! `mv_ohlcv_4h_to_1d` is STARTed it runs the missed slot (START runs a slot
//! whose fake time has passed — verified on 26.3.10.60) and the dailies follow.
//! Without it they run at once, miss the day, and stay wrong.
//!
//! ⚠️ Fake time moves only the scheduler's clock; `now()` inside the refresh
//! query stays real, so the seeded day is anchored to the server's `now()`.

use clickhouse::Client;
use prices_clickhouse::rollup_sql::TIERS;
use std::time::Duration;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Retarget the `prices.*` schema onto a scratch database.
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// A fresh scratch DB with the real `init.sql` tables and the real
/// `rollups.sql` MVs. Every reconciliation MV is STOPPED — no fast MV depends
/// on one, so this changes nothing about the fast chain's order, and it keeps
/// a second writer out of the targets. Waits until every fast MV has finished
/// the refresh a new refreshable MV runs at CREATE, so no seeded row can be
/// picked up by one. Returns a client with the refreshable-MV flag set.
async fn setup(db: &str) -> Client {
    let admin = Client::default().with_url(ch_url());
    admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop scratch db");
    admin
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .expect("create scratch db");
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("apply init schema");

    let client = admin.with_option("allow_experimental_refreshable_materialized_view", "1");
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::ROLLUPS_SQL, db))
        .await
        .expect("create the rollup MVs");

    let reconcile: Vec<String> = client
        .query(&format!(
            "SELECT view FROM system.view_refreshes \
             WHERE database = '{db}' AND startsWith(view, 'mv_reconcile_')"
        ))
        .fetch_all()
        .await
        .expect("list reconcile views");
    for view in reconcile {
        client
            .query(&format!("SYSTEM STOP VIEW {db}.{view}"))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("stop {view}: {e}"));
    }

    for tier in TIERS {
        let mut done = 0_u8;
        for _ in 0..80 {
            done = client
                .query(&format!(
                    "SELECT toUInt8(last_success_time IS NOT NULL AND status = 'Scheduled') \
                     FROM system.view_refreshes WHERE database = '{db}' AND view = '{}'",
                    tier.mv
                ))
                .fetch_one()
                .await
                .unwrap_or_else(|e| panic!("view_refreshes {}: {e}", tier.mv));
            if done == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert_eq!(
            done, 1,
            "{}: the CREATE-time refresh never finished",
            tier.mv
        );
    }
    client
}

async fn drop_scratch(client: &Client, db: &str) {
    client
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .expect("drop scratch db");
}

/// `(status, last_success_time as epoch s or 0)` of one view.
async fn refresh_state(client: &Client, db: &str, view: &str) -> (String, u32) {
    client
        .query(&format!(
            "SELECT toString(status), \
             toUInt32(toUnixTimestamp(ifNull(last_success_time, toDateTime(0)))) \
             FROM system.view_refreshes WHERE database = '{db}' AND view = '{view}'"
        ))
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("view_refreshes {view}: {e}"))
}

/// Poll until `view`'s `last_success_time` (the FAKE time of its last
/// refresh) reaches `slot`, then `SYSTEM WAIT VIEW` it. `SYSTEM WAIT VIEW`
/// alone returns at once if the refresh has not started yet.
async fn wait_for_slot(client: &Client, db: &str, view: &str, slot: u32) {
    let mut state = refresh_state(client, db, view).await;
    for _ in 0..80 {
        if state.1 >= slot {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
        state = refresh_state(client, db, view).await;
    }
    assert!(
        state.1 >= slot,
        "{view} never refreshed for slot {slot}: status {} last_success {}",
        state.0,
        state.1
    );
    client
        .query(&format!("SYSTEM WAIT VIEW {db}.{view}"))
        .execute()
        .await
        .unwrap_or_else(|e| panic!("wait {view}: {e}"));
}

/// `sum(trade_count)` of `table FINAL` over the buckets that contain `day`
/// (epoch s of a day start) — the day itself, its week and its month.
async fn trades_covering(client: &Client, db: &str, table: &str, interval: &str, day: i64) -> u64 {
    client
        .query(&format!(
            "SELECT toUInt64(sum(trade_count)) FROM {db}.{table} FINAL \
             WHERE timestamp = toStartOfInterval(toDateTime({day}), {interval})"
        ))
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("trades {table}: {e}"))
}

/// 0143 AC 1–2 on the post-0286 shape: both dailies read `_1d`, and at 00:00
/// they share a slot with `mv_ohlcv_4h_to_1d`, which writes the day they must
/// include. The day exists in `_4h` only; the three views reach their common
/// slot while `mv_ohlcv_4h_to_1d` is held back. The dailies must wait for it
/// and then include the day in `_1w` and `_1M`.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_dailies_include_the_day_4h_to_1d_writes_in_the_same_slot() {
    let db = "it_rollup_order";
    let client = setup(db).await;
    let daily = ["mv_ohlcv_1d_to_1w", "mv_ohlcv_1d_to_1M"];

    // Held back BEFORE the day is seeded, so no real 4-hourly slot can write
    // it early either.
    client
        .query(&format!("SYSTEM STOP VIEW {db}.mv_ohlcv_4h_to_1d"))
        .execute()
        .await
        .expect("stop 4h_to_1d");

    // Today's 4h buckets up to now, on the server's real clock — the day
    // `mv_ohlcv_4h_to_1d` will roll into `_1d` (its window is seven days).
    let (day, now): (u32, u32) = client
        .query(
            "SELECT toUInt32(toUnixTimestamp(toStartOfDay(now()))), \
             toUInt32(toUnixTimestamp(now()))",
        )
        .fetch_one()
        .await
        .expect("today");
    let (day, now) = (i64::from(day), i64::from(now));
    let buckets: Vec<i64> = (0..6)
        .map(|k| day + k * 4 * 3_600)
        .filter(|&b| b <= now)
        .collect();
    let values: Vec<String> = buckets
        .iter()
        .enumerate()
        .map(|(k, b)| {
            let tc = 3 + k as u64;
            format!(
                "(toDateTime({b}), 1, 2, 'sdex', 1.00, 1.50, 0.90, 1.10, {vb}, {vq}, 0, 0, \
                 1.00, {tc}, {v}, {tc}, {vb}, {vq})",
                vb = tc * 10,
                vq = tc * 50,
                v = 60_000_000_000_u64 + k as u64,
            )
        })
        .collect();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_4h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
              version, pf_trade_count, pf_volume, pf_price_volume) VALUES {}",
            values.join(", ")
        ))
        .execute()
        .await
        .expect("seed today's 4h");
    let today: u64 = (0..buckets.len() as u64).map(|k| 3 + k).sum();

    // The next common slot of all three: tomorrow 00:00. Every fast MV is
    // moved just past it, bottom-up, so each one's dependency (if any) has
    // its own slot due first.
    let slot = (day + 86_400) as u32;
    let fake: String = client
        .query(&format!(
            "SELECT toString(toDateTime({slot}) + INTERVAL 30 SECOND)"
        ))
        .fetch_one()
        .await
        .expect("fake time");
    for tier in TIERS {
        client
            .query(&format!(
                "SYSTEM TEST VIEW {db}.{} SET FAKE TIME '{fake}'",
                tier.mv
            ))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("fake time {}: {e}", tier.mv));
    }

    // What the dailies do at their slot while the day is not in `_1d` yet:
    // wait (with DEPENDS ON) or run (without). Recorded, asserted last, so a
    // missing order fails on the data it gets wrong.
    let mut at_slot = Vec::new();
    for view in daily {
        let mut state = refresh_state(&client, db, view).await;
        for _ in 0..40 {
            if state.0 == "WaitingForDependencies" || state.1 >= slot {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            state = refresh_state(&client, db, view).await;
        }
        at_slot.push((view, state));
    }

    // Release `mv_ohlcv_4h_to_1d`: START runs the slot it missed.
    client
        .query(&format!("SYSTEM START VIEW {db}.mv_ohlcv_4h_to_1d"))
        .execute()
        .await
        .expect("start 4h_to_1d");
    wait_for_slot(&client, db, "mv_ohlcv_4h_to_1d", slot).await;
    for view in daily {
        wait_for_slot(&client, db, view, slot).await;
    }

    assert_eq!(
        trades_covering(&client, db, "price_ohlcv_1d", "INTERVAL 1 DAY", day).await,
        today,
        "mv_ohlcv_4h_to_1d must have written the day once released"
    );
    for (table, interval) in [
        ("price_ohlcv_1w", "INTERVAL 1 WEEK"),
        ("price_ohlcv_1M", "INTERVAL 1 MONTH"),
    ] {
        assert_eq!(
            trades_covering(&client, db, table, interval, day).await,
            today,
            "{table}: the daily sharing the slot with mv_ohlcv_4h_to_1d must include \
             the day it writes (states at the slot: {at_slot:?})"
        );
    }
    for (view, (status, last_success)) in &at_slot {
        assert_eq!(
            status, "WaitingForDependencies",
            "{view} must wait for mv_ohlcv_4h_to_1d at the slot (last_success {last_success})"
        );
    }

    drop_scratch(&client, db).await;
}
