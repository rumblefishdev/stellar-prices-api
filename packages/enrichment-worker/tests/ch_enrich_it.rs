//! Live-ClickHouse integration tests for the production enrichment pass
//! (`ch_enrich`). Gated `#[ignore]` — they need a reachable ClickHouse:
//!
//!   docker compose up -d clickhouse
//!   cargo test -p enrichment-worker --test ch_enrich_it -- --ignored
//!
//! Each test owns an isolated scratch database (real schema applied from
//! `prices-clickhouse::INIT_SQL`, rewritten onto the scratch name) and drops it
//! at the end, so they never touch `prices` and can run in parallel.

use clickhouse::Client;
use enrichment_worker::ch_enrich::{ChEnrichConfig, ChEnrichmentPass};
use prices_clickhouse::USDC_ISSUER;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Create a fresh scratch database with the full `prices` schema applied under
/// the scratch name (the schema SQL is `prices.*`-qualified; rewrite the prefix
/// and the `CREATE DATABASE` target).
async fn setup_scratch(db: &str) -> Client {
    let client = Client::default().with_url(ch_url());
    client
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!("CREATE DATABASE {db}"))
        .execute()
        .await
        .unwrap();
    let schema = prices_clickhouse::INIT_SQL
        .replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"));
    prices_clickhouse::apply_sql(&client, &schema)
        .await
        .unwrap();
    client
}

async fn close_usd(client: &Client, db: &str, asset: u32, quote: u32, ts: u32) -> f64 {
    client
        .query(&format!(
            "SELECT toFloat64(close_usd) FROM {db}.price_ohlcv_1m FINAL \
             WHERE asset_id = ? AND quote_asset_id = ? AND timestamp = ?"
        ))
        .bind(asset)
        .bind(quote)
        .bind(ts)
        .fetch_one::<f64>()
        .await
        .unwrap()
}

fn cfg(db: &str) -> ChEnrichConfig {
    ChEnrichConfig {
        url: ch_url(),
        database: db.to_string(),
        table: "price_ohlcv_1m".to_string(),
        oracle_name: "reflector".to_string(),
        window_s: 300,
        pivot_window_s: 86_400,
        batch_size: 1000,
        max_batches: 10,
    }
}

const ASSETS: &str = "INSERT INTO {db}.assets \
     (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
     (1,'XLM','classic','',''), (2,'USDC','classic','{usdc}',''), \
     (10,'FOO','classic','GFOO',''), (20,'EXO','classic','GEXO','')";

// (asset, quote): recent FOO/USDC (oracle-covered), deep FOO/USDC (peg),
// deep XLM/USDC pivot source, deep FOO/XLM (pivot), deep FOO/EXO (no reference).
const CANDLES: &str = "INSERT INTO {db}.price_ohlcv_1m \
     (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
      volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
     (1700000000,10, 2,'sdex',    5,5,5,5,             1,  5, 0,0, 5,       1,1), \
     (1600000000,10, 2,'sdex',    4,4,4,4,             1,  4, 0,0, 4,       1,1), \
     (1600000000, 1, 2,'sdex',    0.30,0.30,0.30,0.30, 1000,300,0,0,0.30,  1,1), \
     (1600000000,10, 1,'phoenix', 13.3333,13.3333,13.3333,13.3333, 3,40,0,0,13.3333,1,1), \
     (1600000000,10,20,'sdex',    9,9,9,9,             1,  9, 0,0, 9,       1,1)";

#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn enrich_fills_close_usd_across_oracle_peg_and_pivot_tiers() {
    let db = "it_enrich_tiers";
    let client = setup_scratch(db).await;

    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();
    // Recent oracle USDC price (≠ $1, so the oracle tier is distinguishable from peg).
    client
        .query(&format!(
            "INSERT INTO {db}.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1700000000, 2, 'reflector', 1.0012, '{{}}')"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&CANDLES.replace("{db}", db))
        .execute()
        .await
        .unwrap();

    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    let (recent, deep) = (1_700_000_000u32, 1_600_000_000u32);
    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;
    // Oracle tier wins on the recent candle: 5 × 1.0012 = 5.006 (not the $1 peg).
    assert!(
        approx(close_usd(&client, db, 10, 2, recent).await, 5.006),
        "oracle tier"
    );
    // Deep FOO/USDC: peg ($1) → close = 4.0.
    assert!(
        approx(close_usd(&client, db, 10, 2, deep).await, 4.0),
        "stablecoin peg"
    );
    // Deep XLM/USDC pivot source: peg ($1) → 0.30.
    assert!(
        approx(close_usd(&client, db, 1, 2, deep).await, 0.30),
        "xlm/usdc peg"
    );
    // Deep FOO/XLM pivot: 13.3333 × xlm_usd(0.30) = 4.0.
    assert!(
        approx(close_usd(&client, db, 10, 1, deep).await, 4.0),
        "pivot"
    );
    // Deep FOO/EXO: no USD reference → stays 0.
    assert!(
        approx(close_usd(&client, db, 10, 20, deep).await, 0.0),
        "no reference"
    );

    // Idempotent: a second pass leaves the values unchanged.
    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();
    assert!(
        approx(close_usd(&client, db, 10, 2, recent).await, 5.006),
        "idempotent oracle"
    );
    assert!(
        approx(close_usd(&client, db, 10, 1, deep).await, 4.0),
        "idempotent pivot"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Regression for the Tier-2 premature-peg bug: when the oracle tier exhausts its
/// batch budget while still making progress, the rows it did NOT reach must be
/// deferred to the next run's oracle tier — never pegged to a flat $1, which
/// would clobber the depeg-aware oracle price they are still entitled to.
///
/// Two oracle-covered USDC candles + a budget of one row per pass (`batch_size`
/// = `max_batches` = 1): pass 1 enriches only the earlier candle and leaves the
/// oracle tier un-drained, so the later candle must stay `close_usd = 0` (NOT
/// the $1 peg). Pass 2's oracle tier then drains and gives it the oracle value.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn oracle_budget_exhaustion_defers_instead_of_pegging() {
    let db = "it_enrich_undrained";
    let client = setup_scratch(db).await;

    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();
    // One oracle USDC price ≠ $1, in-window for both candles below (window_s=300,
    // both candles are within 300s of this timestamp). Depeg value so the oracle
    // result (× price) is distinguishable from the flat $1 peg.
    client
        .query(&format!(
            "INSERT INTO {db}.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES (1700000000, 2, 'reflector', 1.0012, '{{}}')"
        ))
        .execute()
        .await
        .unwrap();
    // Two FOO/USDC candles, both oracle-eligible. enrich_batch orders by timestamp
    // ASC, so a 1-row batch reaches the earlier (1700000000) first.
    let (early, late) = (1_700_000_000u32, 1_700_000_300u32);
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({early},10,2,'sdex', 5,5,5,5,    1, 5,0,0, 5,1,1), \
             ({late}, 10,2,'sdex', 10,10,10,10, 1,10,0,0,10,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // Pass 1 with a one-row budget: oracle tier makes progress (early candle) but
    // does not drain → peg-pivot tier deferred.
    let mut budget1 = cfg(db);
    budget1.batch_size = 1;
    budget1.max_batches = 1;
    ChEnrichmentPass::new(budget1).run().await.unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;
    // Early candle: oracle applied (5 × 1.0012 = 5.006).
    assert!(
        approx(close_usd(&client, db, 10, 2, early).await, 5.006),
        "early candle oracle-enriched"
    );
    // Late candle: NOT reached by the budget-limited oracle tier, and NOT pegged
    // (the bug would have written 10 × $1 = 10.0). Deferred → still 0.
    assert!(
        approx(close_usd(&client, db, 10, 2, late).await, 0.0),
        "late candle deferred, not pegged to $1"
    );

    // Pass 2: the oracle tier now drains the late candle and gives it the oracle
    // value (10 × 1.0012 = 10.012), NOT the $1 peg (10.0) — confirming the defer
    // preserved its depeg-aware entitlement rather than losing it.
    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();
    assert!(
        approx(close_usd(&client, db, 10, 2, late).await, 10.012),
        "late candle oracle-enriched on the next pass (not the $1 peg)"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
