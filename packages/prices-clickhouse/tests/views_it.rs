//! Live-ClickHouse integration test for the read-surface views
//! (`price_usd_series`, `usd_reference`). Gated `#[ignore]`:
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-clickhouse --test views_it -- --ignored
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
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
             (1620000000,10, 2,'sdex',    5,5,5,5,             10, 50, 50, 5,   5,   1,1), \
             (1620000000,10, 1,'phoenix', 16.6667,16.6667,16.6667,16.6667, 5,83,25,5,16.6667,1,1), \
             (1620000000,30, 2,'soroswap',2,2,2,2,             3,  6,  6,  2,   2,   1,1), \
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
             (1620003600, 1, 2,'sdex', 0.31,0.31,0.31,0.31, 100,31,31,0.31,0.31,1,1), \
             (1620007200, 1, 2,'sdex', 0.32,0.32,0.32,0.32, 100,32,32,0.32,0.32,1,1)"
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
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
         reports 14, the OR REPLACE form is no longer load-bearing and the \
         comment in views.sql is wrong"
    );

    // The shipped form replaces it in place.
    prices_clickhouse::apply_sql(&client, &rewrite(prices_clickhouse::VIEWS_SQL, db))
        .await
        .unwrap();
    let after = columns().await;
    // 14 since task 0178 appended `method`; 13 before it, 6 in v1.
    assert_eq!(
        after.len(),
        14,
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
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
/// rewinds all six views to a one-column stub first and re-applies, which is the
/// actual production upgrade path — ch-prod-01 already holds all six.
///
/// The `IF NOT EXISTS` half is the control: it pins *why* the statement form is
/// load-bearing, so a revert to that form fails here rather than as a silent
/// no-op against prod.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
async fn views_sql_replaces_every_existing_view() {
    let db = "it_views_replace_all";
    let client = setup_scratch(db).await;

    // (view, a column that only the REAL definition has)
    let views = [
        ("usd_reference", "xlm_usd"),
        ("price_usd_series", "close_usd"),
        ("usd_reference_1h", "xlm_usd"),
        ("price_usd_series_1h", "close_usd"),
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

    // The shipped form replaces all six in place.
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
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
             (1620000000,10, 2,'sdex', 5,5,5,5,             10, 50, 50, 5,    5,    1,1), \
             (1620000000, 1, 2,'sdex', 0.30,0.30,0.30,0.30, 1000,300,300,0.30,0.30, 1,1), \
             (1620000000, 3, 2,'sdex', 0.97,0.97,0.97,0.97, 100, 97, 97, 0.97, 0.97, 1,1), \
             (1620000000,10, 3,'sdex', 5.15,5.15,5.15,5.15, 4,  20.6,20.6,5,  5.15, 1,1)"
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
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
             (1620000000,10, 2,'sdex', 5,5,5,5,             10, 50,  50,  5,    5,    1,1), \
             (1620000000, 2, 1,'sdex', 3.2,3.2,3.2,3.2,     50, 160, 52,  1.04, 3.2,  1,1)"
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
/// ⚠️ **CORRECTION (task 0172): fixture A does NOT "publish Decimal128::MIN".**
/// This comment claimed it did. Measured on the prod pin (26.3.10.60), the
/// `nullIf(sum(w), 0)` NULL fails the `CAST` to non-Nullable `Decimal(38,14)`
/// and the query RAISES `CANNOT_INSERT_NULL_IN_ORDINARY_COLUMN` (code 349) —
/// so it does not corrupt one row, it takes down `price_usd_series` for every
/// row in the result. Arm A filters only on `p.close_usd > 0`, never on
/// `volume_base > 0`, so any priced-but-zero-volume bucket can trigger it.
/// That is PRE-EXISTING and not peg-specific; fixing it means deciding whether
/// such a row should be omitted entirely — a change to the "misses are absent"
/// contract that needs BE input. Tracked separately; deliberately NOT fixed here.
///
/// ⚠️ This fixture used USDT until task 0172 removed USDT from the peg set
/// (it depegged in June 2022 and is now priced by measurement, not assumed to
/// be $1). USDT can no longer stand in for a peg asset here — with no
/// placeholder it lands in fixture A and this test fails with code 349 rather
/// than exercising the guard. USDC is now the only peg asset and the only
/// valid subject for this test.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
             (1620000000,10, 2,'sdex', 5,5,5,5,                7,35,35,5,5,1,1)"
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
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
             (1620000000,10, 2,'sdex', 5,5,5,5,             10,50,  50,  5,5,   1,1), \
             (1620000000,10, 3,'sdex', 5.15,5.15,5.15,5.15,  4,20.6,20.6,5,5.15,1,1)"
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
///      replace a measurement; this consumer chooses measured or nothing. The
///      fixture plants a wildly wrong pivot value at the end of the day — if it
///      leaked it would win case 2's "last observation" test.
///   5. **The grains compose.** The daily close equals the LAST hourly close of
///      the same day (task 0167's stated reason for a close rather than an
///      average). Both are the last observation in their span, so this holds by
///      construction — and breaks the moment someone reaches for an average.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
             (toDateTime('2026-03-01 00:00:00'),10,2,'sdex',5,5,5,5,10,50,50,5,5,1,1), \
             (toDateTime('2026-08-10 00:00:00'),10,2,'sdex',5,5,5,5,10,50,50,5,5,1,1), \
             (toDateTime('2026-08-11 00:00:00'),10,2,'sdex',5,5,5,5,10,50,50,5,5,1,1)"
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
             (toDateTime('2026-08-10 09:00:00'),10,2,'sdex',5,5,5,5,10,50,50,5,5,1,1), \
             (toDateTime('2026-08-10 23:00:00'),10,2,'sdex',5,5,5,5,10,50,50,5,5,1,1)"
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

/// Task 0168 — a measured rate of exactly `1.0000…` is still `oracle`.
///
/// This is the acceptance criterion that the fallback must be DISTINGUISHABLE
/// from a measurement that happens to land on par, and it is the one property
/// no value-based check can cover: both rows read `1`. If `method` ever went
/// away, or were derived from the value (`if(close_usd = 1, 'peg', …)`), this
/// test would be the only thing to notice — and the surface would have
/// reproduced the `close_usd = 0` defect class, one value meaning two things.
#[tokio::test]
#[ignore = "requires a local ClickHouse (cargo test -- --ignored)"]
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
             (toDateTime('2026-08-10 00:00:00'),10,2,'sdex',5,5,5,5,10,50,50,5,5,1,1), \
             (toDateTime('2026-08-11 00:00:00'),10,2,'sdex',5,5,5,5,10,50,50,5,5,1,1)"
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
