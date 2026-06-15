//! Live-ClickHouse integration test for the read-surface views
//! (`price_usd_series`, `usd_reference`). Gated `#[ignore]`:
//!
//!   docker compose up -d clickhouse
//!   cargo test -p prices-clickhouse --test views_it -- --ignored
//!
//! Owns an isolated scratch database (the `prices.*` schema + views rewritten
//! onto the scratch name) and drops it at the end.

use clickhouse::Client;

const USDC_ISSUER: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";

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
    // 30=pure soroban contract token.
    client
        .query(&format!(
            "INSERT INTO {db}.assets \
             (asset_id, asset_code, asset_type, issuer_address, contract_address, sac_address) VALUES \
             (1,'XLM','classic','','','CXLMSAC'), (2,'USDC','classic','{USDC_ISSUER}','',''), \
             (10,'FOO','classic','GFOO','',''), (20,'EXO','classic','GEXO','',''), \
             (30,'','soroban','','CTOKEN7XYZ','')"
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
        "soroban token"
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
    client
        .query(&format!(
            "INSERT INTO {db}.current_prices (asset_id, price_usd, updated_at) \
             VALUES (1, 0.1600, toDateTime(1620100000))"
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

    client
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
