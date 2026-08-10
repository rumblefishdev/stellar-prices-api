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
async fn copies_oracle_readings_then_resumes_from_the_watermark() {
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
        .populate_usd_rate_from_oracle(&[usdc()])
        .await
        .unwrap();
    assert_eq!(first.identities, 1);
    assert_eq!(first.watermark_before, Some(0), "nothing stored yet");
    assert_eq!(first.watermark_after, 1750003600);

    let rows = rate_rows(&client).await;
    assert_eq!(rows.len(), 2, "both readings copied");
    assert!((rows[0].1 - 0.9993).abs() < 1e-9, "the real rate, not $1");
    assert_eq!(rows[0].2, "oracle", "method");
    assert_eq!(rows[0].3, 0, "hops = 0 for a measured reading");

    // Re-run with no new readings: idempotent, no duplicates.
    let second = writer
        .populate_usd_rate_from_oracle(&[usdc()])
        .await
        .unwrap();
    assert_eq!(second.watermark_before, Some(1750003600), "watermark held");
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
        .populate_usd_rate_from_oracle(&[usdc()])
        .await
        .unwrap();
    assert_eq!(third.watermark_after, 1750007200);
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
        .populate_usd_rate_from_oracle(&[usdc()])
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
