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
///   7 ZER — priced HISTORY, un-enriched TIP → 0135: latest priced close wins
///   8 DIP — a genuine ~-100% crash → must NOT be swallowed by 0138's guard
///  10 STA — priced history OLDER than the 2h carry bound → back to sentinels,
///           and the change_* numerator guards proven load-bearing
///  11 LAG — SINGLE source, priced history, un-enriched tip → carried (the
///           shape the XLM acceptance criterion names)
///  12 TRI — three sources, one carried past an un-enriched tip → the §5.5
///           mask arms over the carried population and keeps all three
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
                    (8,'DIP','classic','GDIP','',1), (10,'STA','classic','GSTA','',1), \
                    (11,'LAG','classic','GLAG','',1), (12,'TRI','classic','GTRI','',1)"
        ))
        .execute()
        .await
        .expect("assets");

    // Supply for asset 7 so market_cap_usd = price_usd x supply is assertable
    // (without a row the LEFT JOIN misses and market_cap is 0 for any price).
    admin
        .query(&format!(
            "INSERT INTO {db}.asset_supply (asset_id, token_supply) VALUES (7, 1000)"
        ))
        .execute()
        .await
        .expect("asset_supply");

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
        // 0135: priced history + an UN-ENRICHED TIP. Before the guard, the
        // unfiltered argMax landed price_usd on the 0 (the prod shape that
        // yielded 0138's -100%); with argMaxIf(close_usd > 0) the tip is
        // SKIPPED and price_usd is the latest priced close — soroswap's 1.90.
        //
        // TWO sources on purpose, and the un-enriched tip sits on sdex, whose
        // history IS priced (2.00). That pins the C2 half of the contract:
        // per_source's argMaxIf keeps sdex at its latest priced close, so it
        // stays in `sources` and the vwap weighting instead of vanishing with
        // enrichment timing. Production XLM had price_usd = 0 *while* sources
        // carried two live venues — this fixture reproduces the shape and must
        // now publish real numbers on every column.
        (7, 30, "2.00", "500", "sdex"),
        (7, 2, "1.90", "400", "soroswap"),
        (7, 1, "0", "0", "sdex"),
        // 0138 control: a REAL near-total crash. The guard must leave this alone.
        (8, 30, "2.00", "500", "sdex"),
        (8, 1, "0.0001", "10", "sdex"),
        // 10 STA — the carry BOUND: priced history 190 min behind the tip
        // (> 2h), so the carry must NOT apply and every column returns to its
        // sentinel. Its 1h ref row below gives change_7d a REAL denominator,
        // which makes this the regression test for the 7d numerator guard.
        (10, 190, "2.00", "500", "sdex"),
        (10, 1, "0", "0", "sdex"),
        // 11 LAG — single source, priced history INSIDE the bound: the carry
        // must keep the venue alive on every column.
        (11, 30, "2.00", "300", "sdex"),
        (11, 1, "0", "0", "sdex"),
        // 12 TRI — three sources, aquarius carried past an un-enriched tip:
        // the >=3 mask ARMS over a population containing a carried price and
        // must keep all three (all within 2% of the median).
        (12, 10, "1.00", "100000", "sdex"),
        (12, 9, "1.02", "50000", "soroswap"),
        (12, 50, "1.01", "20000", "aquarius"),
        (12, 1, "0", "0", "aquarius"),
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

    // 7d references for assets whose change_7d needs a real denominator:
    // 7/8 (the 0138 pair) and 10 (over-bound price_usd = 0 beside a REAL
    // baseline — the exact shape only the numerator guard protects).
    for asset in [7_u32, 8, 10] {
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
        if n >= 10 {
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
    // ⚠️ This asserts CURRENT behaviour, and that behaviour is questionable.
    // change_*_pct reads price_usd from the `unfiltered` CTE, which includes
    // venues the §5.5 median filter REJECTED. aquarius (5.00) is asserted above
    // to be excluded from vwap_24h and absent from `sources` — yet it is the
    // newest candle, so argMax makes it price_usd and the row publishes
    // "+525% in 24h" beside a sources object showing only ~1.00/1.02 and a VWAP
    // of 1.0067. That is the same class of confident-wrong-answer as the -100
    // this task fixes, by a different route: an outlier venue rather than a
    // zero. Fixing it means outlier-filtering price_usd itself, which is
    // [[0135]]'s scope (it already owns "price_usd is not outlier-filtered" and
    // its propagation into market_cap_usd) — NOT the nullIf guard here.
    // Asserted so the behaviour is pinned and visible, not endorsed.
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

    // ── task 0135: an un-enriched TIP is skipped, not read as 0 ────────────
    // Distinct from asset 6 above: there, EVERY close_usd is 0, so the row
    // legitimately publishes the sentinels. Here the history IS priced, so the
    // 0135 contract requires the latest PRICED close on every column — the
    // case prod hit on 88 of the top-200 volume rows (XLM and EURC included).
    //
    // Control first — prove the fixture genuinely reproduces the pre-guard
    // bug, so the assertions below cannot pass vacuously.
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
        (zer_p - 1.90).abs() < 1e-9,
        "price_usd must be the latest PRICED close (soroswap 1.90), not the \
         un-enriched tip's 0 — the 0135 contract, got {zer_p}"
    );
    let zer_24 = scalar_f64(&admin, &f("change_24h_pct", 7)).await;
    assert!(
        (zer_24 + 5.0).abs() < 1e-2,
        "change_24h_pct must be computed from the priced close: \
         (1.90 - 2.00) / 2.00 = -5%, NOT the 0 sentinel and NOT -100, got {zer_24}"
    );
    let zer_7d = scalar_f64(&admin, &f("change_7d_pct", 7)).await;
    assert!(
        (zer_7d - 90.0).abs() < 1e-2,
        "change_7d_pct must be (1.90 - 1.00) / 1.00 = +90%, got {zer_7d}"
    );

    // The C2 half: sdex's tip is un-enriched but its HISTORY is priced, so it
    // must be carried at its latest priced close (2.00) — present in `sources`
    // and weighted into the vwap — instead of vanishing with enrichment timing.
    let zer_vwap = scalar_f64(&admin, &f("vwap_24h", 7)).await;
    let zer_srcs: String = admin
        .query(&s("sources", 7))
        .fetch_one()
        .await
        .expect("zer sources");
    assert!(
        (zer_vwap - 1.955_555_56).abs() < 1e-6,
        "vwap must weight BOTH sources ((2.00*500 + 1.90*400) / 900 = \
         1.95555556); 1.9 alone means sdex was dropped by enrichment timing, \
         got {zer_vwap}"
    );
    assert!(
        zer_srcs.contains("soroswap") && zer_srcs.contains("sdex"),
        "both venues must be in sources — sdex carried at its latest priced \
         close per C2, got {zer_srcs}"
    );

    // price_xlm follows the priced close too: 1.90 / 0.50 = 3.8. Before the
    // guard this column collapsed to 0 with price_usd (the AC names it).
    let zer_xlm = scalar_f64(&admin, &f("price_xlm", 7)).await;
    assert!(
        (zer_xlm - 3.8).abs() < 1e-6,
        "price_xlm must be 1.90 / 0.50 = 3.8, got {zer_xlm}"
    );

    // ── market_cap follows the priced close too: 1.90 x supply 1000 ───────
    let zer_mc = scalar_f64(&admin, &f("market_cap_usd", 7)).await;
    assert!(
        (zer_mc - 1900.0).abs() < 1e-6,
        "market_cap_usd must be 1.90 * 1000 = 1900, got {zer_mc}"
    );

    // ── 0135 carry BOUND: history older than 2h must NOT be carried ────────
    // Asset 10's only priced candle is 190 min behind its tip. Publishing it
    // would certify a 3h-old close as current (updated_at carries no age), so
    // every column returns to its sentinel instead.
    let sta_p = scalar_f64(&admin, &f("price_usd", 10)).await;
    assert!(
        sta_p.abs() < 1e-9,
        "price_usd beyond the 2h carry bound must be the 0 sentinel, got {sta_p}"
    );
    let sta_srcs: String = admin
        .query(&s("sources", 10))
        .fetch_one()
        .await
        .expect("sta sources");
    assert_eq!(
        sta_srcs, "{}",
        "an over-bound source must drop out of sources"
    );
    // The numerator guards are LOAD-BEARING here, not belt-and-braces:
    // open_24h is a real 2.00 and close_7d_ago a real 1.00, so without the
    // nullIf on the numerator both columns would fabricate -100%.
    let sta_24 = scalar_f64(&admin, &f("change_24h_pct", 10)).await;
    assert!(
        sta_24.abs() < 1e-9,
        "change_24h_pct must be the 0 sentinel, NOT -100 (real open_24h=2.00 \
         beside a zero price_usd), got {sta_24}"
    );
    let sta_7d = scalar_f64(&admin, &f("change_7d_pct", 10)).await;
    assert!(
        sta_7d.abs() < 1e-9,
        "change_7d_pct must be the 0 sentinel, NOT -100 (real 1h baseline 1.00 \
         beside a zero price_usd) — this is the only fixture exercising the 7d \
         numerator guard, got {sta_7d}"
    );

    // ── 0135 single-source carry: the shape the XLM AC names ───────────────
    let lag_p = scalar_f64(&admin, &f("price_usd", 11)).await;
    let lag_vwap = scalar_f64(&admin, &f("vwap_24h", 11)).await;
    let lag_srcs: String = admin
        .query(&s("sources", 11))
        .fetch_one()
        .await
        .expect("lag sources");
    assert!(
        (lag_p - 2.0).abs() < 1e-9,
        "single-source asset with an in-bound un-enriched tip must publish its \
         latest priced close, got {lag_p}"
    );
    assert!(
        (lag_vwap - 2.0).abs() < 1e-6,
        "single carried source must still yield a vwap, got {lag_vwap}"
    );
    assert!(
        lag_srcs.contains("sdex") && lag_srcs.contains("\"2\""),
        "the carried venue must appear in sources at its priced close, got {lag_srcs}"
    );

    // ── 0135 x §5.5: the mask arms over a population WITH a carried price ──
    // Three sources, aquarius carried (1.01 from 50 min ago, tip un-enriched).
    // All three sit within 2% of the median, so arming must keep all three —
    // a carried-but-plausible price must not evict anyone.
    let tri_vwap = scalar_f64(&admin, &f("vwap_24h", 12)).await;
    let tri_srcs: String = admin
        .query(&s("sources", 12))
        .fetch_one()
        .await
        .expect("tri sources");
    assert!(
        (tri_vwap - 1.007_058_823_5).abs() < 1e-6,
        "vwap must weight all three sources ((1.00*100000 + 1.02*50000 + \
         1.01*20000) / 170000 = 1.00705882), got {tri_vwap}"
    );
    assert!(
        tri_srcs.contains("sdex") && tri_srcs.contains("soroswap") && tri_srcs.contains("aquarius"),
        "all three venues incl. the carried one must be in sources, got {tri_srcs}"
    );
    let tri_p = scalar_f64(&admin, &f("price_usd", 12)).await;
    assert!(
        (tri_p - 1.02).abs() < 1e-9,
        "price_usd is the newest priced close (soroswap 1.02), got {tri_p}"
    );

    // ── task 0138: the guard must NOT swallow a genuine crash ──────────────
    // 2.00 -> 0.0001 is a real -99.995%. It keys on price_usd being exactly the
    // 0 sentinel, so a real near-zero price stays fully reportable.
    let dip_24 = scalar_f64(&admin, &f("change_24h_pct", 8)).await;
    assert!(
        (dip_24 + 99.995).abs() < 1e-2,
        "a REAL ~-100% crash must survive the 0138 guard (expect -99.995), got {dip_24}"
    );

    // NOTE: asset 7's price_xlm above exercises the happy path only; the
    // missing-DIVISOR guard needs an asset priced while no XLM market exists,
    // which is `price_xlm_lands_on_the_sentinel_when_the_xlm_divisor_is_missing`
    // below.

    teardown(db).await;
}

/// Task 0138 — `price_xlm`'s divisor guard, exercised rather than assumed.
///
/// The obvious assertion (`price_usd = 0` ⇒ `price_xlm = 0`) is **vacuous**:
/// `0 / anything` is 0 whether or not the `nullIf` is present. The guard only
/// does work when the DIVISOR is missing — no XLM market at all — while the
/// asset itself is priced. Without `nullIf(toFloat64(xlm_usd), 0)` that is a
/// division by zero rather than the 0 = "unavailable" sentinel.
///
/// Needs its own scratch database because `xlm_usd` is a single scalar for the
/// whole MV: the only way to make it absent is to have no XLM row anywhere.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn price_xlm_lands_on_the_sentinel_when_the_xlm_divisor_is_missing() {
    let db = "it_current_mv_0138_no_xlm";
    let admin = setup(db).await;

    // Deliberately NO XLM row — the xlm_usd subquery finds nothing.
    admin
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, is_active) \
             VALUES (9,'SOLO','classic','GSOLO','',1)"
        ))
        .execute()
        .await
        .expect("assets");
    admin
        .query(&insert_row(db, 9, "2.00", "100"))
        .execute()
        .await
        .expect("ohlcv 9");

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
        if n >= 1 {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    assert!(ready, "MV did not populate current_prices in time");

    let q = |col: &str| {
        format!("SELECT toFloat64({col}) FROM {db}.current_prices FINAL WHERE asset_id = 9")
    };

    // Non-vacuity: the numerator is genuinely non-zero, so a passing price_xlm
    // of 0 can only come from the divisor guard.
    let p = scalar_f64(&admin, &q("price_usd")).await;
    assert!(
        (p - 2.0).abs() < 1e-6,
        "precondition: the asset must be priced, else this proves nothing, got {p}"
    );

    let xlm = scalar_f64(&admin, &q("price_xlm")).await;
    assert!(
        xlm.abs() < 1e-9,
        "a missing XLM divisor must land on the 0 = 'unavailable' sentinel, \
         not divide by zero, got {xlm}"
    );

    teardown(db).await;
}
