//! Task 0139 — the rekey tool's window steps, end to end, on scratch
//! databases: the frozen pre-0139 id tables (`fixtures/pre0139_id_tables.sql`),
//! this build's other tables, the six rollup MVs, `mv_current_prices` and the
//! views, all on UInt32 ids. Needs `system.query_log` (fill reads it).
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-clickhouse --test rekey_swap_it -- --ignored --test-threads=1

use std::time::Duration;

use clickhouse::Client;
use prices_clickhouse::asset_id::fixture::fetch_id;
use prices_clickhouse::rekey::gap::{Postcheck, postcheck_sql};
use prices_clickhouse::rekey::swap::{FORCE_LOSE, MvSource};
use prices_clickhouse::rekey::{
    COPIED_TABLES, DDL_TABLE, Fill, LOG_TABLE, MAP_FINAL, MAP_TABLE, Rekey, RekeyError,
};
use prices_clickhouse::rollup_sql::{TIERS, mv_ddl};
use prices_clickhouse::{CURRENT_SQL, INIT_SQL, USDC_ISSUER, VIEWS_SQL};

const PRE0139: &str = include_str!("fixtures/pre0139_id_tables.sql");

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

async fn exec(c: &Client, sql: &str) {
    c.query(sql)
        .execute()
        .await
        .unwrap_or_else(|e| panic!("query failed: {e}\n{sql}"));
}

async fn count(c: &Client, sql: &str) -> u64 {
    c.query(sql)
        .fetch_one()
        .await
        .unwrap_or_else(|e| panic!("query failed: {e}\n{sql}"))
}

fn refused(r: Result<impl std::fmt::Debug, RekeyError>, why: &str) {
    match r {
        Err(RekeyError::Refused(msg)) => assert!(msg.contains(why), "{why} not in: {msg}"),
        other => panic!("expected a refusal ({why}), got {other:?}"),
    }
}

fn gate_failed(r: Result<impl std::fmt::Debug, RekeyError>, gate: &str) {
    match r {
        Err(RekeyError::Gate(msg)) => assert!(msg.contains(gate), "{gate} not in: {msg}"),
        other => panic!("expected gate `{gate}` to fail, got {other:?}"),
    }
}

/// The `INIT_SQL` statements of the tables without asset ids, in `db`.
fn other_tables(db: &str) -> Vec<String> {
    let ids: Vec<&str> = std::iter::once("assets").chain(COPIED_TABLES).collect();
    let stripped: String = INIT_SQL
        .lines()
        .map(|l| l.split("--").next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    stripped
        .split(';')
        .map(str::trim)
        .filter(|s| {
            s.strip_prefix("CREATE TABLE IF NOT EXISTS prices.")
                .or_else(|| s.strip_prefix("ALTER TABLE prices."))
                .and_then(|r| {
                    r.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                        .next()
                })
                .is_some_and(|t| !ids.contains(&t))
        })
        .map(|s| s.replace("prices.", &format!("{db}.")))
        .collect()
}

/// Unix seconds of the last live 1m candle: five hours ago, on the minute.
async fn last_live(c: &Client) -> u32 {
    c.query("SELECT toUInt32(toStartOfMinute(now() - INTERVAL 5 HOUR))")
        .fetch_one()
        .await
        .unwrap()
}

/// A pre-0139 world in `it_rkswap_<name>`. Returns the last live 1m time.
///
/// Ids: 1 XLM, 2 USDC (canonical issuer), 4 STW + ARBRIDGE (colliding), 5 and
/// 6 FOO (one identity), 77 an orphan, 0 the REDSTONE sentinel.
async fn world(name: &str) -> (Client, String, u32) {
    world_with(name, true).await
}

/// [`world`], with or without its MVs and views.
async fn world_with(name: &str, views: bool) -> (Client, String, u32) {
    let db = format!("it_rkswap_{name}");
    let c = Client::default().with_url(ch_url());
    exec(&c, &format!("DROP DATABASE IF EXISTS {db} SYNC")).await;
    exec(&c, &format!("CREATE DATABASE {db}")).await;
    prices_clickhouse::apply_sql(&c, &PRE0139.replace("prices.", &format!("{db}.")))
        .await
        .expect("apply the pre-0139 DDL");
    for s in other_tables(&db) {
        exec(&c, &s).await;
    }
    // Raw duplicates must survive; alter-assets has to restart these merges.
    exec(&c, &format!("SYSTEM STOP MERGES {db}.assets")).await;
    let assets = "(asset_id, asset_code, asset_type, issuer_address, contract_address)";
    for v in [
        "(1, 'XLM', 'native', '', '')".to_string(),
        "(1, 'XLM', 'native', '', '')".to_string(),
        format!("(2, 'USDC', 'classic', '{USDC_ISSUER}', '')"),
        "(4, 'STW', 'classic', 'GSTW', '')".to_string(),
        "(4, 'ARBRIDGE', 'classic', 'GARB', '')".to_string(),
        "(5, 'FOO', 'classic', 'GFOO', '')".to_string(),
        "(6, 'FOO', 'classic', 'GFOO', '')".to_string(),
    ] {
        exec(&c, &format!("INSERT INTO {db}.assets {assets} VALUES {v}")).await;
    }
    let l = last_live(&c).await;
    let candle = "(timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  volume_base, volume_quote, vwap, trade_count, version)";
    let rows = [
        ("price_ohlcv_1m", format!("toDateTime({l})"), 1, 2),
        (
            "price_ohlcv_1m",
            format!("toDateTime({l}) - INTERVAL 10 MINUTE"),
            5,
            1,
        ),
        (
            "price_ohlcv_1m",
            format!("toDateTime({l}) - INTERVAL 20 MINUTE"),
            6,
            1,
        ),
        (
            "price_ohlcv_1m",
            format!("toDateTime({l}) - INTERVAL 60 MINUTE"),
            4,
            1,
        ), // blend
        (
            "price_ohlcv_1m",
            format!("toDateTime({l}) - INTERVAL 30 MINUTE"),
            77,
            1,
        ), // orphan
        (
            "price_ohlcv_1m",
            "toDateTime('2024-01-01 00:00:00')".into(),
            1,
            2,
        ),
        (
            "price_ohlcv_1m",
            "toDateTime('2024-01-01 00:00:00')".into(),
            4,
            1,
        ),
        (
            "price_ohlcv_15m",
            "toDateTime('2024-01-01 00:00:00')".into(),
            1,
            2,
        ),
        (
            "price_ohlcv_1h",
            "toDateTime('2025-01-01 00:00:00')".into(),
            1,
            2,
        ),
        (
            "price_ohlcv_1h",
            "toDateTime('2025-01-01 00:00:00')".into(),
            5,
            1,
        ),
        (
            "price_ohlcv_1h",
            "toDateTime('2025-01-01 00:00:00')".into(),
            1,
            4,
        ), // colliding quote
        (
            "price_ohlcv_1h",
            format!("toStartOfHour(toDateTime({l}))"),
            1,
            2,
        ),
        (
            "price_ohlcv_4h",
            "toDateTime('2024-01-01 00:00:00')".into(),
            1,
            2,
        ),
        (
            "price_ohlcv_1d",
            "toDateTime('2024-01-01 00:00:00')".into(),
            1,
            2,
        ),
        (
            "price_ohlcv_1d",
            "toDateTime('2024-01-01 00:00:00')".into(),
            4,
            2,
        ),
        ("price_ohlcv_1w", "toDate('2024-01-01')".into(), 1, 2),
        ("price_ohlcv_1M", "toDate('2024-01-01')".into(), 1, 2),
    ];
    for (t, ts, base, quote) in rows {
        exec(
            &c,
            &format!(
                "INSERT INTO {db}.{t} {candle} SELECT {ts}, {base}, {quote}, 'sdex', \
                 0.1, 0.1, 0.1, 0.1, 10, 1, 0.1, 1, 1"
            ),
        )
        .await;
    }
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.oracle_prices (timestamp, asset_id, oracle_name, price_usd, \
             raw_data) VALUES ('2024-01-01 00:00:00', 0, 'REDSTONE', 1, ''), \
             ('2024-01-01 00:00:00', 1, 'REFLECTOR', 1, ''), \
             ('2024-01-01 00:00:00', 4, 'REFLECTOR', 1, '')"
        ),
    )
    .await;
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.current_prices (asset_id, price_usd, price_xlm, change_24h_pct, \
             change_7d_pct, volume_24h_usd, market_cap_usd, vwap_24h, sources) VALUES \
             (1, 1, 1, 0, 0, 0, 0, 0, ''), (4, 1, 1, 0, 0, 0, 0, 0, '')"
        ),
    )
    .await;
    exec(
        &c,
        &format!("INSERT INTO {db}.asset_supply (asset_id, token_supply) VALUES (2, 1), (77, 1)"),
    )
    .await;
    exec(
        &c,
        &format!("INSERT INTO {db}.asset_metadata (asset_id, home_domain) VALUES (2, 'x.org')"),
    )
    .await;
    if !views {
        return (c, db, l);
    }
    for t in TIERS {
        exec(&c, &mv_ddl(&t, &db).unwrap()).await;
    }
    prices_clickhouse::apply_sql(&c, &CURRENT_SQL.replace("prices.", &format!("{db}.")))
        .await
        .expect("apply current.sql");
    prices_clickhouse::apply_sql(&c, &VIEWS_SQL.replace("prices.", &format!("{db}.")))
        .await
        .expect("apply views.sql");
    (c, db, l)
}

async fn drop_db(c: &Client, db: &str) {
    exec(c, &format!("DROP DATABASE IF EXISTS {db} SYNC")).await;
}

/// W5: `SYSTEM STOP VIEW` for every refreshable view.
async fn stop_views(c: &Client, db: &str) {
    let views: Vec<String> = c
        .query("SELECT view FROM system.view_refreshes WHERE database = ?")
        .bind(db)
        .fetch_all()
        .await
        .unwrap();
    for v in views {
        exec(c, &format!("SYSTEM STOP VIEW {db}.{v}")).await;
    }
}

fn all_fills() -> Vec<Fill> {
    COPIED_TABLES.iter().map(|t| Fill::table(t)).collect()
}

/// W5 → W10: stop, map, create, fill, check, alter-assets, swap, recreate.
async fn migrate(c: &Client, db: &str, source: MvSource) -> Rekey {
    migrate_from(c, db, source, None).await
}

/// [`migrate`], capturing the MVs and views of `capture_from` (rehearsal).
async fn migrate_from(c: &Client, db: &str, source: MvSource, capture_from: Option<&str>) -> Rekey {
    let r = Rekey::new(c.clone(), db, true);
    r.capture(capture_from).await.unwrap();
    stop_views(c, db).await;
    refused(
        r.alter_assets(Duration::from_secs(60)).await,
        "check is not green",
    );
    r.map().await.unwrap();
    r.create().await.unwrap();
    for f in all_fills() {
        r.fill(&f).await.unwrap();
    }
    r.check(&[Fill::table("price_ohlcv_1d")]).await.unwrap();
    refused(
        r.alter_assets(Duration::from_secs(60)).await,
        "last check did not cover price_ohlcv_1m",
    );
    r.check(&all_fills()).await.unwrap();
    refused(r.swap(false).await, "run alter-assets first");
    r.alter_assets(Duration::from_secs(60)).await.unwrap();
    refused(r.recreate_mvs(source).await, "swap has not run");
    r.swap(false).await.unwrap();
    r.recreate_mvs(source).await.unwrap();
    r
}

/// `(table, column, type)` of every id column outside the old id space.
async fn id_columns(c: &Client, db: &str) -> Vec<(String, String, String)> {
    c.query(
        "SELECT table, name, type FROM system.columns WHERE database = ? \
         AND name IN ('asset_id', 'quote_asset_id') AND NOT endsWith(table, 'pre0139') \
         ORDER BY table, name",
    )
    .bind(db)
    .fetch_all()
    .await
    .unwrap()
}

/// Refresh every MV of `db` once, dependencies first.
async fn refresh_all(c: &Client, db: &str) {
    for mv in TIERS.iter().map(|t| t.mv).chain(["mv_current_prices"]) {
        refresh(c, db, mv).await;
    }
}

async fn refresh(c: &Client, db: &str, mv: &str) {
    exec(c, &format!("SYSTEM REFRESH VIEW {db}.{mv}")).await;
    exec(c, &format!("SYSTEM WAIT VIEW {db}.{mv}")).await;
}

async fn end_to_end(source: MvSource, name: &str) {
    let (c, db, l) = world(name).await;
    refused(
        Rekey::new(c.clone(), &db, true)
            .capture(None)
            .await
            .and(Ok(()))
            .and(
                Rekey::new(c.clone(), &db, true)
                    .swap(false)
                    .await
                    .map(|_| ()),
            ),
        "not Disabled (SYSTEM STOP VIEW)",
    );
    let r = migrate(&c, &db, source).await;

    let ids = id_columns(&c, &db).await;
    assert!(ids.len() >= 12 + 18, "tables and MVs: {ids:?}");
    assert!(
        ids.iter()
            .all(|(_, _, t)| t == "UInt64" || t == "Nullable(UInt64)"),
        "{ids:?}"
    );
    for mv in TIERS.iter().map(|t| t.mv).chain(["mv_current_prices"]) {
        assert!(
            ids.iter().any(|(t, _, _)| t == mv),
            "{mv} declares UInt64 ids"
        );
    }
    let pre: u64 = count(
        &c,
        &format!(
            "SELECT count() FROM system.columns WHERE database = '{db}' \
             AND endsWith(table, '__pre0139') AND name = 'asset_id' AND type = 'UInt32'"
        ),
    )
    .await;
    assert_eq!(
        pre, 12,
        "11 swapped-out tables and assets__pre0139 keep UInt32"
    );

    assert_eq!(
        count(&c, &format!("SELECT count() FROM {db}.assets FINAL")).await,
        count(
            &c,
            &format!("SELECT uniqExact(asset_id) FROM {db}.assets FINAL")
        )
        .await,
    );
    refresh(&c, &db, "mv_current_prices").await;
    let current = count(
        &c,
        &format!("SELECT count() FROM {db}.current_prices FINAL"),
    )
    .await;
    assert!(current > 0);
    assert_eq!(
        count(&c, &format!("SELECT count() FROM {db}.current_price_usd")).await,
        current
    );
    let (from, _): (u32, u32) = c
        .query(&format!(
            "SELECT toUInt32(range_from), toUInt32(range_to) FROM {db}.{LOG_TABLE} \
             WHERE step = 'swap' AND status = 'ok'"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(
        from, l,
        "last_live_1m_ts is the outgoing 1m table's newest row"
    );
    let xlm = fetch_id(&c, "XLM", "", "").await;
    assert_eq!(
        count(
            &c,
            &format!("SELECT count() FROM {db}.price_ohlcv_1m WHERE asset_id = {xlm}")
        )
        .await,
        2
    );

    refresh_all(&c, &db).await;
    let lines = r.verify().await.unwrap();
    assert!(
        lines.iter().any(|l| l == "cross_check_0129: 1"),
        "{lines:?}"
    );
    refused(r.gap_backfill(None).await, "older than 15 minutes");

    refused(r.map().await, MAP_FINAL);
    refused(
        r.fill(&Fill::table("price_ohlcv_1m")).await,
        "already in the new id space",
    );
    refused(r.swap(false).await, "swap already done");
    refused(r.capture(None).await, "swap has run");

    // One MV left on its captured UInt32 text fails the type gate.
    let text: String = c
        .query(&format!(
            "SELECT create_query FROM {db}.{DDL_TABLE} WHERE name = 'mv_ohlcv_1h_to_4h'"
        ))
        .fetch_one()
        .await
        .unwrap();
    exec(&c, &format!("DROP VIEW {db}.mv_ohlcv_1h_to_4h SYNC")).await;
    exec(&c, &text).await;
    gate_failed(r.type_gate().await, "mv_ohlcv_1h_to_4h.asset_id is UInt32");

    // An id assets lacks fails verify, and the post-check that sees it.
    exec(&c, &format!("SYSTEM STOP VIEW {db}.mv_current_prices")).await;
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.current_prices (asset_id, price_usd, price_xlm, change_24h_pct, \
             change_7d_pct, volume_24h_usd, market_cap_usd, vwap_24h, sources) VALUES \
             (77, 1, 1, 0, 0, 0, 0, 0, '')"
        ),
    )
    .await;
    let err = format!("{:?}", r.verify().await);
    for want in [
        "current_prices.asset_id: 1 rows on an id assets lacks",
        "current_price_usd_one_row",
        "mv_ohlcv_1h_to_4h.asset_id is UInt32",
    ] {
        assert!(err.contains(want), "{want} not in {err}");
    }
    drop_db(&c, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_window_migrates_end_to_end_with_prod_text_mvs() {
    end_to_end(MvSource::ProdText, "prod_text").await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_window_migrates_end_to_end_with_generator_mvs() {
    end_to_end(MvSource::Generator, "generator").await;
}

/// `(table, rows)`: raw rows of 1m (no MV writes it), FINAL rows of the rest
/// (an MV re-created on rollback may re-append a bucket, and the restored
/// `assets` merges again).
async fn counts(c: &Client, db: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for t in std::iter::once("assets").chain(COPIED_TABLES) {
        let fin = if t == "price_ohlcv_1m" { "" } else { " FINAL" };
        out.push((
            t.to_string(),
            count(c, &format!("SELECT count() FROM {db}.{t}{fin}")).await,
        ));
    }
    out
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn rollback_restores_the_tables_and_mvs_and_guards_post_swap_rows() {
    let (c, db, _) = world("rollback").await;
    stop_views(&c, &db).await;
    let before = counts(&c, &db).await;
    let mvs = |c: Client, db: String| async move {
        c.query(
            "SELECT name, create_table_query FROM system.tables WHERE database = ? \
             AND engine = 'MaterializedView' ORDER BY name",
        )
        .bind(db)
        .fetch_all::<(String, String)>()
        .await
        .unwrap()
    };
    let mv_texts = mvs(c.clone(), db.clone()).await;
    let r = migrate(&c, &db, MvSource::Generator).await;

    // A writer resumed: one candle and one new identity after the swap.
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.assets (asset_code, asset_type, issuer_address, contract_address) \
             VALUES ('NEW', 'classic', 'GNEW', '')"
        ),
    )
    .await;
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.price_ohlcv_1m (timestamp, base_code, base_issuer, base_contract, \
             quote_code, quote_issuer, quote_contract, source, open, high, low, close, vwap, \
             trade_count, version) SELECT now(), 'NEW', 'GNEW', '', 'XLM', '', '', 'sdex', \
             1, 1, 1, 1, 1, 1, 1"
        ),
    )
    .await;
    refused(r.rollback(false).await, FORCE_LOSE);
    let lines = r.rollback(true).await.unwrap();
    assert!(lines.contains(&"assets: restored".to_string()), "{lines:?}");

    assert_eq!(counts(&c, &db).await, before, "the original rows, by count");
    let types: Vec<String> = c
        .query(
            "SELECT DISTINCT replaceAll(replaceAll(type, 'Nullable(', ''), ')', '') FROM system.columns WHERE database = ? \
             AND name IN ('asset_id', 'quote_asset_id') AND (table IN ('assets', 'price_ohlcv_1m', \
             'price_ohlcv_1M', 'current_prices', 'asset_supply', 'asset_metadata', 'oracle_prices') \
             OR startsWith(table, 'mv_'))",
        )
        .bind(&db)
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(types, vec!["UInt32"]);
    assert_eq!(
        mvs(c.clone(), db.clone()).await,
        mv_texts,
        "the captured MV texts, no others"
    );
    let live: u64 = count(
        &c,
        &format!(
            "SELECT count() FROM system.view_refreshes WHERE database = '{db}' \
             AND toString(status) != 'Disabled'"
        ),
    )
    .await;
    assert_eq!(
        live, 0,
        "re-created MVs are stopped until the operator starts them"
    );
    for t in ["assets__new", "price_ohlcv_1m__new"] {
        assert_eq!(
            count(
                &c,
                &format!(
                    "SELECT count() FROM system.tables WHERE database = '{db}' AND name = '{t}'"
                )
            )
            .await,
            1,
            "{t} is kept aside"
        );
    }
    refused(r.rollback(false).await, "nothing to roll back");
    drop_db(&c, &db).await;
}

/// Rehearsal: a scratch database without MVs captures the MVs and views of
/// another database rewritten into itself, and recreate refuses a definition
/// that would create or write outside it.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_rehearsal_captures_rewritten_definitions_and_never_touches_the_source() {
    let (c, src, _) = world("rw_src").await;
    stop_views(&c, &src).await;
    let source_mvs = |c: Client| async move {
        c.query(
            "SELECT name, create_table_query FROM system.tables WHERE database = 'it_rkswap_rw_src' \
             AND engine IN ('MaterializedView', 'View') ORDER BY name",
        )
        .fetch_all::<(String, String)>()
        .await
        .unwrap()
    };
    let untouched = source_mvs(c.clone()).await;
    let (_, db, _) = world_with("rw_dst", false).await;
    Rekey::new(c.clone(), &db, true)
        .capture(Some(&src))
        .await
        .unwrap();
    let outside = count(
        &c,
        &format!(
            "SELECT count() FROM {db}.{DDL_TABLE} WHERE kind = 'mv' \
             AND (create_query LIKE '%{src}.mv_%' OR create_query LIKE '%TO {src}.%')"
        ),
    )
    .await;
    assert_eq!(outside, 0, "MVs and their targets are rewritten into {db}");
    let reads_src = count(
        &c,
        &format!(
            "SELECT count() FROM {db}.{DDL_TABLE} WHERE kind = 'view' \
             AND create_query LIKE '%{src}.usd_rate%'"
        ),
    )
    .await;
    assert!(
        reads_src > 0,
        "tables outside the tool's set stay on the source"
    );

    // The rehearsal runs the window in the scratch database.
    let r = migrate_from(&c, &db, MvSource::ProdText, Some(&src)).await;
    assert_eq!(
        count(
            &c,
            &format!(
                "SELECT count() FROM system.tables WHERE database = '{db}' \
                 AND engine = 'MaterializedView'"
            )
        )
        .await,
        7
    );
    assert_eq!(
        source_mvs(c.clone()).await,
        untouched,
        "the source is never written"
    );

    exec(
        &c,
        &format!(
            "INSERT INTO {db}.{DDL_TABLE} (kind, name, create_query) VALUES ('mv', 'zz', \
             'CREATE MATERIALIZED VIEW {src}.zz REFRESH EVERY 1 HOUR APPEND TO {db}.price_ohlcv_1h \
             AS SELECT 1')"
        ),
    )
    .await;
    refused(
        r.recreate_mvs(MvSource::ProdText).await,
        &format!("creates or writes outside {db}"),
    );
    drop_db(&c, &db).await;
    drop_db(&c, &src).await;
}

/// Every post-check of `set` on `db`, by name.
async fn postchecks(c: &Client, db: &str, set: Postcheck, m: Option<u32>) -> Vec<(String, u8)> {
    let mut out = Vec::new();
    for (name, sql) in postcheck_sql(set, db) {
        let mut q = c.query(&sql);
        if let Some(m) = m {
            q = q.param("m", m);
        }
        out.push((
            name,
            q.fetch_one().await.unwrap_or_else(|e| panic!("{e}\n{sql}")),
        ));
    }
    out
}

/// A newer `gap-backfill` log row whose gap starts at `from` (SQL), `n`
/// seconds after now so it is the one the checks read.
async fn log_gap(c: &Client, db: &str, from: &str, n: u32) {
    exec(
        c,
        &format!(
            "INSERT INTO {db}.{LOG_TABLE} (at, step, status, range_from, range_to) \
             SELECT now64(3) + INTERVAL {n} SECOND, 'gap-backfill', 'ok', {from}, now()"
        ),
    )
    .await;
}

/// W13 simulated: a catch-up row 3 h old, written through the writer shape,
/// its assets row first.
async fn catch_up(c: &Client, db: &str, base: (&str, &str), quote: (&str, &str), trades: u32) {
    exec(
        c,
        &format!(
            "INSERT INTO {db}.assets (asset_code, asset_type, issuer_address, contract_address) \
             VALUES ('{}', 'classic', '{}', '')",
            base.0, base.1
        ),
    )
    .await;
    exec(
        c,
        &format!(
            "INSERT INTO {db}.price_ohlcv_1m (timestamp, base_code, base_issuer, base_contract, \
             quote_code, quote_issuer, quote_contract, source, open, high, low, close, \
             volume_base, volume_quote, vwap, trade_count, version) \
             SELECT toStartOfMinute(now() - INTERVAL 3 HOUR), '{}', '{}', '', '{}', '{}', '', \
             'sdex', 0.2, 0.2, 0.2, 0.2, 5, 1, 0.2, {trades}, 1",
            base.0, base.1, quote.0, quote.1
        ),
    )
    .await;
}

/// The writer-stop gap: a catch-up older than the 15m MV's 2 h window never
/// reaches 15m until gap-backfill rolls every tier, and the copy's exclusions
/// hold inside the gap.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn gap_backfill_rolls_a_catch_up_older_than_the_15m_window_into_every_tier() {
    let (c, db, _) = world("gap").await;
    refused(
        Rekey::new(c.clone(), &db, true).gap_backfill(None).await,
        "swap has not run",
    );
    let r = migrate(&c, &db, MvSource::ProdText).await;

    // A swap resumed after its per-table rows keeps (last_live_1m_ts, swap_at):
    // drop the final row, as if the run had stopped after the last EXCHANGE.
    stop_views(&c, &db).await;
    for _ in 0..60 {
        let running: u64 = c
            .query(
                "SELECT count() FROM system.view_refreshes WHERE database = ? \
                 AND toString(status) != 'Disabled'",
            )
            .bind(&db)
            .fetch_one()
            .await
            .unwrap();
        if running == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let swap_range = |status: &str| {
        format!(
            "SELECT (toUInt32(range_from), toUInt32(range_to)) FROM {db}.{LOG_TABLE} \
             WHERE step = 'swap' AND status = '{status}' ORDER BY at DESC LIMIT 1"
        )
    };
    let started: (u32, u32) = c.query(&swap_range("started")).fetch_one().await.unwrap();
    assert!(started.0 > 0, "the swap logged last_live_1m_ts");
    exec(
        &c,
        &format!("DELETE FROM {db}.{LOG_TABLE} WHERE step = 'swap' AND status = 'ok'"),
    )
    .await;
    r.swap(false).await.unwrap();
    let resumed: (u32, u32) = c.query(&swap_range("ok")).fetch_one().await.unwrap();
    assert_eq!(
        resumed, started,
        "a resumed swap logs the range it started with"
    );
    for mv in TIERS.iter().map(|t| t.mv).chain(["mv_current_prices"]) {
        exec(&c, &format!("SYSTEM START VIEW {db}.{mv}")).await;
    }
    refresh_all(&c, &db).await;

    catch_up(&c, &db, ("STW", "GSTW"), ("XLM", ""), 3).await;
    catch_up(&c, &db, ("XLM", ""), ("USDC", USDC_ISSUER), 2).await;
    refresh(&c, &db, "mv_ohlcv_1m_to_15m").await;
    refused(
        r.gap_verify(false, None, Postcheck::Window).await,
        "no gap-backfill logged",
    );
    let before = postchecks(&c, &db, Postcheck::Window, None).await;
    assert_eq!(
        before[4],
        ("gap_15m".to_string(), 0),
        "no gap-backfill, no pass"
    );
    let (now, newest, early): (String, String, String) = c
        .query(&format!(
            "SELECT formatDateTime(now(), '%Y-%m-%d %H:%i:%S', 'UTC'), \
             formatDateTime(max(timestamp), '%Y-%m-%d %H:%i:%S', 'UTC'), \
             formatDateTime(max(timestamp) - INTERVAL 1 MINUTE, '%Y-%m-%d %H:%i:%S', 'UTC') \
             FROM {db}.price_ohlcv_1m"
        ))
        .fetch_one()
        .await
        .unwrap();
    gate_failed(
        r.gap_verify(
            false,
            Some(("2000-01-01 00:00:00", now.as_str())),
            Postcheck::Window,
        )
        .await,
        "price_ohlcv_15m: ",
    );
    refused(
        r.gap_backfill(None).await,
        &format!("({newest}) is older than 15 minutes"),
    );
    refused(
        r.gap_backfill(Some(&early)).await,
        "is earlier than the newest",
    );

    r.gap_backfill(Some(&now)).await.unwrap();
    let lines = r.gap_verify(false, None, Postcheck::Window).await.unwrap();
    assert!(lines.iter().any(|l| l == "gap_15m: 1"), "{lines:?}");
    r.gap_backfill(Some(&now)).await.unwrap();
    r.gap_verify(false, None, Postcheck::Window).await.unwrap();

    let stw = fetch_id(&c, "STW", "GSTW", "").await;
    let arb = fetch_id(&c, "ARBRIDGE", "GARB", "").await;
    for t in std::iter::once("price_ohlcv_1m").chain(TIERS.iter().map(|t| t.target)) {
        let (stw_trades, arb_rows, orphans): (u64, u64, u64) = c
            .query(&format!(
                "SELECT sumIf(trade_count, asset_id = {stw}), \
                 countIf(asset_id = {arb} OR quote_asset_id = {arb}), \
                 countIf(asset_id NOT IN (SELECT asset_id FROM {db}.assets) \
                 OR quote_asset_id NOT IN (SELECT asset_id FROM {db}.assets)) \
                 FROM {db}.{t} FINAL"
            ))
            .fetch_one()
            .await
            .unwrap();
        assert_eq!(stw_trades, 3, "{t}: STW holds the post-resume trades only");
        assert_eq!(arb_rows, 0, "{t}: no blend in any tier");
        assert_eq!(orphans, 0, "{t}: the orphan stays out");
    }
    for (name, ok) in postchecks(&c, &db, Postcheck::Window, None).await {
        assert_eq!(ok, 1, "{name}");
    }

    // W16 and W17 run days later. Date the logged gap 70 days back so every
    // tier has settled buckets at any hour: now − 70 d always lies a month
    // boundary before now − 28 h 15, the 1M bound.
    log_gap(&c, &db, "now() - INTERVAL 70 DAY", 1).await;
    for set in [
        Postcheck::Window,
        Postcheck::NextDay,
        Postcheck::PeriodClose,
    ] {
        for (name, ok) in postchecks(&c, &db, set, None).await {
            assert_eq!(ok, 1, "{set:?} {name}");
        }
        r.gap_verify(false, None, set).await.unwrap();
    }

    // A settled 4h bucket that never reached 1d (nor came from 1h): the
    // next-day check fails on it.
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.price_ohlcv_4h (timestamp, base_code, base_issuer, base_contract, \
             quote_code, quote_issuer, quote_contract, source, open, high, low, close, \
             volume_base, volume_quote, vwap, trade_count, version) \
             SELECT toStartOfInterval(now() - INTERVAL 3 DAY, INTERVAL 4 HOUR), 'XLM', '', '', \
             'USDC', '{USDC_ISSUER}', '', 'sdex', 0.2, 0.2, 0.2, 0.2, 5, 1, 0.2, 7, 1"
        ),
    )
    .await;
    let next: Vec<(String, u8)> = postchecks(&c, &db, Postcheck::NextDay, None)
        .await
        .into_iter()
        .filter(|(n, _)| n.starts_with("gap_"))
        .collect();
    let s = |n: &str, v: u8| (n.to_string(), v);
    assert_eq!(
        next,
        vec![
            s("gap_15m", 1),
            s("gap_1h", 1),
            s("gap_4h", 0),
            s("gap_1d", 0)
        ]
    );
    gate_failed(
        r.gap_verify(false, None, Postcheck::NextDay).await,
        "price_ohlcv_1d: 1",
    );

    // A gap none of whose buckets has settled fails rather than passing.
    log_gap(&c, &db, "now()", 2).await;
    for set in [
        Postcheck::Window,
        Postcheck::NextDay,
        Postcheck::PeriodClose,
    ] {
        for (name, ok) in postchecks(&c, &db, set, None).await {
            if name.starts_with("gap_") {
                assert_eq!(ok, 0, "{set:?} {name}: an unsettled gap fails");
            }
        }
        gate_failed(r.gap_verify(false, None, set).await, "not settled");
    }

    // 202401 held STW/ARBRIDGE blends: not restored until it is re-ingested.
    let month = postchecks(&c, &db, Postcheck::Month, Some(202401)).await;
    assert_eq!(
        month,
        vec![
            s("assets_unique", 1),
            s("colliding_restored", 0),
            s("month_1d_equals_1m", 1)
        ]
    );
    let row_202401 = |code: &str, issuer: &str| {
        format!(
            "INSERT INTO {db}.price_ohlcv_1m (timestamp, base_code, base_issuer, base_contract, \
             quote_code, quote_issuer, quote_contract, source, open, high, low, close, vwap, \
             trade_count, version) SELECT toDateTime('2024-01-01 00:00:00'), '{code}', \
             '{issuer}', '', 'XLM', '', '', 'sdex', 1, 1, 1, 1, 1, 1, 1"
        )
    };
    // STW also sits under a clean old id, whose rows the copy kept: a row under
    // STW's new id proves nothing about the colliding rows.
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.{MAP_TABLE} (old_id, new_id, asset_code, issuer_address, \
             contract_address, status) VALUES (98, {stw}, 'STW', 'GSTW', '', 'mapped')"
        ),
    )
    .await;
    exec(&c, &row_202401("STW", "GSTW")).await;
    let copied = postchecks(&c, &db, Postcheck::Month, Some(202401)).await;
    assert_eq!(
        copied[1],
        s("colliding_restored", 0),
        "a row from a clean old id is not a restored colliding row"
    );
    exec(&c, &row_202401("ARBRIDGE", "GARB")).await;
    let restored = postchecks(&c, &db, Postcheck::Month, Some(202401)).await;
    assert_eq!(restored[1], s("colliding_restored", 1));
    let unlisted = postchecks(&c, &db, Postcheck::Month, Some(202403)).await;
    assert_eq!(
        unlisted[1],
        s("colliding_restored", 1),
        "a month not listed passes"
    );
    drop_db(&c, &db).await;
}
