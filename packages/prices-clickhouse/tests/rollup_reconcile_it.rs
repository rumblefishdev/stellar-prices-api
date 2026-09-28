//! Reconciliation MV integration tests (task 0203).
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-clickhouse --test rollup_reconcile_it -- --ignored --test-threads=1
//!
//! The six fast rollup MVs re-aggregate a window measured from `now()`, so a
//! `_1m` row that arrives after its bucket has left the window is never rolled
//! up (2026-08-13: an 11.5 h ingest stall back-filled rows no fast window could
//! still see). `schema/rollups.sql` therefore ships six hourly reconciliation
//! MVs that rebuild any coarse bucket whose `trade_count`/`volume_base`
//! disagree with the tier below, over the last seven days, chained with
//! `DEPENDS ON` so one pass climbs 15m → 1h → 4h → 1d → {1w, 1M}.
//!
//! These tests run the REAL shipped file on a scratch database and drive the
//! schedules deterministically:
//!
//! - the fast chain with `SYSTEM REFRESH VIEW` + a poll on the target, as
//!   `rollup_chain_it.rs` does (`SYSTEM REFRESH` ignores `DEPENDS ON`);
//! - a reconcile pass with `SYSTEM TEST VIEW … SET FAKE TIME` past the next
//!   hourly slot, bottom-up, so the scheduler — and with it the `DEPENDS ON`
//!   chain — decides the order, then a poll on `system.view_refreshes`.
//!
//! ⚠️ Fake time moves only the scheduler's clock; `now()` inside the refresh
//! query stays real (verified on 26.3.10.60). Every seeded row is therefore
//! anchored to the server's real `now()`.
//!
//! Versions are ledger-scale (`LEDGER_VERSION + n`, as ingest writes them:
//! `ledger · 1000 + op`), so a superset re-roll outranks a partial one by
//! orders of magnitude and never ties with a `+1` bump (BRIEF §3.3).

use clickhouse::Client;
use prices_clickhouse::rollup_sql::{self, TIERS, Tier};
use std::time::Duration;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// A `_1m` version the size ingest writes (`ledger_sequence · 1000 + op`).
const LEDGER_VERSION: u64 = 60_000_000_000;

/// Retarget the `prices.*` schema onto a scratch database.
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// A fresh scratch DB holding the real `init.sql` tables and the real
/// `rollups.sql` MVs, with every reconciliation MV's schedule pinned so that
/// no real `:00` crossing can run a pass the test did not ask for.
///
/// Returns a client with the refreshable-MV flag set. The reconcile views are
/// found LIVE (see [`live_reconcile_views`]), so a schema without them sets up
/// fine and fails later, on the convergence assertion.
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

    // Pin each reconcile view's clock just past the CURRENT hour's slot: its
    // next slot is then ~an hour of fake time away, however long the test runs
    // in real time. Only `drive_reconcile_pass` moves it past a slot.
    let pinned: String = client
        .query("SELECT toString(toStartOfHour(now()) + INTERVAL 30 SECOND)")
        .fetch_one()
        .await
        .expect("current slot");
    for view in live_reconcile_views(&client, db).await {
        client
            .query(&format!(
                "SYSTEM TEST VIEW {db}.{view} SET FAKE TIME '{pinned}'"
            ))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("pin {view}: {e}"));
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

/// The server's real `now()`, truncated to the minute, as epoch seconds —
/// the anchor every seeded row is placed relative to.
async fn now_minute(client: &Client) -> i64 {
    let epoch: u32 = client
        .query("SELECT toUInt32(toUnixTimestamp(toStartOfMinute(now())))")
        .fetch_one()
        .await
        .expect("now");
    i64::from(epoch)
}

/// One `_1m` row of the `(asset, quote 2, sdex)` series. `at` is epoch
/// seconds (minute-aligned). `volume_base = trades · 10`, and every trade is
/// price-forming, so the row is a plain, priced candle.
#[derive(Debug, Clone, Copy)]
struct Seed {
    at: i64,
    asset: u32,
    trades: u32,
    version: u64,
}

/// Insert `_1m` rows, in the order given, as ONE insert.
async fn insert_1m(client: &Client, db: &str, rows: &[Seed]) {
    let values: Vec<String> = rows
        .iter()
        .map(|r| {
            let vb = r.trades * 10;
            format!(
                "(toDateTime({at}), {asset}, 2, 'sdex', 1.00, 1.50, 0.90, 1.10, \
                 {vb}, {vq}, 0, 0, 1.00, {tc}, {v}, {tc}, {vb}, {vq})",
                at = r.at,
                asset = r.asset,
                vq = vb * 5,
                tc = r.trades,
                v = r.version,
            )
        })
        .collect();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
              version, pf_trade_count, pf_volume, pf_price_volume) VALUES {}",
            values.join(", ")
        ))
        .execute()
        .await
        .expect("insert _1m rows");
}

/// `(sum(trade_count), toString(sum(volume_base)))` over `table FINAL`.
async fn tier_totals(client: &Client, db: &str, table: &str) -> (u64, String) {
    client
        .query(&format!(
            "SELECT toUInt64(sum(trade_count)), toString(sum(volume_base)) FROM {db}.{table} FINAL"
        ))
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("totals {table}: {e}"))
}

/// Drive the six FAST MVs fine to coarse with `SYSTEM REFRESH VIEW`, waiting
/// at each tier until its FINAL totals equal `want` — the coarser MV reads this
/// tier FINAL, so it must be settled first. `want` must therefore be what the
/// fast windows can see, and must differ from the previous state for the poll
/// to prove a refresh ran.
async fn drive_fast_chain(client: &Client, db: &str, want: &(u64, String)) {
    for tier in TIERS {
        client
            .query(&format!("SYSTEM REFRESH VIEW {db}.{}", tier.mv))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("refresh {}: {e}", tier.mv));
        let mut got = tier_totals(client, db, tier.target).await;
        for _ in 0..40 {
            if &got == want {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
            got = tier_totals(client, db, tier.target).await;
        }
        assert_eq!(
            &got, want,
            "{}: the fast refresh of {} never reached the expected totals",
            tier.target, tier.mv
        );
    }
}

/// The reconciliation views that exist in `db`, found LIVE (not from
/// [`TIERS`]), bottom-up in the generator's tier order. A schema that ships
/// none yields an empty list, so a reverted `rollups.sql` fails a test on its
/// convergence assertion rather than on an unknown view.
async fn live_reconcile_views(client: &Client, db: &str) -> Vec<String> {
    let live: Vec<String> = client
        .query(&format!(
            "SELECT view FROM system.view_refreshes \
             WHERE database = '{db}' AND startsWith(view, 'mv_reconcile_')"
        ))
        .fetch_all()
        .await
        .expect("list reconcile views");
    let rank = |v: &str| {
        TIERS
            .iter()
            .position(|t| t.reconcile_mv == v)
            .unwrap_or(usize::MAX)
    };
    let mut live = live;
    live.sort_by_key(|v| (rank(v), v.clone()));
    live
}

/// The first hourly slot after the server's real `now()`, as epoch seconds.
/// A second pass uses `next_slot(..) + 3600`, and so on.
async fn next_slot(client: &Client) -> u32 {
    client
        .query("SELECT toUInt32(toUnixTimestamp(toStartOfHour(now()) + INTERVAL 1 HOUR))")
        .fetch_one()
        .await
        .expect("next slot")
}

/// Run ONE reconciliation pass for the hourly `slot` (epoch seconds), the way
/// the scheduler would: every live reconcile view's clock is set to `slot +
/// 30 s`, bottom-up, and `DEPENDS ON` makes each wait for the one below. Then
/// poll `system.view_refreshes` until every view's `last_success_time` (which
/// records the FAKE time) has reached the slot, and `SYSTEM WAIT VIEW` each.
///
/// `SYSTEM WAIT VIEW` alone is not enough: it returns at once if the refresh
/// has not started yet.
async fn drive_reconcile_pass(client: &Client, db: &str, slot: u32) {
    let fake: String = client
        .query(&format!(
            "SELECT toString(toDateTime({slot}) + INTERVAL 30 SECOND)"
        ))
        .fetch_one()
        .await
        .expect("fake time");
    let views = live_reconcile_views(client, db).await;
    for view in &views {
        client
            .query(&format!(
                "SYSTEM TEST VIEW {db}.{view} SET FAKE TIME '{fake}'"
            ))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("fake time {view}: {e}"));
    }
    for view in &views {
        let mut state: (u32, String, String) = (0, String::new(), String::new());
        for _ in 0..80 {
            state = client
                .query(&format!(
                    "SELECT toUInt32(toUnixTimestamp(ifNull(last_success_time, toDateTime(0)))), \
                     toString(status), exception \
                     FROM system.view_refreshes WHERE database = '{db}' AND view = '{view}'"
                ))
                .fetch_one()
                .await
                .unwrap_or_else(|e| panic!("view_refreshes {view}: {e}"));
            if state.0 >= slot {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        assert!(
            state.0 >= slot,
            "{view} never refreshed for the slot {slot}: last_success {} status {} exception {:?}",
            state.0,
            state.1,
            state.2
        );
        client
            .query(&format!("SYSTEM WAIT VIEW {db}.{view}"))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("wait {view}: {e}"));
    }
}

/// `rollup_sql::reconcile_mismatch_select` — what the probe publishes: CLOSED
/// buckets, past the grace, that disagree with their source.
async fn mismatched(client: &Client, db: &str, tier: &Tier) -> u64 {
    let sql = rollup_sql::reconcile_mismatch_select(tier, db).expect("a checked rendering");
    client
        .query(&sql)
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("mismatch {}: {e}", tier.name))
}

/// `count()` over `rollup_sql::reconcile_select` — every disagreeing bucket,
/// open or closed: exactly what the next pass would write.
async fn unreconciled(client: &Client, db: &str, tier: &Tier) -> u64 {
    let sql = rollup_sql::reconcile_select(tier, db).expect("a checked rendering");
    client
        .query(&format!("SELECT count() FROM (\n{sql}\n)"))
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("unreconciled {}: {e}", tier.name))
}

/// Is `at`'s bucket in `tier` closed for at least `MISMATCH_GRACE` — i.e. would
/// the probe's mismatch count see a disagreement there? Asked of the server,
/// with the generator's own interval and grace.
async fn is_past_grace(client: &Client, tier: &Tier, at: i64) -> bool {
    let closed: u8 = client
        .query(&format!(
            "SELECT toStartOfInterval(toDateTime({at}), {i}) + {i} <= now() - {g}",
            i = tier.interval,
            g = rollup_sql::MISMATCH_GRACE,
        ))
        .fetch_one()
        .await
        .expect("bucket age");
    closed == 1
}

/// 0203 AC 1 + AC 4. Rows back-dated three days — older than every fast window
/// that reads them (the 15m hop sees 2 hours) — arrive behind a healthy tip.
/// The fast chain runs and misses them in every tier, and the mismatch signal
/// sees the hole. ONE reconcile pass, ordered by `DEPENDS ON`, rebuilds every
/// tier to the `_1m FINAL` totals, and the mismatch signal returns to zero.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_back_dated_hole_behind_a_healthy_tip_heals_through_every_tier_in_one_pass() {
    let db = "it_reconcile_hole";
    let client = setup(db).await;
    let now = now_minute(&client).await;

    // A healthy tip, rolled through every tier by the fast chain.
    let tip = [
        Seed {
            at: now - 5 * 60,
            asset: 1,
            trades: 2,
            version: LEDGER_VERSION + 1,
        },
        Seed {
            at: now - 4 * 60,
            asset: 1,
            trades: 3,
            version: LEDGER_VERSION + 2,
        },
    ];
    insert_1m(&client, db, &tip).await;
    let tip_totals = tier_totals(&client, db, "price_ohlcv_1m").await;
    assert_eq!(tip_totals.0, 5);
    drive_fast_chain(&client, db, &tip_totals).await;

    // The stall ends: ingest back-fills rows labelled three days ago — two
    // series, spread over several 15m, 1h and 4h buckets — together with one
    // more tip row, so the next fast drive provably runs AFTER the back-fill.
    let hole_at = now - 3 * 86_400;
    let mut hole = Vec::new();
    for (n, minutes) in [0_i64, 7, 20, 45, 70, 110, 250].into_iter().enumerate() {
        for asset in [1, 7] {
            hole.push(Seed {
                at: hole_at + minutes * 60,
                asset,
                trades: 1 + n as u32,
                version: LEDGER_VERSION + 100 + n as u64,
            });
        }
    }
    let late_tip = Seed {
        at: now - 3 * 60,
        asset: 1,
        trades: 4,
        version: LEDGER_VERSION + 3,
    };
    let mut arrival = hole.clone();
    arrival.push(late_tip);
    insert_1m(&client, db, &arrival).await;

    let source = tier_totals(&client, db, "price_ohlcv_1m").await;
    let hole_trades: u64 = hole.iter().map(|r| u64::from(r.trades)).sum();
    assert_eq!(source.0, 5 + 4 + hole_trades, "_1m FINAL holds tip + hole");

    // The fast chain picks up the late tip row and misses the whole hole:
    // every tier equals the tip alone. This is the in-test non-vacuity check —
    // without it a pass could "heal" a hole the fast path had already filled.
    let tip_only: (u64, String) = client
        .query(&format!(
            "SELECT toUInt64(sum(trade_count)), toString(sum(volume_base)) \
             FROM {db}.price_ohlcv_1m FINAL WHERE timestamp >= toDateTime({})",
            now - 86_400
        ))
        .fetch_one()
        .await
        .expect("tip totals");
    assert_eq!(tip_only.0, tip_totals.0 + 4, "the late tip row landed");
    drive_fast_chain(&client, db, &tip_only).await;
    let in_hole: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_ohlcv_15m FINAL WHERE timestamp < toDateTime({})",
            now - 86_400
        ))
        .fetch_one()
        .await
        .expect("hole rows at 15m");
    assert_eq!(
        in_hole, 0,
        "the fast 15m window must not reach a 3-day-old row"
    );
    for tier in TIERS {
        let got = tier_totals(&client, db, tier.target).await;
        assert_ne!(
            got, source,
            "{}: the fast path must have missed the hole",
            tier.target
        );
    }

    // The mismatch signal sees the hole behind the healthy tip (AC 4). Each
    // tier compares against the tier directly BELOW it, so before any pass
    // only the 15m tier — the one reading `_1m` — disagrees: 1h..1M agree with
    // sources that miss the same hole. The DEPENDS ON chain is what carries
    // the repair upward inside one pass.
    let tiers = TIERS;
    let (first, upper) = tiers.split_first().expect("six tiers");
    assert!(
        is_past_grace(&client, first, hole_at + 250 * 60).await,
        "the test's hole must lie in 15m buckets closed past the grace"
    );
    assert!(
        unreconciled(&client, db, first).await > 0,
        "15m: the reconcile SELECT must see the hole"
    );
    assert!(
        mismatched(&client, db, first).await > 0,
        "15m: a closed hole behind a healthy tip must count as a mismatch"
    );
    for tier in upper {
        assert_eq!(
            unreconciled(&client, db, tier).await,
            0,
            "{}: agrees with its source until the tier below is repaired",
            tier.name
        );
    }

    // ONE reconcile pass, ordered by the DEPENDS ON chain.
    let slot = next_slot(&client).await;
    drive_reconcile_pass(&client, db, slot).await;

    for tier in TIERS {
        assert_eq!(
            tier_totals(&client, db, tier.target).await,
            source,
            "{}: one reconcile pass must converge the tier to _1m FINAL",
            tier.target
        );
        assert_eq!(
            unreconciled(&client, db, &tier).await,
            0,
            "{}: nothing left to rewrite",
            tier.name
        );
        assert_eq!(
            mismatched(&client, db, &tier).await,
            0,
            "{}: the mismatch signal returns to zero",
            tier.name
        );
    }

    drop_scratch(&client, db).await;
}
