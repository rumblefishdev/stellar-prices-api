//! Task 0167 — `populate_usd_rate_from_oracle` against a live ClickHouse.
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-ingest-core --test usd_rate_population_it -- --ignored
//!
//! Uses the real `prices` schema rewritten onto a scratch database. The writer
//! hardcodes `prices.*` table names, so the scratch db is selected on the
//! client rather than by rewriting the writer's SQL.

use clickhouse::Client;
use prices_ingest_core::{AssetIdentity, OhlcvWriter};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn usdc() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "USDC".to_string(),
        issuer: prices_clickhouse::USDC_ISSUER.to_string(),
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

async fn seed_usdc(client: &Client, asset_id: u32) {
    client
        .query(&format!(
            "INSERT INTO prices.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) \
             VALUES ({asset_id},'USDC','classic','{}','','')",
            prices_clickhouse::USDC_ISSUER
        ))
        .execute()
        .await
        .unwrap();
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
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn copies_oracle_readings_and_re_runs_without_duplicating() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client, 3).await;
    let writer = OhlcvWriter::new(client.clone());

    // Two readings, deliberately NOT $1 — the whole point is a depeg-aware rate.
    client
        .query(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750000000, 3, 'reflector', 0.9993, ''), \
                    (1750003600, 3, 'reflector', 1.0004, '')",
        )
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
        .query(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750007200, 3, 'reflector', 0.9987, '')",
        )
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
/// With 3,281 ids serving 6,568 identities on prod, translating without
/// checking would file one asset's readings under another's identity — in a
/// table built to be trusted forever. The write must be REFUSED, not attempted.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn refuses_to_write_when_the_peg_asset_id_is_shared() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client, 3).await;
    // A second, unrelated identity squatting on the same surrogate id.
    client
        .query(
            "INSERT INTO prices.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) \
             VALUES (3,'ARBRIDGE','classic','GARB','','')",
        )
        .execute()
        .await
        .unwrap();
    client
        .query(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750000000, 3, 'reflector', 0.9993, '')",
        )
        .execute()
        .await
        .unwrap();

    let writer = OhlcvWriter::new(client.clone());
    let err = writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .expect_err("a shared asset_id must refuse the write");
    let msg = err.to_string();
    assert!(msg.contains("0139"), "error must name the cause: {msg}");

    assert_eq!(
        rate_rows(&client).await.len(),
        0,
        "refusing means writing NOTHING — a partial write is the failure mode \
         this guard exists to prevent"
    );
}

fn usdt() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "USDT".to_string(),
        issuer: prices_clickhouse::USDT_ISSUER.to_string(),
    }
}

/// Review finding 2. `write_oracle` is also called by `sdex-backfill` and the
/// ledger processor's reconcile path, which decode oracle readings from
/// **historical** ledgers — i.e. with timestamps BELOW the current frontier.
/// A `max(timestamp)` watermark would skip those forever, and they would then
/// age out of `oracle_prices` at 13 months: the exact permanent loss this table
/// exists to prevent. The copy must fill gaps wherever they sit.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn snapshots_a_backdated_reading_that_lands_below_the_frontier() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client, 3).await;
    let writer = OhlcvWriter::new(client.clone());

    client
        .query(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750003600, 3, 'reflector', 1.0004, '')",
        )
        .execute()
        .await
        .unwrap();
    writer
        .populate_usd_rate_from_oracle(&[usdc()], "reflector")
        .await
        .unwrap();

    // A backfill now writes an OLDER reading — below the frontier just set.
    client
        .query(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1740000000, 3, 'reflector', 0.9981, '')",
        )
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

/// Review finding 1. Guarding per-identity *inside* the write loop meant a
/// failure on a later identity left earlier identities already written — a
/// partial write, which is the failure mode the guard exists to prevent. The
/// original test only used one identity, so it could not catch this.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_collision_on_one_peg_writes_nothing_for_any_peg() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client, 3).await;
    // USDT is clean...
    client
        .query(&format!(
            "INSERT INTO prices.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) \
             VALUES (111,'USDT','classic','{}','','')",
            prices_clickhouse::USDT_ISSUER
        ))
        .execute()
        .await
        .unwrap();
    // ...but USDC's surrogate id is shared, and USDC is processed FIRST.
    client
        .query(
            "INSERT INTO prices.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) \
             VALUES (3,'ARBRIDGE','classic','GARB','','')",
        )
        .execute()
        .await
        .unwrap();
    client
        .query(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750000000, 3, 'reflector', 0.9993, ''), \
                    (1750000000, 111, 'reflector', 0.9997, '')",
        )
        .execute()
        .await
        .unwrap();

    let writer = OhlcvWriter::new(client.clone());
    let err = writer
        .populate_usd_rate_from_oracle(&[usdc(), usdt()], "reflector")
        .await
        .expect_err("one bad peg must fail the whole pass");
    assert!(err.to_string().contains("0139"), "{err}");

    let total: u64 = client
        .query("SELECT count() FROM prices.usd_rate")
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(
        total, 0,
        "USDT is clean but must NOT be written — a guard that fails after a \
         partial write is worse than no guard"
    );
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
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn does_not_snapshot_the_0086_junk_1970_timestamps() {
    let _guard = DB_LOCK.lock().await;
    let client = fresh_prices_schema().await;
    seed_usdc(&client, 3).await;
    let writer = OhlcvWriter::new(client.clone());

    // One good reading and one 0086-shaped row: correct price, epoch/1000.
    client
        .query(
            "INSERT INTO prices.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1750000000, 3, 'reflector', 0.9993, ''), \
                    (   1750000, 3, 'reflector', 0.9991, '')",
        )
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
