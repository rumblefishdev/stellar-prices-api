//! Live-ClickHouse integration test for the read-surface views
//! (`price_usd_series`, `usd_reference`). Gated `#[ignore]`:
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-clickhouse --test views_it -- --ignored
//!
//! Owns an isolated scratch database (the `prices.*` schema + views rewritten
//! onto the scratch name) and drops it at the end.

use clickhouse::Client;
use prices_clickhouse::USDC_ISSUER;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
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

    // EXO only appears as an unpriced quote leg → not a priced row.
    let priced_assets: u64 = client
        .query(&format!("SELECT count() FROM {db}.price_usd_series"))
        .fetch_one::<u64>()
        .await
        .unwrap();
    assert_eq!(priced_assets, 3, "only XLM, FOO, CTOKEN are priced");

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
