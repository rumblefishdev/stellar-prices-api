//! Task 0167 — `populate_usd_rate_from_oracle` against a live ClickHouse.
//!
//!   tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!   cargo test -p prices-ingest-core --test usd_rate_population_it -- --ignored --test-threads=1
//!
//! Uses the real `prices` schema rewritten onto a scratch database. The writer
//! hardcodes `prices.*` table names, so the scratch db is selected on the
//! client rather than by rewriting the writer's SQL.

use clickhouse::Client;
use prices_clickhouse::asset_id::fixture::{AssetFixture, assets_insert};
use prices_clickhouse::{USDC_ISSUER, USDT_ISSUER};
use prices_ingest_core::{AssetIdentity, OhlcvWriter};

// The peg assets. Each displays as its id, derived from its identity
// (`asset_id::fixture`), so an oracle row written `({USDC}, …)` agrees with
// the `assets` row.
const USDC: AssetFixture = AssetFixture::new("USDC", "classic", USDC_ISSUER, "");
const USDT: AssetFixture = AssetFixture::new("USDT", "classic", USDT_ISSUER, "");

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn usdc() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "USDC".to_string(),
        issuer: USDC_ISSUER.to_string(),
    }
}

/// The writer's SQL is hardcoded against `prices.*`, so tests cannot each own a
/// scratch database — they share the real one and reset it. That makes them
/// mutually destructive under cargo's default parallel runner (the first
/// version of this file truncated one test's fixture out from under the
/// other's, and BOTH failed in ways that looked like product bugs). This lock
/// serialises them; it is test-harness plumbing, not a statement about the
/// writer, which is safe to call concurrently against distinct identities.
static DB_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn fresh_prices_schema() -> Client {
    let admin = Client::default().with_url(ch_url());
    prices_clickhouse::apply_sql(&admin, prices_clickhouse::INIT_SQL)
        .await
        .unwrap();
    for t in ["usd_rate", "oracle_prices", "assets"] {
        admin
            .query(&format!("TRUNCATE TABLE IF EXISTS prices.{t}"))
            .execute()
            .await
            .unwrap();
    }
    admin
}

async fn seed_usdc(client: &Client) {
    client
        .query(&assets_insert("prices", &[USDC]))
        .execute()
        .await
        .unwrap();
}

/// ARBRIDGE, the asset that sat on another's id on prod (4194, task 0139).
const ARBRIDGE: AssetFixture = AssetFixture::new("ARBRIDGE", "classic", "GARB", "");

/// Tries to put ARBRIDGE on USDC's id, the shape the 0139 guard used to
/// refuse, then inserts ARBRIDGE as a writer must. Returns the first insert's
/// error: the schema derives `assets.asset_id`, so naming it is refused.
async fn try_to_squat_on_usdc_id(client: &Client) -> clickhouse::error::Error {
    let squat = client
        .query(&format!(
            "INSERT INTO prices.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) \
             VALUES ({USDC},'ARBRIDGE','classic','GARB','','')"
        ))
        .execute()
        .await
        .expect_err("assets must refuse a writer that names asset_id");
    client
        .query(&assets_insert("prices", &[ARBRIDGE]))
        .execute()
        .await
        .unwrap();
    squat
}

/// `(rows, distinct ids)` in `assets FINAL`.
async fn identities_and_ids(client: &Client) -> (u64, u64) {
    client
        .query("SELECT count(), uniqExact(asset_id) FROM prices.assets FINAL")
        .fetch_one::<(u64, u64)>()
        .await
        .unwrap()
}

async fn rate_rows(client: &Client) -> Vec<(u32, f64, String, u8)> {
    client
        .query(
            "SELECT toUInt32(timestamp), toFloat64(usd_rate), method, hops \
             FROM prices.usd_rate FINAL WHERE asset_code = 'USDC' ORDER BY timestamp",
        )
        .fetch_all::<(u32, f64, String, u8)>()
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn copies_oracle_readings_and_re_runs_without_duplicating() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client).await;
    let writer = OhlcvWriter::new(client.clone());

    // Two readings, deliberately NOT $1 — the whole point is a depeg-aware rate.
    client
        .query(&format!(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750000000, {USDC}, 'reflector', 0.9993, ''), \
                    (1750003600, {USDC}, 'reflector', 1.0004, '')"
        ))
        .execute()
        .await
        .unwrap();

    let first = writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .unwrap();
    assert_eq!(first.identities, 1);
    assert_eq!(first.rows_inserted, 2, "both readings copied");
    assert_eq!(first.newest, vec![("USDC".to_string(), 1750003600)]);

    let rows = rate_rows(&client).await;
    assert_eq!(rows.len(), 2, "both readings copied");
    assert!((rows[0].1 - 0.9993).abs() < 1e-9, "the real rate, not $1");
    assert_eq!(rows[0].2, "oracle", "method");
    assert_eq!(rows[0].3, 0, "hops = 0 for a measured reading");

    // Re-run with no new readings: idempotent, no duplicates.
    let second = writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .unwrap();
    assert_eq!(second.rows_inserted, 0, "a no-op re-run writes nothing");
    assert_eq!(
        rate_rows(&client).await.len(),
        2,
        "re-running must not duplicate"
    );

    // A new reading arrives; only it is copied.
    client
        .query(&format!(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750007200, {USDC}, 'reflector', 0.9987, '')"
        ))
        .execute()
        .await
        .unwrap();
    let third = writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .unwrap();
    assert_eq!(third.rows_inserted, 1, "only the new reading");
    assert_eq!(third.newest, vec![("USDC".to_string(), 1750007200)]);
    assert_eq!(rate_rows(&client).await.len(), 3, "incremental append");
}

/// ⚠️ The 0139 guard. `oracle_prices` is keyed on `asset_id` and `usd_rate` on
/// natural identity, so this copy is the one place the two key spaces meet.
/// On prod 3,281 ids once served 6,568 identities, and translating through a
/// shared id would file one asset's readings under another's identity. Since
/// 0139 the schema derives the id from the identity, so the shared id cannot
/// be built: the squat is refused, the guard finds one identity per id, and
/// the copy goes ahead. The guard stays as a cheap uniqueness assertion.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_peg_asset_id_cannot_be_shared_so_the_guard_lets_the_copy_through() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client).await;
    let squat = try_to_squat_on_usdc_id(&client).await;
    assert!(squat.to_string().contains("asset_id"), "{squat}");
    assert_eq!(identities_and_ids(&client).await, (2, 2), "one id each");
    client
        .query(&format!(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750000000, {USDC}, 'reflector', 0.9993, '')"
        ))
        .execute()
        .await
        .unwrap();

    let writer = OhlcvWriter::new(client.clone());
    let stats = writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .expect("no id is shared, so nothing is refused");
    assert_eq!(stats.rows_inserted, 1);
    assert_eq!(rate_rows(&client).await.len(), 1);
}

fn usdt() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "USDT".to_string(),
        issuer: USDT_ISSUER.to_string(),
    }
}

/// Review finding 2. `write_oracle` is also called by `sdex-backfill` and the
/// ledger processor's reconcile path, which decode oracle readings from
/// **historical** ledgers — i.e. with timestamps BELOW the current frontier.
/// A `max(timestamp)` watermark would skip those forever, and they would then
/// age out of `oracle_prices` at 13 months: the exact permanent loss this table
/// exists to prevent. The copy must fill gaps wherever they sit.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn snapshots_a_backdated_reading_that_lands_below_the_frontier() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client).await;
    let writer = OhlcvWriter::new(client.clone());

    client
        .query(&format!(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750003600, {USDC}, 'reflector', 1.0004, '')"
        ))
        .execute()
        .await
        .unwrap();
    writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .unwrap();

    // A backfill now writes an OLDER reading — below the frontier just set.
    client
        .query(&format!(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1740000000, {USDC}, 'reflector', 0.9981, '')"
        ))
        .execute()
        .await
        .unwrap();

    let stats = writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .unwrap();
    assert_eq!(
        stats.rows_inserted, 1,
        "a backdated reading must still be snapshotted — a max() watermark \
         would skip it, and it then expires from oracle_prices for good"
    );
    let rows = rate_rows(&client).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].0, 1740000000, "the older row is present");
}

/// Review finding 1 was that a guard failing on a later peg left the earlier
/// pegs written. The guard is now a pre-pass, and since 0139 the collision it
/// refused cannot be stored: with the squatter refused, every peg is copied.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn no_peg_can_collide_so_every_peg_is_copied() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client).await;
    client
        .query(&assets_insert("prices", &[USDT]))
        .execute()
        .await
        .unwrap();
    try_to_squat_on_usdc_id(&client).await;
    assert_eq!(identities_and_ids(&client).await, (3, 3), "one id each");
    client
        .query(&format!(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750000000, {USDC}, 'reflector', 0.9993, ''), \
                    (1750000000, {USDT}, 'reflector', 0.9997, '')"
        ))
        .execute()
        .await
        .unwrap();

    let writer = OhlcvWriter::new(client.clone());
    let stats = writer
        .populate_usd_rate_from_oracle(&[usdc(), usdt()], "reflector")
        .await
        .expect("no peg shares its id");
    assert_eq!((stats.identities, stats.rows_inserted), (2, 2));

    let total: u64 = client
        .query("SELECT count() FROM prices.usd_rate")
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(total, 2, "both pegs copied");
}

/// Task 0086 — folded into 0227 and FIXED 2026-08-27. Kept as a regression
/// guard on the snapshotter, which is the layer that made the defect harmless.
///
/// The shape: the oracle worker's POLL path took `lastprice`'s SECONDS
/// timestamp and divided it by 1000, landing every reading in 1970-01 with a
/// *correct* price. Not intermittent, as 0086 inferred — 100% of that writer's
/// readings, while the EVENT decoder (`soroban.rs`) stamped its own correctly
/// throughout. Two writers, two units. Found in prod while sizing 0167, where
/// `min(timestamp)` on `oracle_prices` read `1970-01-21`.
///
/// Copying those into `usd_rate` would be worse than leaving them upstream:
/// `oracle_prices` sheds them at 13 months, `usd_rate` is retained forever, so
/// a known upstream defect would become permanent history. That reasoning is
/// why the guard outlives the bug — the writer is fixed, but this test pins the
/// property that a malformed upstream timestamp never reaches the forever-table.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn does_not_snapshot_the_0086_junk_1970_timestamps() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client).await;
    let writer = OhlcvWriter::new(client.clone());

    // One good reading and one 0086-shaped row: correct price, epoch/1000.
    client
        .query(&format!(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750000000, {USDC}, 'reflector', 0.9993, ''), \
                    (   1750000, {USDC}, 'reflector', 0.9991, '')"
        ))
        .execute()
        .await
        .unwrap();

    let stats = writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .unwrap();
    assert_eq!(stats.rows_inserted, 1, "only the good reading is copied");

    let rows = rate_rows(&client).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].0, 1750000000,
        "the 1970 row must not be snapshotted"
    );

    let junk: u64 = client
        .query("SELECT count() FROM prices.usd_rate WHERE timestamp < toDateTime('2020-01-01')")
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(
        junk, 0,
        "no pre-2020 rows may reach the forever-retained table"
    );
}
