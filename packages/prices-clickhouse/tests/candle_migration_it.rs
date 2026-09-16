//! Task 0286 / ADR 0287 — the pf-column migration, against a real ClickHouse.
//!
//!     cargo test -p prices-clickhouse --test candle_migration_it -- --ignored
//!
//! `init.sql` is applied to databases that already hold billions of pre-0286
//! candle rows. The three price-forming columns are added with DEFAULT
//! expressions over the columns those rows already carry, so an untouched
//! fifteen-column part keeps reading with its OLD meaning (every fill was
//! price-forming) instead of reading zero — a re-ingest is what changes it
//! (0286 phase 3), not the ALTER.
//!
//! What this pins, none of which the unit tests in `src/lib.rs` can see:
//!   - the DEFAULT expressions are actually computed on read for old parts
//!     (verified fact F6a, here re-proved against the shipped DDL),
//!   - all SEVEN candle tables end up with the columns — a `CREATE … AS` copy
//!     made from a pre-0286 base table does NOT inherit the base's later ALTER,
//!   - `INIT_SQL` stays idempotent: applying it twice errors nowhere and
//!     changes no value.
//!
//! Owns an isolated scratch database and drops it at the end — the real
//! `prices` database is never touched.

use clickhouse::Client;

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Rewrite the `prices.*` schema onto an isolated scratch database name (same
/// trick as `rollup_chain_it.rs` / `views_it.rs`).
fn rewrite(sql: &str, db: &str) -> String {
    sql.replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"))
}

/// The pre-0286 `price_ohlcv_1m`, by hand: exactly the fifteen columns the
/// table carried before this task, in their shipped order. Written out rather
/// than derived from `INIT_SQL` on purpose — the point is to reproduce a
/// database provisioned by an OLDER build, which is what production is.
fn legacy_1m_create(db: &str) -> String {
    format!(
        "CREATE TABLE {db}.price_ohlcv_1m ( \
            timestamp        DateTime      CODEC(DoubleDelta), \
            asset_id         UInt32, \
            quote_asset_id   UInt32, \
            source           LowCardinality(String), \
            open             Decimal(38, 14), \
            high             Decimal(38, 14), \
            low              Decimal(38, 14), \
            close            Decimal(38, 14), \
            volume_base      Decimal(38, 14) DEFAULT 0, \
            volume_quote     Decimal(38, 14) DEFAULT 0, \
            volume_quote_usd Decimal(38, 14) DEFAULT 0, \
            close_usd        Decimal(38, 14) DEFAULT 0, \
            vwap             Decimal(38, 14), \
            trade_count      UInt32        DEFAULT 0, \
            version          UInt64 \
        ) \
        ENGINE = ReplacingMergeTree(version) \
        PARTITION BY toYYYYMM(timestamp) \
        ORDER BY (asset_id, quote_asset_id, source, timestamp)"
    )
}

/// The one legacy row. The three source values are DISTINCT (4 / 10 / 12) so a
/// DEFAULT wired to the wrong column fails instead of coincidentally matching.
fn insert_legacy_row(db: &str) -> String {
    format!(
        "INSERT INTO {db}.price_ohlcv_1m \
         (timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
          volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) \
         VALUES (toDateTime(1700000000), 1, 2, 'sdex', 1.0, 1.5, 0.9, 1.1, \
                 10, 12, 0, 0, 1.2, 4, 100000)"
    )
}

async fn pf_row(client: &Client, db: &str) -> (u32, f64, f64) {
    client
        .query(&format!(
            "SELECT pf_trade_count, toFloat64(pf_volume), toFloat64(pf_price_volume) \
             FROM {db}.price_ohlcv_1m FINAL"
        ))
        .fetch_one()
        .await
        .expect("read pf columns")
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn pf_columns_migrate_every_candle_table_and_preserve_old_rows() {
    let db = "it_candle_migration";
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

    // A pre-0286 database: the fifteen-column _1m table with one row in it.
    admin
        .query(&legacy_1m_create(db))
        .execute()
        .await
        .expect("create legacy _1m");
    admin
        .query(&insert_legacy_row(db))
        .execute()
        .await
        .expect("insert legacy row");

    // Apply the shipped schema over it — the migration under test. The coarse
    // tables are created here, as `AS` copies of the LEGACY base table, which is
    // exactly why every grain needs its own ALTER.
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("apply init schema over a pre-0286 database");

    // The old row keeps its old meaning through the DEFAULT expressions: every
    // fill of that minute counted as price-forming, so pf_trade_count reads
    // trade_count (4), pf_volume reads volume_base (10) and pf_price_volume
    // reads volume_quote (12) — Σ price × base volume for a single-price bucket.
    assert_eq!(
        pf_row(&admin, db).await,
        (4, 10.0, 12.0),
        "a pre-0286 part must read its pf columns from the DEFAULT expressions"
    );

    // All seven grains carry all three columns, with the DEFAULT expressions
    // intact — a column added without its DEFAULT would read 0 on old parts.
    for grain in prices_clickhouse::CANDLE_GRAINS {
        let table = format!("price_ohlcv_{grain}");
        for (col, want_default) in [
            ("pf_trade_count", "trade_count"),
            ("pf_volume", "volume_base"),
            ("pf_price_volume", "volume_quote"),
        ] {
            let got: String = admin
                .query(&format!(
                    "SELECT default_expression FROM system.columns \
                     WHERE database = '{db}' AND table = '{table}' AND name = '{col}'"
                ))
                .fetch_optional::<String>()
                .await
                .unwrap_or_else(|e| panic!("{table}.{col}: {e}"))
                .unwrap_or_else(|| panic!("{table} is missing column {col}"));
            assert_eq!(got, want_default, "{table}.{col} DEFAULT expression");
        }
    }

    // Idempotent: re-applying errors nowhere (IF NOT EXISTS on every clause)
    // and changes no value.
    prices_clickhouse::apply_sql(&admin, &rewrite(prices_clickhouse::INIT_SQL, db))
        .await
        .expect("re-apply init schema");
    assert_eq!(
        pf_row(&admin, db).await,
        (4, 10.0, 12.0),
        "re-applying INIT_SQL must not change a migrated row"
    );

    admin
        .query(&format!("DROP DATABASE {db}"))
        .execute()
        .await
        .unwrap();
}
