//! Task 0139 — on the real `init.sql`, two identities cannot share an
//! `asset_id`, so no join on it can fan out.
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-clickhouse --test asset_id_schema_it -- --ignored --test-threads=1
//!
//! The identities below are the pairs measured on one id in prod (4194 STW and
//! ARBRIDGE, …) plus a case-only pair. Under the old per-process counter they
//! could share an id; here ClickHouse derives each id from its identity.
//!
//! Each test owns a scratch database built from `INIT_SQL` and drops it.

use clickhouse::Client;
use prices_clickhouse::asset_id::fixture::{self, AssetFixture, assets_insert};
use prices_clickhouse::asset_id::id_of;
use prices_clickhouse::{CANDLE_COLUMNS, CANDLE_GRAINS, USDC_ISSUER};

const XLM: AssetFixture = AssetFixture::new("XLM", "native", "", "");
const USDC: AssetFixture = AssetFixture::new("USDC", "classic", USDC_ISSUER, "");

/// Identities that shared an id on prod (2026-08-06 sample), a case-only pair
/// under one issuer, and a Soroban token.
const IDENTITIES: [AssetFixture; 12] = [
    XLM,
    USDC,
    AssetFixture::new("usdc", "classic", USDC_ISSUER, ""),
    AssetFixture::new("STW", "classic", "GA2LHOPXZF", ""),
    AssetFixture::new("ARBRIDGE", "classic", "GBACKRJVX7", ""),
    AssetFixture::new("GESARA", "classic", "GGESARA", ""),
    AssetFixture::new("GL1", "classic", "GGL1", ""),
    AssetFixture::new("SPACEWALK", "classic", "GSPACEWALK", ""),
    AssetFixture::new("GIFT", "classic", "GGIFT", ""),
    AssetFixture::new("INSILVERMINE", "classic", "GINSILVER", ""),
    AssetFixture::new("NUTT", "classic", "GNUTT", ""),
    AssetFixture::new("", "soroban", "", "CTOKEN0139"),
];

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

async fn exec(c: &Client, sql: &str) {
    c.query(sql)
        .execute()
        .await
        .unwrap_or_else(|e| panic!("query failed: {e}\n{sql}"));
}

async fn scratch(name: &str, views: bool) -> (Client, String) {
    let db = format!("it_asset_id_schema_{name}");
    let c = Client::default().with_url(ch_url());
    exec(&c, &format!("DROP DATABASE IF EXISTS {db} SYNC")).await;
    exec(&c, &format!("CREATE DATABASE {db}")).await;
    prices_clickhouse::apply_sql(&c, &rewrite(prices_clickhouse::INIT_SQL, &db))
        .await
        .expect("apply init schema");
    if views {
        prices_clickhouse::apply_sql(&c, &rewrite(prices_clickhouse::VIEWS_SQL, &db))
            .await
            .expect("apply views");
    }
    (c, db)
}

async fn drop_db(c: &Client, db: &str) {
    exec(c, &format!("DROP DATABASE IF EXISTS {db} SYNC")).await;
}

async fn count(c: &Client, sql: &str) -> u64 {
    c.query(sql)
        .fetch_one::<u64>()
        .await
        .unwrap_or_else(|e| panic!("{e}\n{sql}"))
}

/// `sql` must be refused with `want` in the server's message.
async fn assert_refused(c: &Client, sql: &str, want: &str) {
    let err = c
        .query(sql)
        .execute()
        .await
        .expect_err(&format!("must be refused: {sql}"));
    assert!(err.to_string().contains(want), "want {want}, got: {err}");
}

/// `(rows, distinct ids)` of `assets FINAL`.
async fn rows_and_ids(c: &Client, db: &str) -> (u64, u64) {
    c.query(&format!(
        "SELECT count(), uniqExact(asset_id) FROM {db}.assets FINAL"
    ))
    .fetch_one()
    .await
    .unwrap()
}

/// The lore AC: `count() = countDistinct(asset_id)` on `assets FINAL`, for
/// identities that collided under the counter, and still after a re-emit.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn identities_that_once_collided_get_one_id_each() {
    let (c, db) = scratch("unique", false).await;
    exec(&c, &assets_insert(&db, &IDENTITIES)).await;
    // A second writer re-emits every row: same identity, same id.
    exec(&c, &assets_insert(&db, &IDENTITIES)).await;

    let n = IDENTITIES.len() as u64;
    assert_eq!(rows_and_ids(&c, &db).await, (n, n));
    assert_eq!(
        count(&c, &format!("SELECT uniqExact(asset_id) FROM {db}.assets")).await,
        n,
        "a re-emitted row carries the id its identity always had"
    );
    let want = fixture::fetch_ids(&c, &IDENTITIES).await;
    for (fx, want) in IDENTITIES.iter().zip(want) {
        let got = count(
            &c,
            &format!(
                "SELECT asset_id FROM {db}.assets FINAL WHERE asset_code = '{}' \
                 AND issuer_address = '{}' AND contract_address = '{}'",
                fx.code, fx.issuer, fx.contract
            ),
        )
        .await;
        assert_eq!(got, want, "{} keeps its derived id", fx.code);
    }

    drop_db(&c, &db).await;
}

/// The fan-out this task is named for: `current_price_usd` joins
/// `current_prices` to `assets FINAL` on `asset_id`, so it returns exactly one
/// row per `current_prices` row once no id serves two identities.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn current_price_usd_returns_one_row_per_current_prices_row() {
    let (c, db) = scratch("current", true).await;
    exec(&c, &assets_insert(&db, &IDENTITIES)).await;
    let rows: Vec<String> = IDENTITIES
        .iter()
        .map(|fx| format!("({fx}, 1, 0.1, 0, 0, 10, 0, 1, '')"))
        .collect();
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.current_prices \
             (asset_id, price_usd, price_xlm, change_24h_pct, change_7d_pct, \
              volume_24h_usd, market_cap_usd, vwap_24h, sources) VALUES {}",
            rows.join(", ")
        ),
    )
    .await;

    let current = count(
        &c,
        &format!("SELECT count() FROM {db}.current_prices FINAL"),
    )
    .await;
    assert_eq!(current, IDENTITIES.len() as u64);
    assert_eq!(
        count(&c, &format!("SELECT count() FROM {db}.current_price_usd")).await,
        current,
        "one view row per current_prices row"
    );
    assert_eq!(
        count(
            &c,
            &format!(
                "SELECT count() FROM (SELECT asset_kind, asset_code, issuer_address, \
                 contract_address FROM {db}.current_price_usd \
                 GROUP BY ALL HAVING count() > 1)"
            )
        )
        .await,
        0,
        "no identity is published twice"
    );
    assert_eq!(
        count(
            &c,
            &format!("SELECT toUInt64(sum(volume_24h_usd)) FROM {db}.current_price_usd")
        )
        .await,
        10 * IDENTITIES.len() as u64,
        "volume_24h_usd is not inflated by duplicated rows"
    );

    drop_db(&c, &db).await;
}

/// `assets.asset_id` is MATERIALIZED: a writer cannot choose an id, so it
/// cannot put a second identity on an existing one.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn assets_refuses_a_writer_that_names_asset_id() {
    let (c, db) = scratch("named", false).await;
    exec(&c, &assets_insert(&db, &[USDC])).await;
    assert_refused(
        &c,
        &format!(
            "INSERT INTO {db}.assets (asset_id, asset_code, asset_type, issuer_address) \
             VALUES ({USDC}, 'ARBRIDGE', 'classic', 'GBACKRJVX7')"
        ),
        "asset_id",
    )
    .await;
    assert_eq!(rows_and_ids(&c, &db).await, (1, 1), "nothing stored");
    // A blank identity would get the id CHECK refuses.
    assert_refused(
        &c,
        &format!("INSERT INTO {db}.assets (asset_code, asset_type) VALUES ('', 'classic')"),
        "Code: 469",
    )
    .await;

    drop_db(&c, &db).await;
}

/// The candle ids the writer gets by sending identities are the `assets` ids,
/// so a candle joins its asset. A candle with no identity, or with id 0, is
/// refused by the CHECK in every tier.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn candle_ids_come_from_identities_and_a_blank_identity_is_refused_in_every_tier() {
    let (c, db) = scratch("candles", false).await;
    exec(&c, &assets_insert(&db, &IDENTITIES)).await;
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, base_code, base_issuer, base_contract, \
              quote_code, quote_issuer, quote_contract, source, \
              open, high, low, close, vwap, trade_count, version) VALUES \
             (1790000000, 'STW', 'GA2LHOPXZF', '', 'XLM', '', '', 'sdex', 1, 1, 1, 1, 1, 1, 1), \
             (1790000000, 'ARBRIDGE', 'GBACKRJVX7', '', 'XLM', '', '', 'sdex', 2, 2, 2, 2, 2, 1, 1)"
        ),
    )
    .await;
    let joined: Vec<(String, String)> = c
        .query(&format!(
            "SELECT b.asset_code, q.asset_code FROM {db}.price_ohlcv_1m AS p FINAL \
             INNER JOIN {db}.assets AS b FINAL ON b.asset_id = p.asset_id \
             INNER JOIN {db}.assets AS q FINAL ON q.asset_id = p.quote_asset_id \
             ORDER BY p.close"
        ))
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(
        joined,
        [
            ("STW".into(), "XLM".into()),
            ("ARBRIDGE".into(), "XLM".into())
        ],
        "each candle joins its own asset, once"
    );

    for grain in CANDLE_GRAINS {
        let table = format!("{db}.price_ohlcv_{grain}");
        assert_refused(
            &c,
            &format!(
                "INSERT INTO {table} (timestamp, quote_code, source, close, vwap, version) \
                 VALUES (1790000000, 'XLM', 'sdex', 1, 1, 1)"
            ),
            "Code: 469",
        )
        .await;
        assert_refused(
            &c,
            &format!(
                "INSERT INTO {table} (timestamp, asset_id, quote_asset_id, source, close, vwap, version) \
                 VALUES (1790000000, 0, {XLM}, 'sdex', 1, 1, 1)"
            ),
            "Code: 469",
        )
        .await;
    }

    drop_db(&c, &db).await;
}

/// The rollup MVs and enrichment copy ids between tiers with `INSERT … SELECT`
/// naming them. The ids are DEFAULT, not MATERIALIZED, so that works in every
/// tier and keeps the ids it was given.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_insert_select_naming_the_ids_works_in_every_tier() {
    let (c, db) = scratch("copy", false).await;
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.price_ohlcv_1m \
             (timestamp, base_code, base_issuer, base_contract, \
              quote_code, quote_issuer, quote_contract, source, \
              open, high, low, close, vwap, trade_count, version) VALUES \
             (1790000000, 'USDC', '{USDC_ISSUER}', '', 'XLM', '', '', 'sdex', 1, 1, 1, 1, 1, 1, 1)"
        ),
    )
    .await;
    let cols = CANDLE_COLUMNS.join(", ");
    let pair = format!(
        "asset_id = {} AND quote_asset_id = {}",
        id_of("USDC", USDC_ISSUER, ""),
        id_of("XLM", "", "")
    );
    for grain in CANDLE_GRAINS.iter().skip(1) {
        exec(
            &c,
            &format!(
                "INSERT INTO {db}.price_ohlcv_{grain} ({cols}) \
                 SELECT {cols} FROM {db}.price_ohlcv_1m FINAL"
            ),
        )
        .await;
        let same = count(
            &c,
            &format!("SELECT count() FROM {db}.price_ohlcv_{grain} WHERE {pair}"),
        )
        .await;
        assert_eq!(same, 1, "price_ohlcv_{grain} keeps the copied ids");
    }

    drop_db(&c, &db).await;
}

/// `oracle_prices` derives its id from the sampled identity, and a feed with no
/// asset (REDSTONE) sends a blank identity, stored under the sentinel 0.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_oracle_row_with_a_blank_identity_stores_the_sentinel_zero() {
    let (c, db) = scratch("oracle", false).await;
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.oracle_prices \
             (timestamp, asset_code, issuer_address, contract_address, oracle_name, price_usd, raw_data) \
             VALUES (1790000000, 'XLM', '', '', 'reflector', 0.1, ''), \
                    (1790000000, '', '', '', 'redstone', 0, '')"
        ),
    )
    .await;
    let ids: Vec<(String, u64)> = c
        .query(&format!(
            "SELECT oracle_name, asset_id FROM {db}.oracle_prices ORDER BY oracle_name"
        ))
        .fetch_all()
        .await
        .unwrap();
    let xlm = fixture::fetch_id(&c, "XLM", "", "").await;
    assert_eq!(
        ids,
        [("redstone".into(), 0), ("reflector".into(), xlm)],
        "blank identity → 0; XLM → its derived id"
    );

    drop_db(&c, &db).await;
}
