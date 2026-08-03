//! Integration test for the `mv_current_prices` refreshable MV (task 0039),
//! against a local Docker ClickHouse:
//!
//!     docker compose up -d clickhouse
//!     cargo test -p prices-clickhouse --test current_mv_it -- --ignored
//!
//! Verifies the MV computes price_usd / volume_24h_usd / market_cap_usd from
//! price_ohlcv_1m + asset_supply, and that a missing supply → market_cap 0
//! (best-effort), plus the task-0072 column set and the §5.5 outlier filter.
//!
//! Each test owns an ISOLATED scratch database (the `prices.*` schema rewritten
//! onto the scratch name), matching `rollup_append_it.rs` / `views_it.rs`. This
//! matters here specifically: every test in this file DROPs and re-CREATEs
//! `mv_current_prices`, so sharing one database would make them race under
//! cargo's default parallel test threads.

use clickhouse::Client;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// Fresh scratch database with the full schema + the MV applied to it.
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
        .expect("init schema");
    // Refreshable MVs need the experimental flag on older builds.
    let mv_client = admin
        .clone()
        .with_option("allow_experimental_refreshable_materialized_view", "1");
    prices_clickhouse::apply_sql(&mv_client, &rewrite(prices_clickhouse::CURRENT_SQL, db))
        .await
        .expect("create mv_current_prices");
    admin
}

async fn teardown(db: &str) {
    let admin = Client::default().with_url(ch_url());
    let _ = admin
        .query(&format!("DROP DATABASE IF EXISTS {db}"))
        .execute()
        .await;
}

async fn scalar_f64(client: &Client, sql: &str) -> f64 {
    client
        .query(sql)
        .fetch_one::<f64>()
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"))
}

fn insert_row(db: &str, asset: u32, close_usd: &str, vol_usd: &str) -> String {
    format!(
        "INSERT INTO {db}.price_ohlcv_1m \
         (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
          volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
         VALUES (now(), {asset}, 2, 'sdex', {close_usd}, {close_usd}, {close_usd}, {close_usd}, \
          50, {vol_usd}, {vol_usd}, {close_usd}, {close_usd}, 1, 1)"
    )
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn current_prices_mv_computes_price_volume_and_market_cap() {
    let db = "it_current_mv_0039";
    let admin = setup(db).await;

    // asset 1 has supply (→ market_cap); asset 3 has none (→ 0).
    admin
        .query(&format!(
            "INSERT INTO {db}.asset_supply (asset_id, token_supply) VALUES (1, 1000)"
        ))
        .execute()
        .await
        .expect("insert supply");
    admin
        .query(&insert_row(db, 1, "2", "100"))
        .execute()
        .await
        .expect("ohlcv 1");
    admin
        .query(&insert_row(db, 3, "5", "40"))
        .execute()
        .await
        .expect("ohlcv 3");

    // Force a refresh and wait for current_prices to populate.
    admin
        .query(&format!("SYSTEM REFRESH VIEW {db}.mv_current_prices"))
        .execute()
        .await
        .expect("refresh view");
    let mut ready = false;
    for _ in 0..30 {
        let n: u64 = admin
            .query(&format!("SELECT count() FROM {db}.current_prices FINAL"))
            .fetch_one()
            .await
            .expect("count current_prices");
        if n >= 2 {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(ready, "MV did not populate current_prices in time");

    let where1 = format!("FROM {db}.current_prices FINAL WHERE asset_id = 1");
    let p = scalar_f64(&admin, &format!("SELECT toFloat64(price_usd) {where1}")).await;
    let v = scalar_f64(
        &admin,
        &format!("SELECT toFloat64(volume_24h_usd) {where1}"),
    )
    .await;
    let m = scalar_f64(
        &admin,
        &format!("SELECT toFloat64(market_cap_usd) {where1}"),
    )
    .await;
    assert!((p - 2.0).abs() < 1e-6, "price_usd should be 2.0, got {p}");
    assert!(
        (v - 100.0).abs() < 1e-6,
        "volume_24h_usd should be 100, got {v}"
    );
    assert!(
        (m - 2000.0).abs() < 1e-6,
        "market_cap = 2 * 1000 = 2000, got {m}"
    );

    let m3 = scalar_f64(
        &admin,
        &format!(
            "SELECT toFloat64(market_cap_usd) FROM {db}.current_prices FINAL WHERE asset_id = 3"
        ),
    )
    .await;
    assert!(
        m3.abs() < 1e-6,
        "market_cap must be 0 without supply, got {m3}"
    );

    teardown(db).await;
}

/// Task 0072 — the columns `mv_current_prices` did NOT write in v1
/// (`price_xlm`, `change_24h_pct`, `change_7d_pct`, `sources`) plus the §5.5
/// inter-source median-outlier filter on `vwap_24h`.
///
/// Fixture, one behaviour per asset:
///   1 XLM — quoted in USDC at $0.50; the divisor for every `price_xlm`
///   3 FOO — three sources, one a gross outlier → the filter must ARM
///   4 BAR — one source → the filter must be a NO-OP
///   5 DUO — exactly two far-apart sources → the >=3 guard must keep BOTH
///   6 EXO — every source unpriced → no divide-by-zero, `sources` = `{}`
///   7 ZER — priced HISTORY, un-enriched TIP → the task-0138 -100% case
///   8 DIP — a genuine ~-100% crash → must NOT be swallowed by 0138's guard
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn current_prices_mv_writes_0072_columns_and_filters_outliers() {
    let db = "it_current_mv_0072";
    let admin = setup(db).await;

    // XLM must be resolvable by natural key (no issuer, no contract) — this is
    // how the MV finds the price_xlm divisor, mirroring ch_enrich.rs:447.
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, is_active) \
             VALUES (1,'XLM','classic','','',1), (3,'FOO','classic','GFOO','',1), \
                    (4,'BAR','classic','GBAR','',1), (5,'DUO','classic','GDUO','',1), \
                    (6,'EXO','classic','GEXO','',1), (7,'ZER','classic','GZER','',1), \
                    (8,'DIP','classic','GDIP','',1)"
        ))
        .execute()
        .await
        .expect("assets");

    // ts_min_ago / close_usd / volume_quote_usd / source, per asset.
    let rows: &[(u32, i64, &str, &str, &str)] = &[
        (1, 2, "0.5", "50", "sdex"),    // XLM/USD = 0.50
        (3, 1380, "0.80", "0", "sdex"), // oldest priced close in window
        (3, 10, "1.00", "100000", "sdex"),
        (3, 9, "1.02", "50000", "soroswap"),
        (3, 8, "5.00", "1000", "aquarius"), // outlier: 390% off the median
        (4, 5, "2.00", "2000", "sdex"),
        (5, 7, "1.00", "1000", "sdex"),
        (5, 6, "3.00", "3000", "soroswap"),
        (6, 4, "0", "0", "sdex"), // exotic: never USD-priceable
        // 0138: priced history + an UN-ENRICHED TIP. price_usd is an unfiltered
        // argMax so it lands on the 0, while open_24h filters close_usd > 0 and
        // lands on the real 2.00 — the exact prod shape that yielded -100%.
        (7, 30, "2.00", "500", "sdex"),
        (7, 1, "0", "0", "sdex"),
        // 0138 control: a REAL near-total crash. The guard must leave this alone.
        (8, 30, "2.00", "500", "sdex"),
        (8, 1, "0.0001", "10", "sdex"),
    ];
    for (i, (asset, mins, cu, vol, src)) in rows.iter().enumerate() {
        admin
            .query(&format!(
                "INSERT INTO {db}.price_ohlcv_1m \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
                 VALUES (now() - INTERVAL {mins} MINUTE, {asset}, 2, '{src}', \
                  {cu},{cu},{cu},{cu}, 10, {vol}, {vol}, {cu}, {cu}, 1, {v})",
                v = i + 1
            ))
            .execute()
            .await
            .unwrap_or_else(|e| panic!("ohlcv row {i}: {e}"));
    }

    // 7d reference lives in the COARSE table on purpose: _1m is retained only
    // INTERVAL 7 DAY (cleanup-worker/src/lib.rs), so a 7-day-old row sits on the
    // retention floor. The close_usd = 0 row is OLDER and must be SKIPPED —
    // picking it would report -100% (the task-0114 un-enriched-coarse case).
    for (mins, cu) in [(6 * 24 * 60_i64, "0.50"), (7 * 24 * 60 - 60, "0")] {
        admin
            .query(&format!(
                "INSERT INTO {db}.price_ohlcv_1h \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
                 VALUES (now() - INTERVAL {mins} MINUTE, 3, 2, 'sdex', \
                  {cu},{cu},{cu},{cu}, 10, 5, 5, {cu}, {cu}, 1, 1)"
            ))
            .execute()
            .await
            .expect("1h ref row");
    }

    // 7d references for the 0138 pair, so change_7d_pct has a real denominator
    // to divide the zero/near-zero tip by.
    for asset in [7_u32, 8] {
        admin
            .query(&format!(
                "INSERT INTO {db}.price_ohlcv_1h \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
                 VALUES (now() - INTERVAL {mins} MINUTE, {asset}, 2, 'sdex', \
                  1.00,1.00,1.00,1.00, 10, 5, 5, 1.00, 1.00, 1, 1)",
                mins = 6 * 24 * 60_i64
            ))
            .execute()
            .await
            .expect("0138 7d ref row");
    }

    admin
        .query(&format!("SYSTEM REFRESH VIEW {db}.mv_current_prices"))
        .execute()
        .await
        .expect("refresh view");
    let mut ready = false;
    for _ in 0..30 {
        let n: u64 = admin
            .query(&format!("SELECT count() FROM {db}.current_prices FINAL"))
            .fetch_one()
            .await
            .expect("count");
        if n >= 7 {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(ready, "MV did not populate current_prices in time");

    let f = |col: &str, id: u32| {
        format!("SELECT toFloat64({col}) FROM {db}.current_prices FINAL WHERE asset_id = {id}")
    };
    let s = |col: &str, id: u32| {
        format!("SELECT {col} FROM {db}.current_prices FINAL WHERE asset_id = {id}")
    };

    // ── the outlier filter actually fires ──────────────────────────────────
    // Unfiltered the VWAP would be 156000/151000 = 1.03311258; dropping the
    // aquarius outlier gives 151000/150000 = 1.00666667. Asserting the filtered
    // value is what makes this test able to FAIL if the filter silently no-ops.
    let vwap = scalar_f64(&admin, &f("vwap_24h", 3)).await;
    assert!(
        (vwap - 1.006_666_67).abs() < 1e-6,
        "vwap must exclude the outlier (expect ~1.00666667, unfiltered would be \
         ~1.03311258), got {vwap}"
    );

    // volume is a TOTAL: the outlier's 1000 is still counted.
    let vol = scalar_f64(&admin, &f("volume_24h_usd", 3)).await;
    assert!(
        (vol - 151_000.0).abs() < 1e-3,
        "volume_24h_usd must include every source (151000), got {vol}"
    );

    // ── sources JSON: excluded source absent, precision preserved ──────────
    let srcs: String = admin
        .query(&s("sources", 3))
        .fetch_one()
        .await
        .expect("sources");
    assert!(
        !srcs.contains("aquarius"),
        "outlier source must be ABSENT from sources, got {srcs}"
    );
    assert!(
        srcs.contains("sdex") && srcs.contains("soroswap"),
        "got {srcs}"
    );
    assert!(
        srcs.contains("\"1.02\""),
        "per-source price must keep full Decimal precision as a STRING, got {srcs}"
    );

    // ── the >=3-source guard: with 2 far-apart sources BOTH survive ────────
    // Without the guard, median([1,3]) = 2 and both deviate 50% > 20% → the mask
    // clears every source and vwap collapses to 0. This is the regression.
    let duo = scalar_f64(&admin, &f("vwap_24h", 5)).await;
    assert!(
        (duo - 2.5).abs() < 1e-6,
        "2-source assets must bypass the filter (expect 2.5, a collapsed mask \
         would give 0), got {duo}"
    );

    // ── single source: filter is a no-op ───────────────────────────────────
    let bar = scalar_f64(&admin, &f("vwap_24h", 4)).await;
    assert!(
        (bar - 2.0).abs() < 1e-6,
        "single-source vwap 2.0, got {bar}"
    );

    // ── price_xlm = price_usd / xlm_usd ────────────────────────────────────
    let bar_xlm = scalar_f64(&admin, &f("price_xlm", 4)).await;
    assert!(
        (bar_xlm - 4.0).abs() < 1e-6,
        "price_xlm = 2.00 / 0.50 = 4, got {bar_xlm}"
    );
    let xlm_xlm = scalar_f64(&admin, &f("price_xlm", 1)).await;
    assert!(
        (xlm_xlm - 1.0).abs() < 1e-6,
        "XLM priced in XLM must be exactly 1, got {xlm_xlm}"
    );

    // ── change columns ─────────────────────────────────────────────────────
    let c24 = scalar_f64(&admin, &f("change_24h_pct", 3)).await;
    assert!(
        (c24 - 525.0).abs() < 1e-2,
        "0.80 -> 5.00 is +525%, got {c24}"
    );
    let c7d = scalar_f64(&admin, &f("change_7d_pct", 3)).await;
    assert!(
        (c7d - 900.0).abs() < 1e-2,
        "0.50 -> 5.00 is +900%; a value near -100 means the un-enriched \
         close_usd = 0 coarse row was picked, got {c7d}"
    );

    // ── exotic asset: degrades cleanly, never divides by zero ──────────────
    let exo_p = scalar_f64(&admin, &f("price_usd", 6)).await;
    let exo_c = scalar_f64(&admin, &f("change_24h_pct", 6)).await;
    let exo_s: String = admin
        .query(&s("sources", 6))
        .fetch_one()
        .await
        .expect("exotic sources");
    assert!(exo_p.abs() < 1e-9, "unpriced asset must be 0, got {exo_p}");
    assert!(
        exo_c.abs() < 1e-9,
        "change must be 0, not -100, got {exo_c}"
    );
    assert_eq!(exo_s, "{}", "exotic sources must be an empty object");

    // ── task 0138: an un-enriched TIP must not fabricate -100% ─────────────
    // Distinct from asset 6 above: there, EVERY close_usd is 0, so open_24h is
    // 0 too and the denominator guard alone lands on the sentinel. Here the
    // history IS priced, so the denominator is real and only the NUMERATOR
    // guard can save it. That is the case prod hit on 889 assets.
    //
    // Control first — prove the fixture genuinely reproduces the bug, so the
    // assertions below cannot pass vacuously.
    let unguarded = scalar_f64(
        &admin,
        &format!(
            // ifNull because the division yields Nullable(Float64), which
            // fetch_one::<f64> decodes as garbage. A NULL here would surface as
            // 0 and fail the assert below, which is the correct outcome anyway.
            "SELECT ifNull( \
                    (toFloat64(argMax(close_usd, timestamp)) \
                     - toFloat64(argMinIf(close_usd, timestamp, close_usd > 0))) \
                    / nullIf(toFloat64(argMinIf(close_usd, timestamp, close_usd > 0)), 0) * 100, \
                    0) \
             FROM {db}.price_ohlcv_1m FINAL \
             WHERE asset_id = 7 AND timestamp >= now() - INTERVAL 24 HOUR"
        ),
    )
    .await;
    assert!(
        (unguarded + 100.0).abs() < 1e-6,
        "control: the UNGUARDED expression must yield -100 on this fixture, else \
         the test below proves nothing; got {unguarded}"
    );

    let zer_p = scalar_f64(&admin, &f("price_usd", 7)).await;
    assert!(
        zer_p.abs() < 1e-9,
        "fixture precondition: un-enriched tip means price_usd 0, got {zer_p}"
    );
    let zer_24 = scalar_f64(&admin, &f("change_24h_pct", 7)).await;
    assert!(
        zer_24.abs() < 1e-9,
        "an un-enriched tip must yield the 0 = 'no signal' sentinel, NOT -100 \
         (task 0138: 889 prod assets published -100 this way), got {zer_24}"
    );
    let zer_7d = scalar_f64(&admin, &f("change_7d_pct", 7)).await;
    assert!(
        zer_7d.abs() < 1e-9,
        "change_7d_pct must be sentinel 0, not -100, got {zer_7d}"
    );

    // ── task 0138: the guard must NOT swallow a genuine crash ──────────────
    // 2.00 -> 0.0001 is a real -99.995%. It keys on price_usd being exactly the
    // 0 sentinel, so a real near-zero price stays fully reportable.
    let dip_24 = scalar_f64(&admin, &f("change_24h_pct", 8)).await;
    assert!(
        (dip_24 + 99.995).abs() < 1e-2,
        "a REAL ~-100% crash must survive the 0138 guard (expect -99.995), got {dip_24}"
    );

    // price_xlm's own zero-guard, asserted rather than assumed (0138 AC).
    let zer_xlm = scalar_f64(&admin, &f("price_xlm", 7)).await;
    assert!(
        zer_xlm.abs() < 1e-9,
        "price_xlm must land on the 0 sentinel for a zero price_usd, got {zer_xlm}"
    );

    teardown(db).await;
}
