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
///   6 EXO — every source unpriced, but WITH a real 7d baseline → the only
///           fixture exercising the load-bearing change_7d numerator guard
///   7 ZER — priced HISTORY, un-enriched TIP → 0135: latest priced close wins
///   8 DIP — a genuine ~-100% crash → must NOT be swallowed by 0138's guard
///  10 STA — EVERY venue dead → the conditional bound must NOT fire; the
///           venue is kept, because there is no live venue to protect
///  14 MIX — one live venue, TWO dead → the bound FIRES: the dead pair is
///           dropped before it can outvote the live one in the §5.5 median
///  15 EVN — four dead venues straddling the interpolated median → the mask
///           clears ALL of them, leaving a price beside empty `sources`
///  16 LIV — a LIVE venue whose newest priced close is past the bound → it
///           must be KEPT: liveness is measured on candles, not enrichment
///  11 LAG — SINGLE source, priced history, un-enriched tip → carried (the
///           shape the XLM acceptance criterion names)
///  12 TRI — three sources, one carried past an un-enriched tip → the §5.5
///           mask arms over the carried population and keeps all three
///  13 NRB — priced, but its only 1h reference sits at 4 d, INSIDE the [7d, 5d]
///           band's recent cutoff → change_7d_pct must be the sentinel rather
///           than a 4-day move published as a 7-day one
///  17 DST — a dust venue (below MIN_VOLUME_USD) with an absurd price → the
///           0118 threshold drops it before the median; price_usd and
///           volume_24h_usd stay demonstrably UNAFFECTED
///  18 ATK — two live dust venues straddling a deep market → without the
///           threshold they OWN the median and evict the deep venue; with it
///           they never vote (the exact defect 0118 closes)
///  19 THL — live dust + stale real venue → the threshold runs BEFORE the
///           asset_has_live window, so the asset counts as all-quiet and the
///           stale venue is kept
///  20 SUB — every venue below the threshold → the CONDITIONAL threshold is
///           a no-op (no funded venue to defend) and the dust venue is KEPT,
///           exactly like the liveness bound's all-dead arm
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
                    (11,'LAG','classic','GLAG','',1), (12,'TRI','classic','GTRI','',1), \
                    (13,'NRB','classic','GNRB','',1), (14,'MIX','classic','GMIX','',1)"
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
        // 10 STA — ARM A of the conditional bound. EVERY venue here is dead:
        // sdex's newest candle is 190 min old, past the 2h bound. With no live
        // venue to protect there is nothing to defend, so the bound must NOT
        // fire — sdex is KEPT, `sources` names it and `vwap_24h` comes from
        // it. Dropping it would blank the row for an asset we hold a perfectly
        // good, merely stale, price for: that is the 2026-08-21 prod rollback,
        // reproduced as a test.
        //
        // There is deliberately NO un-enriched tip row here. Liveness is
        // measured on the newest CANDLE, not the newest priced close (PR #241
        // review, finding 1), so a tip at 1 min would make sdex LIVE and this
        // fixture would silently stop testing ARM A at all.
        (10, 190, "2.00", "500", "sdex"),
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
        // 13 NRB — ordinary priced asset; the interesting part is its 1h
        // reference below, which is too RECENT for the [7d, 5d] band.
        (13, 5, "2.00", "1000", "sdex"),
        // 14 MIX — the conditional bound FIRING. sdex last quoted 190 min ago
        // (stale), soroswap 5 min ago (fresh). Because a fresh venue survives,
        // the stale one is dropped: it must not vote in the §5.5 median.
        // 14 MIX — ARM B: a live venue survives, so the bound FIRES. Two dead
        // venues at 5.00 and one live at 1.00. THREE sources, because that is
        // the whole point: the §5.5 mask is a documented no-op below 3
        // (current.sql), so the two-source version of this fixture asserted
        // the bound while never exercising the defect the bound exists for
        // (PR #241 review, finding 5). At three, WITHOUT the bound the median
        // is 5.00, the live venue deviates 80% and IS the one evicted —
        // publishing a vwap of 5.00 from two venues that stopped quoting hours
        // ago. With the bound the dead pair is dropped before it can vote.
        (14, 190, "5.00", "800", "sdex"),
        (14, 200, "5.00", "700", "aquarius"),
        (14, 5, "1.00", "400", "soroswap"),
        // 15 EVN — the §5.5 mask clearing EVERY source. Four dead venues at
        // 1.00, 1.00, 3.00, 3.00: `median` is `quantile(0.5)` and interpolates
        // to 2.00, so all four deviate 50% and none survives. The row still
        // publishes `price_usd` 1.00, which is venue-blind. That pairing —
        // a real price beside `sources = {}` and `vwap_24h = 0` — disproves
        // the "price, sources and VWAP, or none of the three, BY CONSTRUCTION"
        // claim (PR #241 review, finding 2). Reachable only because C2's carry
        // brings all-dead assets into the mask at all; before it they arrived
        // with zero kept sources and it never armed.
        // Volumes 200, not 100: kept ABOVE MIN_VOLUME_USD (0118) so the
        // threshold is provably not what clears this fixture. At $100 the
        // conditional arm would keep every venue anyway (nothing here is
        // funded), so the mask would still be the cause — but only by
        // accident of the conditional rule, and a later move back to an
        // unconditional threshold would silently empty this fixture and make
        // its mask assertions vacuous.
        (15, 200, "1.00", "200", "sdex"),
        (15, 210, "1.00", "200", "soroswap"),
        (15, 220, "3.00", "200", "aquarius"),
        (15, 230, "3.00", "200", "phoenix"),
        // 16 LIV — the ONLY fixture that discriminates candle-liveness from
        // enrichment-freshness, and the shape PR #241's review found (finding
        // 1). sdex is quoting RIGHT NOW and carries essentially all the
        // volume, but its newest candle is un-enriched and its newest PRICED
        // close is 3 h old — older than the bound. soroswap is tiny and
        // happens to have been enriched 5 min ago.
        //
        // Liveness on CANDLES (correct): sdex is live, both venues are kept,
        // and vwap ~= 1.00 because sdex carries the weight.
        // Liveness on PRICED CLOSES (the defect): sdex reads as dead, is
        // evicted in favour of soroswap, and the row publishes vwap 1.20 —
        // drawn from $150 of the $1,000,160 the same row reports as volume.
        //
        // Assets 10/14/15 behave IDENTICALLY under both predicates, so
        // without this fixture a revert to the old form stays green.
        // soroswap at $150, not $10 as the PR #241 probe had it: it must clear
        // the 0118 threshold to stay in the fixture's cast at all — the point
        // here is liveness, and $150 is still noise against sdex's $1M.
        (16, 1, "0", "1000000", "sdex"),
        (16, 180, "1.00", "10", "sdex"),
        (16, 5, "1.20", "150", "soroswap"),
        // 17 DST — the 0118 threshold. soroswap holds $50 of volume and quotes
        // an absurd 5.00; it must be dropped BEFORE the median so the mask
        // arms over the two real venues only. Its row is deliberately the
        // NEWEST priced close, so price_usd = 5.00 — pinning that the
        // threshold is a weighting rule and price_usd stays venue-blind
        // (pinned-not-endorsed, same note as asset 3's change columns).
        (17, 10, "1.00", "100000", "sdex"),
        (17, 9, "1.02", "20000", "aquarius"),
        (17, 8, "5.00", "50", "soroswap"),
        // 18 ATK — the manipulation shape 0118 closes. Two live dust venues
        // straddle one deep market. WITHOUT the threshold the dust pair owns
        // the unweighted median (1.35), the deep venue deviates 26% and is
        // evicted, and the published vwap is ~1.3615 — drawn from $65 of the
        // $500,065 the row reports. WITH it the dust never votes.
        (18, 5, "1.00", "500000", "sdex"),
        (18, 4, "1.35", "40", "soroswap"),
        (18, 3, "1.38", "25", "phoenix"),
        // 19 THL — threshold × liveness ordering. soroswap is LIVE but dust;
        // sdex is STALE (190 min) but real. Threshold-first means the
        // asset_has_live window sees only sdex → all-quiet arm → sdex kept.
        // Threshold-after-window would let dust soroswap arm the guard, evict
        // sdex as stale, then vanish itself — sources {} on an asset we hold
        // a good price for.
        (19, 190, "2.00", "10000", "sdex"),
        (19, 5, "1.00", "50", "soroswap"),
        // 20 SUB — every venue below the threshold: the CONDITIONAL arm.
        // There is no funded venue to defend, so the threshold must NOT fire
        // and the $50 venue is kept — the same "nothing to defend, dropping
        // is pure loss" argument as the liveness bound's all-dead arm.
        // Measured on prod 2026-08-27: the unconditional form would have
        // blanked 2,960 of 3,068 priced assets (96.5%).
        (20, 5, "3.00", "50", "sdex"),
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
    // 7/8 (the 0138 pair), 10 (over-bound), and 6 — the unpriced asset, now
    // the ONLY fixture pairing `price_usd = 0` with a REAL 7d baseline. That
    // pairing is what the numerator guard protects, and it is reachable
    // because ref_7d reads price_ohlcv_1h, a different table from the 24h
    // window that zeroes price_usd.
    for asset in [6_u32, 7, 8, 10] {
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

    // Asset 13's ONLY 1h reference sits at 4 days — newer than the band's
    // `now() - 5 DAY` cutoff. Without that cutoff argMinIf would pick it and
    // publish a 4-day move as `change_7d_pct`; with it, close_7d_ago stays 0
    // and the denominator guard lands on the sentinel. This is the only
    // fixture that exercises the recent edge of the band.
    admin
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
             VALUES (now() - INTERVAL {mins} MINUTE, 13, 2, 'sdex', \
              1.00,1.00,1.00,1.00, 10, 5, 5, 1.00, 1.00, 1, 1)",
            mins = 4 * 24 * 60_i64
        ))
        .execute()
        .await
        .expect("13 too-recent 7d ref row");

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
        if n >= 18 {
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

    // ── conditional bound, ARM A: every venue stale → do NOT fire ─────────
    // Asset 10's only venue last quoted 190 min ago, past the 2h bound. There
    // is no live venue to protect, so dropping it would be pure loss — it
    // would empty `sources` and zero `vwap_24h` on an asset whose price we
    // hold. Measured on prod 2026-08-21, the unconditional form did exactly
    // that to 2,284 of 4,365 assets (52%) while preventing zero evictions.
    let sta_p = scalar_f64(&admin, &f("price_usd", 10)).await;
    assert!(
        (sta_p - 2.0).abs() < 1e-9,
        "price_usd is not age-bounded, so the 190-min-old close still publishes, got {sta_p}"
    );
    let sta_srcs: String = admin
        .query(&s("sources", 10))
        .fetch_one()
        .await
        .expect("sta sources");
    assert!(
        sta_srcs.contains("sdex"),
        "with NO fresh venue on the asset the stale one must be KEPT — dropping \
         it defends nothing and blanks the row, got {sta_srcs}"
    );
    let sta_vwap = scalar_f64(&admin, &f("vwap_24h", 10)).await;
    assert!(
        (sta_vwap - 2.0).abs() < 1e-6,
        "vwap must still come from the kept venue, got {sta_vwap}"
    );

    // ── conditional bound, ARM B: a live venue survives → DO fire ──────────
    // Asset 14 has sdex and aquarius DEAD at 5.00 (190/200 min) and soroswap
    // LIVE at 1.00 (5 min). Three sources, so the §5.5 mask genuinely arms and
    // the defect is real: unweighted, the dead pair's 5.00 is the median, the
    // live venue deviates 80% and is the one the mask would evict. The bound
    // drops the dead pair before it can vote.
    let mix_srcs: String = admin
        .query(&s("sources", 14))
        .fetch_one()
        .await
        .expect("mix sources");
    assert!(
        mix_srcs.contains("soroswap")
            && !mix_srcs.contains("sdex")
            && !mix_srcs.contains("aquarius"),
        "both dead venues must be dropped when a live one survives, got {mix_srcs}"
    );
    let mix_vwap = scalar_f64(&admin, &f("vwap_24h", 14)).await;
    assert!(
        (mix_vwap - 1.0).abs() < 1e-6,
        "vwap must come from the LIVE venue (1.00). 5.00 means the bound did \
         not fire and the dead pair outvoted it — the exact defect this arm \
         exists to pin; got {mix_vwap}"
    );
    // price_usd is unbounded and venue-blind: it is simply the newest priced
    // close on the asset, which here is the live one.
    let mix_p = scalar_f64(&admin, &f("price_usd", 14)).await;
    assert!(
        (mix_p - 1.0).abs() < 1e-9,
        "price_usd is the newest priced close regardless of venue, got {mix_p}"
    );

    // ── the §5.5 mask can clear EVERY source (review finding 2) ────────────
    // Asset 15's four dead venues straddle the interpolated median 2.00 by
    // 50%, so nothing survives — while price_usd still publishes. This is the
    // counterexample to "by construction": an asset CAN hold a price with no
    // sources and no VWAP beside it. Whether interpolation is the right median
    // for an even count is task 0238's decision, not this test's.
    let evn_srcs: String = admin
        .query(&s("sources", 15))
        .fetch_one()
        .await
        .expect("evn sources");
    assert!(
        !evn_srcs.contains("sdex") && !evn_srcs.contains("phoenix"),
        "an even-count set straddling the median by > OUTLIER_PCT must clear \
         entirely — if this keeps sources, the mask's arming rule changed and \
         current.sql's note about it is stale; got {evn_srcs}"
    );
    let evn_vwap = scalar_f64(&admin, &f("vwap_24h", 15)).await;
    assert!(
        evn_vwap.abs() < 1e-9,
        "vwap must be 0 when every source is filtered out, got {evn_vwap}"
    );
    let evn_p = scalar_f64(&admin, &f("price_usd", 15)).await;
    assert!(
        (evn_p - 1.0).abs() < 1e-9,
        "price_usd is venue-blind and must still publish the newest priced \
         close — this pairing is what disproves the three-way invariant; \
         got {evn_p}"
    );

    // ── liveness is a property of QUOTING, not of enrichment (finding 1) ──
    // Asset 16's sdex quotes at 1 min with $1,000,000 of volume but was last
    // enriched 3 h ago; soroswap is $10 enriched 5 min ago. Gating on the
    // newest PRICED close evicts sdex and publishes soroswap's 1.20 as the
    // VWAP of a $1,000,020 asset. Gating on the newest CANDLE keeps it.
    let liv_srcs: String = admin
        .query(&s("sources", 16))
        .fetch_one()
        .await
        .expect("liv sources");
    assert!(
        liv_srcs.contains("sdex") && liv_srcs.contains("soroswap"),
        "a venue quoting 1 min ago must be LIVE even though its newest priced \
         close is past the bound — dropping it is the defect PR #241's review \
         found; got {liv_srcs}"
    );
    let liv_vwap = scalar_f64(&admin, &f("vwap_24h", 16)).await;
    assert!(
        (liv_vwap - 1.0).abs() < 1e-3,
        "vwap must be dominated by the venue holding the volume (~1.00). 1.20 \
         means liveness was gated on enrichment and the $1,000,000 venue was \
         evicted in favour of a $150 one; got {liv_vwap}"
    );

    // ── 0118: the min_volume_usd threshold ─────────────────────────────────
    // Asset 17 — a $50 venue quoting an absurd 5.00 is dropped BEFORE the
    // median: the two real venues vwap among themselves and the dust never
    // skews the vote.
    let dst_vwap = scalar_f64(&admin, &f("vwap_24h", 17)).await;
    assert!(
        (dst_vwap - 1.003_333_33).abs() < 1e-6,
        "vwap must weight only the above-threshold venues ((1.00*100000 + \
         1.02*20000) / 120000 = 1.00333333), got {dst_vwap}"
    );
    let dst_srcs: String = admin
        .query(&s("sources", 17))
        .fetch_one()
        .await
        .expect("dst sources");
    assert!(
        dst_srcs.contains("sdex")
            && dst_srcs.contains("aquarius")
            && !dst_srcs.contains("soroswap"),
        "a below-threshold source must be ABSENT from sources (§3.3), got {dst_srcs}"
    );
    // The threshold is a WEIGHTING rule only: volume_24h_usd still counts the
    // dust, and price_usd is still the venue-blind newest priced close — which
    // here IS the dust venue's 5.00 (pinned-not-endorsed; the same asymmetry
    // asset 3's change columns pin for the outlier mask, owned by 0217).
    let dst_vol = scalar_f64(&admin, &f("volume_24h_usd", 17)).await;
    assert!(
        (dst_vol - 120_050.0).abs() < 1e-3,
        "volume_24h_usd is a total the threshold never touches (120050), got {dst_vol}"
    );
    let dst_p = scalar_f64(&admin, &f("price_usd", 17)).await;
    assert!(
        (dst_p - 5.0).abs() < 1e-9,
        "price_usd must be demonstrably unaffected by the threshold — the \
         newest priced close even when it comes from a below-threshold venue, \
         got {dst_p}"
    );

    // Asset 18 — the defect 0118 closes. Without the threshold the two dust
    // venues own the unweighted median (1.35), sdex deviates 26% > 20% and is
    // EVICTED, and the row publishes vwap ~1.3615 from $65 of turnover. The
    // threshold drops them before the vote, so the deep market prices itself.
    let atk_vwap = scalar_f64(&admin, &f("vwap_24h", 18)).await;
    assert!(
        (atk_vwap - 1.0).abs() < 1e-6,
        "dust venues must not be able to move vwap_24h: expect 1.00 from the \
         deep venue; ~1.3615 means they owned the median and evicted it — the \
         exact pre-0118 defect; got {atk_vwap}"
    );
    let atk_srcs: String = admin
        .query(&s("sources", 18))
        .fetch_one()
        .await
        .expect("atk sources");
    assert!(
        atk_srcs.contains("sdex")
            && !atk_srcs.contains("soroswap")
            && !atk_srcs.contains("phoenix"),
        "only the deep venue may remain in sources, got {atk_srcs}"
    );

    // Asset 19 — ordering: threshold BEFORE the asset_has_live window. The
    // live venue is dust, so after the threshold the asset is all-quiet and
    // the stale-but-real sdex must be KEPT. If the window ran first, dust
    // soroswap would arm the guard, evict sdex, then vanish itself.
    let thl_srcs: String = admin
        .query(&s("sources", 19))
        .fetch_one()
        .await
        .expect("thl sources");
    assert!(
        thl_srcs.contains("sdex") && !thl_srcs.contains("soroswap"),
        "with the only live venue below the threshold the asset counts as \
         all-quiet and the stale venue survives; sources = {{}} means the \
         liveness window ran before the threshold, got {thl_srcs}"
    );
    let thl_vwap = scalar_f64(&admin, &f("vwap_24h", 19)).await;
    assert!(
        (thl_vwap - 2.0).abs() < 1e-6,
        "vwap must come from the kept stale venue (2.00), got {thl_vwap}"
    );

    // Asset 20 — the threshold's CONDITIONAL arm: with no funded venue on the
    // asset there is nothing to defend, so the $50 venue must be KEPT — its
    // price is all we hold. An empty sources object here means the threshold
    // went unconditional and just blanked 96.5% of prod (measured 2026-08-27).
    let sub_vwap = scalar_f64(&admin, &f("vwap_24h", 20)).await;
    let sub_srcs: String = admin
        .query(&s("sources", 20))
        .fetch_one()
        .await
        .expect("sub sources");
    let sub_p = scalar_f64(&admin, &f("price_usd", 20)).await;
    let sub_vol = scalar_f64(&admin, &f("volume_24h_usd", 20)).await;
    assert!(
        (sub_vwap - 3.0).abs() < 1e-6 && sub_srcs.contains("sdex"),
        "an all-below-threshold asset must keep its sources (conditional arm), \
         got vwap {sub_vwap} sources {sub_srcs}"
    );
    assert!(
        (sub_p - 3.0).abs() < 1e-9 && (sub_vol - 50.0).abs() < 1e-9,
        "price_usd (3.00) and volume_24h_usd (50) must be untouched by the \
         threshold, got {sub_p} / {sub_vol}"
    );

    // ── the change_7d numerator guard, on the only shape that reaches it ───
    // Asset 6 has no priced candle at all (price_usd = 0) but DOES have a real
    // 1h baseline of 1.00 — ref_7d reads a different table, so the pairing is
    // reachable. Without the numerator nullIf this publishes a fabricated
    // -100%, which is what 0138 measured on 396 prod assets.
    let exo_7d = scalar_f64(&admin, &f("change_7d_pct", 6)).await;
    assert!(
        exo_7d.abs() < 1e-9,
        "an unpriced asset with a REAL 7d baseline must land on the 0 = 'no \
         signal' sentinel, NOT -100, got {exo_7d}"
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

    // ── ref_7d band, recent edge: a too-new baseline is NOT a 7d baseline ──
    // Asset 13 is priced (2.00) and has a 1h close of 1.00 four days ago. Before
    // the band's upper cutoff that close was the baseline and the row published
    // +100% as a SEVEN-day move. The cutoff makes it the sentinel instead.
    let nrb_p = scalar_f64(&admin, &f("price_usd", 13)).await;
    assert!(
        (nrb_p - 2.0).abs() < 1e-9,
        "precondition: asset 13 must be priced, else the 7d assert is vacuous, got {nrb_p}"
    );
    let nrb_7d = scalar_f64(&admin, &f("change_7d_pct", 13)).await;
    assert!(
        nrb_7d.abs() < 1e-9,
        "a baseline newer than the [7d, 5d] band must yield the 0 sentinel, not \
         a 4-day move labelled 7-day (+100 here), got {nrb_7d}"
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

// ───────────────────────────────────────────────────────────────────────────
// Task 0178 — the quote-leg surface: USDC's row, both-leg volume, provenance.
// ───────────────────────────────────────────────────────────────────────────

const USDC_ISSUER: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
/// Stands in for the canonical Stellar USDT (GCQTGZQQ…TG6V) — the identity the
/// oracle prices at par while the token itself trades at a deep discount.
const USDT_ISSUER: &str = "GCQTGZQQTG6VZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZTG6V";

fn insert_asset(db: &str, id: u32, code: &str, issuer: &str) -> String {
    format!(
        "INSERT INTO {db}.assets \
         (asset_id, asset_code, asset_type, issuer_address, contract_address, \
          sac_address, home_domain, is_active, created_at, updated_at) \
         VALUES ({id}, '{code}', 'classic', '{issuer}', '', '', '', 1, now(), now())"
    )
}

/// A candle for `base` quoted in `quote`, carrying `vol_usd` of USD value.
fn insert_pair(db: &str, base: u32, quote: u32, close_usd: &str, vol_usd: &str) -> String {
    format!(
        "INSERT INTO {db}.price_ohlcv_1m \
         (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
          volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
         VALUES (now() - INTERVAL 5 MINUTE, {base}, {quote}, 'sdex', {close_usd}, {close_usd}, \
          {close_usd}, {close_usd}, 50, {vol_usd}, {vol_usd}, {close_usd}, {close_usd}, 1, 1)"
    )
}

fn insert_rate(db: &str, code: &str, issuer: &str, rate: &str) -> String {
    format!(
        "INSERT INTO {db}.usd_rate \
         (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
          usd_rate, method, reference_asset, hops, version) \
         VALUES ('credit', '{code}', '{issuer}', '', now() - INTERVAL 5 MINUTE, \
          {rate}, 'oracle', '', 0, 1)"
    )
}

/// SYSTEM REFRESH is asynchronous, so every caller must WAIT for the write to
/// land. Polling on a row COUNT rather than sleeping keeps the test honest on a
/// slow machine and fast on a quick one.
async fn refresh(admin: &Client, db: &str, expect_rows: u64) {
    admin
        .query(&format!("SYSTEM REFRESH VIEW {db}.mv_current_prices"))
        .execute()
        .await
        .expect("refresh view");
    for _ in 0..30 {
        let n: u64 = admin
            .query(&format!("SELECT count() FROM {db}.current_prices FINAL"))
            .fetch_one()
            .await
            .expect("count current_prices");
        if n >= expect_rows {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    panic!("MV did not write {expect_rows} row(s) to {db}.current_prices in time");
}

async fn method_of(admin: &Client, db: &str, asset: u32) -> String {
    admin
        .query(&format!(
            "SELECT method FROM {db}.current_prices FINAL WHERE asset_id = {asset}"
        ))
        .fetch_one::<String>()
        .await
        .unwrap_or_else(|e| panic!("method for {asset}: {e}"))
}

/// The headline defect: canonical USDC never trades as a BASE leg, so every
/// base-keyed aggregate skipped it and `/price` returned 404. It must now
/// publish a row, priced from the measured rate rather than a $1 placeholder,
/// and tagged so a consumer can tell the two apart.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn usdc_publishes_a_row_from_the_measured_rate_and_is_tagged_oracle() {
    let db = "it_current_mv_0178_usdc";
    let admin = setup(db).await;

    for q in [
        insert_asset(db, 1, "XLM", ""),
        insert_asset(db, 2, "USDC", USDC_ISSUER),
        insert_asset(db, 4, "TKN", "GTKN"),
        // USDC appears ONLY as a quote leg, exactly as on prod.
        insert_pair(db, 1, 2, "0.5", "10000"),
        insert_pair(db, 4, 2, "2.0", "2000"),
        insert_rate(db, "USDC", USDC_ISSUER, "0.9993"),
    ] {
        admin.query(&q).execute().await.expect("fixture");
    }
    refresh(&admin, db, 3).await;

    let w = format!("FROM {db}.current_prices FINAL WHERE asset_id = 2");
    let price = scalar_f64(&admin, &format!("SELECT toFloat64(price_usd) {w}")).await;
    assert!(
        (price - 0.9993).abs() < 1e-9,
        "USDC must carry the MEASURED rate, not a $1 placeholder — got {price}"
    );
    assert_eq!(method_of(&admin, db, 2).await, "oracle");

    // Volume counts the quote leg: 10,000 + 2,000. Base-only summed an empty
    // set and published 0, which is the bug.
    let vol = scalar_f64(&admin, &format!("SELECT toFloat64(volume_24h_usd) {w}")).await;
    assert!(
        (vol - 12_000.0).abs() < 1e-6,
        "USDC volume must sum its quote-leg trades — got {vol}"
    );

    // Derived columns land on documented sentinels rather than fabrications:
    // there is no 24h baseline for an asset that does not trade as a base.
    let chg = scalar_f64(&admin, &format!("SELECT toFloat64(change_24h_pct) {w}")).await;
    assert_eq!(chg, 0.0, "no fabricated 24h change for a synthesised row");
    let vwap = scalar_f64(&admin, &format!("SELECT toFloat64(vwap_24h) {w}")).await;
    assert_eq!(
        vwap, 0.0,
        "vwap is a per-venue statistic; USDC has no venues"
    );
    let srcs: String = admin
        .query(&format!("SELECT sources {w}"))
        .fetch_one()
        .await
        .expect("sources");
    assert_eq!(srcs, "{}", "no venues → empty sources object, not an error");

    // price_xlm IS derivable and must be real: 0.9993 / 0.5.
    let pxlm = scalar_f64(&admin, &format!("SELECT toFloat64(price_xlm) {w}")).await;
    assert!(
        (pxlm - 1.9986).abs() < 1e-9,
        "price_xlm is a real quotient here — got {pxlm}"
    );

    teardown(db).await;
}

/// 🔴 The regression this task must never cause. Reflector prices the TICKER
/// "USDT" at par, and `usd_rate` files that under the Stellar issuer's address
/// — whose token really is worth ~$0.13 since it depegged in June 2022 (task
/// 0172, not a defect). Reading the oracle for that identity would republish a
/// ~7.4x error wearing the MORE authoritative label. The allowlist is USDC by
/// name; widening it is gated on task 0173.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn the_oracle_allowlist_is_usdc_only_and_never_repegs_stellar_usdt() {
    let db = "it_current_mv_0178_usdt";
    let admin = setup(db).await;

    for q in [
        insert_asset(db, 1, "XLM", ""),
        insert_asset(db, 3, "USDT", USDT_ISSUER),
        // USDT trades as a base at its real, depegged value.
        insert_pair(db, 3, 1, "0.13", "65"),
        // The trap: a par rate filed under the depegged issuer's identity.
        insert_rate(db, "USDT", USDT_ISSUER, "1.0"),
    ] {
        admin.query(&q).execute().await.expect("fixture");
    }
    refresh(&admin, db, 1).await;

    let w = format!("FROM {db}.current_prices FINAL WHERE asset_id = 3");
    let price = scalar_f64(&admin, &format!("SELECT toFloat64(price_usd) {w}")).await;
    assert!(
        (price - 0.13).abs() < 1e-9,
        "Stellar USDT must keep its MARKET value; the oracle's par reading for \
         the ticker must not reach it — got {price}"
    );
    assert_eq!(
        method_of(&admin, db, 3).await,
        "traded",
        "a market-priced asset is 'traded'; tagging it 'oracle' would assert \
         authority this price does not have"
    );

    teardown(db).await;
}

/// `method` must follow the ARM that produced the row, not the asset's
/// identity. Caught by this fixture during 0178's implementation: an earlier
/// revision keyed on `asset_id = usdc_asset_id`, so the moment USDC gained a
/// base candle it served a real TRADED close tagged 'oracle'.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn usdc_with_a_base_candle_is_traded_not_oracle_and_never_doubles() {
    let db = "it_current_mv_0178_arm";
    let admin = setup(db).await;

    for q in [
        insert_asset(db, 1, "XLM", ""),
        insert_asset(db, 2, "USDC", USDC_ISSUER),
        insert_pair(db, 1, 2, "0.5", "10000"),
        insert_rate(db, "USDC", USDC_ISSUER, "0.9993"),
        // USDC now ALSO trades as a base — the synthesised arm must stand down.
        insert_pair(db, 2, 1, "1.01", "50"),
    ] {
        admin.query(&q).execute().await.expect("fixture");
    }
    refresh(&admin, db, 2).await;

    let rows: u64 = admin
        .query(&format!(
            "SELECT count() FROM {db}.current_prices FINAL WHERE asset_id = 2"
        ))
        .fetch_one()
        .await
        .expect("count");
    assert_eq!(
        rows, 1,
        "the synthesised arm and the traded row must be mutually exclusive — \
         two rows would let the ReplacingMergeTree coin-flip between them"
    );

    let price = scalar_f64(
        &admin,
        &format!("SELECT toFloat64(price_usd) FROM {db}.current_prices FINAL WHERE asset_id = 2"),
    )
    .await;
    assert!(
        (price - 1.01).abs() < 1e-9,
        "a real traded close outranks the synthesised rate — got {price}"
    );
    assert_eq!(
        method_of(&admin, db, 2).await,
        "traded",
        "the label follows the producing arm; a traded price tagged 'oracle' \
         is worse than either alone"
    );

    teardown(db).await;
}

/// Both legs, one rule for every asset — and the per-venue weighting stays
/// base-only. Asserted together so a later edit cannot quietly re-base one
/// without the other.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn volume_counts_both_legs_while_per_source_weighting_stays_base_only() {
    let db = "it_current_mv_0178_vol";
    let admin = setup(db).await;

    for q in [
        insert_asset(db, 1, "XLM", ""),
        insert_asset(db, 2, "USDC", USDC_ISSUER),
        insert_asset(db, 3, "USDT", USDT_ISSUER),
        // XLM as a BASE: $10,000.
        insert_pair(db, 1, 2, "0.5", "10000"),
        // XLM as a QUOTE: $65 more.
        insert_pair(db, 3, 1, "0.13", "65"),
    ] {
        admin.query(&q).execute().await.expect("fixture");
    }
    refresh(&admin, db, 2).await;

    let w = format!("FROM {db}.current_prices FINAL WHERE asset_id = 1");
    let vol = scalar_f64(&admin, &format!("SELECT toFloat64(volume_24h_usd) {w}")).await;
    assert!(
        (vol - 10_065.0).abs() < 1e-6,
        "volume_24h_usd counts BOTH legs (10,000 base + 65 quote) — got {vol}"
    );

    // The per-venue figure inside `sources` feeds the §5.5 threshold and the
    // VWAP weighting, where the base leg is the right unit. It must NOT move.
    let srcs: String = admin
        .query(&format!("SELECT sources {w}"))
        .fetch_one()
        .await
        .expect("sources");
    assert!(
        srcs.contains("\"volume_24h\":\"10000\""),
        "per-source volume must stay BASE-only — got {srcs}"
    );

    teardown(db).await;
}

/// An asset whose candles are all un-enriched has `price_usd = 0` and no
/// method that applies. It must carry the '' sentinel, not 'traded' — labelling
/// a missing price as a real aggregate is the ambiguity this column removes.
#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn an_unpriced_asset_carries_the_empty_sentinel_not_traded() {
    let db = "it_current_mv_0178_sentinel";
    let admin = setup(db).await;

    for q in [
        insert_asset(db, 1, "XLM", ""),
        insert_asset(db, 5, "RAW", "GRAW"),
        // close_usd = 0: ingested but never enriched.
        insert_pair(db, 5, 1, "0", "0"),
    ] {
        admin.query(&q).execute().await.expect("fixture");
    }
    refresh(&admin, db, 1).await;

    let price = scalar_f64(
        &admin,
        &format!("SELECT toFloat64(price_usd) FROM {db}.current_prices FINAL WHERE asset_id = 5"),
    )
    .await;
    assert_eq!(price, 0.0, "no priced candle → the 0 sentinel");
    assert_eq!(
        method_of(&admin, db, 5).await,
        "",
        "no method applies to a missing price"
    );

    teardown(db).await;
}
