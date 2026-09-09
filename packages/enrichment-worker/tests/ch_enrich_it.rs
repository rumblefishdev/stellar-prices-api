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
use enrichment_worker::ch_enrich::{ChEnrichConfig, ChEnrichError, ChEnrichmentPass, UsdResetSpec};
use enrichment_worker::repair::{
    CoarseRepairConfig, CoarseRepairDriver, CoarseSweepConfig, run_coarse_sweep,
};
use prices_clickhouse::{USDC_ISSUER, USDC_ORACLE_EPOCH_S, USDT_ISSUER};

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

/// Read a scalar `Float64` expression from the FOO/USDC bucket of the coarse
/// `price_ohlcv_1h` table (task 0114 repair-target). `FINAL` collapses the RMT so
/// the value returned is the version-winning row.
async fn coarse_1h_f64(client: &Client, db: &str, col_expr: &str, ts: u32) -> f64 {
    client
        .query(&format!(
            "SELECT toFloat64({col_expr}) FROM {db}.price_ohlcv_1h FINAL \
             WHERE asset_id = 10 AND quote_asset_id = 2 AND timestamp = ?"
        ))
        .bind(ts)
        .fetch_one::<f64>()
        .await
        .unwrap()
}

/// Read a scalar `UInt64` expression from the same coarse bucket (used for
/// `version` and `trade_count`).
async fn coarse_1h_u64(client: &Client, db: &str, col_expr: &str, ts: u32) -> u64 {
    client
        .query(&format!(
            "SELECT toUInt64({col_expr}) FROM {db}.price_ohlcv_1h FINAL \
             WHERE asset_id = 10 AND quote_asset_id = 2 AND timestamp = ?"
        ))
        .bind(ts)
        .fetch_one::<u64>()
        .await
        .unwrap()
}

/// `close_usd` of an arbitrary `price_ohlcv_1h` bucket (asset/quote/ts), for the
/// coarse-repair driver test which spans multiple pairs and months.
async fn coarse_1h_close_usd_of(client: &Client, db: &str, asset: u32, quote: u32, ts: u32) -> f64 {
    client
        .query(&format!(
            "SELECT toFloat64(close_usd) FROM {db}.price_ohlcv_1h FINAL \
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
        // Wide (10y) so the recency bound never excludes fixture candles: the
        // existing assertions are on the *total* volume-zero backlog. The
        // recency split itself is exercised by `recency_bounded_backlog_*`.
        recent_window_s: 315_360_000,
        batch_size: 1000,
        max_batches: 10,
        one_shot: false,
        time_window: None,
        usd_reset: None,
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

    // Task 0083: the XLM/USDC reference is now computed inline as an ASOF-join
    // subquery — no `CREATE TABLE` at all — so no `*_xlmusd_ref_*` table is ever
    // created (previously a run-scoped table dropped on completion, review #10).
    let leftover: u64 = client
        .query(&format!(
            "SELECT count() FROM system.tables WHERE database = '{db}' AND name LIKE '%xlmusd_ref%'"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(
        leftover, 0,
        "no pivot reference table should ever be created (inline subquery)"
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
    let pass1 = ChEnrichmentPass::new(budget1).run().await.unwrap();

    // The oracle tier exhausted its 1-batch budget while still making progress
    // (not drained), so the unreached late candle must NOT be reported as an
    // oracle miss — `oracle_misses` is 0, not the un-processed remainder (1).
    // (Regression guard for the EnrichmentOracleMiss over-count.)
    assert_eq!(
        pass1.oracle_misses, 0,
        "budget-exhausted (un-drained) oracle tier reports 0 confirmed misses"
    );

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

/// Snapshot-watermark bound (review #5): a pass only enriches candidates with
/// `timestamp <= watermark`, so a candle the live writer inserts after the
/// snapshot is taken (newer timestamp) is deferred to the next run rather than
/// being counted — which would otherwise inflate the candidate count and falsely
/// trip the no-progress break. `run_through` pins the boundary so a single-
/// threaded test can stand in for the concurrent insert.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn watermark_defers_candles_newer_than_the_snapshot() {
    let db = "it_enrich_watermark";
    let client = setup_scratch(db).await;

    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();
    // Two FOO/USDC candles enrichable via the $1 peg (no oracle row needed). The
    // 'old' one is at/below the pinned watermark; the 'new' one is above it,
    // standing in for a candle inserted by the live processor mid-pass.
    let (old, new) = (1_700_000_000u32, 1_700_000_600u32);
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({old},10,2,'sdex', 5,5,5,5, 1, 5,0,0, 5,1,1), \
             ({new},10,2,'sdex', 8,8,8,8, 1, 8,0,0, 8,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;

    // Pin the watermark at `old`: only that candle is in this pass's population.
    let stats = ChEnrichmentPass::new(cfg(db))
        .run_through(old)
        .await
        .unwrap();
    assert_eq!(
        stats.candidates_before, 1,
        "newer candle excluded from the snapshot"
    );
    assert!(
        approx(close_usd(&client, db, 10, 2, old).await, 5.0),
        "old candle pegged"
    );
    // The newer candle was above the watermark cutoff → untouched this pass.
    assert!(
        approx(close_usd(&client, db, 10, 2, new).await, 0.0),
        "newer candle deferred, not enriched"
    );

    // A normal run (watermark = max(timestamp) = new) now picks it up.
    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();
    assert!(
        approx(close_usd(&client, db, 10, 2, new).await, 8.0),
        "newer candle enriched on the next run"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// One-shot mode (`max_batches = 0`): a single run drains the entire backlog to
/// completion, instead of the bounded `MAX_BATCHES × BATCH_SIZE` rows the hourly
/// cron caps at (spec §4). Seeds five peg-enrichable candles and runs with a
/// one-row batch + unbounded budget — all five must be enriched in the one pass,
/// with the new `ChPassStats` fields populated (oracle_misses = 5 handed to the
/// peg tier, rows_enriched = 5, candidates_after = 0, duration_ms > 0).
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn one_shot_drains_full_backlog() {
    let db = "it_enrich_oneshot";
    let client = setup_scratch(db).await;
    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    // Five FOO/USDC candles, all peg-enrichable ($1), distinct timestamps and
    // closes (close=i+2 → close_usd=i+2 under the peg).
    let values: Vec<String> = (0..5u32)
        .map(|i| {
            let (ts, c) = (1_600_000_000 + i * 60, i + 2);
            format!("({ts},10,2,'sdex', {c},{c},{c},{c}, 1,{c},0,0,{c},1,1)")
        })
        .collect();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES {}",
            values.join(", ")
        ))
        .execute()
        .await
        .unwrap();

    // One-row batch + explicit one-shot flag: drain all five at once regardless
    // of max_batches (left at the cfg default, proving one_shot overrides it).
    let mut oneshot = cfg(db);
    oneshot.batch_size = 1;
    oneshot.one_shot = true;
    let stats = ChEnrichmentPass::new(oneshot).run().await.unwrap();

    assert_eq!(stats.candidates_before, 5);
    assert_eq!(stats.rows_enriched, 5, "one-shot drained the full backlog");
    assert_eq!(stats.candidates_after, 0, "no candidate left at zero");
    assert_eq!(
        stats.oracle_misses, 5,
        "no oracle rows → all five handed to the peg tier"
    );

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;
    for i in 0..5u32 {
        let (ts, c) = (1_600_000_000 + i * 60, (i + 2) as f64);
        assert!(
            approx(close_usd(&client, db, 10, 2, ts).await, c),
            "candle {i} pegged to close_usd"
        );
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Recency-bounded backlog (task 0026 finding #5): the permanent exotic-quote
/// floor — pairs with no oracle/peg reference that never enrich — is deep in
/// history, so a recency-windowed count excludes it while still catching a
/// *fresh* stuck candle. This is the series the stall alarm gates on, so an idle
/// env (no new candles) reports zero instead of latching on the floor.
///
/// Fixture: an exotic FOO/EXO pair with no USDC/USDT/XLM reference and no oracle
/// price, so neither candle enriches. One candle sits deep in history, one at
/// `now()`. With a 1-hour recency window the full backlog is 2 but the recency-
/// bounded count is just the fresh candle (1).
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn recency_bounded_backlog_excludes_deep_history_floor() {
    let db = "it_enrich_recency";
    let client = setup_scratch(db).await;

    // Only exotic assets: no USDC/USDT/XLM reference exists, so the peg-pivot
    // tier finds nothing and FOO/EXO is the permanent, never-draining floor.
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (10,'FOO','classic','GFOO',''), (20,'EXO','classic','GEXO','')"
        ))
        .execute()
        .await
        .unwrap();

    // Two zero-USD FOO/EXO candles: one deep in history, one at `now()`.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (1600000000,        10,20,'sdex', 9,9,9,9, 1,9,0,0, 9,1,1), \
             (toUnixTimestamp(now()),10,20,'sdex', 9,9,9,9, 1,9,0,0, 9,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // 1-hour recency window: the deep-history candle is far outside it.
    let mut recency = cfg(db);
    recency.recent_window_s = 3_600;
    let stats = ChEnrichmentPass::new(recency).run().await.unwrap();

    // Nothing enriches — no reference of any kind for the exotic quote …
    assert_eq!(stats.rows_enriched, 0, "exotic floor never enriches");
    // … so the full volume-zero backlog is both candles …
    assert_eq!(
        stats.rows_remaining_at_volume_zero, 2,
        "full backlog counts the whole floor"
    );
    // … but only the `now()` candle falls inside the recency window: the alarm's
    // series excludes the permanent deep-history floor (finding #5), so an idle
    // env with only deep floor would read 0 here and never trip the stall alarm.
    assert_eq!(
        stats.rows_remaining_recent, 1,
        "recency-bounded count excludes the deep-history floor"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Version-arithmetic safety for the coarse-table repair (task 0114).
///
/// The repair retargets the enrichment INSERT at a coarse table (`price_ohlcv_1h`)
/// whose `version` is a *sum* of source versions (`rollups.sql:24-37`), so a
/// repair row must outrank a potentially large existing value — not a naive `+1`
/// from a fresh row. Every write statement projects `p.version + 1 AS version`
/// reading `FROM {tbl} AS p FINAL` (`ch_enrich.rs:380`/`:696`/`:729`), i.e. it
/// derives the new version from the existing (post-collapse) row, so V → V+1 wins
/// even when V is large.
///
/// This is the AC that "fails silently": a constant/fresh version would LOSE the
/// RMT race to the seeded sum, leaving the zero in place — a repair that appears
/// to run yet changes nothing. The test seeds a large summed version and pins the
/// win. It also asserts the repair is ADDITIVE and non-destructive (OHLC/volume
/// carried through verbatim; two physical rows coexist pre-merge — no in-place
/// DELETE/UPDATE), which is the sole-copy safety property: the 1m source for the
/// affected historical span is gone, so the coarse table cannot be rebuilt.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn coarse_repair_row_outranks_large_summed_version() {
    let db = "it_enrich_coarse_version";
    let client = setup_scratch(db).await;

    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    // One coarse FOO/USDC `price_ohlcv_1h` bucket, zero USD, carrying a LARGE
    // version that stands in for a rolled-up `sum(version)`. Distinct OHLC values
    // (7/9/6/8) and volume (base 5, quote 40, trade_count 3) so the pass-through
    // assertion is meaningful. A fresh-row `+1` (=2) would lose to 5000; only
    // `existing + 1` (=5001) wins the ReplacingMergeTree merge.
    let ts = 1_600_000_000u32;
    let seeded_version = 5000u64;
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({ts},10,2,'sdex', 7,9,6,8, 5,40,0,0, 8, 3, {seeded_version})"
        ))
        .execute()
        .await
        .unwrap();

    // Enrichment retargeted at the coarse table (the whole point of the repair):
    // FOO/USDC hits the stablecoin-direct (peg) tier — no oracle row seeded.
    let mut coarse = cfg(db);
    coarse.table = "price_ohlcv_1h".to_string();
    ChEnrichmentPass::new(coarse).run().await.unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;

    // 1) The zero was repaired: peg ($1) → close_usd = close = 8.0,
    //    volume_quote_usd = volume_quote = 40.0. If the repair row had LOST the
    //    version race, FINAL would still return the seeded zeros — this is the
    //    core silent-failure assertion.
    assert!(
        approx(coarse_1h_f64(&client, db, "close_usd", ts).await, 8.0),
        "coarse close_usd repaired via peg"
    );
    assert!(
        approx(
            coarse_1h_f64(&client, db, "volume_quote_usd", ts).await,
            40.0
        ),
        "coarse volume_quote_usd repaired via peg"
    );

    // 2) The winning row's version strictly increased and derives from the
    //    existing summed value: 5000 → 5001. NOT a constant/fresh version.
    assert_eq!(
        coarse_1h_u64(&client, db, "version", ts).await,
        seeded_version + 1,
        "repair version = existing sum + 1 (derives from the existing row)"
    );

    // 3) Non-USD columns carried through verbatim — additive enrichment, NOT a
    //    re-aggregation. Any drift here would mean the repair rewrote price/volume
    //    data it was only meant to leave alone.
    assert!(
        approx(coarse_1h_f64(&client, db, "open", ts).await, 7.0),
        "open kept"
    );
    assert!(
        approx(coarse_1h_f64(&client, db, "high", ts).await, 9.0),
        "high kept"
    );
    assert!(
        approx(coarse_1h_f64(&client, db, "low", ts).await, 6.0),
        "low kept"
    );
    assert!(
        approx(coarse_1h_f64(&client, db, "close", ts).await, 8.0),
        "close kept"
    );
    assert!(
        approx(coarse_1h_f64(&client, db, "volume_base", ts).await, 5.0),
        "volume_base kept"
    );
    assert!(
        approx(coarse_1h_f64(&client, db, "volume_quote", ts).await, 40.0),
        "volume_quote kept"
    );
    assert_eq!(
        coarse_1h_u64(&client, db, "trade_count", ts).await,
        3,
        "trade_count kept"
    );

    // 4) ADDITIVE, not in-place: two physical rows exist pre-merge (the seeded
    //    zero at v5000 and the repair at v5001) — the enrichment issued no
    //    DELETE/UPDATE, only an INSERT. Sole-copy safety: nothing is destroyed at
    //    write time, so a bad run can be reverted from the pre-merge state.
    let physical: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_ohlcv_1h \
             WHERE asset_id = 10 AND quote_asset_id = 2 AND timestamp = ?"
        ))
        .bind(ts)
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(physical, 2, "append-only: seed + repair coexist pre-merge");

    // 5) Idempotent: the FINAL row is now fully enriched (volume_quote_usd>0 AND
    //    close_usd>0), so a second pass admits no candidate and writes nothing —
    //    version stays 5001. Re-running the repair is safe.
    let mut coarse2 = cfg(db);
    coarse2.table = "price_ohlcv_1h".to_string();
    ChEnrichmentPass::new(coarse2).run().await.unwrap();
    assert_eq!(
        coarse_1h_u64(&client, db, "version", ts).await,
        seeded_version + 1,
        "idempotent: no re-write on an already-enriched coarse row"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The coarse-repair driver (task 0114): enumerate months-with-zeros in a span,
/// snapshot + partition-bounded-repair each, and report per-month before/after.
///
/// Fixture — all in `price_ohlcv_1h`, each a zero-USD FOO/USDC (peg-enrichable)
/// bucket except one exotic FOO/EXO with no reference, seeded at a large
/// `version` so the repair also has to win the RMT race:
///   - 2025-02 FOO/USDC (in span)      → repaired to close_usd = 8
///   - 2025-02 FOO/EXO  (in span)      → no reference → stays 0 (the floor)
///   - 2025-05 FOO/USDC (in span)      → repaired to close_usd = 11
///   - 2024-01 FOO/USDC (OUT of span)  → untouched → stays 0
///
/// Asserts: only the two in-span USDC buckets are repaired; the exotic row and
/// the out-of-span row stay zero (span + reference bounding both hold); and the
/// summary reports the correct per-month before/after (exotic floor surfaces as
/// `zeros_after = 1`, not a failure).
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn coarse_repair_driver_bounds_span_and_reports_per_month() {
    let db = "it_coarse_repair_driver";
    let client = setup_scratch(db).await;

    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    // Month anchors (unix seconds, 15th 00:00 UTC): 2025-02, 2025-05, 2024-01.
    let (feb2025, may2025, jan2024) = (1_739_577_600u32, 1_747_267_200u32, 1_705_276_800u32);
    let v = 5000u64; // large seeded version — repair must outrank it
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({feb2025},10, 2,'sdex', 8,8,8,8,     1,40,0,0, 8,1,{v}), \
             ({feb2025},10,20,'sdex', 9,9,9,9,     1,40,0,0, 9,1,{v}), \
             ({may2025},10, 2,'sdex', 11,11,11,11, 1,40,0,0,11,1,{v}), \
             ({jan2024},10, 2,'sdex', 5,5,5,5,     1,40,0,0, 5,1,{v})"
        ))
        .execute()
        .await
        .unwrap();

    // Repair the span [2025-02, 2025-05], snapshotting each partition first.
    let mut enrich = cfg(db);
    enrich.table = "price_ohlcv_1h".to_string();
    let driver = CoarseRepairDriver::with_client(
        client.clone(),
        CoarseRepairConfig {
            enrich,
            start_month: 202_502,
            end_month: 202_505,
            snapshot: true,
            dry_run: false,
            one_shot: true,
            deadline: None,
        },
    );
    let summary = driver.run().await.unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;

    // In-span USDC buckets repaired via the peg tier …
    assert!(
        approx(
            coarse_1h_close_usd_of(&client, db, 10, 2, feb2025).await,
            8.0
        ),
        "2025-02 FOO/USDC repaired"
    );
    assert!(
        approx(
            coarse_1h_close_usd_of(&client, db, 10, 2, may2025).await,
            11.0
        ),
        "2025-05 FOO/USDC repaired"
    );
    // … the exotic in-span pair has no reference → stays the no_reference floor …
    assert!(
        approx(
            coarse_1h_close_usd_of(&client, db, 10, 20, feb2025).await,
            0.0
        ),
        "2025-02 FOO/EXO has no USD path — stays 0"
    );
    // … and the out-of-span month is never touched (span bound holds).
    assert!(
        approx(
            coarse_1h_close_usd_of(&client, db, 10, 2, jan2024).await,
            0.0
        ),
        "2024-01 is outside [202502,202505] — untouched"
    );

    // Summary: two months with zeros, in order; per-month before/after correct.
    assert_eq!(
        summary.months.len(),
        2,
        "only the two in-span months had zeros"
    );
    let feb = &summary.months[0];
    assert_eq!(feb.month, 202_502);
    assert_eq!(feb.zeros_before, 2, "2025-02 had USDC + EXO zeros");
    assert_eq!(feb.zeros_after, 1, "EXO remains — the no_reference floor");
    assert_eq!(feb.rows_enriched, 1);
    assert!(feb.snapshot_name.is_some(), "partition was frozen");
    let may = &summary.months[1];
    assert_eq!(may.month, 202_505);
    assert_eq!(may.zeros_before, 1);
    assert_eq!(may.zeros_after, 0);
    assert_eq!(may.rows_enriched, 1);
    assert_eq!(summary.total_enriched(), 2);
    assert_eq!(
        summary.total_remaining(),
        1,
        "one exotic floor row across the span"
    );

    // Clean up the server-global FREEZE snapshots this test created.
    for m in &summary.months {
        if let Some(name) = &m.snapshot_name {
            let _ = client
                .query(&format!("SYSTEM UNFREEZE WITH NAME '{name}'"))
                .execute()
                .await;
        }
    }
    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The recurring coarse sweep folded into the hourly enrichment Lambda (task
/// 0114). Exercises the three properties the handler relies on:
///
///   1. **Trailing window from `now()`** — the sweep computes `[prev-month,
///      this-month]` (lookback 2) off the CH server clock, so an in-window bucket
///      is enriched while a 6-months-ago bucket (out of window) is left at zero,
///      proving partition-bounding (task 0111) without a fixed month argument.
///   2. **Multi-table** — it sweeps every configured coarse table.
///   3. **Non-coarse tables are refused** — `price_ohlcv_1m` (the live base) is
///      recorded under `skipped_tables` (NOT `failed_tables`, which is the alarm
///      series) and never touched.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn coarse_sweep_bounds_trailing_window_and_refuses_the_1m_base() {
    let db = "it_coarse_sweep";
    let client = setup_scratch(db).await;

    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    // Peg-enrichable FOO/USDC buckets. For each coarse table: one in the current
    // month, one in the previous month (both inside a lookback-2 window), and one
    // six months back (outside it). The 1m base gets one current-month zero row
    // to prove the sweep refuses it. All timestamps are computed from now() so the
    // window match is clock-relative, not hard-coded.
    for tbl in ["price_ohlcv_1h", "price_ohlcv_4h", "price_ohlcv_1m"] {
        let rows = if tbl == "price_ohlcv_1m" {
            "(toUnixTimestamp(toStartOfMonth(now())), 10,2,'sdex', 8,8,8,8, 1,40,0,0,8,1,1)"
                .to_string()
        } else {
            "(toUnixTimestamp(toStartOfMonth(now())),                    10,2,'sdex', 8,8,8,8, 1,40,0,0,8,1,1), \
             (toUnixTimestamp(toStartOfMonth(now() - INTERVAL 1 MONTH)), 10,2,'sdex', 8,8,8,8, 1,40,0,0,8,1,1), \
             (toUnixTimestamp(now() - INTERVAL 6 MONTH),                 10,2,'sdex', 5,5,5,5, 1,40,0,0,5,1,1)".to_string()
        };
        client
            .query(&format!(
                "INSERT INTO {db}.{tbl} \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
                 VALUES {rows}"
            ))
            .execute()
            .await
            .unwrap();
    }

    // Expected trailing window, straight from the same clock the sweep reads.
    let start_expected: u32 = client
        .query("SELECT toYYYYMM(now() - INTERVAL 1 MONTH)")
        .fetch_one::<u32>()
        .await
        .unwrap();
    let end_expected: u32 = client
        .query("SELECT toYYYYMM(now())")
        .fetch_one::<u32>()
        .await
        .unwrap();

    let sweep_cfg = CoarseSweepConfig {
        base: cfg(db),
        // 1m is deliberately included to prove it is refused, not swept.
        tables: vec![
            "price_ohlcv_1h".to_string(),
            "price_ohlcv_4h".to_string(),
            "price_ohlcv_1m".to_string(),
        ],
        lookback_months: 2,
        max_batches: 10,
    };
    // No wall-clock limit for this test — exercise the full window.
    let sum = run_coarse_sweep(&client, &sweep_cfg, None).await.unwrap();

    // Window resolved from now() …
    assert_eq!(sum.start_month, start_expected);
    assert_eq!(sum.end_month, end_expected);
    // … two coarse tables swept, the 1m base refused.
    assert_eq!(sum.tables.len(), 2, "1h and 4h swept");
    // The refusal lands on `skipped_tables` (benign config), NOT `failed_tables`
    // (the dead-sweep alarm series) — a mis-config must never trip the alarm.
    assert!(sum.failed_tables.is_empty(), "no runtime failures");
    assert_eq!(
        sum.skipped_tables,
        vec!["price_ohlcv_1m".to_string()],
        "the live 1m base is refused (skipped), never swept"
    );
    // Two in-window rows per table enriched; the out-of-window rows aren't even
    // enumerated, so nothing is left "remaining" in the swept months.
    assert_eq!(
        sum.total_enriched(),
        4,
        "2 in-window rows × 2 coarse tables"
    );
    assert_eq!(sum.total_remaining(), 0);

    // Partition-bounding held: each coarse table keeps exactly ONE zero — the
    // six-months-ago bucket the trailing window never reached.
    for tbl in ["price_ohlcv_1h", "price_ohlcv_4h"] {
        let zeros: u64 = client
            .query(&format!(
                "SELECT count() FROM {db}.{tbl} FINAL WHERE close_usd = 0"
            ))
            .fetch_one::<u64>()
            .await
            .unwrap();
        assert_eq!(zeros, 1, "{tbl}: only the out-of-window bucket stays zero");
    }
    // And the 1m base is untouched — its current-month zero is still zero.
    let base_zeros: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_ohlcv_1m FINAL WHERE close_usd = 0"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(base_zeros, 1, "the sweep never wrote the 1m base table");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Wall-clock budget (task 0114). A slow catch-up must not run into the Lambda
/// hard-timeout — which is an invocation error the best-effort handler cannot
/// catch — so the sweep stops at its deadline and defers the rest. With an
/// already-elapsed deadline it must enrich NOTHING and record no failures/skips
/// (deferred ≠ failed), leaving the seeded zero for the next run.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn coarse_sweep_defers_all_work_past_its_deadline() {
    let db = "it_coarse_sweep_deadline";
    let client = setup_scratch(db).await;

    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    // One in-window, peg-enrichable FOO/USDC bucket that WOULD be repaired with no
    // deadline.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES (toUnixTimestamp(toStartOfMonth(now())), 10,2,'sdex', 8,8,8,8, 1,40,0,0,8,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    let sweep_cfg = CoarseSweepConfig {
        base: cfg(db),
        tables: vec!["price_ohlcv_1h".to_string()],
        lookback_months: 2,
        max_batches: 10,
    };
    // Deadline already in the past → the pre-table check trips immediately.
    let past = std::time::Instant::now();
    let sum = run_coarse_sweep(&client, &sweep_cfg, Some(past))
        .await
        .unwrap();

    // Nothing swept, and — crucially — the deferred table is NOT a failure/skip.
    assert!(
        sum.tables.is_empty(),
        "deadline stopped the sweep before any table"
    );
    assert!(sum.failed_tables.is_empty(), "deferred is not failed");
    assert!(sum.skipped_tables.is_empty(), "deferred is not skipped");
    assert_eq!(sum.total_enriched(), 0);

    // The seeded zero is untouched — it will be picked up on a future run.
    let zeros: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_ohlcv_1h FINAL WHERE close_usd = 0"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(zeros, 1, "the in-window zero was deferred, not repaired");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The `one_shot` knob for the recurring sweep (task 0114).
///
/// The manual historical repair drains each month fully (`one_shot: true`, proven
/// by `coarse_repair_driver_bounds_span_and_reports_per_month`). The recurring
/// sweep folded into the hourly enrichment Lambda must instead be **bounded**
/// (`one_shot: false`): each run enriches at most `max_batches × batch_size` rows
/// of a month and defers the overflow to the next run, so an unexpectedly large
/// recent backlog can never exceed the function timeout. This proves both halves —
/// a single bounded run does NOT drain the whole backlog, and successive runs
/// converge it to the `no_reference` floor.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn coarse_repair_bounded_mode_defers_overflow_across_runs() {
    let db = "it_coarse_repair_bounded";
    let client = setup_scratch(db).await;

    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    // Seven peg-enrichable FOO/USDC 1h buckets in one month (2025-02), all at
    // close_usd = 0. The peg tier fills close_usd = close × $1.
    let feb2025 = 1_739_577_600u32; // 2025-02-15 00:00 UTC
    let values: Vec<String> = (0..7u32)
        .map(|i| {
            let (ts, c) = (feb2025 + i * 3600, i + 2);
            format!("({ts},10,2,'sdex', {c},{c},{c},{c}, 1,40,0,0,{c},1,1)")
        })
        .collect();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES {}",
            values.join(", ")
        ))
        .execute()
        .await
        .unwrap();

    // Bounded config: max_batches = 2, batch_size = 2 → at most 4 rows per run.
    // (One oracle batch no-ops and drains — no oracle_prices — then the peg tier
    // runs its own 2-batch budget: 2 batches × 2 rows = 4 enriched, 3 deferred.)
    let mut enrich = cfg(db);
    enrich.table = "price_ohlcv_1h".to_string();
    enrich.max_batches = 2;
    enrich.batch_size = 2;
    let repair_cfg = CoarseRepairConfig {
        enrich,
        start_month: 202_502,
        end_month: 202_502,
        snapshot: false, // the recurring sweep never freezes (recent, not sole-copy)
        dry_run: false,
        one_shot: false, // ← the knob under test: BOUNDED, not full-drain
        deadline: None,  // no wall-clock limit here — testing the batch bound
    };

    // Run 1 — bounded: 4 of the 7 enriched, the overflow deferred (not drained).
    let s1 = CoarseRepairDriver::with_client(client.clone(), repair_cfg.clone())
        .run()
        .await
        .unwrap();
    assert_eq!(s1.months.len(), 1);
    let feb1 = &s1.months[0];
    assert_eq!(feb1.month, 202_502);
    assert_eq!(feb1.zeros_before, 7, "full month backlog before run 1");
    assert_eq!(
        feb1.rows_enriched, 4,
        "bounded run capped at max_batches × batch_size = 4 (overflow deferred)"
    );
    assert_eq!(
        feb1.zeros_after, 3,
        "3 rows deferred to the next run, not drained"
    );
    assert!(
        feb1.snapshot_name.is_none(),
        "recurring sweep does not FREEZE"
    );

    // Run 2 — same bounded config: sees only the deferred overflow and converges.
    let s2 = CoarseRepairDriver::with_client(client.clone(), repair_cfg.clone())
        .run()
        .await
        .unwrap();
    let feb2 = &s2.months[0];
    assert_eq!(feb2.zeros_before, 3, "run 2 sees only the run-1 overflow");
    assert_eq!(feb2.rows_enriched, 3);
    assert_eq!(
        feb2.zeros_after, 0,
        "backlog converged to the floor after run 2"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0172 regression — a USDT-quoted candle must be priced at the MEASURED
/// USDT rate, not at $1.
///
/// The canonical Stellar USDT (`USDT_ISSUER`) depegged in June 2022 and trades
/// at ~$0.13. It used to sit in the peg tier alongside USDC, so every
/// USDT-quoted candle got `close_usd = close × $1` — a ~7.4x overstatement
/// across 44,657 candles and 495 base assets on prod. It now takes the pivot
/// tier instead, exactly like XLM: its USD value is read from its own USDC
/// market rather than assumed.
///
/// The fixture makes the two outcomes numerically unmistakable: FOO trades at
/// 10.0 against USDT, so the correct answer is 10 × 0.13 = **1.3** and the old
/// buggy answer is **10.0**.
///
/// ⚠️ This also guards the failure mode that would look like a fix: simply
/// deleting USDT from the peg set, with no pivot, leaves these candles at
/// `close_usd = 0`. That is NOT acceptable — zero is indistinguishable from
/// "genuinely zero" and "not yet enriched" in this schema, and ~130
/// `argMax(close_usd, …)` sites read it unguarded. Hence the explicit `> 0`
/// assertion below.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn usdt_quoted_candles_pivot_on_the_measured_rate_not_a_dollar_peg() {
    let db = "it_enrich_0172_usdt_pivot";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (1,'XLM','classic','',''), (2,'USDC','classic','{USDC_ISSUER}',''), \
             (3,'USDT','classic','{USDT_ISSUER}',''), (10,'FOO','classic','GFOO','')"
        ))
        .execute()
        .await
        .unwrap();

    // USDT/USDC at 0.13 is the pivot SOURCE — it is USDC-quoted, so the peg tier
    // prices it first (0.13 × $1). FOO/USDT at 10.0 is the SUBJECT: the pivot
    // must then value it at 10 × 0.13 = 1.3, not at the old 10 × $1 = 10.0.
    let deep = 1_600_000_000u32;
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({deep}, 3, 2,'sdex', 0.13,0.13,0.13,0.13, 1000,130,0,0,0.13, 1,1), \
             ({deep},10, 3,'sdex', 10,10,10,10,          5,  50, 0,0,10,    1,1)"
        ))
        .execute()
        .await
        .unwrap();

    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;

    // The pivot source itself: USDC-quoted, so the peg tier gives it 0.13.
    let usdt_usd = close_usd(&client, db, 3, 2, deep).await;
    assert!(
        approx(usdt_usd, 0.13),
        "USDT/USDC must price at its market value 0.13, got {usdt_usd}"
    );

    let foo_via_usdt = close_usd(&client, db, 10, 3, deep).await;
    assert!(
        foo_via_usdt > 0.0,
        "USDT-quoted candle must not be left unpriced at 0 — that trades a wrong \
         number for a silent one (see the ~130 unguarded argMax(close_usd) sites)"
    );
    assert!(
        approx(foo_via_usdt, 1.3),
        "USDT-quoted candle must use the MEASURED rate: 10 x 0.13 = 1.3, got \
         {foo_via_usdt}. A value of 10.0 means USDT is being pegged to $1 again \
         and every USDT-quoted candle is ~7.4x overstated."
    );

    // Idempotent, like the other tiers: a second pass must not re-multiply.
    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();
    let after = close_usd(&client, db, 10, 3, deep).await;
    assert!(
        approx(after, 1.3),
        "second pass must leave the pivot value unchanged, got {after}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Fixture for the task 0182 reset tests: a USDT/USDC pivot reference at two
/// instants, and a FOO/USDT candle at each **already carrying the wrong `$1`
/// peg values** — `close_usd = 10.0` where the measured rate says 1.3.
///
/// That "already written" part is the whole point. Every tier filters on
/// `close_usd = 0`, so these rows are inert: the 0172 writer fix does not reach
/// them and never will.
async fn setup_0182(db: &str, t_old: u32, t_new: u32) -> Client {
    let client = setup_scratch(db).await;
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (1,'XLM','classic','',''), (2,'USDC','classic','{USDC_ISSUER}',''), \
             (3,'USDT','classic','{USDT_ISSUER}',''), (10,'FOO','classic','GFOO','')"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({t_old}, 3, 2,'sdex', 0.13,0.13,0.13,0.13, 1000,130, 0, 0, 0.13, 1,1), \
             ({t_new}, 3, 2,'sdex', 0.13,0.13,0.13,0.13, 1000,130, 0, 0, 0.13, 1,1), \
             ({t_old},10, 3,'sdex', 10,10,10,10,            5, 50,50,10, 10,   1,1), \
             ({t_new},10, 3,'sdex', 10,10,10,10,            5, 50,50,10, 10,   1,1)"
        ))
        .execute()
        .await
        .unwrap();
    client
}

/// Control for the reset tests below, and a standing regression for the trap
/// that hid task 0182 for a month: an ordinary pass over rows whose `close_usd`
/// is **wrong but non-zero** does nothing at all, and reports success doing it.
///
/// If this test ever starts failing because the values moved, the tiers have
/// stopped being idempotent — which is a much bigger problem than 0182.
#[tokio::test]
#[ignore]
async fn an_ordinary_pass_cannot_see_a_wrong_but_written_close_usd() {
    let db = "it_enrich_0182_control";
    let (t_old, t_new) = (1_500_000_000u32, 1_600_000_000u32);
    let client = setup_0182(db, t_old, t_new).await;

    let stats = ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    assert_eq!(
        stats.rows_reset, 0,
        "no reset was configured, so the pass must not discard anything"
    );
    for ts in [t_old, t_new] {
        let v = close_usd(&client, db, 10, 3, ts).await;
        assert!(
            (v - 10.0).abs() < 1e-4,
            "an ordinary pass must leave a written close_usd alone (that is what \
             makes it idempotent); at {ts} got {v}, expected the untouched 10.0"
        );
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The task 0182 repair, end to end: re-open the written values, let the pivot
/// recompute them from the measured USDT/USDC market, and leave the pre-epoch
/// window untouched.
///
/// Both halves are asserted deliberately. A test that checked only "the new row
/// got fixed" would also pass for a reset with no epoch bound at all — which
/// would zero the pre-2021 rows that have no pivot reference and strand them at
/// `close_usd = 0` permanently.
#[tokio::test]
#[ignore]
async fn the_usd_reset_recomputes_written_values_but_respects_the_epoch() {
    let db = "it_enrich_0182_reset";
    let (t_old, t_new) = (1_500_000_000u32, 1_600_000_000u32);
    let client = setup_0182(db, t_old, t_new).await;

    let mut c = cfg(db);
    // A reset requires a draining pass — see `the_usd_reset_refuses_a_bounded_pass`.
    // The operator CLI hard-codes this; the IT helper defaults to bounded.
    c.one_shot = true;
    c.usd_reset = Some(UsdResetSpec {
        quote_asset_id: 3,
        not_before: t_new,
        // The 0182 shape, stated at every site rather than defaulted: unbounded
        // above and no reference join. Task 0268 added both fields precisely so
        // each caller has to say which repair it is running.
        not_after: None,
        require_external_rate: false,
    });
    let stats = ChEnrichmentPass::new(c).run().await.unwrap();

    assert_eq!(
        stats.rows_reset, 1,
        "exactly the one post-epoch FOO/USDT row should have been re-opened"
    );

    let fixed = close_usd(&client, db, 10, 3, t_new).await;
    assert!(
        (fixed - 1.3).abs() < 1e-4,
        "the re-opened candle must be recomputed at the MEASURED rate \
         (10 x 0.13 = 1.3), got {fixed}. 10.0 means the reset never happened; \
         0.0 means it was re-opened and then not refilled, which is worse than \
         the defect."
    );

    let protected = close_usd(&client, db, 10, 3, t_old).await;
    assert!(
        (protected - 10.0).abs() < 1e-4,
        "the pre-epoch candle must keep its stored value, got {protected}. \
         Below the epoch the pivot has no reference, so zeroing it here would \
         strand it at 0 forever."
    );

    // Both USD columns come from one reference, or the row is incoherent.
    let vq: f64 = client
        .query(&format!(
            "SELECT toFloat64(volume_quote_usd) FROM {db}.price_ohlcv_1m FINAL \
             WHERE asset_id = 10 AND quote_asset_id = 3 AND timestamp = ?"
        ))
        .bind(t_new)
        .fetch_one::<f64>()
        .await
        .unwrap();
    assert!(
        (vq - 6.5).abs() < 1e-4,
        "volume_quote_usd must be recomputed from the same 0.13 rate \
         (50 x 0.13 = 6.5), got {vq}. 50.0 means it kept the old peg while \
         close_usd moved, leaving two USD columns that disagree by ~7.4x."
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// A mistyped `--reset-quote-asset-id` must not be able to zero rows nothing can
/// re-price.
///
/// This is the one way the repair ends up strictly worse than the defect: a
/// wrong-but-visible number becomes the ambiguous zero that ~130 unguarded
/// `argMax(close_usd, …)` sites read as a real price. And the oracle gate does
/// not catch it — an unknown asset has no Reflector rows either, which is
/// exactly what that gate is looking for.
#[tokio::test]
#[ignore]
async fn the_usd_reset_refuses_a_quote_leg_that_no_tier_can_reprice() {
    let db = "it_enrich_0182_unpriceable";
    let (t_old, t_new) = (1_500_000_000u32, 1_600_000_000u32);
    let client = setup_0182(db, t_old, t_new).await;

    let mut c = cfg(db);
    c.one_shot = true;
    // 10 is FOO — a real asset in the fixture, but not a peg or pivot reference.
    // Stands in for the realistic slip of typing 11 for 111.
    c.usd_reset = Some(UsdResetSpec {
        quote_asset_id: 10,
        not_before: t_new,
        not_after: None,
        require_external_rate: false,
    });
    let err = ChEnrichmentPass::new(c).run().await.unwrap_err();

    assert!(
        matches!(err, ChEnrichError::ResetTargetHasNoPricingPath { quote_asset_id, .. }
                 if quote_asset_id == 10),
        "expected the reset to refuse an unpriceable quote leg, got {err:?}"
    );

    let v = close_usd(&client, db, 10, 3, t_new).await;
    assert!(
        (v - 10.0).abs() < 1e-4,
        "a refused reset must not have written anything, got {v}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// A bounded pass can defer the peg-pivot tier (it is gated on the oracle tier
/// draining), which would leave the reset's zeroes published until some later
/// run. The combination is refused rather than risked.
#[tokio::test]
#[ignore]
async fn the_usd_reset_refuses_a_bounded_pass() {
    let db = "it_enrich_0182_bounded";
    let (t_old, t_new) = (1_500_000_000u32, 1_600_000_000u32);
    let client = setup_0182(db, t_old, t_new).await;

    let mut c = cfg(db);
    c.one_shot = false;
    c.usd_reset = Some(UsdResetSpec {
        quote_asset_id: 3,
        not_before: t_new,
        // The 0182 shape, stated at every site rather than defaulted: unbounded
        // above and no reference join. Task 0268 added both fields precisely so
        // each caller has to say which repair it is running.
        not_after: None,
        require_external_rate: false,
    });
    let err = ChEnrichmentPass::new(c).run().await.unwrap_err();

    assert!(
        matches!(err, ChEnrichError::ResetRequiresOneShot { quote_asset_id } if quote_asset_id == 3),
        "expected a bounded pass to refuse the reset, got {err:?}"
    );

    let v = close_usd(&client, db, 10, 3, t_new).await;
    assert!(
        (v - 10.0).abs() < 1e-4,
        "a refused reset must not have written anything, got {v}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0182's first ordering constraint, enforced rather than documented.
///
/// The oracle tier runs before the peg-pivot tier and wins where it applies, so
/// a reset performed while `oracle_prices` still holds rows for the quote leg
/// would be undone by the very next statement in the same pass — and the run
/// would report a healthy repair over unchanged values, now labelled
/// `method = 'oracle'`. The pass must refuse instead.
///
/// ⚠️ **This test now pins the UNBOUNDED half of the guard.** Task 0268 scoped
/// the count to `[not_before, not_after)`; with `not_after = None` — the spec
/// this test builds — it still counts to the end of time, so 0182's all-time
/// refusal is unchanged. The bounded half is
/// `the_bounded_usd_reset_is_not_refused_by_oracle_rows_above_its_window`.
/// 🔑 **The forward-fill band below `not_before`.** The oracle tier ASOFs
/// `o.timestamp <= p.timestamp` and accepts a reading up to `window_s` old, so a
/// row stamped just BELOW the reset's floor still re-prices candles just ABOVE
/// it — inside the very range the reset is about to zero. A guard that starts
/// counting at `not_before` cannot see that row: it reports a clean premise, the
/// reset zeroes the band, and the oracle tier refills it from the same reading
/// the reset existed to remove, relabelled `method = 'oracle'`.
///
/// Task 0268 scoped this guard's UPPER bound to the reset window and gave it a
/// lower bound at the same time; only the upper one needed to move. The floor is
/// therefore widened by one `window_s`, and this is the test that it is.
#[tokio::test]
#[ignore]
async fn the_usd_reset_is_refused_by_an_oracle_row_that_forward_fills_into_it() {
    let db = "it_enrich_0182_shadow_band";
    let (t_old, t_new) = (1_500_000_000u32, 1_600_000_000u32);
    let client = setup_0182(db, t_old, t_new).await;

    // 100 s below the floor — outside [not_before, ..), inside the 300 s window
    // the oracle tier will forward-fill across.
    let shadow = t_new - 100;
    client
        .query(&format!(
            "INSERT INTO {db}.oracle_prices \
             (asset_id, oracle_name, timestamp, price_usd) VALUES \
             (3, 'reflector', {shadow}, 1.0)"
        ))
        .execute()
        .await
        .unwrap();

    let mut c = cfg(db);
    c.one_shot = true;
    c.usd_reset = Some(UsdResetSpec {
        quote_asset_id: 3,
        not_before: t_new,
        not_after: None,
        require_external_rate: false,
    });
    let err = ChEnrichmentPass::new(c).run().await.unwrap_err();

    assert!(
        matches!(err, ChEnrichError::ResetBlockedByOracleRows { quote_asset_id, rows, .. }
                 if quote_asset_id == 3 && rows == 1),
        "a reading {}s below the floor still forward-fills into the reset window \
         and must refuse it, got {err:?}",
        t_new - shadow
    );

    let v = close_usd(&client, db, 10, 3, t_new).await;
    assert!(
        (v - 10.0).abs() < 1e-4,
        "a refused reset must not have written anything, got {v}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The external path is canonical-USDC-only at every site, but the priceability
/// gate accepts USDT (the peg tier can price it), so
/// `--reset-quote-asset-id <USDT> --reset-require-external-rate` used to be a
/// legal combination: it zeroed USDT-quoted rows on USDC's rate days, the
/// external tier — which only ever runs for USDC — refilled none of them, and
/// the peg tier wrote $1 back over the lot. Task 0182's incident, different
/// quote asset. Refused in the library, so no driver can assemble it.
#[tokio::test]
#[ignore]
async fn an_external_reset_refuses_a_quote_leg_that_is_not_canonical_usdc() {
    let db = "it_enrich_0268_usdt_external";
    let (t_old, t_new) = (1_500_000_000u32, 1_600_000_000u32);
    // The 0182 fixture is the one that tracks USDT (asset_id 3) as a stable
    // reference, so `assert_reset_target_is_priceable` passes it and this test
    // reaches the guard it is about.
    let client = setup_0182(db, t_old, t_new).await;

    let mut c = cfg(db);
    c.one_shot = true;
    c.usd_reset = Some(UsdResetSpec {
        // asset_id 3 is USDT in this fixture: a stable reference, so the
        // priceability gate passes it, and the external tier cannot touch it.
        quote_asset_id: 3,
        not_before: 0,
        not_after: Some(prices_clickhouse::USDC_ORACLE_EPOCH_S),
        require_external_rate: true,
    });
    let err = ChEnrichmentPass::new(c).run().await.unwrap_err();
    assert!(
        matches!(err, ChEnrichError::ResetExternalRateLegIsNotUsdc { quote_asset_id, .. }
                 if quote_asset_id == 3),
        "an external reset on a non-USDC leg must be refused, got {err:?}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore]
async fn the_usd_reset_refuses_to_run_while_the_oracle_still_shadows_the_quote_leg() {
    let db = "it_enrich_0182_oracle_gate";
    let (t_old, t_new) = (1_500_000_000u32, 1_600_000_000u32);
    let client = setup_0182(db, t_old, t_new).await;

    // A Reflector row for USDT at par — exactly the mis-attribution task 0196
    // purged from prod.
    client
        .query(&format!(
            "INSERT INTO {db}.oracle_prices \
             (asset_id, oracle_name, timestamp, price_usd) VALUES \
             (3, 'reflector', {t_new}, 1.0)"
        ))
        .execute()
        .await
        .unwrap();

    let mut c = cfg(db);
    c.one_shot = true;
    c.usd_reset = Some(UsdResetSpec {
        quote_asset_id: 3,
        not_before: t_new,
        // The 0182 shape, stated at every site rather than defaulted: unbounded
        // above and no reference join. Task 0268 added both fields precisely so
        // each caller has to say which repair it is running.
        not_after: None,
        require_external_rate: false,
    });
    let err = ChEnrichmentPass::new(c).run().await.unwrap_err();

    assert!(
        matches!(err, ChEnrichError::ResetBlockedByOracleRows { quote_asset_id, rows, .. }
                 if quote_asset_id == 3 && rows == 1),
        "expected the reset to be refused while oracle rows shadow the quote leg, got {err:?}"
    );

    // And it refused *before* writing: the stored value is untouched, so the
    // operator can purge and re-run without a half-applied repair in the way.
    let v = close_usd(&client, db, 10, 3, t_new).await;
    assert!(
        (v - 10.0).abs() < 1e-4,
        "a refused reset must not have written anything, got {v}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Task 0111 phase 2 — frontier-driven historical sweep
// ---------------------------------------------------------------------------

/// Read a month's stored frontier state, or `None` if the sweep never recorded
/// it. `FINAL` collapses the ReplacingMergeTree so a re-recorded month reads as
/// its latest state, which is exactly how the sweep reads it.
async fn frontier_state(client: &Client, db: &str, month: u32) -> Option<String> {
    client
        .query(&format!(
            "SELECT CAST(state AS String) FROM {db}.enrichment_frontier FINAL \
             WHERE tbl = 'price_ohlcv_1m' AND month = ?"
        ))
        .bind(month)
        .fetch_optional::<String>()
        .await
        .unwrap()
}

/// End-to-end frontier walk against a live ClickHouse. This is the half the
/// pure `months_to_sweep` unit tests cannot reach: the actual SQL — the
/// `Enum8`-by-name insert, `CAST(state AS String)` on read, the server-side
/// `toUnixTimestamp64Milli` version, and `toYYYYMM(min|max(timestamp))` as the
/// partition-span source.
///
/// Fixture spans three monthly partitions below the live window:
///   * 202101 — FOO/XLM with **no** XLM/USDC reference anywhere before it, so
///     nothing can price it. The permanently-unpriceable floor in miniature.
///   * 202102 — FOO/USDC, peggable.
///   * 202103 — FOO/USDC, peggable.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn the_frontier_advances_exhausts_and_never_revisits() {
    use enrichment_worker::frontier::{HistoricalSweepConfig, run_historical_sweep};

    let db = "it_enrich_frontier";
    let client = setup_scratch(db).await;
    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    // 2021-01-15, 2021-02-15, 2021-03-15 — one candle per monthly partition.
    let (jan, feb, mar) = (1_610_712_000u32, 1_613_390_400u32, 1_615_809_600u32);
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({jan},10, 1,'phoenix', 7,7,7,7, 1,7,0,0,7,1,1), \
             ({feb},10, 2,'sdex',    4,4,4,4, 1,4,0,0,4,1,1), \
             ({mar},10, 2,'sdex',    9,9,9,9, 1,9,0,0,9,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    let sweep = |max_months: u32| {
        let mut base = cfg(db);
        base.url = ch_url();
        HistoricalSweepConfig {
            base,
            // Everything from 2026-07 on belongs to the live pass; the whole
            // fixture is below it.
            live_start_month: 202607,
            max_months,
            deadline: None,
            // Drift correction off for the walk assertions below: a re-check
            // firing mid-walk would re-stamp months and blur what is being
            // asserted. Its own behaviour is covered by
            // `a_backfill_into_an_exhausted_month_reopens_it`.
            recheck_after_secs: u32::MAX,
            max_rechecks: 0,
        }
    };
    let client_for_sweep = Client::default().with_url(ch_url()).with_database(db);

    // --- run 1: the oldest month, which nothing can price -------------------
    let r1 = run_historical_sweep(&client_for_sweep, &sweep(1))
        .await
        .unwrap();
    assert_eq!(
        r1.months.len(),
        1,
        "max_months = 1 works exactly one partition"
    );
    assert_eq!(r1.months[0].month, 202101, "oldest-first");
    assert_eq!(
        r1.frontier_month,
        Some(202101),
        "frontier position is the oldest pending month"
    );
    assert_eq!(
        r1.total_enriched(),
        0,
        "202101 has no USD reference of any kind"
    );
    assert_eq!(
        frontier_state(&client, db, 202101).await.as_deref(),
        Some("exhausted"),
        "a month that makes no progress is terminal — this is the pre-reference \
         floor dropping out with no hard-coded cutoff date"
    );

    // --- run 2: must SKIP the exhausted month and advance -------------------
    let r2 = run_historical_sweep(&client_for_sweep, &sweep(1))
        .await
        .unwrap();
    assert_eq!(
        r2.months[0].month, 202102,
        "the exhausted month is never revisited"
    );
    assert!(r2.total_enriched() > 0, "202102 is peggable");
    assert_eq!(
        frontier_state(&client, db, 202102).await.as_deref(),
        Some("exhausted"),
        "a month drained to zero candidates in one pass is terminal immediately — \
         marking it pending would cost an extra visit per month across ~102 \
         partitions purely to learn there is nothing left"
    );
    assert!(
        close_usd(&client, db, 10, 2, feb).await > 0.0,
        "the sweep actually wrote a USD value, not just a frontier row"
    );

    // --- run 3: the last month below the live window ------------------------
    let r3 = run_historical_sweep(&client_for_sweep, &sweep(1))
        .await
        .unwrap();
    assert_eq!(r3.months[0].month, 202103);
    assert!(close_usd(&client, db, 10, 2, mar).await > 0.0);

    // --- run 4: nothing left ------------------------------------------------
    let r4 = run_historical_sweep(&client_for_sweep, &sweep(5))
        .await
        .unwrap();
    assert!(
        r4.months.is_empty(),
        "a fully exhausted history yields no work"
    );
    assert_eq!(
        r4.months_pending, 0,
        "the drain-progress metric reaches zero"
    );
    assert_eq!(r4.frontier_month, None, "no frontier position remains");
}

/// The sweep must never touch a partition the live pass owns — otherwise the
/// two contend for the same rows every hour, which is the coupling task 0111
/// exists to remove.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn the_sweep_never_enters_the_live_window() {
    use enrichment_worker::frontier::{HistoricalSweepConfig, run_historical_sweep};

    let db = "it_enrich_frontier_live";
    let client = setup_scratch(db).await;
    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    // Both candles are inside the declared live window (202102 onward).
    let (feb, mar) = (1_613_390_400u32, 1_615_809_600u32);
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({feb},10,2,'sdex', 4,4,4,4, 1,4,0,0,4,1,1), \
             ({mar},10,2,'sdex', 9,9,9,9, 1,9,0,0,9,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    let client_for_sweep = Client::default().with_url(ch_url()).with_database(db);
    let summary = run_historical_sweep(
        &client_for_sweep,
        &HistoricalSweepConfig {
            base: cfg(db),
            live_start_month: 202102,
            max_months: 12,
            deadline: None,
            recheck_after_secs: u32::MAX,
            max_rechecks: 0,
        },
    )
    .await
    .unwrap();

    assert!(
        summary.months.is_empty(),
        "every partition belongs to the live pass"
    );
    assert_eq!(
        close_usd(&client, db, 10, 2, feb).await,
        0.0,
        "the sweep must leave live-window rows for the live pass"
    );
}

/// 🔴 `exhausted` must be a hint with an expiry, not a verdict.
///
/// A month is marked exhausted because nothing there could be priced *at that
/// moment*. A backfill writing into a historical partition falsifies that later
/// — which is exactly what tasks 0088 and 0201 do. Without the re-check those
/// rows would sit unenriched forever while the frontier read clean, which is
/// the "skipped rows that look healthy" failure class that cost 26 days in 0215.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_backfill_into_an_exhausted_month_reopens_it() {
    use enrichment_worker::frontier::{HistoricalSweepConfig, run_historical_sweep};

    let db = "it_enrich_frontier_drift";
    let client = setup_scratch(db).await;
    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();

    let feb = 1_613_390_400u32; // 2021-02-15
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({feb},10,2,'sdex', 4,4,4,4, 1,4,0,0,4,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    let client_for_sweep = Client::default().with_url(ch_url()).with_database(db);
    let sweep_cfg = |recheck_after_secs: u32, max_rechecks: u32| HistoricalSweepConfig {
        base: cfg(db),
        live_start_month: 202607,
        max_months: 1,
        deadline: None,
        recheck_after_secs,
        max_rechecks,
    };

    // Drain 202102 to zero candidates → exhausted.
    run_historical_sweep(&client_for_sweep, &sweep_cfg(u32::MAX, 0))
        .await
        .unwrap();
    assert_eq!(
        frontier_state(&client, db, 202102).await.as_deref(),
        Some("exhausted")
    );

    // A backfill lands a NEW unenriched candle in that finished partition.
    let feb2 = feb + 60;
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({feb2},10,2,'sdex', 6,6,6,6, 1,6,0,0,6,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // With the mark still fresh, the walk skips the month entirely — the
    // partition is invisible and the new row would never be enriched. This
    // asserts the hazard exists, so the fix below is testing something real.
    let blind = run_historical_sweep(&client_for_sweep, &sweep_cfg(u32::MAX, 4))
        .await
        .unwrap();
    assert_eq!(blind.months_rechecked, 0, "nothing is stale yet");
    assert_eq!(
        close_usd(&client, db, 10, 2, feb2).await,
        0.0,
        "the backfilled row is invisible while the exhausted mark is trusted"
    );

    // recheck_after_secs = 0 makes every exhausted mark stale immediately.
    let corrected = run_historical_sweep(&client_for_sweep, &sweep_cfg(0, 4))
        .await
        .unwrap();
    assert_eq!(
        corrected.months_rechecked, 1,
        "the stale month was re-counted"
    );
    assert_eq!(corrected.months_reopened, 1, "and it had gained work");
    assert_eq!(
        frontier_state(&client, db, 202102).await.as_deref(),
        Some("pending"),
        "re-opened, so the next walk will work it"
    );

    // The next ordinary run picks it up and prices the backfilled row.
    run_historical_sweep(&client_for_sweep, &sweep_cfg(u32::MAX, 0))
        .await
        .unwrap();
    assert!(
        close_usd(&client, db, 10, 2, feb2).await > 0.0,
        "drift correction closed the loop — the backfilled row is enriched"
    );
}

// ---- task 0268: the external tier ---------------------------------------

/// Seed one `method = 'external'` row for canonical USDC into a scratch
/// `usd_rate`, with the writer's exact column list.
///
/// ⚠️ **`ts` is stamped at the START of the UTC day, and that pins task 0267's
/// convention.** The tier resolves at the bucket's END with an ASOF
/// `rts < bend`, so a day-start stamp is the same day's rate for every bucket in
/// that day. If 0267 instead stamps its daily rows at day END, every bucket
/// resolves to the PREVIOUS day's rate — an off-by-one-day error that produces
/// entirely plausible numbers and fails nowhere. This helper is the thing that
/// will catch it: 0268's runbook Appendix B, precondition 2, is the query that
/// confirms the convention on prod before the pass runs.
async fn seed_external_rate(client: &Client, db: &str, ts: u32, rate: f64) {
    client
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, \
              timestamp, usd_rate, method, reference_asset, hops, version) VALUES \
             ('credit', 'USDC', '{USDC_ISSUER}', '', {ts}, {rate}, 'external', '', 0, 1)"
        ))
        .execute()
        .await
        .unwrap();
}

async fn seed_assets_and_candles(client: &Client, db: &str) {
    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();
    client
        .query(&CANDLES.replace("{db}", db))
        .execute()
        .await
        .unwrap();
}

// The deep FOO/USDC fixture candle (`close = 4`, `volume_quote = 4`) and the
// start of its UTC day, 2020-09-13.
const DEEP_TS: u32 = 1_600_000_000;
const DEEP_DAY_START: u32 = 1_599_955_200;
// The recent oracle-covered FOO/USDC candle (`close = 5`) and its day start.
const RECENT_TS: u32 = 1_700_000_000;
const RECENT_DAY_START: u32 = 1_699_920_000;
// USDC's real close on 2023-03-11 — the number this whole task exists for.
const DEPEG_RATE: f64 = 0.9681;

/// (a) A USDC-quoted candle the oracle tier left at zero is priced from the
/// MEASURED rate, not from `× $1`, and `volume_quote_usd` is scaled by the same
/// rate. `4 × 0.9681 = 3.8724` — the 3.19% the peg tier used to discard.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn external_tier_prices_a_usdc_leg_from_the_measured_rate() {
    let db = "it_enrich_external_measured";
    let client = setup_scratch(db).await;
    seed_assets_and_candles(&client, db).await;
    seed_external_rate(&client, db, DEEP_DAY_START, DEPEG_RATE).await;

    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    let got = close_usd(&client, db, 10, 2, DEEP_TS).await;
    assert!(
        (got - 3.8724).abs() < 1e-4,
        "close_usd must be close × the measured rate (4 × 0.9681 = 3.8724), got {got} \
         — 4.0 means the peg tier priced it and the external tier never ran"
    );

    let vqu: f64 = client
        .query(&format!(
            "SELECT toFloat64(volume_quote_usd) FROM {db}.price_ohlcv_1m FINAL \
             WHERE asset_id = 10 AND quote_asset_id = 2 AND timestamp = {DEEP_TS}"
        ))
        .fetch_one::<f64>()
        .await
        .unwrap();
    assert!(
        (vqu - 3.8724).abs() < 1e-4,
        "volume_quote_usd scales by the same rate, got {vqu}"
    );

    // (d) Idempotent: the tier's candidate filter is `close_usd = 0`, so a second
    // pass must not find the row again — and must not re-scale an already-scaled
    // value into 4 × 0.9681².
    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();
    let again = close_usd(&client, db, 10, 2, DEEP_TS).await;
    assert!(
        (again - 3.8724).abs() < 1e-4,
        "a second pass must leave the value alone, got {again}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// (b) 🔑 THE ORACLE WINS. A polled Reflector reading and an imported rate both
/// cover the same bucket at DIFFERENT rates: the oracle value is what lands,
/// because the oracle tier runs first and the external tier's candidate filter
/// is `close_usd = 0`. A second pass does not move it.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn external_tier_never_overwrites_a_candle_the_oracle_tier_priced() {
    let db = "it_enrich_external_oracle_wins";
    let client = setup_scratch(db).await;
    seed_assets_and_candles(&client, db).await;
    // The oracle says 1.0012; the imported series says 0.90 for the same day.
    client
        .query(&format!(
            "INSERT INTO {db}.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES ({RECENT_TS}, 2, 'reflector', 1.0012, '{{}}')"
        ))
        .execute()
        .await
        .unwrap();
    seed_external_rate(&client, db, RECENT_DAY_START, 0.90).await;

    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    let got = close_usd(&client, db, 10, 2, RECENT_TS).await;
    assert!(
        (got - 5.006).abs() < 1e-4,
        "the oracle value (5 × 1.0012 = 5.006) must win over the import \
         (5 × 0.90 = 4.5), got {got}"
    );

    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();
    let again = close_usd(&client, db, 10, 2, RECENT_TS).await;
    assert!((again - 5.006).abs() < 1e-4, "idempotent, got {again}");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// (c) 🔑 NO REFERENCE, NO CHANGE — and this is the case task 0268's reset must
/// never zero.
///
/// The only `external` row is 200 days before the candle's bucket, far outside
/// the derived staleness bound, so the external tier cannot price it. The candle
/// falls through to the peg tier and keeps `close × $1 = 4.0` exactly. It is not
/// left at 0, and the pass still makes progress.
///
/// This is the 0182 lesson in the write path: task 0182's reset epoch sat 19
/// hours before the reference market's first candle, so 157 candles were zeroed
/// and then could not be recomputed. A row with no usable reference must come
/// out of the pass exactly as it went in.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn external_tier_leaves_a_bucket_with_no_usable_rate_on_the_peg_value() {
    let db = "it_enrich_external_no_reference";
    let client = setup_scratch(db).await;
    seed_assets_and_candles(&client, db).await;
    seed_external_rate(&client, db, DEEP_DAY_START - 200 * 86_400, DEPEG_RATE).await;

    let stats = ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    let got = close_usd(&client, db, 10, 2, DEEP_TS).await;
    assert!(
        (got - 4.0).abs() < 1e-4,
        "with no in-window external rate the peg tier's close × $1 = 4.0 must \
         stand, got {got} — 0.0 would be the 0182 failure and 3.8724 would mean \
         the staleness bound never bit"
    );
    assert!(
        stats.rows_enriched > 0,
        "the pass still prices the other tiers' candles"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Read `volume_quote_usd` from the 1m base table, the companion of [`close_usd`].
async fn volume_quote_usd(client: &Client, db: &str, asset: u32, quote: u32, ts: u32) -> f64 {
    client
        .query(&format!(
            "SELECT toFloat64(volume_quote_usd) FROM {db}.price_ohlcv_1m FINAL \
             WHERE asset_id = ? AND quote_asset_id = ? AND timestamp = ?"
        ))
        .bind(asset)
        .bind(quote)
        .bind(ts)
        .fetch_one::<f64>()
        .await
        .unwrap()
}

/// Insert one FOO/USDC candle into the 1m base table with the USD columns as
/// given, so a test can stage a row in a specific pre-enrichment state.
async fn seed_foo_usdc_candle(client: &Client, db: &str, ts: u32, close: f64, vqu: f64) {
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES ({ts}, 10, 2, 'sdex', {close}, {close}, {close}, {close}, \
                     1, {close}, {vqu}, 0, {close}, 1, 1)"
        ))
        .execute()
        .await
        .unwrap();
}

/// (d) 🔑 REVIEW CR-01. A `_1m` candle at **23:30 UTC** resolves to ITS OWN
/// day's rate, not the next day's.
///
/// The first `bucket_end_expr` floored every sub-daily grain to `+ 3600`, so a
/// 23:30 one-minute bucket got `bend = 00:30` next day and the ASOF `rts < bend`
/// picked the NEXT day's row from a series stamped at day start. On the depeg
/// day that is 0.99 instead of 0.9681 — a plausible number, ~2% wrong in the
/// wrong direction, on the scheduled Lambda's own table. Two rates a day apart,
/// distinguishable by value, and the candle in the last hour of the first day.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_one_minute_bucket_in_the_last_hour_of_a_day_gets_that_days_rate() {
    let db = "it_enrich_external_day_boundary";
    let client = setup_scratch(db).await;
    seed_assets_and_candles(&client, db).await;
    // 23:30 UTC on the deep fixture's day (2020-09-13).
    let late = DEEP_DAY_START + 23 * 3_600 + 30 * 60;
    seed_foo_usdc_candle(&client, db, late, 4.0, 0.0).await;
    seed_external_rate(&client, db, DEEP_DAY_START, DEPEG_RATE).await;
    seed_external_rate(&client, db, DEEP_DAY_START + 86_400, 0.99).await;

    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    let got = close_usd(&client, db, 10, 2, late).await;
    assert!(
        (got - 4.0 * DEPEG_RATE).abs() < 1e-4,
        "a 23:30 bucket belongs to its own day: expected 4 × 0.9681 = 3.8724, got {got} \
         — 3.96 means the bucket end overshot midnight and the next day's rate was used"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Read `close_usd` from any candle table — the coarse-grain twin of
/// [`close_usd`], which is pinned to the 1m base table.
async fn close_usd_in(client: &Client, db: &str, table: &str, ts: u32) -> f64 {
    client
        .query(&format!(
            "SELECT toFloat64(close_usd) FROM {db}.{table} FINAL \
             WHERE asset_id = 10 AND quote_asset_id = 2 AND timestamp = ?"
        ))
        .bind(ts)
        .fetch_one::<f64>()
        .await
        .unwrap()
}

/// Insert one unpriced FOO/USDC candle (`close = 4`, `volume_quote = 4`) into
/// any candle table, so the calendar grains can be exercised on their own
/// table rather than on the 1m fixture.
async fn seed_foo_usdc_candle_in(client: &Client, db: &str, table: &str, ts: u32) {
    client
        .query(&format!(
            "INSERT INTO {db}.{table} \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES ({ts}, 10, 2, 'sdex', 4, 4, 4, 4, 1, 4, 0, 0, 4, 1, 1)"
        ))
        .execute()
        .await
        .unwrap();
}

/// (d2) 🔑 REVIEW WR-10. The `'UTC'` CALENDAR branch of `bucket_end_expr`,
/// executed against a server, on the table the campaign actually targets.
///
/// Every other external-tier fixture runs on `_1m` or `_1h` and takes the
/// `timestamp + N` branch; `addDays(…, 1, 'UTC')` was asserted as a string
/// only. A `_1d` candle at day start with rates at day start AND one day later:
/// the bucket end is `addDays(ts, 1, 'UTC')` = the next midnight, the ASOF is
/// strict (`rts < bend`), so the SAME day's rate must win and the +1d rate must
/// not. `3.96` here means the calendar end overshot or the strictness was lost.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_daily_bucket_resolves_the_calendar_bucket_end_against_the_server() {
    let db = "it_enrich_external_calendar_1d";
    let client = setup_scratch(db).await;
    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();
    seed_foo_usdc_candle_in(&client, db, "price_ohlcv_1d", DEEP_DAY_START).await;
    seed_external_rate(&client, db, DEEP_DAY_START, DEPEG_RATE).await;
    seed_external_rate(&client, db, DEEP_DAY_START + 86_400, 0.99).await;

    let mut c = cfg(db);
    c.table = "price_ohlcv_1d".to_string();
    ChEnrichmentPass::new(c).run().await.unwrap();

    let got = close_usd_in(&client, db, "price_ohlcv_1d", DEEP_DAY_START).await;
    assert!(
        (got - 4.0 * DEPEG_RATE).abs() < 1e-4,
        "a daily bucket takes ITS day's rate: expected 4 × 0.9681 = 3.8724, got {got} \
         — 3.96 means the +1d rate won (rts < bend lost, or the calendar end overshot); \
         4.0 means the statement never priced it (an execution error would have \
         surfaced as Err, so check the day-set / staleness bound)"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// (d3) 🔑 REVIEW WR-10, the `addMonths(…, 1, 'UTC')` form. A September 2020
/// monthly bucket: rates on 09-01, on 09-30 and on 10-01. The calendar end is
/// 10-01 00:00, so the 09-30 rate wins. A fixed 31-day end (`+ 2_678_400`, the
/// bug `bucket_end_expr`'s doc block describes) would land on 10-02 and select
/// the 10-01 rate instead — `3.96`, a plausible number, from the wrong month.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn a_monthly_bucket_resolves_the_calendar_month_end_against_the_server() {
    let db = "it_enrich_external_calendar_1m_month";
    let client = setup_scratch(db).await;
    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();
    // 2020-09-01 00:00 UTC; 2020-09-30 is 29 days later; 2020-10-01 is 30.
    let month_start: u32 = 1_598_918_400;
    seed_foo_usdc_candle_in(&client, db, "price_ohlcv_1M", month_start).await;
    seed_external_rate(&client, db, month_start, 0.95).await;
    seed_external_rate(&client, db, month_start + 29 * 86_400, DEPEG_RATE).await;
    seed_external_rate(&client, db, month_start + 30 * 86_400, 0.99).await;

    let mut c = cfg(db);
    c.table = "price_ohlcv_1M".to_string();
    ChEnrichmentPass::new(c).run().await.unwrap();

    let got = close_usd_in(&client, db, "price_ohlcv_1M", month_start).await;
    assert!(
        (got - 4.0 * DEPEG_RATE).abs() < 1e-4,
        "a monthly bucket is priced at the month's LAST day: expected 4 × 0.9681 = \
         3.8724, got {got} — 3.96 is the 10-01 rate (a 31-day end overshot September), \
         3.80 is the 09-01 rate (resolved at the bucket start)"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// (e) 🔑 REVIEW WR-04. Above `USDC_ORACLE_EPOCH_S` the external tier does NOT
/// run, even with a covering imported rate and no oracle reading.
///
/// The API labels every scaled USDC-quoted candle at or after the epoch
/// `oracle`. If the tier priced a post-epoch oracle miss from the import, the
/// wire would report a Reflector poll that never happened. The candle must fall
/// to the peg tier instead, where its `close_usd = close` signature reads as
/// `assumed-par` — the truth.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn external_tier_never_prices_a_candle_above_the_oracle_epoch() {
    let db = "it_enrich_external_epoch_bound";
    let client = setup_scratch(db).await;
    seed_assets_and_candles(&client, db).await;
    // 2026-03-12 01:00 UTC: the day after the epoch, with a rate for that day.
    let day_after = USDC_ORACLE_EPOCH_S + 36_000;
    let post = day_after + 3_600;
    seed_foo_usdc_candle(&client, db, post, 6.0, 0.0).await;
    seed_external_rate(&client, db, day_after, 0.90).await;

    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    let got = close_usd(&client, db, 10, 2, post).await;
    assert!(
        (got - 6.0).abs() < 1e-4,
        "above the epoch the tier must not write: expected the peg tier's 6 × $1 = 6.0, \
         got {got} — 5.4 means the import priced a candle the wire will label `oracle`"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// (f) 🔑 REVIEW WR-06. A row enriched before `close_usd` existed
/// (`volume_quote_usd = volume_quote × $1`, `close_usd = 0`) comes out with BOTH
/// USD columns from the one imported rate — never `close_usd` at 0.9681 beside a
/// `volume_quote_usd` still at par.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn external_tier_recomputes_a_half_priced_row_from_the_one_reference() {
    let db = "it_enrich_external_half_priced";
    let client = setup_scratch(db).await;
    client
        .query(&ASSETS.replace("{db}", db).replace("{usdc}", USDC_ISSUER))
        .execute()
        .await
        .unwrap();
    // volume_quote_usd already at ×$1 (4.0), close_usd still 0.
    seed_foo_usdc_candle(&client, db, DEEP_TS, 4.0, 4.0).await;
    seed_external_rate(&client, db, DEEP_DAY_START, DEPEG_RATE).await;

    ChEnrichmentPass::new(cfg(db)).run().await.unwrap();

    let c = close_usd(&client, db, 10, 2, DEEP_TS).await;
    let v = volume_quote_usd(&client, db, 10, 2, DEEP_TS).await;
    assert!(
        (c - 3.8724).abs() < 1e-4,
        "close_usd from the measured rate, got {c}"
    );
    assert!(
        (v - 3.8724).abs() < 1e-4,
        "volume_quote_usd must be recomputed from the SAME rate, got {v} — 4.0 is the \
         half-priced row: two USD figures for one candle at different rates"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

// ---- task 0268: the external-scoped reset --------------------------------

/// FOO/USDC candles carrying the peg tier's EXACT signature (`close_usd = close`,
/// `volume_quote_usd = volume_quote`) — the population task 0268 re-opens. One
/// falls on a day the imported series covers, one does not.
async fn setup_0268(db: &str, t_covered: u32, t_uncovered: u32) -> Client {
    let client = setup_scratch(db).await;
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address) VALUES \
             (1,'XLM','classic','',''), (2,'USDC','classic','{USDC_ISSUER}',''), \
             (10,'FOO','classic','GFOO','')"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             ({t_covered},   10, 2,'sdex', 4,4,4,4, 1, 4, 4, 4, 4, 1,1), \
             ({t_uncovered}, 10, 2,'sdex', 4,4,4,4, 1, 4, 4, 4, 4, 1,1)"
        ))
        .execute()
        .await
        .unwrap();
    client
}

fn external_reset() -> UsdResetSpec {
    UsdResetSpec {
        quote_asset_id: 2,
        not_before: 0,
        not_after: Some(prices_clickhouse::USDC_ORACLE_EPOCH_S),
        require_external_rate: true,
    }
}

/// 🔑 **THE 157-CANDLE REGRESSION.** Task 0182's reset epoch sat 19 hours before
/// its reference market's first candle. 157 candles were zeroed, nothing could
/// refill them, and they were left publishing `close_usd = 0` — which ~130
/// unguarded `argMax(close_usd, …)` sites read as a real price of zero. That is
/// strictly worse than the wrong-but-visible number the repair set out to fix.
///
/// An epoch cannot express "a reference exists HERE". A semi-join can, and this
/// is the test that it does: two candles with identical signatures, one on a day
/// the imported series covers and one not. The covered candle is re-priced at the
/// measured rate; the uncovered one comes out EXACTLY as it went in.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn the_external_reset_never_zeroes_a_bucket_it_cannot_refill() {
    let db = "it_enrich_0268_reset";
    let (covered, uncovered) = (1_600_000_000u32, 1_600_432_000u32);
    let client = setup_0268(db, covered, uncovered).await;
    // Day start of `covered` (2020-09-13) only. `uncovered` (2020-09-18) has no row.
    seed_external_rate(&client, db, 1_599_955_200, DEPEG_RATE).await;

    let mut c = cfg(db);
    c.one_shot = true;
    c.usd_reset = Some(external_reset());
    let stats = ChEnrichmentPass::new(c).run().await.unwrap();

    assert_eq!(
        stats.rows_reset, 1,
        "exactly the covered candle should have been re-opened — re-opening the \
         uncovered one is the 0182 incident"
    );

    let fixed = close_usd(&client, db, 10, 2, covered).await;
    assert!(
        (fixed - 3.8724).abs() < 1e-4,
        "the covered candle must be re-priced at the measured rate \
         (4 x 0.9681 = 3.8724), got {fixed}"
    );

    let untouched = close_usd(&client, db, 10, 2, uncovered).await;
    assert!(
        (untouched - 4.0).abs() < 1e-4,
        "the uncovered candle must be EXACTLY as it was, got {untouched}. \
         0.0 is the 0182 incident reproduced: zeroed with nothing able to refill it"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// D-03: with no `external` rows loaded the tool refuses, rather than running and
/// silently doing nothing. A clean, healthy, entirely empty repair is the same
/// green all-clear that hid task 0182 for a month.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn the_external_reset_refuses_when_no_external_rates_are_loaded() {
    let db = "it_enrich_0268_no_rates";
    let (covered, uncovered) = (1_600_000_000u32, 1_600_432_000u32);
    let client = setup_0268(db, covered, uncovered).await;
    // usd_rate deliberately left empty.

    let mut c = cfg(db);
    c.one_shot = true;
    c.usd_reset = Some(external_reset());
    let err = ChEnrichmentPass::new(c).run().await.unwrap_err();

    assert!(
        matches!(err, ChEnrichError::ResetRequiresExternalRates { quote_asset_id }
                 if quote_asset_id == 2),
        "expected a refusal when 0267's series is not loaded, got {err:?}"
    );

    // And it refused BEFORE writing anything.
    for ts in [covered, uncovered] {
        let v = close_usd(&client, db, 10, 2, ts).await;
        assert!(
            (v - 4.0).abs() < 1e-4,
            "a refused reset must not have written anything at {ts}, got {v}"
        );
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// D-06, both halves. The oracle-shadow guard counts inside
/// `[not_before, not_after)` only.
///
/// Canonical USDC has held live oracle rows since 2026-03-11. An all-time count
/// therefore refuses every reset of the 2020-2025 history those rows cannot
/// reach — a guard applied where its own premise is false. Bounded below them,
/// the reset RUNS. Move a single oracle row inside the window and it is refused
/// again, because there the premise holds: the oracle tier runs first, wins, and
/// would re-apply the very value the reset removed.
/// 🔑 REVIEW WR-09. A canonical USDC reading in `oracle_prices` BELOW the epoch
/// refuses the 0268 mode even when it sits below `--reset-not-before` — where
/// the oracle-shadow guard's window does not reach but the external tier's
/// candidate set does. Nothing is written.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn the_external_reset_refuses_a_pre_epoch_oracle_reading_below_its_own_window() {
    let db = "it_enrich_0268_pre_epoch_oracle";
    let client = setup_scratch(db).await;
    seed_assets_and_candles(&client, db).await;
    seed_external_rate(&client, db, DEEP_DAY_START, DEPEG_RATE).await;
    // A Reflector reading for USDC a year BEFORE the reset's lower bound and
    // far below the epoch: invisible to the windowed shadow guard.
    let reading = DEEP_DAY_START - 365 * 86_400;
    client
        .query(&format!(
            "INSERT INTO {db}.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES ({reading}, 2, 'reflector', 1.0, '{{}}')"
        ))
        .execute()
        .await
        .unwrap();

    let mut c = cfg(db);
    c.one_shot = true;
    c.usd_reset = Some(UsdResetSpec {
        quote_asset_id: 2,
        not_before: DEEP_DAY_START,
        not_after: Some(USDC_ORACLE_EPOCH_S),
        require_external_rate: true,
    });
    let err = ChEnrichmentPass::new(c).run().await.unwrap_err();
    assert!(
        matches!(
            err,
            ChEnrichError::ResetBlockedByPreEpochOracleRows {
                quote_asset_id: 2,
                rows: 1,
                ..
            }
        ),
        "expected the pre-epoch oracle refusal, got {err:?}"
    );
    // Refused before writing: the deep fixture candle is untouched.
    let v = close_usd(&client, db, 10, 2, DEEP_TS).await;
    assert!(
        (v - 0.0).abs() < 1e-9,
        "a refused reset must not have written anything, got {v}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn the_bounded_usd_reset_is_not_refused_by_oracle_rows_above_its_window() {
    let db = "it_enrich_0268_bounded_guard";
    let (covered, uncovered) = (1_600_000_000u32, 1_600_432_000u32);
    let client = setup_0268(db, covered, uncovered).await;
    seed_external_rate(&client, db, 1_599_955_200, DEPEG_RATE).await;

    // A live USDC oracle row ABOVE the reset's upper bound — prod's actual shape.
    let above = prices_clickhouse::USDC_ORACLE_EPOCH_S + 3_600;
    client
        .query(&format!(
            "INSERT INTO {db}.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES ({above}, 2, 'reflector', 1.0001, '{{}}')"
        ))
        .execute()
        .await
        .unwrap();

    let mut c = cfg(db);
    c.one_shot = true;
    c.usd_reset = Some(external_reset());
    let stats = ChEnrichmentPass::new(c).run().await.unwrap();
    assert_eq!(
        stats.rows_reset, 1,
        "an oracle row above the window must not refuse a reset below it"
    );

    // Now one INSIDE the window: the premise holds again, so it must refuse.
    client
        .query(&format!(
            "INSERT INTO {db}.oracle_prices (timestamp, asset_id, oracle_name, price_usd, raw_data) \
             VALUES ({covered}, 2, 'reflector', 1.0001, '{{}}')"
        ))
        .execute()
        .await
        .unwrap();

    let mut c2 = cfg(db);
    c2.one_shot = true;
    c2.usd_reset = Some(external_reset());
    let err = ChEnrichmentPass::new(c2).run().await.unwrap_err();
    assert!(
        matches!(err, ChEnrichError::ResetBlockedByOracleRows { quote_asset_id, .. }
                 if quote_asset_id == 2),
        "an oracle row inside the window must still refuse, got {err:?}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// The reset obeys `time_window` (the 0111/0114 partition bound), so the repair
/// driver's month-at-a-time walk stays month-at-a-time. A reset that leaked past
/// its window would discard values in months the operator never snapshotted, and
/// the rollback point would not cover them.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn the_external_reset_touches_only_the_bounded_month() {
    let db = "it_enrich_0268_month_bound";
    // 2020-09-13 and 2020-10-02 — different monthly partitions, both covered by
    // an imported rate, so the ONLY thing that can separate them is the window.
    let (sep, oct) = (1_600_000_000u32, 1_601_600_000u32);
    let client = setup_0268(db, sep, oct).await;
    seed_external_rate(&client, db, 1_599_955_200, DEPEG_RATE).await;
    seed_external_rate(&client, db, 1_601_596_800, DEPEG_RATE).await;

    let mut c = cfg(db);
    c.one_shot = true;
    c.usd_reset = Some(external_reset());
    // [2020-09-01, 2020-10-01)
    c.time_window = Some((1_598_918_400, 1_601_510_400));
    let stats = ChEnrichmentPass::new(c).run().await.unwrap();

    assert_eq!(
        stats.rows_reset, 1,
        "only September's candle is in the window"
    );
    let s = close_usd(&client, db, 10, 2, sep).await;
    assert!((s - 3.8724).abs() < 1e-4, "September re-priced, got {s}");
    let o = close_usd(&client, db, 10, 2, oct).await;
    assert!(
        (o - 4.0).abs() < 1e-4,
        "October is outside the window and must be untouched, got {o}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
