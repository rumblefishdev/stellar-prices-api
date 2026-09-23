//! Live-ClickHouse integration test for the read-surface views
//! (`price_usd_series`, `usd_reference`). Gated `#[ignore]`:
//!
//!   tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!   cargo test -p prices-clickhouse --test views_it -- --ignored --test-threads=1
//!
//! Owns an isolated scratch database (the `prices.*` schema + views rewritten
//! onto the scratch name) and drops it at the end.

use clickhouse::Client;
use prices_clickhouse::{USDC_ISSUER, USDT_ISSUER};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Retarget embedded schema SQL onto a scratch database.
///
/// The second replace looks like it exists for the views, but it does not, and
/// task 0134 did NOT make it dead (as that task's plan assumed). The first
/// replace already catches every `prices.<object>` reference — including the
/// view names, which are all qualified — so the only thing reaching the second
/// replace is `init.sql`'s unqualified `CREATE DATABASE IF NOT EXISTS prices`.
/// It stays load-bearing: drop it and `setup_scratch` creates the real `prices`
/// database instead of the scratch one.
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

async fn view_columns(client: &Client, db: &str, view: &str) -> Vec<String> {
    client
        .query(
            "SELECT name FROM system.columns \
             WHERE database = ? AND table = ? ORDER BY position",
        )
        .bind(db)
        .bind(view)
        .fetch_all::<String>()
        .await
        .unwrap()
}

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
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::SEED_SQL, db))
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::VIEWS_SQL, db))
        .await
        .unwrap();
    client
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn views_expose_usd_series_and_reference() {
    let db = "it_views_series";
    let client = setup_scratch(db).await;

    // 1=XLM native (with a SAC), 2=USDC, 10=FOO credit, 20=EXO quote,
    // 30=soroban contract token. Token 30 deliberately carries a non-empty
    // asset_code ('CTK') — discovery/metadata could populate a symbol — to prove
    // the views normalize a 'contract' kind to asset_code='' (review #6), not just
    // rely on the writer leaving it blank.
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (1,'XLM','classic','','','CXLMSAC'), (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','',''), (20,'EXO','classic','GEXO','',''), \
             (30,'CTK','soroban','','CTOKEN7XYZ','')"
        ))
        .execute()
        .await
        .unwrap();
    // Day-bucket candles with close_usd already baked. FOO = $5 from both its
    // USDC and XLM legs; XLM = $0.30; CTOKEN = $2; FOO/EXO leg unpriced (0).
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (1620000000, 1, 2,'sdex',    0.30,0.30,0.30,0.30, 1000,300,300,0.30,0.30,1,1), \
             (1620000000,10, 2,'sdex',    5,5,5,5,             10, 50, 500, 5,   5,   1,1), \
             (1620000000,10, 1,'phoenix', 16.6667,16.6667,16.6667,16.6667, 5,83,250,5,16.6667,1,1), \
             (1620000000,30, 2,'soroswap',2,2,2,2,             3,  6,  600,  2,   2,   1,1), \
             (1620000000,10,20,'sdex',    9,9,9,9,             1,  9,  0,  0,   9,   1,1)"
        ))
        .execute()
        .await
        .unwrap();

    let series_close = |kind: &'static str, code: &'static str| {
        let client = client.clone();
        let db = db.to_string();
        async move {
            client
                .query(&format!(
                    "SELECT toFloat64(close_usd) FROM {db}.price_usd_series \
                     WHERE asset_kind = ? AND asset_code = ?"
                ))
                .bind(kind)
                .bind(code)
                .fetch_one::<f64>()
                .await
                .unwrap()
        }
    };
    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;

    // Natural-identity keying + volume-weighted cross-quote collapse.
    assert!(
        approx(series_close("native", "XLM").await, 0.30),
        "native XLM"
    );
    assert!(
        approx(series_close("credit", "FOO").await, 5.0),
        "credit FOO (weighted)"
    );
    assert!(
        approx(series_close("contract", "").await, 2.0),
        "soroban token (asset_code normalized to '')"
    );

    // Review #6: the stored 'CTK' symbol must NOT leak through — the contract row
    // is keyed by contract_address with asset_code/issuer_address forced to ''.
    let leaked: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_usd_series \
             WHERE asset_kind = 'contract' AND (asset_code != '' OR issuer_address != '')"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(
        leaked, 0,
        "contract kind must blank asset_code/issuer_address"
    );

    // EXO only appears as an unpriced quote leg → not a priced row. It is NOT a
    // peg asset, so the 0165 arm does not rescue it: the peg-fill arm is keyed
    // to the two canonical peg identities, not to "any quote leg".
    let non_peg_quote: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_usd_series WHERE asset_code = 'EXO'"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(non_peg_quote, 0, "a non-peg quote leg gets no row");

    // ⚠️ Was `== 3` before task 0165. USDC (asset 2) is the quote on three of
    // these candles and the base of none, so it previously had NO row at all —
    // that is the whole defect. It now gets a 'peg' row, hence 4. The change is
    // intentional; a regression to 3 means the peg-fill arm stopped firing.
    let priced_assets: u64 = client
        .query(&format!("SELECT count() FROM {db}.price_usd_series"))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(
        priced_assets, 4,
        "XLM, FOO, CTOKEN traded + USDC via the peg arm"
    );

    // The peg row is keyed on the canonical USDC identity and flagged 'peg'.
    let (usdc_close, usdc_method): (f64, String) = client
        .query(&format!(
            "SELECT toFloat64(close_usd), method FROM {db}.price_usd_series \
             WHERE asset_kind = 'credit' AND asset_code = 'USDC' AND issuer_address = ?"
        ))
        .bind(USDC_ISSUER)
        .fetch_one::<(f64, String)>()
        .await
        .unwrap();
    assert!(approx(usdc_close, 1.0), "USDC falls back to the peg value");
    assert_eq!(usdc_method, "peg", "USDC row is flagged as a fallback");

    // Every genuinely traded row keeps method='traded' — the provenance column
    // must not relabel existing rows.
    let mislabelled: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_usd_series \
             WHERE asset_code != 'USDC' AND method != 'traded'"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(mislabelled, 0, "traded rows must be labelled 'traded'");

    // usd_reference: one bucket, xlm_usd = 0.30 (XLM/USDC volume-weighted close).
    let xlm_usd: f64 = client
        .query(&format!(
            "SELECT toFloat64(xlm_usd) FROM {db}.usd_reference WHERE bucket = toDateTime(1620000000)"
        ))
        .fetch_one::<f64>()
        .await
        .unwrap();
    assert!(approx(xlm_usd, 0.30), "usd_reference xlm_usd");

    // Hourly-grain variants: same shape on price_ohlcv_1h. Two hourly XLM/USDC
    // candles (different prices) must surface as two distinct hourly buckets.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (1620003600, 1, 2,'sdex', 0.31,0.31,0.31,0.31, 100,31,310,0.31,0.31,1,1), \
             (1620007200, 1, 2,'sdex', 0.32,0.32,0.32,0.32, 100,32,320,0.32,0.32,1,1)"
        ))
        .execute()
        .await
        .unwrap();
    let hourly_xlm: Vec<f64> = client
        .query(&format!(
            "SELECT toFloat64(close_usd) FROM {db}.price_usd_series_1h \
             WHERE asset_kind = 'native' ORDER BY bucket"
        ))
        .fetch_all::<f64>()
        .await
        .unwrap();
    assert_eq!(hourly_xlm.len(), 2, "two hourly native buckets");
    assert!(
        approx(hourly_xlm[0], 0.31) && approx(hourly_xlm[1], 0.32),
        "hourly XLM close_usd"
    );
    let hourly_ref: u64 = client
        .query(&format!("SELECT count() FROM {db}.usd_reference_1h"))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(hourly_ref, 2, "two hourly reference buckets");

    // identity_by_contract (SAC read-seam): XLM has a SAC, the soroban token its
    // own contract; resolving each contract returns the right natural identity.
    let (kind, code): (String, String) = client
        .query(&format!(
            "SELECT asset_kind, asset_code FROM {db}.identity_by_contract WHERE contract = 'CXLMSAC'"
        ))
        .fetch_one::<(String, String)>()
        .await
        .unwrap();
    assert_eq!(
        (kind.as_str(), code.as_str()),
        ("native", "XLM"),
        "SAC resolves to native XLM"
    );
    let pure: String = client
        .query(&format!(
            "SELECT asset_kind FROM {db}.identity_by_contract WHERE contract = 'CTOKEN7XYZ'"
        ))
        .fetch_one::<String>()
        .await
        .unwrap();
    assert_eq!(pure, "contract", "pure soroban token maps to itself");

    // current_price_usd (live spot): one row per asset, natural-identity keyed.
    // Include the contract token (30) to confirm the same #6 normalization here.
    // Asset 1 carries every task-0072 column with a DISTINCT value, so a view
    // that mixed up two forwarded columns cannot pass; asset 30 stays on the
    // table DEFAULTs, standing in for an asset the MV has no breakdown for.
    client
        .query(&format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct, \
              volume_24h_usd, market_cap_usd, vwap_24h, sources, updated_at) VALUES \
             (1, 0.1600, 1.0000, -3.2500, 7.7500, 125000.0000, 4500000.0000, 0.1580, \
              '{{\"sdex\":{{\"price\":\"0.16\",\"volume_24h\":\"125000\"}}}}', \
              toDateTime(1620100000)), \
             (30, 2.5000, 0, 0, 0, 0, 0, 0, '', toDateTime(1620100000))"
        ))
        .execute()
        .await
        .unwrap();
    let spot: f64 = client
        .query(&format!(
            "SELECT toFloat64(price_usd) FROM {db}.current_price_usd WHERE asset_kind = 'native'"
        ))
        .fetch_one::<f64>()
        .await
        .unwrap();
    assert!(approx(spot, 0.16), "live spot XLM price");
    let (ckind, ccode): (String, String) = client
        .query(&format!(
            "SELECT asset_kind, asset_code FROM {db}.current_price_usd WHERE contract_address = 'CTOKEN7XYZ'"
        ))
        .fetch_one::<(String, String)>()
        .await
        .unwrap();
    assert_eq!(
        (ckind.as_str(), ccode.as_str()),
        ("contract", ""),
        "current_price_usd blanks the contract token's asset_code"
    );

    // Task 0072 — the view forwards the rest of current_prices. BE reads this
    // surface in-cluster, so a column the view drops is unreachable to them
    // however well the MV writes it. Distinct seeded values catch a swap.
    let (xlm, ch24, ch7d, vol, mcap, vwap, sources): (f64, f64, f64, f64, f64, f64, String) =
        client
            .query(&format!(
                "SELECT toFloat64(price_xlm), toFloat64(change_24h_pct), \
                    toFloat64(change_7d_pct), toFloat64(volume_24h_usd), \
                    toFloat64(market_cap_usd), toFloat64(vwap_24h), sources \
             FROM {db}.current_price_usd WHERE asset_kind = 'native'"
            ))
            .fetch_one()
            .await
            .unwrap();
    assert!(approx(xlm, 1.0), "price_xlm forwarded, got {xlm}");
    assert!(approx(ch24, -3.25), "change_24h_pct forwarded, got {ch24}");
    assert!(approx(ch7d, 7.75), "change_7d_pct forwarded, got {ch7d}");
    assert!(approx(vol, 125000.0), "volume_24h_usd forwarded, got {vol}");
    assert!(
        approx(mcap, 4500000.0),
        "market_cap_usd forwarded, got {mcap}"
    );
    assert!(approx(vwap, 0.158), "vwap_24h forwarded, got {vwap}");
    assert_eq!(
        sources, r#"{"sdex":{"price":"0.16","volume_24h":"125000"}}"#,
        "sources JSON forwarded verbatim — the view must not re-serialise it"
    );

    // An asset the MV has no breakdown for still reads cleanly: the columns are
    // the table's DEFAULT sentinels, never an error or a dropped row.
    let (dxlm, dsources): (f64, String) = client
        .query(&format!(
            "SELECT toFloat64(price_xlm), sources FROM {db}.current_price_usd \
             WHERE contract_address = 'CTOKEN7XYZ'"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert!(approx(dxlm, 0.0), "unpopulated price_xlm reads as 0");
    assert_eq!(
        dsources, "",
        "unpopulated sources reads as the empty string"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0072 — `views.sql` must REPLACE an existing `current_price_usd`, not
/// silently skip it.
///
/// `setup_scratch` always builds on a freshly-created database, so every other
/// assertion in this file lands on a target with no pre-existing view and would
/// pass identically under the old `CREATE VIEW IF NOT EXISTS` form. This test
/// seeds the v1 six-column shape first and re-applies, which is the actual
/// production upgrade path — ch-prod-01 already holds the v1 view.
///
/// The `IF NOT EXISTS` half is the control: it pins *why* the statement had to
/// change, so an edit back to that form fails here rather than as a silent
/// no-op against prod. Without it the assertion below only proves that applying
/// `views.sql` twice is harmless.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn views_sql_replaces_an_existing_v1_current_price_usd() {
    let db = "it_views_replace";
    let client = setup_scratch(db).await;

    let columns = || {
        let client = client.clone();
        let db = db.to_string();
        async move {
            client
                .query(
                    "SELECT name FROM system.columns \
                     WHERE database = ? AND table = 'current_price_usd' ORDER BY position",
                )
                .bind(db)
                .fetch_all::<String>()
                .await
                .unwrap()
        }
    };

    // Rewind to v1: the six columns current_price_usd shipped with (task 0039).
    client
        .query(&format!("DROP VIEW IF EXISTS {db}.current_price_usd"))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "CREATE VIEW {db}.current_price_usd AS SELECT \
             multiIf(a.contract_address != '', 'contract', \
                     a.asset_code = 'XLM' AND a.issuer_address = '', 'native', \
                     'credit') AS asset_kind, \
             if(a.contract_address != '', '', a.asset_code)     AS asset_code, \
             if(a.contract_address != '', '', a.issuer_address) AS issuer_address, \
             a.contract_address AS contract_address, \
             c.price_usd        AS price_usd, \
             c.updated_at       AS updated_at \
             FROM {db}.current_prices AS c FINAL \
             INNER JOIN {db}.assets AS a FINAL ON a.asset_id = c.asset_id"
        ))
        .execute()
        .await
        .unwrap();
    assert_eq!(columns().await.len(), 6, "seeded the v1 shape");

    // Control — the OLD statement form leaves the v1 view standing. This is the
    // silent no-op that would have shipped a green deploy with none of the
    // task-0072 columns actually reachable.
    let as_if_not_exists = rewrite(prices_clickhouse::VIEWS_SQL, db)
        .replace("CREATE OR REPLACE VIEW", "CREATE VIEW IF NOT EXISTS");
    assert!(
        as_if_not_exists.contains("CREATE VIEW IF NOT EXISTS")
            && !as_if_not_exists.contains("CREATE OR REPLACE VIEW"),
        "control rewrite must actually swap the statement form"
    );
    prices_clickhouse::apply_sql(&client, &as_if_not_exists)
        .await
        .unwrap();
    assert_eq!(
        columns().await.len(),
        6,
        "CREATE VIEW IF NOT EXISTS must NOT redefine an existing view — if this \
         reports 16, the OR REPLACE form is no longer load-bearing and the \
         comment in views.sql is wrong"
    );

    // The shipped form replaces it in place.
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::VIEWS_SQL, db))
        .await
        .unwrap();
    let after = columns().await;
    // 16 since task 0216 appended `as_of` and `price_status`; 14 since 0178
    // appended `method`; 13 before it, 6 in v1.
    assert_eq!(
        after.len(),
        16,
        "views.sql must replace the v1 view, got {after:?}"
    );
    for col in [
        "price_xlm",
        "change_24h_pct",
        "change_7d_pct",
        "volume_24h_usd",
        "market_cap_usd",
        "vwap_24h",
        "sources",
        "method",
    ] {
        assert!(
            after.contains(&col.to_string()),
            "{col} missing after replace"
        );
    }
    // Appended, not inserted — the six v1 columns keep their positions, so an
    // ordinal-based consumer of the original shape still reads the same fields.
    assert_eq!(
        &after[..6],
        &[
            "asset_kind".to_string(),
            "asset_code".to_string(),
            "issuer_address".to_string(),
            "contract_address".to_string(),
            "price_usd".to_string(),
            "updated_at".to_string(),
        ],
        "the v1 columns must keep positions 1-6"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// `backfill_progress` is seeded with exactly the two canonical streams, and
/// re-running the seed is a no-op that does not reset live progress (task 0051
/// Step 1). `setup_scratch` already applies `SEED_SQL` once.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn backfill_progress_seed_is_idempotent() {
    let db = "it_backfill_seed";
    let client = setup_scratch(db).await;

    // Exactly the two canonical streams after the initial apply.
    let names: Vec<String> = client
        .query(&format!(
            "SELECT DISTINCT task_name FROM {db}.backfill_progress ORDER BY task_name"
        ))
        .fetch_all::<String>()
        .await
        .unwrap();
    assert_eq!(
        names,
        vec!["sdex_archive".to_string(), "soroban_amm".to_string()],
        "seed creates exactly the two canonical streams"
    );

    // Advance a stream, then re-run the seed. The explicit far-future updated_at
    // guarantees this row wins the ReplacingMergeTree(updated_at) merge over the
    // seed row regardless of wall-clock timing.
    client
        .query(&format!(
            "INSERT INTO {db}.backfill_progress \
             (task_name, start_ledger, target_ledger, current_ledger, status, updated_at) VALUES \
             ('sdex_archive', 0, 1000, 500, 'running', toDateTime(4000000000))"
        ))
        .execute()
        .await
        .unwrap();
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::SEED_SQL, db))
        .await
        .unwrap();

    // Still exactly two distinct streams — the re-run inserted nothing.
    let distinct: u64 = client
        .query(&format!(
            "SELECT uniqExact(task_name) FROM {db}.backfill_progress"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(distinct, 2, "re-running the seed adds no new streams");

    // Progress is preserved — the seed did not clobber current_ledger back to 0.
    let current: u64 = client
        .query(&format!(
            "SELECT current_ledger FROM {db}.backfill_progress FINAL WHERE task_name = 'sdex_archive'"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(current, 500, "re-running the seed preserves live progress");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0134 — EVERY view in `views.sql` must replace an existing definition,
/// not just `current_price_usd` (which 0072 converted).
///
/// `setup_scratch` builds on a freshly-created database, so every other
/// assertion in this file lands on a target with no pre-existing view and would
/// pass identically under the old `CREATE VIEW IF NOT EXISTS` form. This test
/// rewinds every view to a one-column stub first and re-applies, which is the
/// actual production upgrade path — ch-prod-01 already holds the six that
/// predate task 0147.
///
/// The `IF NOT EXISTS` half is the control: it pins *why* the statement form is
/// load-bearing, so a revert to that form fails here rather than as a silent
/// no-op against prod.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn views_sql_replaces_every_existing_view() {
    let db = "it_views_replace_all";
    let client = setup_scratch(db).await;

    // (view, a column that only the REAL definition has)
    let views = [
        ("usd_reference", "xlm_usd"),
        ("price_usd_series", "close_usd"),
        ("usd_reference_1h", "xlm_usd"),
        ("price_usd_series_1h", "close_usd"),
        ("price_usd_series_coverage", "status"),
        ("price_usd_series_coverage_1h", "status"),
        ("identity_by_contract", "contract"),
        ("current_price_usd", "vwap_24h"),
    ];

    // Rewind every view to a stub shape that shares no column with the real one.
    for (v, _) in views {
        client
            .query(&format!("DROP VIEW IF EXISTS {db}.{v}"))
            .execute()
            .await
            .unwrap();
        client
            .query(&format!(
                "CREATE VIEW {db}.{v} AS SELECT 1 AS stub_sentinel"
            ))
            .execute()
            .await
            .unwrap();
        assert_eq!(
            view_columns(&client, db, v).await,
            vec!["stub_sentinel".to_string()],
            "seeded the stub shape for {v}"
        );
    }

    // Control — the OLD statement form leaves every stub standing. This is the
    // silent no-op the task exists to remove: a green apply that changes nothing.
    let as_if_not_exists = rewrite(prices_clickhouse::VIEWS_SQL, db)
        .replace("CREATE OR REPLACE VIEW", "CREATE VIEW IF NOT EXISTS");
    assert!(
        as_if_not_exists.contains("CREATE VIEW IF NOT EXISTS")
            && !as_if_not_exists.contains("CREATE OR REPLACE VIEW"),
        "control rewrite must actually swap the statement form"
    );
    prices_clickhouse::apply_sql(&client, &as_if_not_exists)
        .await
        .unwrap();
    for (v, _) in views {
        assert_eq!(
            view_columns(&client, db, v).await,
            vec!["stub_sentinel".to_string()],
            "CREATE VIEW IF NOT EXISTS must NOT redefine the existing {v} — if \
             this fails, the OR REPLACE form is no longer load-bearing and the \
             views.sql header is wrong"
        );
    }

    // The shipped form replaces every one of them in place.
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::VIEWS_SQL, db))
        .await
        .unwrap();
    for (v, real_col) in views {
        let cols = view_columns(&client, db, v).await;
        assert!(
            !cols.contains(&"stub_sentinel".to_string()),
            "views.sql must replace the stub {v}, got {cols:?}"
        );
        assert!(
            cols.contains(&real_col.to_string()),
            "{v} must expose `{real_col}` after the apply, got {cols:?}"
        );
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0165 — the peg-fill arm of `price_usd_series*`.
///
/// Three cases, each of which a plausible-but-wrong implementation fails:
///
///   1. A peg asset seeded **quote-only** returns the fallback. Fails before
///      0165 — the asset has no row at all, which is the reported defect.
///   2. A peg asset seeded as **both** base and quote returns its MARKET value,
///      not the fallback. This is the USDT-flattening regression: an
///      implementation that lets the peg arm own the peg identities would report
///      $1 here and silently destroy 102 genuinely priceable pools on prod.
///   3. A non-peg asset is **unchanged** — same close_usd, and `method` does not
///      relabel it.
///
/// ⚠️ The assertions are written against FALLBACK SEMANTICS, not the literal 1,
/// and task 0168 has now shipped — so this fixture seeds NO `usd_rate` rows and
/// exercises exactly the no-measured-rate half. That is why it still passes
/// unchanged: `PEG_FALLBACK` is what the view substitutes when no observation
/// exists, which is still `$1` (and still `method = 'peg'`). The rate-available
/// half lives in
/// `peg_fill_publishes_the_measured_rate_and_falls_back_only_without_one`.
/// A test asserting "peg asset → exactly 1.0" would have had to be rewritten
/// here instead of surviving 0168 untouched.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn price_usd_series_fills_peg_assets_without_overriding_market_data() {
    let db = "it_views_peg_fill";
    let client = setup_scratch(db).await;

    /// The value the view substitutes when NO measured rate is available.
    /// Since task 0168 a bucket WITH an observation publishes that instead;
    /// this fixture deliberately seeds none.
    const PEG_FALLBACK: f64 = 1.0;

    // 2 = USDC (top-preference quote → never a base, the defect).
    // 3 = USDT at its canonical issuer (a peg asset that DOES trade as a base —
    //     the control that catches the flattening regression).
    // 10 = FOO, an ordinary credit asset; 1 = native XLM.
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (1,'XLM','classic','','',''), \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (3,'USDT','classic','{USDT_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // One bucket. FOO/USDC and XLM/USDC price normally (USDC is quote-only).
    // USDT trades as a BASE against USDC at 0.97 — a deliberate de-peg, so a
    // fallback leaking into case 2 is unmistakable — and also appears as a quote
    // leg (FOO/USDT), which is what makes case 2 non-trivial: USDT gets BOTH a
    // real arm-A row and a zero-weight arm-B placeholder in the same bucket.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (1620000000,10, 2,'sdex', 5,5,5,5,             10, 50, 500, 5,    5,    1,1), \
             (1620000000, 1, 2,'sdex', 0.30,0.30,0.30,0.30, 1000,300,300,0.30,0.30, 1,1), \
             (1620000000, 3, 2,'sdex', 0.97,0.97,0.97,0.97, 100, 97, 970, 0.97, 0.97, 1,1), \
             (1620000000,10, 3,'sdex', 5.15,5.15,5.15,5.15, 4,  20.6,206,5,  5.15, 1,1)"
        ))
        .execute()
        .await
        .unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-4;
    let row = |view: &'static str, code: &'static str, issuer: String| {
        let client = client.clone();
        let db = db.to_string();
        async move {
            client
                .query(&format!(
                    "SELECT toFloat64(close_usd), method FROM {db}.{view} \
                     WHERE asset_code = ? AND issuer_address = ?"
                ))
                .bind(code)
                .bind(issuer)
                .fetch_one::<(f64, String)>()
                .await
                .unwrap()
        }
    };

    // The 1h view reads price_ohlcv_1h. Seed it BEFORE the loop, not inside a
    // `if view == …` guard: it is a precondition of the 1h iteration, not part
    // of it, and an in-loop insert silently depends on the 1h grain being last
    // — reorder the array and `fetch_one` panics on an empty result instead of
    // failing an assertion. Column-list-free so it cannot drift if the table
    // gains a column; price_ohlcv_1h is `AS price_ohlcv_1m`, same shape.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h SELECT * FROM {db}.price_ohlcv_1d"
        ))
        .execute()
        .await
        .unwrap();

    for view in ["price_usd_series", "price_usd_series_1h"] {
        // CASE 1 — quote-only peg asset gets the fallback. Fails before 0165.
        let (usdc, usdc_method) = row(view, "USDC", USDC_ISSUER.to_string()).await;
        assert!(
            approx(usdc, PEG_FALLBACK),
            "{view}: quote-only USDC must fall back, got {usdc}"
        );
        assert_eq!(usdc_method, "peg", "{view}: USDC provenance");

        // CASE 2 — an asset that is BOTH a quote leg and a traded base keeps its
        // MARKET value. A fallback here would read 1.0 and flatten a real de-peg.
        //
        // ⚠️ WEAKENED by task 0172, deliberately. USDT used to be a peg member,
        // so this case pinned "arm B must not override arm A for a PEG asset".
        // USDT is no longer in the peg set, so what remains is only "a non-peg
        // asset is priced from its trades" — which CASE 3 already covers. The
        // peg-specific half moved to
        // `peg_member_that_also_trades_as_a_base_keeps_its_market_value`, which
        // uses USDC because it is now the sole peg member. Keep both: this one
        // still pins that a *former* peg member is not silently re-pegged.
        let (usdt, usdt_method) = row(view, "USDT", USDT_ISSUER.to_string()).await;
        assert!(
            approx(usdt, 0.97),
            "{view}: traded USDT must keep its market value (0.97), got {usdt} \
             — the zero-weight placeholder must not perturb the average"
        );
        assert_eq!(usdt_method, "traded", "{view}: USDT provenance");

        // CASE 3 — ordinary asset unchanged. FOO trades against both USDC (5.0,
        // vol 10) and USDT (5.0, vol 4); the volume-weighted collapse is 5.0 and
        // the arm-B rows for USDC/USDT contribute nothing to it.
        let (foo, foo_method) = row(view, "FOO", "GFOO".to_string()).await;
        assert!(
            approx(foo, 5.0),
            "{view}: non-peg asset must be unchanged, got {foo}"
        );
        assert_eq!(foo_method, "traded", "{view}: FOO provenance");
    }

    // The peg arm must not invent identities: only assets that actually appear
    // as a peg quote leg get a placeholder. Nothing is keyed on a non-peg quote.
    let rows: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_usd_series WHERE method = 'peg'"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(rows, 1, "exactly one peg-filled row (USDC); USDT traded");

    // ⚠️ Assert on the VALUE, not on `close_usd IS NULL`. That check was written
    // first and is VACUOUS: `close_usd` is a non-Nullable Decimal(38,14), so
    // `CAST` strips the Nullable that `nullIf` introduces and the count is
    // structurally always 0. A zero-denominator does not surface as NULL — it
    // lands as Decimal128::MIN (≈ -1.7e24). Caught in review; see the guard
    // note in views.sql.
    let garbage: u64 = client
        .query(&format!(
            "SELECT countIf(toFloat64(close_usd) <= 0) FROM {db}.price_usd_series"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(garbage, 0, "no row may publish a non-positive close_usd");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0172 review finding 6 — restores the guard CASE 2 above used to carry.
///
/// `views.sql` warns that letting the peg arm OWN the peg identities flattens a
/// genuinely priceable asset to $1 — "a regression dressed as a fix". USDT was
/// the control for that, and task 0172 removed it from the peg set, so the suite
/// was left with **no** peg member that also trades as a base and nothing would
/// have caught a reimplementation where arm B wins over arm A.
///
/// USDC is the sole remaining peg member, and on prod it never trades as a base
/// (task 0165: it is the top-preference quote, 0 candles). So this fixture is
/// deliberately synthetic — USDC quoted in XLM. That is the point: the guard has
/// to survive someone *adding* a peg member that does trade, which is live in
/// tasks 0173/0196, not hypothetical.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn peg_member_that_also_trades_as_a_base_keeps_its_market_value() {
    let db = "it_views_peg_member_trades";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (1,'XLM','classic','','',''), \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // FOO/USDC makes USDC a peg QUOTE leg, so arm B emits its zero-weight
    // placeholder. USDC/XLM then makes USDC a traded BASE at 1.04 — off par by
    // enough that a fallback leaking through is unmistakable. Both in one bucket,
    // so the two arms collide on the same (identity, bucket) key.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (1620000000,10, 2,'sdex', 5,5,5,5,             10, 50,  500,  5,    5,    1,1), \
             (1620000000, 2, 1,'sdex', 3.2,3.2,3.2,3.2,     50, 160, 520,  1.04, 3.2,  1,1)"
        ))
        .execute()
        .await
        .unwrap();

    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h SELECT * FROM {db}.price_ohlcv_1d"
        ))
        .execute()
        .await
        .unwrap();

    for view in ["price_usd_series", "price_usd_series_1h"] {
        let (usdc, method) = client
            .query(&format!(
                "SELECT toFloat64(close_usd), method FROM {db}.{view} \
                 WHERE asset_code = ? AND issuer_address = ?"
            ))
            .bind("USDC")
            .bind(USDC_ISSUER)
            .fetch_one::<(f64, String)>()
            .await
            .unwrap();

        assert!(
            (usdc - 1.04).abs() < 1e-4,
            "{view}: a peg member that trades as a base must publish its MARKET \
             value 1.04, got {usdc}. 1.0 means the peg arm overrode arm A and \
             every genuinely priceable pool on that identity is flattened to par."
        );
        assert_eq!(
            method, "traded",
            "{view}: measured data must be labelled traded, not peg"
        );
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0165 review finding 1 — the zero-volume peg case.
///
/// A peg asset whose ONLY priced candle carries `volume_base = 0` has a real
/// arm-A row that contributes nothing to the weighted average. The first
/// implementation guarded on `countIf(is_peg = 0) = 0` ("no traded rows at
/// all"), which is FALSE here, so it fell through to
/// `sum(v) / nullIf(sum(w), 0)`.
///
/// That does NOT yield NULL. `close_usd` is a non-Nullable `Decimal(38,14)`, so
/// `CAST` strips the Nullable and the row publishes **Decimal128::MIN**
/// (≈ -1.7e24) flagged `method = 'traded'` — a catastrophic value labelled as
/// measured, in the column BE multiplies into TVL.
///
/// The shipped guard is `sum(w) = 0`, which returns the fallback instead. This
/// test pins that: it fails with the `countIf` form and passes with `sum(w)`.
///
/// ⚠️ SCOPE — the guard only reaches this case because arm B emitted a
/// placeholder for USDC (it is the quote leg of the FOO/USDC candle below).
/// "Fixture A" — an asset appearing ONLY as a zero-volume base, with no
/// placeholder — cannot reach the fallback at all, because `max(is_peg)` is 0.
///
/// Fixture A was CLOSED by tasks 0171/0198: arm A now requires
/// `volume_base > 0`, so a zero-weight group never forms and such an asset is
/// absent (BE's "misses are absent" contract), see
/// `a_zero_volume_only_base_is_absent_and_its_neighbours_still_publish`.
/// A task 0172 note here once said fixture A RAISES code 349 rather than
/// publishing Decimal128::MIN. Both happen on 26.3.10.60 — interpreted it
/// raises, JIT-compiled it publishes the sentinel (see the 0171/0198 block
/// below) — and the fix is the same either way.
///
/// This test still pins the OTHER half — the 0165 guard for a peg asset that
/// trades as a zero-volume base beside its placeholder — which the 0171 filter
/// does not reach: with the zero-volume arm-A row gone, USDC's group is the
/// placeholder alone, sum(w) = 0, and the fallback must still fire.
///
/// ⚠️ This fixture used USDT until task 0172 removed USDT from the peg set
/// (it depegged in June 2022 and is now priced by measurement, not assumed to
/// be $1). USDC is now the only peg asset and the only valid subject here.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn peg_asset_with_only_zero_volume_candles_falls_back_instead_of_publishing_garbage() {
    let db = "it_views_peg_zero_vol";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // USDC trades as a base against FOO at 0.97 with ZERO volume (so its only
    // arm-A row contributes w = 0), AND is the quote leg of a FOO/USDC candle,
    // which is what makes arm B emit a placeholder for it. Both are required:
    // without the placeholder `max(is_peg)` is 0 for USDC's group and neither
    // guard can fire — see the fixture-A note on this test.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (1620000000, 2,10,'sdex', 0.97,0.97,0.97,0.97, 0,0,0,0.97,0.97,1,1), \
             (1620000000,10, 2,'sdex', 5,5,5,5,                7,35,350,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    let (close, method): (f64, String) = client
        .query(&format!(
            "SELECT toFloat64(close_usd), method FROM {db}.price_usd_series \
             WHERE asset_code = 'USDC'"
        ))
        .fetch_one::<(f64, String)>()
        .await
        .unwrap();

    assert!(
        close > 0.0,
        "zero-volume peg candle must not publish a negative/garbage close_usd, got {close}"
    );
    assert!(
        (close - 1.0).abs() < 1e-4,
        "expected the peg fallback, got {close}"
    );
    assert_eq!(
        method, "peg",
        "a row the weighted average could not compute must not claim 'traded'"
    );

    // Nothing anywhere in the view may publish a non-positive close_usd.
    let garbage: u64 = client
        .query(&format!(
            "SELECT countIf(toFloat64(close_usd) <= 0) FROM {db}.price_usd_series"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(garbage, 0, "no row may publish a non-positive close_usd");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0172 regression — USDT is NOT a peg asset and must never receive the
/// `$1` fallback.
///
/// The canonical Stellar USDT (`USDT_ISSUER`) depegged in June 2022 and has
/// traded at ~$0.13 ever since. Two markets sharing no legs and no code path
/// agree (its own USDC pair, and `XLM/USDC ÷ XLM/USDT`); four sibling
/// stablecoins held par through the same window in the same pipeline.
///
/// Before this fix, arm B emitted a zero-weight placeholder keyed on USDT as a
/// QUOTE leg, so in any bucket where USDT did not also trade as a base the view
/// published `close_usd = 1.0, method = 'peg'` — a ~7.4x overstatement, and the
/// source of the $0.14 ↔ $1.00 flapping BE reported.
///
/// This test pins BOTH halves of the change: USDT gets nothing, and USDC — the
/// only remaining peg asset, which genuinely cannot be priced as a base — still
/// gets its fallback. Asserting only the first half would pass just as well if
/// someone deleted arm B entirely, which would re-break task 0165.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn usdt_quote_only_gets_no_peg_fallback_but_usdc_still_does() {
    let db = "it_views_0172_usdt_not_pegged";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (3,'USDT','classic','{USDT_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // FOO trades against BOTH stablecoins. Neither USDC nor USDT trades as a
    // base, so each appears ONLY as a quote leg — the exact shape that used to
    // hand USDT a $1 row.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (1620000000,10, 2,'sdex', 5,5,5,5,             10,50,  500,  5,5,   1,1), \
             (1620000000,10, 3,'sdex', 5.15,5.15,5.15,5.15,  4,20.6,206,5,5.15,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h SELECT * FROM {db}.price_ohlcv_1d"
        ))
        .execute()
        .await
        .unwrap();

    for view in ["price_usd_series", "price_usd_series_1h"] {
        let usdt_rows: u64 = client
            .query(&format!(
                "SELECT count() FROM {db}.{view} \
                 WHERE asset_code = 'USDT' AND issuer_address = ?"
            ))
            .bind(USDT_ISSUER)
            .fetch_one::<u64>()
            .await
            .unwrap();
        assert_eq!(
            usdt_rows, 0,
            "{view}: quote-only USDT must publish NOTHING. A row here means the \
             peg placeholder is back and every USDT-quoted candle is ~7.4x high."
        );

        // Control: the mechanism itself must still work for the real peg asset.
        let (usdc, usdc_method): (f64, String) = client
            .query(&format!(
                "SELECT toFloat64(close_usd), method FROM {db}.{view} \
                 WHERE asset_code = 'USDC' AND issuer_address = ?"
            ))
            .bind(USDC_ISSUER)
            .fetch_one::<(f64, String)>()
            .await
            .unwrap();
        assert!(
            (usdc - 1.0).abs() < 1e-4,
            "{view}: USDC must still fall back to $1, got {usdc} — removing arm B \
             entirely would re-break task 0165"
        );
        assert_eq!(usdc_method, "peg", "{view}: USDC provenance");

        // FOO is priced from its own trades, across both quote legs, unaffected.
        let (foo, foo_method): (f64, String) = client
            .query(&format!(
                "SELECT toFloat64(close_usd), method FROM {db}.{view} \
                 WHERE asset_code = 'FOO'"
            ))
            .fetch_one::<(f64, String)>()
            .await
            .unwrap();
        assert!((foo - 5.0).abs() < 1e-4, "{view}: FOO unchanged, got {foo}");
        assert_eq!(foo_method, "traded", "{view}: FOO provenance");
    }

    let peg_rows: u64 = client
        .query(&format!(
            "SELECT count() FROM {db}.price_usd_series WHERE method = 'peg'"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(
        peg_rows, 1,
        "exactly one peg-filled row, and it must be USDC"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0168 — the peg placeholder publishes the MEASURED rate, not `$1`.
///
/// `price_usd_series*` used to emit a flat `1` for every peg-filled row. USDC
/// does not sit at exactly `$1` — measured on prod 2026-08-10 it was
/// `1.00066784838102`, stable to four decimals across five consecutive
/// 5-minute readings — so the constant was a ~0.07% systematic error on EVERY
/// published row, permanently, and it contradicted our own candles: the oracle
/// enrichment tier already prices a USDC-quoted candle off the same Reflector
/// feed, so the same bucket read 0.9993 there and 1.0000 here.
///
/// Five properties, each of which a plausible implementation gets wrong:
///
///   1. **Observation in the bucket → that rate, tagged `oracle`.** Asserted on
///      the exact decimal string, not a float epsilon: a `toFloat64` round-trip
///      would hide precision loss at the 14th place, which is the whole point of
///      the `Decimal(38, 14)` column.
///   2. **The bucket's LAST observation wins.** Two readings in the same day;
///      an implementation that averaged them (forbidden by task 0167 — averages
///      do not compose across the six grains) or took the first would differ.
///   3. **No observation → `$1`, tagged `peg`, and NO FORWARD-FILL.** The day
///      after the readings has none of its own and must fall back rather than
///      carry yesterday's rate forward. An unbounded `ASOF` would publish a dead
///      oracle's last reading across years of buckets.
///   4. **`method = 'pivot'` rows are ignored.** `usd_rate` keys on
///      (identity, timestamp, method) exactly so a task 0154 pivot row cannot
///      replace a measurement; this consumer chooses, and it chooses a
///      MEASUREMENT or nothing. ⚠️ Since task 0267 "a measurement" is two words,
///      not one: an IMPORTED reading (`method = 'external'`, an outside USD
///      series that actually observed the rate) is accepted here, while a
///      DERIVED `pivot`/`pivot2` rate — computed from another asset's price
///      rather than observed — still is not. Provenance is what separates them,
///      not authorship. The rule that oracle outranks an import in a bucket
///      holding both is
///      [`an_oracle_row_outranks_an_imported_row_in_the_same_bucket`]. The
///      fixture plants a wildly wrong pivot value at the end of the day — if it
///      leaked it would win case 2's "last observation" test.
///   5. **The grains compose where the oracle observed.** The daily close equals
///      the LAST hourly close of the same day (task 0167's stated reason for a
///      close rather than an average). Both are the same observation here, so it
///      holds by construction — and breaks the moment someone reaches for an
///      average. ⚠️ It holds only because this fixture's last reading (23:55)
///      sits INSIDE the day's last candle-bearing hour. When it does not, the
///      grains legitimately diverge; that case is
///      [`a_day_whose_last_candle_hour_holds_no_reading_diverges_between_grains`].
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn peg_fill_publishes_the_measured_rate_and_falls_back_only_without_one() {
    let db = "it_views_0168_measured_peg_rate";
    let client = setup_scratch(db).await;

    /// The last reading of 2026-08-10, and the value both grains must publish
    /// for that day. Prod's actual measurement for that date.
    const MEASURED: &str = "1.00066784838102";
    /// An earlier reading the same day — case 2's loser.
    const EARLIER: &str = "1.00050000000000";

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // FOO/USDC in three day buckets, so USDC is a quote-only peg leg in each and
    // arm B emits a placeholder for all three:
    //   2026-03-01 — BEFORE the oracle window (prod's first reading is
    //                2026-03-11), the permanent deep-history fallback case;
    //   2026-08-10 — the day that has readings;
    //   2026-08-11 — the day AFTER, which has none of its own.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2026-03-01 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1), \
             (toDateTime('2026-08-10 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1), \
             (toDateTime('2026-08-11 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // The hourly grain gets the same three buckets plus two INSIDE 2026-08-10 —
    // 09:00 (the hour holding the earlier reading) and 23:00 (the hour holding
    // the last one). Those two are what make case 5 a real test: the day's close
    // must equal the 23:00 hour's close, not the 09:00 one.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h SELECT * FROM {db}.price_ohlcv_1d"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2026-08-10 09:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1), \
             (toDateTime('2026-08-10 23:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // Two measured readings on 2026-08-10, plus a `pivot` row planted LAST in
    // the day at a value no peg asset could hold. Only the two 'oracle' rows may
    // be seen, and only the later of them may win.
    client
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, hops, version) VALUES \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2026-08-10 09:00:00'),{EARLIER},'oracle','',0,1), \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2026-08-10 23:55:00'),{MEASURED},'oracle','',0,1), \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2026-08-10 23:59:00'),0.50000000000000,'pivot','XLM',1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // Read the exact decimal, NOT toFloat64 — see the doc comment on case 1.
    let usdc_at = |view: &'static str, bucket: &'static str| {
        let client = client.clone();
        let db = db.to_string();
        async move {
            client
                .query(&format!(
                    "SELECT toString(close_usd), method FROM {db}.{view} \
                     WHERE asset_code = ? AND issuer_address = ? AND bucket = toDateTime(?)"
                ))
                .bind("USDC")
                .bind(USDC_ISSUER)
                .bind(bucket)
                .fetch_one::<(String, String)>()
                .await
                .unwrap()
        }
    };

    for (view, bucket) in [
        ("price_usd_series", "2026-08-10 00:00:00"),
        ("price_usd_series_1h", "2026-08-10 23:00:00"),
    ] {
        // CASES 1, 2 and 4 — the measured rate, the LAST one, and not the pivot.
        let (close, method) = usdc_at(view, bucket).await;
        assert_eq!(
            close, MEASURED,
            "{view} @ {bucket}: must publish the bucket's last MEASURED rate. \
             `1` means the constant is still there; `{EARLIER}` means the first \
             reading won instead of the last; `0.5` means a task 0154 'pivot' \
             row was accepted as a measurement."
        );
        assert_eq!(
            method, "oracle",
            "{view} @ {bucket}: a measured rate must be labelled 'oracle' — a \
             consumer cannot otherwise tell it from the $1 fallback"
        );
    }

    // CASE 3 — the two buckets with no reading of their own. Deep history is the
    // permanent case (no oracle reading exists before 2026-03-11 on prod); the
    // day AFTER the readings is the forward-fill guard.
    for (view, bucket, why) in [
        (
            "price_usd_series",
            "2026-03-01 00:00:00",
            "deep history, before the oracle window",
        ),
        (
            "price_usd_series",
            "2026-08-11 00:00:00",
            "the day after — yesterday's rate must NOT forward-fill",
        ),
        (
            "price_usd_series_1h",
            "2026-08-10 00:00:00",
            "an hour of the measured day that itself holds no reading",
        ),
    ] {
        let (close, method) = usdc_at(view, bucket).await;
        assert_eq!(
            close, "1",
            "{view} @ {bucket}: expected the $1 fallback ({why})"
        );
        assert_eq!(
            method, "peg",
            "{view} @ {bucket}: the fallback must be labelled 'peg' ({why})"
        );
    }

    // CASE 5 — the grains compose: the daily close IS the last hourly close of
    // the same day. Asserted against the views rather than against the constant,
    // so it still means something if the fixture's numbers change.
    let (daily, _) = usdc_at("price_usd_series", "2026-08-10 00:00:00").await;
    let (last_hour, _) = usdc_at("price_usd_series_1h", "2026-08-10 23:00:00").await;
    assert_eq!(
        daily, last_hour,
        "the daily close must equal the last hourly close of the same day — \
         task 0167's reason for a close rather than an average"
    );

    // The earlier reading is not lost, it is just not the day's close: the hour
    // that holds it publishes it. This is what makes case 2 a choice of rule
    // rather than a choice of row.
    let (nine, nine_method) = usdc_at("price_usd_series_1h", "2026-08-10 09:00:00").await;
    assert_eq!(nine, "1.0005", "the 09:00 hour publishes its own reading");
    assert_eq!(nine_method, "oracle");

    // Unchanged invariants from task 0165: one row per (identity, bucket), and
    // nothing publishes a non-positive close_usd.
    let dupes: u64 = client
        .query(&format!(
            "SELECT count() FROM (SELECT count() AS c FROM {db}.price_usd_series \
             GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket \
             HAVING c > 1)"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(dupes, 0, "the rate join must not multiply rows");

    let garbage: u64 = client
        .query(&format!(
            "SELECT countIf(toFloat64(close_usd) <= 0) FROM {db}.price_usd_series"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(garbage, 0, "no row may publish a non-positive close_usd");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0267 — where ONE bucket holds both a polled reading and an imported one,
/// the ORACLE rate is published and the label reads `oracle`.
///
/// This happens on prod in exactly ONE daily bucket (review IN-01): task 0267
/// loads only rows strictly below `USDC_ORACLE_EPOCH_S` and our own polling
/// starts at it, so the two populations share no KEY — but the epoch is 14:00
/// UTC and the import is stamped at 00:00, so the 1d bucket of the epoch day
/// (2026-03-11) holds one `external` row and the day's `oracle` polls. The rule
/// asserted here is what makes that bucket read `oracle`; it is also the SAFETY
/// RULE the whole read-path widening rests on — the moment `method IN ('oracle',
/// 'external')` replaced a single-method equality, "which one wins" stopped being
/// a question the schema answered and became one the query has to.
///
/// ⚠️ The fixture plants the import at a LATER timestamp than the oracle reading
/// and at a deliberately wrong value, exactly as case 4 does for `pivot`. Both
/// details are load-bearing:
///
///   * **Later** — because the shape this replaced was `argMax(usd_rate,
///     timestamp)`. Under that key the import wins, so a regression to it fails
///     here. Had the import been seeded earlier, the old key would have produced
///     the right answer for the wrong reason and this test would have passed
///     against the very bug it exists to catch.
///   * **A wrong value, not just a different method** — so a preference
///     regression changes the published NUMBER and not merely the label. A test
///     that only checked `method` would let a mislabelled-but-correct rate pass,
///     which is the less dangerous half of the failure.
///
/// Both grains are checked: the daily and hourly rate subqueries are separate SQL
/// and a fix applied to one only is this file's recurring defect.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_oracle_row_outranks_an_imported_row_in_the_same_bucket() {
    let db = "it_views_0267_oracle_outranks_external";
    let client = setup_scratch(db).await;

    /// The polled reading — what both grains must publish.
    const ORACLE: &str = "1.00066784838102";
    /// The import, planted LATER in the same bucket at a value no measured USDC
    /// rate could hold, so a rank regression is visible in the number itself.
    const IMPORT: &str = "0.50000000000000";

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // One FOO/USDC candle, so USDC is a quote-only peg leg and arm B emits its
    // placeholder for the bucket. The 23:00 hour carries the hourly case.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2026-08-10 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2026-08-10 23:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // The two rows COEXIST rather than one replacing the other: `method` is part
    // of the ReplacingMergeTree sorting key (init.sql), which is exactly the
    // property that makes "the consumer chooses" possible — and necessary.
    // The imported row carries task 0267's provenance columns; the oracle row
    // leaves `quality` at its '' default, because "how confident was the outside
    // series" is not a question a poll answers.
    client
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2026-08-10 23:30:00'),{ORACLE},'oracle','','',0,1), \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2026-08-10 23:45:00'),{IMPORT},'external','chainlink','measured',0,1)"
        ))
        .execute()
        .await
        .unwrap();

    let usdc_at = |view: &'static str, bucket: &'static str| {
        let client = client.clone();
        let db = db.to_string();
        async move {
            client
                .query(&format!(
                    "SELECT toString(close_usd), method FROM {db}.{view} \
                     WHERE asset_code = ? AND issuer_address = ? AND bucket = toDateTime(?)"
                ))
                .bind("USDC")
                .bind(USDC_ISSUER)
                .bind(bucket)
                .fetch_one::<(String, String)>()
                .await
                .unwrap()
        }
    };

    for (view, bucket) in [
        ("price_usd_series", "2026-08-10 00:00:00"),
        ("price_usd_series_1h", "2026-08-10 23:00:00"),
    ] {
        let (close, method) = usdc_at(view, bucket).await;
        assert_eq!(
            close, ORACLE,
            "{view} @ {bucket}: the POLLED rate must win a bucket that holds \
             both. `{IMPORT}` means the preference is keyed on the timestamp \
             again and the later import outranked the measurement."
        );
        assert_eq!(
            method, "oracle",
            "{view} @ {bucket}: a bucket whose published rate came from the \
             oracle must say so — the label follows the row that WON, not the \
             row that arrived last"
        );
    }

    // The import is not lost, it is outranked: on its own it is published and
    // labelled 'external'. Without this half, a view that simply ignored every
    // imported row would pass the assertions above — and task 0267's entire
    // purpose is to serve those rows where no oracle reading exists.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2023-03-11 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2023-03-11 00:00:00'),0.96812000000000,'external','chainlink','measured',0,1)"
        ))
        .execute()
        .await
        .unwrap();

    let (depeg, depeg_method) = usdc_at("price_usd_series", "2023-03-11 00:00:00").await;
    assert_eq!(
        depeg, "0.96812",
        "the depeg day must publish the IMPORTED rate. `1` means the widened \
         predicate is not reaching method = 'external' rows at all."
    );
    assert_eq!(
        depeg_method, "external",
        "an imported measurement must be labelled 'external' — a consumer has \
         to be able to tell it from a polled reading AND from the $1 fallback"
    );

    // The pre-promotion staging word is INERT. Nothing reads it, which is what
    // makes a shadow load safe to leave in place while an operator verifies it.
    client
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2023-03-11 23:00:00'),0.11100000000000,'external-candidate','chainlink','measured',0,1)"
        ))
        .execute()
        .await
        .unwrap();
    let (still, still_method) = usdc_at("price_usd_series", "2023-03-11 00:00:00").await;
    assert_eq!(
        still, "0.96812",
        "an 'external-candidate' row must not reach this surface — staged rows \
         are unverified by definition"
    );
    assert_eq!(still_method, "external");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// 🔑 Task 0267, review round 2 WR-09 — `price_usd_series_1h` publishes the
/// IMPORTED rate of each HOUR, from the hourly grain of task 0265's composed
/// series (Adam, 2026-09-09).
///
/// This is the view half of
/// `ohlcv_usdc_serves_the_depeg_day_hour_by_hour_from_the_hourly_import`
/// (prices-api `ohlcv_it.rs`), seeded identically and asserted against the same
/// three numbers, because the two surfaces must agree and the cheapest way to
/// keep them agreeing is to make the same fixture answer both.
///
/// ⚠️ THE DIVERGENCE THIS CLOSES. `/ohlcv` floors an `external` row at
/// `toStartOfDay(bkt, 'UTC')` — one DAILY imported row serves all 24 hours of
/// its day, matching task 0268's external enrichment tier. This view has no such
/// net: it buckets an imported row exactly like a poll. From a daily-only load
/// the two therefore disagree on 23 of every 24 hours, which is what WR-09
/// found. Loading the HOURLY file removes the disagreement at its source: there
/// is a row per hour, so both surfaces resolve the same row and the net is never
/// exercised. That is the reasoning behind NOT widening this view with a
/// UNION ALL / ARRAY JOIN over the day's hours — see the comment above the rate
/// join in `views.sql`.
///
/// The 2023-03-12 00:00 bucket is the control: a candle, no rate. It must read
/// `1`/'peg'. Without it a regression that forward-filled the previous day's
/// import across the UTC day boundary would pass.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn price_usd_series_1h_publishes_the_imported_rate_of_each_hour() {
    let db = "it_views_0267_hourly_import";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // FOO/USDC candles, so USDC is a quote-only peg leg and the view's peg arm
    // emits its placeholder for each bucket — the same shape every other peg
    // test in this file uses.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2023-03-11 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1), \
             (toDateTime('2023-03-11 07:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1), \
             (toDateTime('2023-03-11 23:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1), \
             (toDateTime('2023-03-12 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // The real hours from composed_usdc_usd_1h.csv: the day opens near par,
    // troughs at 07:00 and closes at 0.96812 — the value the DAILY file carries
    // for the whole day. Three distinct numbers, so a surface resolving the
    // wrong hour's row cannot still match.
    client
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, quality, hops, version) VALUES \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2023-03-11 00:00:00'),0.99503491000000,'external','chainlink','measured',0,1), \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2023-03-11 07:00:00'),0.88330000000000,'external','chainlink','measured',0,1), \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2023-03-11 23:00:00'),0.96812000000000,'external','chainlink','measured',0,1)"
        ))
        .execute()
        .await
        .unwrap();

    for (bucket, want, method) in [
        ("2023-03-11 00:00:00", "0.99503491", "external"),
        ("2023-03-11 07:00:00", "0.8833", "external"),
        ("2023-03-11 23:00:00", "0.96812", "external"),
        ("2023-03-12 00:00:00", "1", "peg"),
    ] {
        let (close, rate_method) = client
            .query(&format!(
                "SELECT toString(close_usd), method FROM {db}.price_usd_series_1h \
                 WHERE asset_code = ? AND issuer_address = ? AND bucket = toDateTime(?)"
            ))
            .bind("USDC")
            .bind(USDC_ISSUER)
            .bind(bucket)
            .fetch_one::<(String, String)>()
            .await
            .unwrap_or_else(|e| panic!("no row for {bucket}: {e}"));
        assert_eq!(
            close, want,
            "{bucket}: the hourly view must publish that HOUR's imported rate. \
             `0.96812` on the 00:00 or 07:00 bucket means a daily row is being \
             spread across the day; `1` means the widened predicate is not \
             reaching method = 'external' at all."
        );
        assert_eq!(
            rate_method, method,
            "{bucket}: an imported hour must be labelled 'external', and the day \
             AFTER the import must fall back to a labelled peg rather than \
             forward-filling across the UTC day boundary"
        );
    }

    // The daily surface is INDIFFERENT to the hourly rows: it argMaxes over the
    // whole day and lands on 23:00, whose close IS the daily close (asserted for
    // all 2049 days in enrichment-worker's composed_usdc_csv.rs). This is the
    // property that makes "load daily first, hourly second" safe — the hourly
    // rows win the shared midnight key and the daily number does not move.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2023-03-11 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();
    let (daily, daily_method) = client
        .query(&format!(
            "SELECT toString(close_usd), method FROM {db}.price_usd_series \
             WHERE asset_code = ? AND issuer_address = ? AND bucket = toDateTime(?)"
        ))
        .bind("USDC")
        .bind(USDC_ISSUER)
        .bind("2023-03-11 00:00:00")
        .fetch_one::<(String, String)>()
        .await
        .unwrap();
    assert_eq!(
        daily, "0.96812",
        "the daily bucket must still publish the DAY's close, taken from the \
         23:00 hourly row"
    );
    assert_eq!(daily_method, "external");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0168 — the two grains DIVERGE across an oracle gap, and that is expected.
///
/// The sibling test
/// [`peg_fill_publishes_the_measured_rate_and_falls_back_only_without_one`]
/// asserts that a daily close equals the last hourly close of the same day. That
/// invariant holds only when the day's last candle-bearing hour actually holds a
/// reading. This test is the misaligned case, and it exists because the sibling's
/// fixture (last reading at 23:55, inside the 23:00 hour) cannot see it and would
/// otherwise read as a guarantee the design does not make.
///
/// One reading at 09:05, hourly candles at 09:00 and 23:00. The daily bucket
/// contains the reading; the 23:00 bucket does not. So:
///
/// | grain  | bucket | close  | method   |
/// |--------|--------|--------|----------|
/// | daily  | 00:00  | 0.9993 | `oracle` |
/// | hourly | 09:00  | 0.9993 | `oracle` |
/// | hourly | 23:00  | 1      | `peg`    |
///
/// Both values are correct under task 0167's rule (the bucket's LAST observation)
/// and both are labelled, so nothing here is silent. Making them agree would need
/// a resolution rule that reaches outside the bucket — the unbounded forward-fill
/// this view deliberately refuses, and the one `/ohlcv`'s peg series does apply.
/// Written down as EXPECTED so that a future change which "fixes" the divergence
/// has to delete an assertion rather than merely satisfy a green suite.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_day_whose_last_candle_hour_holds_no_reading_diverges_between_grains() {
    let db = "it_views_0168_grain_divergence_across_a_gap";
    let client = setup_scratch(db).await;

    /// The day's only reading, landing in the 09:00 hour.
    const MEASURED: &str = "0.99930223861292";

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // FOO/USDC — USDC is quote-only, so arm B emits a peg placeholder per bucket.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2026-08-12 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // The day's candles sit in two hours; the LAST one (23:00) is the one the
    // oracle sat out.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2026-08-12 09:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1), \
             (toDateTime('2026-08-12 23:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // A single measurement, 09:05 — inside the day, inside the 09:00 hour, and
    // NOT inside the 23:00 hour.
    client
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, hops, version) VALUES \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2026-08-12 09:05:00'),{MEASURED},'oracle','',0,1)"
        ))
        .execute()
        .await
        .unwrap();

    let usdc_at = |view: &'static str, bucket: &'static str| {
        let client = client.clone();
        let db = db.to_string();
        async move {
            client
                .query(&format!(
                    "SELECT toString(close_usd), method FROM {db}.{view} \
                     WHERE asset_code = ? AND issuer_address = ? AND bucket = toDateTime(?)"
                ))
                .bind("USDC")
                .bind(USDC_ISSUER)
                .bind(bucket)
                .fetch_one::<(String, String)>()
                .await
                .unwrap()
        }
    };

    // The day contains the reading, so the daily bucket publishes it.
    let (daily, daily_method) = usdc_at("price_usd_series", "2026-08-12 00:00:00").await;
    assert_eq!(
        daily, MEASURED,
        "the daily bucket contains the 09:05 reading and must publish it"
    );
    assert_eq!(daily_method, "oracle");

    // So does the hour that holds it.
    let (nine, nine_method) = usdc_at("price_usd_series_1h", "2026-08-12 09:00:00").await;
    assert_eq!(nine, MEASURED, "the 09:00 hour holds the reading");
    assert_eq!(nine_method, "oracle");

    // The day's LAST candle-bearing hour does not, and falls back — labelled.
    let (last_hour, last_method) = usdc_at("price_usd_series_1h", "2026-08-12 23:00:00").await;
    assert_eq!(
        last_hour, "1",
        "the 23:00 hour holds no reading and must fall back to $1 rather than \
         carry the 09:00 hour's rate forward"
    );
    assert_eq!(
        last_method, "peg",
        "the fallback must stay labelled 'peg' — the divergence below is only \
         acceptable because a consumer can SEE which value is measured"
    );

    // The divergence itself, asserted rather than assumed. This is the property
    // the sibling test's fixture cannot reach.
    assert_ne!(
        daily, last_hour,
        "EXPECTED: across an oracle gap covering the day's last candle-bearing \
         hour, the daily close ({daily}) and the last hourly close ({last_hour}) \
         differ. If this now passes, the resolution rule has gained a \
         forward-fill outside the bucket — see the comment in views.sql"
    );

    // The 0165 invariants survive the gap.
    let garbage: u64 = client
        .query(&format!(
            "SELECT countIf(toFloat64(close_usd) <= 0) FROM {db}.price_usd_series_1h"
        ))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(garbage, 0, "no row may publish a non-positive close_usd");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0168 — a measured rate of exactly `1.0000…` is still `oracle`.
///
/// This is the acceptance criterion that the fallback must be DISTINGUISHABLE
/// from a measurement that happens to land on par, and it is the one property
/// no value-based check can cover: both rows read `1`. If `method` ever went
/// away, or were derived from the value (`if(close_usd = 1, 'peg', …)`), this
/// test would be the only thing to notice — and the surface would have
/// reproduced the `close_usd = 0` defect class, one value meaning two things.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_measured_rate_at_exactly_par_is_labelled_oracle_not_peg() {
    let db = "it_views_0168_par_is_still_measured";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // Two identical buckets, distinguished only by whether a reading exists.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
             (toDateTime('2026-08-10 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1), \
             (toDateTime('2026-08-11 00:00:00'),10,2,'sdex',5,5,5,5,10,50,500,5,5,1,1)"
        ))
        .execute()
        .await
        .unwrap();

    // A real reading, at exactly par.
    client
        .query(&format!(
            "INSERT INTO {db}.usd_rate \
             (asset_kind, asset_code, issuer_address, contract_address, timestamp, \
              usd_rate, method, reference_asset, hops, version) VALUES \
             ('credit','USDC','{USDC_ISSUER}','',toDateTime('2026-08-10 12:00:00'),1.00000000000000,'oracle','',0,1)"
        ))
        .execute()
        .await
        .unwrap();

    let row = |bucket: &'static str| {
        let client = client.clone();
        let db = db.to_string();
        async move {
            client
                .query(&format!(
                    "SELECT toString(close_usd), method FROM {db}.price_usd_series \
                     WHERE asset_code = 'USDC' AND bucket = toDateTime(?)"
                ))
                .bind(bucket)
                .fetch_one::<(String, String)>()
                .await
                .unwrap()
        }
    };

    let (measured, measured_method) = row("2026-08-10 00:00:00").await;
    let (fallback, fallback_method) = row("2026-08-11 00:00:00").await;

    assert_eq!(
        measured, fallback,
        "fixture precondition: both buckets must read the same value, otherwise \
         this test proves nothing about the discriminator"
    );
    assert_eq!(
        measured_method, "oracle",
        "a measurement that lands on par is still a measurement"
    );
    assert_eq!(
        fallback_method, "peg",
        "the bucket with no reading is the fallback and must say so"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

// ----------------------------------------------------------------------
// Tasks 0171 / 0198 — a zero-weight group is ABSENT, never a sentinel.
//
// Arm A admitted a candle on `close_usd > 0` alone. A group whose priced
// candles ALL carry `volume_base = 0` therefore reached
// `CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14))` with `sum(w) = 0`.
// What that CAST does with the NULL on 26.3.10.60 depends on the expression
// JIT, which is why 0171 and 0198 measured different things and were BOTH
// right:
//
//   * interpreted (`compile_expressions = 0`, or a cold server before the
//     expression has run `min_count_to_compile_expression` = 3 times): it
//     RAISES CANNOT_INSERT_NULL_IN_ORDINARY_COLUMN (code 349) and the whole
//     query fails — 0198's reading, an availability failure;
//   * JIT-compiled (the default once warm, which prod always is): it strips
//     the Nullable and publishes Decimal128::MIN (≈ -1.7e24) flagged
//     `method = 'traded'` — 0171's reading, a correctness failure.
//
// The tests below run every read in both modes, each FORCED by settings, so a
// regression fails either way regardless of how warm the server is. (Review
// on PR #312: with the server default, a cold server stays interpreted for
// the first 3 executions, so whether the compiled path was reached at all
// depended on execution counts leaked from other tests.) The fix is the same
// for both: BE's 2026-08-11 decision on 0171 is "omit the row", and arm A now
// requires `volume_base > 0`, which such a group cannot satisfy, so it never
// forms and the CAST never sees NULL.
// ----------------------------------------------------------------------

/// The two JIT modes a read can hit; see the block comment above. Interpreted
/// first, because that is the whole-query failure; then compiled on the FIRST
/// execution (`min_count_to_compile_expression = 0`), which is the silent one.
const JIT_MODES: [&str; 2] = [
    " SETTINGS compile_expressions = 0",
    " SETTINGS compile_expressions = 1, min_count_to_compile_expression = 0",
];

/// Seeds three assets and, on BOTH candle grains, a FOO/USDC candle with real
/// volume (so FOO publishes 5 and USDC gets its 0165 placeholder) plus a
/// BAR/FOO candle that is priced but carries ZERO volume — task 0198's
/// "fixture A": an asset that is only ever a zero-volume BASE and never a peg
/// quote leg, so neither the 0165 guard nor anything else can rescue it.
async fn seed_zero_volume_only_base(client: &Client, db: &str) {
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','',''), \
             (11,'BAR','classic','GBAR','','')"
        ))
        .execute()
        .await
        .unwrap();
    for tbl in ["price_ohlcv_1d", "price_ohlcv_1h"] {
        client
            .query(&format!(
                "INSERT INTO {db}.{tbl} \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
                 (1620000000,10, 2,'sdex', 5,5,5,5,           7,35,350,5,5,1,1), \
                 (1620000000,11,10,'sdex', 0.5,0.5,0.5,0.5,   0,0,0,0.5,0.5,1,1)"
            ))
            .execute()
            .await
            .unwrap();
    }
}

/// Tasks 0171 / 0198 — the non-peg zero-volume case, at both grains, in
/// both JIT modes.
///
/// ⚠️ Since task 0147 this fixture is absent for TWO independent reasons, and
/// the test still proves the 0198 one. BAR trades only against FOO, which is
/// not an eligible quote (not USDC / XLM / USDT and with no prices.usd_rate
/// row), so BAR's bucket now also has NO eligible volume — `ew = 0`, coverage
/// `unpriceable`. That is a different mechanism from the zero-weight group this
/// test was written for; what it pins either way is that BAR is ABSENT and that
/// nothing raises or publishes a sentinel in either JIT mode.
///
/// Before the fix BAR published Decimal128::MIN as `traded` (compiled) or the
/// query raised code 349 (interpreted). After it BAR is absent in both modes,
/// and — the availability half of 0198 — its neighbours FOO and USDC in the
/// SAME query still publish exactly what they published before.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_zero_volume_only_base_is_absent_and_its_neighbours_still_publish() {
    let db = "it_views_zero_weight_absent";
    let client = setup_scratch(db).await;
    seed_zero_volume_only_base(&client, db).await;

    for view in ["price_usd_series", "price_usd_series_1h"] {
        for jit in JIT_MODES {
            let rows: Vec<(String, f64, String)> = client
                .query(&format!(
                    "SELECT asset_code, toFloat64(close_usd), method FROM {db}.{view} \
                     ORDER BY asset_code{jit}"
                ))
                .fetch_all::<(String, f64, String)>()
                .await
                .unwrap_or_else(|e| {
                    panic!("{view}{jit}: must not raise on a zero-weight group: {e}")
                });

            assert_eq!(
                rows.iter().map(|(c, _, _)| c.as_str()).collect::<Vec<_>>(),
                vec!["FOO", "USDC"],
                "{view}{jit}: BAR's only priced candle has no volume, so BAR must be \
                 ABSENT (BE's 'misses are absent' contract) and nothing else may go missing"
            );
            assert_eq!(
                rows[0].1, 5.0,
                "{view}{jit}: FOO's traded value must be untouched"
            );
            assert_eq!(rows[0].2, "traded");
            assert_eq!(
                rows[1].1, 1.0,
                "{view}{jit}: USDC's peg fallback must survive the fix"
            );
            assert_eq!(rows[1].2, "peg");

            // Asserted by VALUE: `IS NULL` is vacuously false on a non-Nullable column.
            let garbage: u64 = client
                .query(&format!(
                    "SELECT countIf(toFloat64(close_usd) <= 0) FROM {db}.{view}{jit}"
                ))
                .fetch_one::<u64>()
                .await
                .unwrap();
            assert_eq!(
                garbage, 0,
                "{view}{jit}: no row may publish a non-positive close_usd"
            );
        }
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// A zero-volume candle beside a real one was already dead weight (v = 0,
/// w = 0 add nothing to a weighted average). Dropping it from arm A must
/// therefore change NO published value — this pins that the fix is exactly the
/// omission of the un-computable group and nothing else.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_zero_volume_candle_beside_a_real_one_changes_nothing() {
    let db = "it_views_zero_weight_mixed";
    let client = setup_scratch(db).await;
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();
    for tbl in ["price_ohlcv_1d", "price_ohlcv_1h"] {
        // Two sources in one bucket: a real 7-unit print at 5 and a zero-volume
        // print at a DIFFERENT price (4), which must not be able to pull the
        // average — the weighted mean of the pair is 5 whether or not it is there.
        client
            .query(&format!(
                "INSERT INTO {db}.{tbl} \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
                 (1620000000,10,2,'sdex',    5,5,5,5, 7,35,350,5,5,1,1), \
                 (1620000000,10,2,'soroswap',4,4,4,4, 0,0,0,4,4,1,1)"
            ))
            .execute()
            .await
            .unwrap();
    }

    for view in ["price_usd_series", "price_usd_series_1h"] {
        let (close, method): (f64, String) = client
            .query(&format!(
                "SELECT toFloat64(close_usd), method FROM {db}.{view} WHERE asset_code = 'FOO'"
            ))
            .fetch_one::<(f64, String)>()
            .await
            .unwrap();
        assert_eq!(
            close, 5.0,
            "{view}: the zero-volume print must carry no weight"
        );
        assert_eq!(method, "traded");
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0171's audit item — `usd_reference{,_1h}` carry the same
/// `CAST(… / nullIf(sum(volume_base), 0) AS Decimal)` shape over XLM/USDC
/// candles, filtered on `close > 0` only. A bucket whose reference candles all
/// carry zero volume published Decimal128::MIN as the XLM reference (or raised
/// code 349, interpreted), which every pivot-priced asset's status
/// classification hangs off. It must be absent instead: `no_reference` is a
/// legitimate §12.3 state, a garbage reference is not.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn usd_reference_omits_a_bucket_whose_reference_candles_have_no_volume() {
    let db = "it_views_zero_weight_reference";
    let client = setup_scratch(db).await;
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (1,'XLM','native','','',''), \
             (2,'USDC','classic','{USDC_ISSUER}','','')"
        ))
        .execute()
        .await
        .unwrap();
    for tbl in ["price_ohlcv_1d", "price_ohlcv_1h"] {
        // Bucket 1620000000: a real XLM/USDC print. Bucket 1620086400: only a
        // zero-volume one.
        client
            .query(&format!(
                "INSERT INTO {db}.{tbl} \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES \
                 (1620000000,1,2,'sdex', 0.1,0.1,0.1,0.1, 100,10,10,0.1,0.1,1,1), \
                 (1620086400,1,2,'sdex', 0.2,0.2,0.2,0.2,   0, 0, 0,0.2,0.2,1,1)"
            ))
            .execute()
            .await
            .unwrap();
    }

    for view in ["usd_reference", "usd_reference_1h"] {
        for jit in JIT_MODES {
            let rows: Vec<(u32, f64)> = client
                .query(&format!(
                    "SELECT toUInt32(bucket), toFloat64(xlm_usd) FROM {db}.{view} \
                     ORDER BY bucket{jit}"
                ))
                .fetch_all::<(u32, f64)>()
                .await
                .unwrap_or_else(|e| {
                    panic!("{view}{jit}: must not raise on a zero-weight bucket: {e}")
                });
            assert_eq!(
                rows,
                vec![(1620000000, 0.1)],
                "{view}{jit}: the zero-volume bucket must be ABSENT (no_reference), \
                 not a sentinel"
            );
        }
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

// ----------------------------------------------------------------------
// Task 0147 — the priced-volume coverage gate.
//
// Arm A used to filter `close_usd > 0` BEFORE the weighted average, so
// whichever rows enrichment happened to reach became 100 % of the weight. BE
// measured the consequence on yXLM (2026-08-04 13:00): a single 0.764-unit
// print at 1.3085 was the only enriched row in the bucket, and the view
// published 1.3085 against a true ~0.170 — a 7.7x overstatement in the column
// they multiply into TVL.
//
// The POPULATION was wrong, not the arithmetic. The unpriced rows are real
// trades and belong in the denominator, so the fix makes the population
// explicit: `priced_volume_share` is published on every row, and a bucket whose
// priced volume does not clear the gate is ABSENT — with a row in
// `price_usd_series_coverage{,_1h}` that says why.
//
// ⚠️ The fixtures below carry `volume_quote_usd` values scaled to clear
// `FLOOR_USD` (a PLACEHOLDER 100 until task 0147 phase 2 measures it). The
// floor itself is exercised by
// `only_the_dust_print_is_priced_and_the_absolute_floor_withholds_the_bucket`;
// every other 0147 test is about the SHARE, so its fixture must clear the floor
// or it would prove nothing.
// ----------------------------------------------------------------------

/// Task 0147 (a) — BE's yXLM case, RED→GREEN.
///
/// One 0.764-unit print is priced at 1.3085 beside 1000 unpriced units of the
/// same identity in the same bucket. Before the gate the view published 1.3085,
/// the dust print's own price, because the unpriced 99.92 % of the bucket was
/// filtered out before the weighting. After it the bucket is WITHHELD, and
/// `price_usd_series_coverage` reports `pending` with the share that explains
/// the withholding. Enrich the 1000 units at their true 0.0065 and the bucket
/// publishes — the gate withholds a bucket, it never deletes an identity.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_dust_print_cannot_price_a_bucket_whose_volume_is_unpriced() {
    let db = "it_views_0147_dust_share";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();

    // ⚠️ The pf columns are named EXPLICITLY. The 15-column idiom the rest of
    // this file uses lets them fall back to their init.sql DEFAULTs
    // (`pf_volume` = `volume_base`, `pf_trade_count` = `trade_count`), which is
    // right for an ordinary fixture and silently destroys a dust one.
    //
    // sdex: the enriched dust print — 0.764 units at 1.3085, ~1 USD changed hands.
    // soroswap: 1000 units at a true 0.0065 that enrichment has NOT reached
    //           (`close_usd` = 0, and `volume_quote_usd` = 0 with it — the same
    //           pass writes both).
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
              pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
             (1620000000,10,2,'sdex',     1.3085,1.3085,1.3085,1.3085, 0.764,1.0,1.0,1.3085,1.3085,1, 1,0.764,1.0, 1), \
             (1620000000,10,2,'soroswap', 0.0065,0.0065,0.0065,0.0065, 1000,6.5,0,0,0.0065,1,        1,1000,6.5, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-6;

    // --- WITHHELD ---------------------------------------------------------
    // Both JIT modes: the withheld group reaches the outer CAST with a priced
    // weight of 0.764 and an eligible weight of 1000.764, and a group made only
    // of unpriced rows reaches it with NO priced weight at all — the 0171/0198
    // zero-denominator hazard, re-armed by arm A no longer filtering.
    for jit in JIT_MODES {
        let published: Vec<(String, f64)> = client
            .query(&format!(
                "SELECT asset_code, toFloat64(close_usd) FROM {db}.price_usd_series \
                 WHERE asset_code = 'FOO'{jit}"
            ))
            .fetch_all::<(String, f64)>()
            .await
            .unwrap_or_else(|e| panic!("price_usd_series{jit} must not raise: {e}"));
        assert!(
            published.is_empty(),
            "price_usd_series{jit}: the only priced row in this bucket is a \
             0.764-unit print holding 0.0763 % of its eligible volume, so the \
             bucket must be WITHHELD — got {published:?} (1.3085 is the dust \
             print's own price, BE's 7.7x yXLM defect)"
        );
    }

    // ...and the coverage view says WHY it is missing, rather than leaving a
    // withheld bucket indistinguishable from one that never traded.
    for jit in JIT_MODES {
        let cov: Vec<(f64, f64, String)> = client
            .query(&format!(
                "SELECT toFloat64(priced_volume_share), toFloat64(priced_volume_usd), status \
                 FROM {db}.price_usd_series_coverage WHERE asset_code = 'FOO'{jit}"
            ))
            .fetch_all::<(f64, f64, String)>()
            .await
            .unwrap_or_else(|e| panic!("price_usd_series_coverage{jit} must not raise: {e}"));
        assert_eq!(cov.len(), 1, "exactly one coverage row for (FOO, bucket)");
        assert!(
            approx(cov[0].0, 0.000763),
            "coverage share = 0.764 / 1000.764 published as Decimal(10,6), got {}",
            cov[0].0
        );
        assert!(
            approx(cov[0].1, 1.0),
            "priced_volume_usd is the PRICED leg's volume_quote_usd, got {}",
            cov[0].1
        );
        assert_eq!(
            cov[0].2, "pending",
            "eligible volume exists and the share is short — `pending`, not `unpriceable`"
        );
    }

    // --- ENRICHED ---------------------------------------------------------
    // The same row, re-inserted at a higher `version` with its true price. Its
    // `volume_quote_usd` arrives with `close_usd` (one enrichment pass writes
    // both) and is scaled to clear the placeholder floor — see the block comment.
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
              pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
             (1620000000,10,2,'soroswap', 0.0065,0.0065,0.0065,0.0065, 1000,6.5,650,0.0065,0.0065,1, 1,1000,6.5, 2)"
        ))
        .execute()
        .await
        .unwrap();

    let (close, method, share): (f64, String, f64) = client
        .query(&format!(
            "SELECT toFloat64(close_usd), method, toFloat64(priced_volume_share) \
             FROM {db}.price_usd_series WHERE asset_code = 'FOO'"
        ))
        .fetch_one::<(f64, String, f64)>()
        .await
        .unwrap();
    // The bucket's volume-weighted mean over its NOW-COMPLETE population:
    // (1.3085 x 0.764 + 0.0065 x 1000) / 1000.764. The dust print is a real
    // trade and keeps its weight — 0.0764 % of the bucket, worth +0.001 on the
    // published price. That residual is the point: before the gate the same
    // print WAS the price.
    assert!(
        approx(close, 0.00749394),
        "the enriched bucket publishes its weighted mean, got {close}"
    );
    assert_eq!(method, "traded");
    assert!(
        approx(share, 1.0),
        "every eligible unit is priced now, got {share}"
    );

    let (cshare, cusd, cstatus): (f64, f64, String) = client
        .query(&format!(
            "SELECT toFloat64(priced_volume_share), toFloat64(priced_volume_usd), status \
             FROM {db}.price_usd_series_coverage WHERE asset_code = 'FOO'"
        ))
        .fetch_one::<(f64, f64, String)>()
        .await
        .unwrap();
    assert!(approx(cshare, 1.0), "coverage share, got {cshare}");
    assert!(
        approx(cusd, 651.0),
        "1 USD priced + 650 USD enriched, got {cusd}"
    );
    assert_eq!(cstatus, "priced");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0147 (b) — the ABSOLUTE floor, which the share alone cannot express.
///
/// A bucket holding ONLY the 0.764-unit print is 100 % priced: every eligible
/// unit that traded has a USD price, so the coverage share says nothing is
/// missing. It is still a dollar of trade, and one dollar of trade does not
/// establish a price for the asset — that is what `FLOOR_USD` is for.
///
/// ⚠️ The bucket reads `pending`, NOT `unpriceable`: eligible volume exists and
/// more of it may yet arrive. `unpriceable` means "we have no USD path at all".
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn only_the_dust_print_is_priced_and_the_absolute_floor_withholds_the_bucket() {
    let db = "it_views_0147_floor";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
              pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
             (1620000000,10,2,'sdex', 1.3085,1.3085,1.3085,1.3085, 0.764,1.0,1.0,1.3085,1.3085,1, 1,0.764,1.0, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-6;

    for jit in JIT_MODES {
        let published: Vec<f64> = client
            .query(&format!(
                "SELECT toFloat64(close_usd) FROM {db}.price_usd_series \
                 WHERE asset_code = 'FOO'{jit}"
            ))
            .fetch_all::<f64>()
            .await
            .unwrap_or_else(|e| panic!("price_usd_series{jit} must not raise: {e}"));
        assert!(
            published.is_empty(),
            "price_usd_series{jit}: a fully-priced bucket worth 1 USD is below \
             FLOOR_USD and must be WITHHELD, got {published:?}"
        );

        let (share, usd, status): (f64, f64, String) = client
            .query(&format!(
                "SELECT toFloat64(priced_volume_share), toFloat64(priced_volume_usd), status \
                 FROM {db}.price_usd_series_coverage WHERE asset_code = 'FOO'{jit}"
            ))
            .fetch_one::<(f64, f64, String)>()
            .await
            .unwrap_or_else(|e| panic!("coverage{jit} must not raise: {e}"));
        assert!(
            approx(share, 1.0),
            "{jit}: nothing is unpriced here — the SHARE is 1, got {share}"
        );
        assert!(
            approx(usd, 1.0),
            "{jit}: one dollar changed hands, got {usd}"
        );
        assert_eq!(
            status, "pending",
            "{jit}: withheld by the floor is still `pending` — eligible volume \
             exists, so more of it may yet arrive"
        );
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0147 (c) — UNPRICEABLE: no USD path at all.
///
/// Every row of the bucket is quoted in an asset that is neither the canonical
/// USDC, native XLM nor the canonical USDT, and that has no `prices.usd_rate`
/// row of its own. There is no eligible price-forming volume, so there is no
/// denominator: the bucket is absent and the coverage row says `unpriceable`
/// rather than `pending`, because nothing about enrichment would change it.
///
/// ⚠️ This is ONE of the two buckets that read `unpriceable` — the NO USD PATH
/// one, which flips to `pending` retroactively the moment a `usd_rate` row
/// appears for EXO (ADR 0292). The other is a bucket whose quote IS eligible
/// but whose every candle is stroop-dust, so `pf_volume` sums to 0: no
/// `usd_rate` row will ever move that one, only a real fill. The word means
/// "no price-forming volume we could price", not "no USD path" — see the
/// coverage header in views.sql.
///
/// ⚠️ The share is the LITERAL 0, never NULL (D-06). BE renders a NULL as a
/// dash and drops the pool, so a zero denominator must not surface as one —
/// which is also why the guard is an `if` rather than a `nullIf` that a
/// non-Nullable CAST would turn into Decimal128::MIN or code 349.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_bucket_quoted_only_in_an_ineligible_asset_reads_unpriceable_with_a_zero_share() {
    let db = "it_views_0147_unpriceable";
    let client = setup_scratch(db).await;

    // 20 = EXO: an ordinary credit asset, not in the eligible set by literal and
    // with no usd_rate row. 2 = USDC is seeded only so the peg arm has its
    // canonical identity to key on.
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','',''), \
             (20,'EXO','classic','GEXO','','')"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1d \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
              pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
             (1620000000,10,20,'sdex', 9,9,9,9, 500,4500,0,0,9,1, 1,500,4500, 1)"
        ))
        .execute()
        .await
        .unwrap();

    for jit in JIT_MODES {
        let published: Vec<f64> = client
            .query(&format!(
                "SELECT toFloat64(close_usd) FROM {db}.price_usd_series \
                 WHERE asset_code = 'FOO'{jit}"
            ))
            .fetch_all::<f64>()
            .await
            .unwrap_or_else(|e| panic!("price_usd_series{jit} must not raise: {e}"));
        assert!(
            published.is_empty(),
            "price_usd_series{jit}: nothing in this bucket is convertible, got {published:?}"
        );

        let (share, usd, status): (f64, f64, String) = client
            .query(&format!(
                "SELECT toFloat64(priced_volume_share), toFloat64(priced_volume_usd), status \
                 FROM {db}.price_usd_series_coverage WHERE asset_code = 'FOO'{jit}"
            ))
            .fetch_one::<(f64, f64, String)>()
            .await
            .unwrap_or_else(|e| panic!("coverage{jit} must not raise: {e}"));
        assert_eq!(
            share, 0.0,
            "{jit}: a zero denominator publishes the literal 0, never a NULL \
             and never a sentinel"
        );
        assert_eq!(usd, 0.0, "{jit}: no priced USD volume");
        assert_eq!(
            status, "unpriceable",
            "{jit}: no eligible volume at all — enrichment cannot change this, \
             only a usd_rate row for EXO can (retroactively, per ADR 0292)"
        );
    }

    // The column is non-Nullable, so `IS NULL` would be vacuously false —
    // assert the TYPE instead, which is what BE's decoder actually sees.
    let ty: String = client
        .query(&format!(
            "SELECT type FROM system.columns WHERE database = '{db}' \
             AND table = 'price_usd_series_coverage' AND name = 'priced_volume_share'"
        ))
        .fetch_one::<String>()
        .await
        .unwrap();
    assert!(
        !ty.contains("Nullable"),
        "priced_volume_share must never be Nullable on the wire, got {ty}"
    );

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0147 (d) — dust changes nothing, in either direction.
///
/// A row with `pf_trade_count = 0` and `pf_volume = 0` carried real
/// `volume_base` before task 0286 taught the pipeline to tell a price-forming
/// fill from stroop-dust. It must now move neither the published `close_usd`
/// nor `priced_volume_share`: it is not priced (no price-forming weight) and it
/// is not eligible weight either (the eligible sum is over `pf_volume` too).
///
/// The control is BAR, an identity with the same real print and no dust beside
/// it: the two must agree to the last digit.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn dust_rows_move_neither_the_published_close_nor_the_share() {
    let db = "it_views_0147_dust_noop";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','',''), \
             (11,'BAR','classic','GBAR','','')"
        ))
        .execute()
        .await
        .unwrap();
    // FOO: a real 7-unit print at 5, plus a dust row at a DIFFERENT price (4)
    // carrying 1000 units of volume_base and 4000 USD of quote volume — every
    // number that could perturb a mean or a floor, priced to be unmistakable.
    // BAR: the same real print, nothing else.
    for tbl in ["price_ohlcv_1d", "price_ohlcv_1h"] {
        client
            .query(&format!(
                "INSERT INTO {db}.{tbl} \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
                  pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
                 (1620000000,10,2,'sdex',     5,5,5,5, 7,35,350,5,5,1,       1,7,35, 1), \
                 (1620000000,10,2,'soroswap', 4,4,4,4, 1000,4000,4000,4,4,3, 0,0,0,  1), \
                 (1620000000,11,2,'sdex',     5,5,5,5, 7,35,350,5,5,1,       1,7,35, 1)"
            ))
            .execute()
            .await
            .unwrap();
    }

    for view in ["price_usd_series", "price_usd_series_1h"] {
        let rows: Vec<(String, f64, f64)> = client
            .query(&format!(
                "SELECT asset_code, toFloat64(close_usd), toFloat64(priced_volume_share) \
                 FROM {db}.{view} WHERE asset_code IN ('FOO','BAR') ORDER BY asset_code"
            ))
            .fetch_all::<(String, f64, f64)>()
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            2,
            "{view}: both identities must publish, got {rows:?}"
        );
        assert_eq!(
            (rows[0].1, rows[0].2),
            (rows[1].1, rows[1].2),
            "{view}: the dust row must leave FOO reading exactly what BAR reads \
             without it, got {rows:?}"
        );
        assert_eq!(rows[0].1, 5.0, "{view}: the real print is the price");
        assert_eq!(rows[0].2, 1.0, "{view}: dust is not eligible weight either");
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0147 (e) — the same withhold-then-publish at the HOURLY grain.
///
/// A fix applied to one grain and forgotten on the other is the exact defect the
/// hourly variant carried before task 0165 found it in both. `views.sql`'s two
/// bodies are kept in step by a text test in src/lib.rs; this is the
/// behavioural half.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_gate_and_the_coverage_view_behave_the_same_at_the_hourly_grain() {
    let db = "it_views_0147_hourly";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','','')"
        ))
        .execute()
        .await
        .unwrap();
    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
              pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
             (1620000000,10,2,'sdex',     1.3085,1.3085,1.3085,1.3085, 0.764,1.0,1.0,1.3085,1.3085,1, 1,0.764,1.0, 1), \
             (1620000000,10,2,'soroswap', 0.0065,0.0065,0.0065,0.0065, 1000,6.5,0,0,0.0065,1,        1,1000,6.5, 1)"
        ))
        .execute()
        .await
        .unwrap();

    let approx = |a: f64, b: f64| (a - b).abs() < 1e-6;

    for jit in JIT_MODES {
        let published: Vec<f64> = client
            .query(&format!(
                "SELECT toFloat64(close_usd) FROM {db}.price_usd_series_1h \
                 WHERE asset_code = 'FOO'{jit}"
            ))
            .fetch_all::<f64>()
            .await
            .unwrap_or_else(|e| panic!("price_usd_series_1h{jit} must not raise: {e}"));
        assert!(
            published.is_empty(),
            "price_usd_series_1h{jit}: the hourly grain must withhold the same \
             bucket the daily one does, got {published:?}"
        );

        let (share, status): (f64, String) = client
            .query(&format!(
                "SELECT toFloat64(priced_volume_share), status \
                 FROM {db}.price_usd_series_coverage_1h WHERE asset_code = 'FOO'{jit}"
            ))
            .fetch_one::<(f64, String)>()
            .await
            .unwrap_or_else(|e| panic!("coverage_1h{jit} must not raise: {e}"));
        assert!(approx(share, 0.000763), "{jit}: hourly share, got {share}");
        assert_eq!(status, "pending", "{jit}: hourly status");
    }

    client
        .query(&format!(
            "INSERT INTO {db}.price_ohlcv_1h \
             (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
              volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
              pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
             (1620000000,10,2,'soroswap', 0.0065,0.0065,0.0065,0.0065, 1000,6.5,650,0.0065,0.0065,1, 1,1000,6.5, 2)"
        ))
        .execute()
        .await
        .unwrap();

    let (close, share): (f64, f64) = client
        .query(&format!(
            "SELECT toFloat64(close_usd), toFloat64(priced_volume_share) \
             FROM {db}.price_usd_series_1h WHERE asset_code = 'FOO'"
        ))
        .fetch_one::<(f64, f64)>()
        .await
        .unwrap();
    assert!(
        approx(close, 0.00749394),
        "hourly weighted mean, got {close}"
    );
    assert!(
        approx(share, 1.0),
        "hourly share after enrichment, got {share}"
    );
    let status: String = client
        .query(&format!(
            "SELECT status FROM {db}.price_usd_series_coverage_1h WHERE asset_code = 'FOO'"
        ))
        .fetch_one::<String>()
        .await
        .unwrap();
    assert_eq!(status, "priced");

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0147 (f) — `usd_reference{,_1h}` take the same predicate.
///
/// The reference is the XLM/USD signal every pivot-priced asset's status hangs
/// off, so the rows it averages must clear the same bars the series' do: the
/// `1e-12` precision floor (a 9e-14 close beside a real one is rounding noise,
/// not a price) and a price-forming trade with price-forming weight.
///
/// Bucket A holds a real print PLUS one sub-floor row PLUS one dust row, and
/// must publish exactly what the real print alone says. Buckets B and C hold
/// only the sub-floor row and only the dust row respectively, and must be
/// ABSENT — `no_reference` is a legitimate state, a garbage reference is not.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn usd_reference_ignores_sub_floor_and_dust_only_rows_at_both_grains() {
    let db = "it_views_0147_reference";
    let client = setup_scratch(db).await;

    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (1,'XLM','native','','',''), \
             (2,'USDC','classic','{USDC_ISSUER}','','')"
        ))
        .execute()
        .await
        .unwrap();
    for tbl in ["price_ohlcv_1d", "price_ohlcv_1h"] {
        client
            .query(&format!(
                "INSERT INTO {db}.{tbl} \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
                  pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
                 (1620000000,1,2,'sdex',     0.1,0.1,0.1,0.1, 100,10,10,0.1,0.1,1, 1,100,10, 1), \
                 (1620000000,1,2,'soroswap', 0.0000000000001,0.0000000000001,0.0000000000001,0.0000000000001, 900,0,0,0,0,1, 1,900,0, 1), \
                 (1620000000,1,2,'phoenix',  0.5,0.5,0.5,0.5, 900,450,450,0.5,0.5,5, 0,0,0, 1), \
                 (1620086400,1,2,'soroswap', 0.0000000000001,0.0000000000001,0.0000000000001,0.0000000000001, 900,0,0,0,0,1, 1,900,0, 1), \
                 (1620172800,1,2,'phoenix',  0.5,0.5,0.5,0.5, 900,450,450,0.5,0.5,5, 0,0,0, 1)"
            ))
            .execute()
            .await
            .unwrap();
    }

    for view in ["usd_reference", "usd_reference_1h"] {
        for jit in JIT_MODES {
            let rows: Vec<(u32, f64)> = client
                .query(&format!(
                    "SELECT toUInt32(bucket), toFloat64(xlm_usd) FROM {db}.{view} \
                     ORDER BY bucket{jit}"
                ))
                .fetch_all::<(u32, f64)>()
                .await
                .unwrap_or_else(|e| panic!("{view}{jit}: must not raise: {e}"));
            assert_eq!(
                rows,
                vec![(1620000000, 0.1)],
                "{view}{jit}: only the bucket with a real, price-forming, \
                 above-floor print may publish, and its value must be that \
                 print's — the sub-floor row (1e-13) and the dust row \
                 (pf_trade_count = 0) carry no weight and form no bucket"
            );
        }
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}

/// Task 0147 review SF-01 — a PRICED row is always inside the share's
/// denominator, whatever its quote leg is.
///
/// `is_priced` admits a row on `close_usd != close` for ANY quote; `is_eligible`
/// names a QUOTE SET. The two are therefore not nested by definition, and before
/// this fix a row that was priced against a quote outside the eligible set went
/// into the numerator and not into the denominator. Measured on 26.3.10.60 with
/// exactly this fixture: `priced_volume_share = 5000000` in a `Decimal(10, 6)`
/// column `views.sql` documents as `[0, 1]` (CAST does not range-check `P`, so
/// nothing raised), and a fully-priced $5,000 bucket read `unpriceable` beside
/// `priced_volume_usd = 5000`.
///
/// `rew` now weights on `is_priced OR is_eligible`, so `priced ⊆ eligible` holds
/// BY CONSTRUCTION — the share cannot leave `[0, 1]` no matter what a future
/// writer puts in `close_usd`. Unreachable through today's write path
/// (`TRACKED_SYMBOLS` and the enrichment identities are all eligible quotes);
/// reachable the moment a symbol is tracked without being made eligible, which
/// is task 0173's shape.
///
/// FOO holds the priced-but-ineligible row BESIDE a dust-sized eligible unpriced
/// one — the combination that produced the 5000000. BAR holds the ineligible
/// priced row ALONE — the combination that produced `unpriceable` with a priced
/// USD volume. Both grains, both JIT modes.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_priced_row_outside_the_eligible_quote_set_stays_inside_the_share() {
    let db = "it_views_0147_priced_not_eligible";
    let client = setup_scratch(db).await;

    // 20 = EXO: an ordinary credit asset, in neither the three eligible
    // literals nor `prices.usd_rate`. 2 = USDC is the canonical eligible quote.
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','',''), \
             (11,'BAR','classic','GBAR','',''), \
             (20,'EXO','classic','GEXO','','')"
        ))
        .execute()
        .await
        .unwrap();
    for tbl in ["price_ohlcv_1d", "price_ohlcv_1h"] {
        client
            .query(&format!(
                "INSERT INTO {db}.{tbl} \
                 (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, \
                  pf_trade_count, pf_volume, pf_price_volume, version) VALUES \
                 (1620000000,10,20,'sdex', 9,9,9,9, 500,4500,5000,2.5,9,7, 7,500,4500, 1), \
                 (1620000000,10,2,'soroswap', 9,9,9,9, 0.0001,0.0009,0,0,9,1, 1,0.0001,0.0009, 1), \
                 (1620000000,11,20,'sdex', 9,9,9,9, 500,4500,5000,2.5,9,7, 7,500,4500, 1)"
            ))
            .execute()
            .await
            .unwrap();
    }

    for (series, coverage) in [
        ("price_usd_series", "price_usd_series_coverage"),
        ("price_usd_series_1h", "price_usd_series_coverage_1h"),
    ] {
        for jit in JIT_MODES {
            let rows: Vec<(String, f64, f64, String)> = client
                .query(&format!(
                    "SELECT asset_code, toFloat64(priced_volume_share), \
                            toFloat64(priced_volume_usd), status \
                     FROM {db}.{coverage} WHERE asset_code IN ('FOO','BAR') \
                     ORDER BY asset_code{jit}"
                ))
                .fetch_all::<(String, f64, f64, String)>()
                .await
                .unwrap_or_else(|e| panic!("{coverage}{jit} must not raise: {e}"));
            assert_eq!(rows.len(), 2, "{coverage}{jit}: one row per identity");

            for (code, share, usd, status) in &rows {
                assert!(
                    (0.0..=1.0).contains(share),
                    "{coverage}{jit}: {code}'s share must stay inside the \
                     [0, 1] this column is declared as — got {share}, which is \
                     what a priced row outside the eligible quote set does to a \
                     denominator that does not contain it"
                );
                assert_eq!(
                    usd, &5000.0,
                    "{coverage}{jit}: {code}'s priced USD volume is the \
                     ineligible-quoted row's own 5000"
                );
                assert_eq!(
                    status, "priced",
                    "{coverage}{jit}: {code} is FULLY priced — every unit that \
                     formed a price has a close_usd. `unpriceable` beside a \
                     priced_volume_usd of 5000 is the contradiction this fixes"
                );
            }

            let (foo_share, bar_share) = (rows[1].1, rows[0].1);
            assert!(
                foo_share > 0.99,
                "{coverage}{jit}: FOO's 0.0001 eligible unpriced unit against \
                 500 priced ones is a share just under 1, got {foo_share}"
            );
            assert_eq!(
                bar_share, 1.0,
                "{coverage}{jit}: BAR's only row is priced, so its share is \
                 exactly 1 — the priced weight IS the eligible weight"
            );

            let published: Vec<(String, f64)> = client
                .query(&format!(
                    "SELECT asset_code, toFloat64(close_usd) FROM {db}.{series} \
                     WHERE asset_code IN ('FOO','BAR') ORDER BY asset_code{jit}"
                ))
                .fetch_all::<(String, f64)>()
                .await
                .unwrap_or_else(|e| panic!("{series}{jit} must not raise: {e}"));
            assert_eq!(
                published,
                vec![("BAR".to_string(), 2.5), ("FOO".to_string(), 2.5)],
                "{series}{jit}: a bucket whose price-forming volume is entirely \
                 priced must be PUBLISHED at that price — withholding a \
                 fully-priced $5,000 bucket because its quote leg is not in a \
                 literal set is the same defect seen from the other side"
            );
        }
    }

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
