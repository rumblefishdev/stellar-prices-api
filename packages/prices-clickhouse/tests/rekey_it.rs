//! Task 0139 — the rekey tool, part 1, on scratch databases built from the
//! frozen pre-0139 DDL (`fixtures/pre0139_id_tables.sql`).
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-clickhouse --test rekey_it -- --ignored --test-threads=1
//!
//! The seed holds a colliding id (4: STW and ARBRIDGE), an orphan (77), the
//! REDSTONE sentinel (0), two old ids of one identity (5, 6), a case variant
//! (2 USDC, 3 usdc), an unmerged duplicate key and two partitions.

use clickhouse::Client;
use prices_clickhouse::asset_id::fixture::fetch_id;
use prices_clickhouse::asset_id::id_expr;
use prices_clickhouse::rekey::{
    COPIED_TABLES, Fill, FillReport, LOG_TABLE, MAP_FINAL, MAP_TABLE, Rekey, RekeyError,
    map_gates_sql, tool_tables_ddl, written_rows,
};

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

/// A scratch database on the pre-0139 shape, seeded.
async fn seed(name: &str) -> (Client, String) {
    let db = format!("it_rekey_{name}");
    let c = Client::default().with_url(ch_url());
    exec(&c, &format!("DROP DATABASE IF EXISTS {db} SYNC")).await;
    exec(&c, &format!("CREATE DATABASE {db}")).await;
    prices_clickhouse::apply_sql(&c, &PRE0139.replace("prices.", &format!("{db}.")))
        .await
        .expect("apply the pre-0139 DDL");
    // Raw duplicates must survive: map reads raw assets, fill raw candles.
    for t in ["assets", "price_ohlcv_1m"] {
        exec(&c, &format!("SYSTEM STOP MERGES {db}.{t}")).await;
    }
    let assets = "(asset_id, asset_code, asset_type, issuer_address, contract_address)";
    for v in [
        "(1, 'XLM', 'native', '', '')",
        "(1, 'XLM', 'native', '', '')", // a re-emitted raw row
        "(2, 'USDC', 'classic', 'GUSDC', '')",
        "(3, 'usdc', 'classic', 'GUSDC', '')",
        "(4, 'STW', 'classic', 'GSTW', '')",
        "(4, 'ARBRIDGE', 'classic', 'GARB', '')",
        "(5, 'FOO', 'classic', 'GFOO', '')",
        "(6, 'FOO', 'classic', 'GFOO', '')",
    ] {
        exec(&c, &format!("INSERT INTO {db}.assets {assets} VALUES {v}")).await;
    }
    let candle = "(timestamp, asset_id, quote_asset_id, source, open, high, low, close, \
                  vwap, trade_count, version)";
    for (ts, base, quote, version) in [
        ("2024-01-01 00:00:00", 1, 2, 1),
        ("2024-01-01 00:00:00", 1, 2, 2), // unmerged duplicate key
        ("2024-01-01 00:01:00", 1, 2, 1),
        ("2024-01-01 00:00:00", 3, 1, 1),
        ("2024-01-01 00:00:00", 4, 1, 1),  // colliding base
        ("2024-01-01 00:02:00", 1, 4, 1),  // colliding quote
        ("2024-01-01 00:00:00", 77, 1, 1), // orphan
        ("2024-02-01 00:00:00", 5, 2, 1),  // 5 and 6: one identity
        ("2024-02-01 00:00:00", 6, 2, 2),
        ("2024-02-01 00:00:00", 1, 2, 1),
        ("2024-02-02 00:00:00", 4, 2, 1),
    ] {
        exec(
            &c,
            &format!(
                "INSERT INTO {db}.price_ohlcv_1m {candle} VALUES \
                 ('{ts}', {base}, {quote}, 'sdex', 1, 1, 1, 1, 1, 1, {version})"
            ),
        )
        .await;
    }
    for (base, quote) in [(1, 2), (4, 2)] {
        exec(
            &c,
            &format!(
                "INSERT INTO {db}.price_ohlcv_1d {candle} VALUES \
                 ('2024-01-01 00:00:00', {base}, {quote}, 'sdex', 1, 1, 1, 1, 1, 1, 1)"
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
    (c, db)
}

async fn drop_db(c: &Client, db: &str) {
    exec(c, &format!("DROP DATABASE IF EXISTS {db} SYNC")).await;
}

fn refused(r: Result<impl std::fmt::Debug, RekeyError>, why: &str) {
    match r {
        Err(RekeyError::Refused(msg)) => assert!(msg.contains(why), "{msg}"),
        other => panic!("expected a refusal ({why}), got {other:?}"),
    }
}

fn gate_failed(r: Result<impl std::fmt::Debug, RekeyError>, gate: &str) {
    match r {
        Err(RekeyError::Gate(msg)) => assert!(msg.contains(gate), "{gate} not in: {msg}"),
        other => panic!("expected gate `{gate}` to fail, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn map_classifies_every_old_id_and_a_rerun_reclassifies() {
    let (c, db) = seed("map").await;

    let dry = Rekey::new(c.clone(), &db, false).map().await.unwrap();
    assert!(dry.contains("colliding: 1 ids / 2 rows"), "{dry}");
    let tables = format!("SELECT count() FROM system.tables WHERE database = '{db}'");
    assert_eq!(count(&c, &tables).await, 12, "a dry run creates nothing");

    let r = Rekey::new(c.clone(), &db, true);
    let summary = r.map().await.unwrap();
    assert_eq!(
        summary,
        "colliding: 1 ids / 2 rows, mapped: 5 ids / 5 rows, \
         orphan: 1 ids / 1 rows, sentinel: 1 ids / 1 rows"
    );

    let rows: Vec<(u32, u64, String, String)> = c
        .query(&format!(
            "SELECT old_id, new_id, asset_code, status FROM {db}.{MAP_TABLE} \
             ORDER BY old_id, asset_code"
        ))
        .fetch_all()
        .await
        .unwrap();
    let xlm = fetch_id(&c, "XLM", "", "").await;
    let usdc = fetch_id(&c, "USDC", "GUSDC", "").await;
    let lower = fetch_id(&c, "usdc", "GUSDC", "").await;
    let foo = fetch_id(&c, "FOO", "GFOO", "").await;
    let arb = fetch_id(&c, "ARBRIDGE", "GARB", "").await;
    let stw = fetch_id(&c, "STW", "GSTW", "").await;
    let s = |x: &str| x.to_string();
    assert_eq!(
        rows,
        vec![
            (0, 0, s(""), s("sentinel")),
            (1, xlm, s("XLM"), s("mapped")),
            (2, usdc, s("USDC"), s("mapped")),
            (3, lower, s("usdc"), s("mapped")),
            (4, arb, s("ARBRIDGE"), s("colliding")),
            (4, stw, s("STW"), s("colliding")),
            (5, foo, s("FOO"), s("mapped")),
            (6, foo, s("FOO"), s("mapped")),
            (77, 0, s(""), s("orphan")),
        ]
    );
    assert_ne!(usdc, lower, "case is preserved");

    assert_eq!(r.map().await.unwrap(), summary, "a re-run is idempotent");

    // Id 2 gains a second identity; a new orphan appears.
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.assets (asset_id, asset_code, asset_type, issuer_address) \
             VALUES (2, 'EURC', 'classic', 'GEUR')"
        ),
    )
    .await;
    exec(
        &c,
        &format!("INSERT INTO {db}.asset_supply (asset_id, token_supply) VALUES (88, 1)"),
    )
    .await;
    assert_eq!(
        r.map().await.unwrap(),
        "colliding: 2 ids / 4 rows, mapped: 4 ids / 4 rows, \
         orphan: 2 ids / 2 rows, sentinel: 1 ids / 1 rows"
    );
    let logged = format!(
        "SELECT count() FROM {db}.{LOG_TABLE} WHERE step = 'map' AND status = 'ok' \
         AND user = currentUser()"
    );
    assert_eq!(count(&c, &logged).await, 3);
    drop_db(&c, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn each_map_gate_trips_on_a_planted_fault() {
    let (c, db) = seed("map_gates").await;
    let r = Rekey::new(c.clone(), &db, true);
    let gates = map_gates_sql(&db, MAP_TABLE);

    // A blank identity in assets maps to xxh3('::'): map stops before the swap.
    exec(
        &c,
        &format!(
            "INSERT INTO {db}.assets (asset_id, asset_code, asset_type) VALUES (9, '', 'classic')"
        ),
    )
    .await;
    gate_failed(r.map().await, gates[0].0);
    let mapped = format!("SELECT count() FROM {db}.{MAP_TABLE}");
    assert_eq!(count(&c, &mapped).await, 0, "the map is not swapped in");

    let planted = [
        "(10, 0, 'A', 'G', '', 'mapped')",
        "(10, 123, 'A', 'G', '', 'mapped'), (11, 123, 'B', 'G', '', 'mapped')",
        "(10, 1, 'A', 'G', '', 'mapped'), (10, 2, 'B', 'G', '', 'mapped')",
    ];
    for (i, rows) in planted.iter().enumerate() {
        let t = format!("planted_{i}");
        exec(&c, &format!("CREATE TABLE {db}.{t} AS {db}.{MAP_TABLE}")).await;
        exec(
            &c,
            &format!(
                "INSERT INTO {db}.{t} (old_id, new_id, asset_code, issuer_address, \
                 contract_address, status) VALUES {rows}"
            ),
        )
        .await;
        gate_failed(r.map_gates(&t).await, gates[i].0);
        for (j, (name, _)) in gates.iter().enumerate() {
            if j != i {
                let msg = format!("{:?}", r.map_gates(&t).await);
                assert!(!msg.contains(name), "only gate {i} trips: {msg}");
            }
        }
    }
    drop_db(&c, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn map_refuses_once_assets_carry_derived_ids() {
    let (c, db) = seed("map_refuse_alter").await;
    let r = Rekey::new(c.clone(), &db, true);
    r.map().await.unwrap();
    // A type change is a mutation: it waits for merges.
    exec(&c, &format!("SYSTEM START MERGES {db}.assets")).await;
    exec(
        &c,
        &format!(
            "ALTER TABLE {db}.assets MODIFY COLUMN asset_id UInt64 MATERIALIZED {}",
            id_expr("asset_code", "issuer_address", "contract_address")
        ),
    )
    .await;
    refused(r.map().await, MAP_FINAL);
    refused(Rekey::new(c.clone(), &db, false).map().await, MAP_FINAL);
    drop_db(&c, &db).await;

    let (c, db) = seed("map_refuse_log").await;
    for ddl in tool_tables_ddl(&db) {
        exec(&c, &ddl).await;
    }
    exec(
        &c,
        &format!("INSERT INTO {db}.{LOG_TABLE} (step, status) VALUES ('alter-assets', 'ok')"),
    )
    .await;
    refused(Rekey::new(c.clone(), &db, true).map().await, MAP_FINAL);
    drop_db(&c, &db).await;
}

async fn ids_of(c: &Client, sql: &str) -> Vec<(u64, u64)> {
    c.query(sql)
        .fetch_all()
        .await
        .unwrap_or_else(|e| panic!("query failed: {e}\n{sql}"))
}

async fn fill_all(r: &Rekey) -> Vec<(&'static str, FillReport)> {
    let mut out = Vec::new();
    for t in COPIED_TABLES {
        out.push((t, r.fill(&Fill::table(t)).await.unwrap()));
    }
    out
}

fn copied(n: usize, skipped: usize) -> FillReport {
    FillReport { copied: n, skipped }
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn create_builds_new_tables_from_init_sql_and_gates_parity() {
    let (c, db) = seed("create").await;
    Rekey::new(c.clone(), &db, false).create().await.unwrap();
    let new_tables =
        format!("SELECT count() FROM system.tables WHERE database = '{db}' AND name LIKE '%__new'");
    assert_eq!(count(&c, &new_tables).await, 0, "a dry run creates nothing");

    let r = Rekey::new(c.clone(), &db, true);
    r.create().await.unwrap();
    r.create().await.unwrap();
    assert_eq!(count(&c, &new_tables).await, 11);
    let types = format!(
        "SELECT countIf(type = 'UInt64'), countIf(type != 'UInt64') FROM system.columns \
         WHERE database = '{db}' AND table LIKE '%__new' AND name IN ('asset_id', 'quote_asset_id')"
    );
    let (u64s, other): (u64, u64) = c.query(&types).fetch_one().await.unwrap();
    assert_eq!((u64s, other), (7 * 2 + 4, 0));
    let eph = format!(
        "SELECT count() FROM system.columns WHERE database = '{db}' \
         AND table IN ('price_ohlcv_1M__new', 'oracle_prices__new') AND default_kind = 'EPHEMERAL'"
    );
    assert_eq!(count(&c, &eph).await, 6 + 3);

    exec(
        &c,
        &format!("ALTER TABLE {db}.price_ohlcv_4h ADD COLUMN extra UInt8 DEFAULT 0"),
    )
    .await;
    gate_failed(r.create().await, "price_ohlcv_4h__new lacks extra");
    drop_db(&c, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn fill_copies_mapped_rows_and_both_gates_survive_a_merge() {
    let (plain, db) = seed("fill").await;
    // Async inserts, unwaited: the tool writes only INSERT … SELECT, which
    // ignores both and runs synchronously.
    let c = plain
        .clone()
        .with_option("async_insert", "1")
        .with_option("wait_for_async_insert", "0");
    let r = Rekey::new(c.clone(), &db, true);
    r.map().await.unwrap();
    r.create().await.unwrap();

    let dry = Rekey::new(c.clone(), &db, false)
        .fill(&Fill::table("price_ohlcv_1m"))
        .await
        .unwrap();
    assert_eq!(dry, copied(2, 0));
    let new_1m = format!("SELECT count() FROM {db}.price_ohlcv_1m__new");
    assert_eq!(count(&c, &new_1m).await, 0, "a dry run copies nothing");

    let reports = fill_all(&r).await;
    let by = |t: &str| &reports.iter().find(|(n, _)| *n == t).unwrap().1;
    assert_eq!(by("price_ohlcv_1m"), &copied(2, 0));
    assert_eq!(by("price_ohlcv_1d"), &copied(1, 0));
    assert_eq!(by("price_ohlcv_15m"), &copied(0, 0));
    assert_eq!(by("current_prices"), &copied(1, 0));

    let (xlm, usdc) = (
        fetch_id(&c, "XLM", "", "").await,
        fetch_id(&c, "USDC", "GUSDC", "").await,
    );
    let lower = fetch_id(&c, "usdc", "GUSDC", "").await;
    let foo = fetch_id(&c, "FOO", "GFOO", "").await;
    // Read at once: the copy is synchronous. `optimize_on_insert` already
    // collapsed duplicate keys (the unmerged pair, old ids 5 and 6) to their
    // highest version, while `written_rows` counted 7: gate 2 is for this.
    let stored: Vec<(u64, u64, u64)> = c
        .query(&format!(
            "SELECT asset_id, quote_asset_id, version FROM {db}.price_ohlcv_1m__new \
             ORDER BY timestamp, asset_id"
        ))
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(
        stored,
        vec![
            (xlm, usdc, 2),
            (lower, xlm, 1),
            (xlm, usdc, 1),
            (foo, usdc, 2), // ordered by id within 2024-02-01
            (xlm, usdc, 1),
        ]
    );
    assert_eq!(
        ids_of(
            &c,
            &format!("SELECT asset_id, toUInt64(0) FROM {db}.oracle_prices__new ORDER BY asset_id")
        )
        .await,
        vec![(0, 0), (xlm, 0)],
        "the REDSTONE sentinel is copied as 0"
    );
    for (t, want) in [
        ("current_prices", xlm),
        ("asset_supply", usdc),
        ("asset_metadata", usdc),
    ] {
        assert_eq!(
            ids_of(
                &c,
                &format!("SELECT asset_id, toUInt64(0) FROM {db}.{t}__new")
            )
            .await,
            vec![(want, 0)],
            "{t}"
        );
    }
    let logged = format!(
        "SELECT partition, written, expected, expected_keys, target_keys FROM {db}.{LOG_TABLE} \
         WHERE step = 'fill' AND target = 'price_ohlcv_1m__new' AND status = 'verified' \
         ORDER BY partition"
    );
    let rows: Vec<(String, u64, u64, u64, u64)> = c.query(&logged).fetch_all().await.unwrap();
    assert_eq!(
        rows,
        vec![(s("202401"), 4, 4, 3, 3), (s("202402"), 3, 3, 2, 2)]
    );

    // Distinct keys are what a merge preserves: a re-run after one copies nothing.
    exec(
        &c,
        &format!("OPTIMIZE TABLE {db}.price_ohlcv_1m__new FINAL"),
    )
    .await;
    assert_eq!(count(&c, &new_1m).await, 5);
    for (t, report) in fill_all(&r).await {
        assert_eq!(report.copied, 0, "{t}: a re-run copies nothing");
    }

    // A changed source partition is refilled, the other skipped.
    exec(
        &plain,
        &format!(
            "INSERT INTO {db}.price_ohlcv_1m (timestamp, asset_id, quote_asset_id, source, \
             open, high, low, close, vwap, version) \
             VALUES ('2024-02-03 00:00:00', 1, 2, 'sdex', 1, 1, 1, 1, 1, 1)"
        ),
    )
    .await;
    assert_eq!(
        r.fill(&Fill::table("price_ohlcv_1m")).await.unwrap(),
        copied(1, 1)
    );
    // An id that becomes colliding: its partition is refilled without it.
    exec(
        &plain,
        &format!(
            "INSERT INTO {db}.assets (asset_id, asset_code, asset_type, issuer_address) \
             VALUES (3, 'EURX', 'classic', 'GX')"
        ),
    )
    .await;
    r.map().await.unwrap();
    assert_eq!(
        r.fill(&Fill::table("price_ohlcv_1m")).await.unwrap(),
        copied(1, 1)
    );
    let lower_rows =
        format!("SELECT count() FROM {db}.price_ohlcv_1m__new WHERE asset_id = {lower}");
    assert_eq!(count(&c, &lower_rows).await, 0);
    assert_eq!(count(&c, &new_1m).await, 2 + 3, "stored keys per partition");
    drop_db(&c, &db).await;
}

fn s(x: &str) -> String {
    x.to_string()
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn fill_gates_trip_on_planted_faults() {
    let (c, db) = seed("fill_gates").await;
    let r = Rekey::new(c.clone(), &db, true);
    r.map().await.unwrap();
    r.create().await.unwrap();

    // Planted faults on 202401 (4 rows, 3 distinct keys), each logged failed:
    // a row with its own key not copied trips both gates; the second row of a
    // duplicate key only `written_rows`; keys merged by the copy only gate 2.
    let fault = |pred: Option<&str>, expr: Option<(&str, &str)>| Fill {
        partition: Some("202401".into()),
        fault: pred.map(str::to_string),
        fault_expr: expr.map(|(c, e)| (c.to_string(), e.to_string())),
        ..Fill::table("price_ohlcv_1m")
    };
    for (f, msg) in [
        (
            fault(Some("s.timestamp != '2024-01-01 00:01:00'"), None),
            "written Some(3) of 4, keys 2 of 3",
        ),
        (
            fault(Some("NOT (s.asset_id = 1 AND s.version = 2)"), None),
            "written Some(3) of 4, keys 3 of 3",
        ),
        (
            fault(None, Some(("timestamp", "toStartOfDay(s.timestamp)"))),
            "written Some(4) of 4, keys 2 of 3",
        ),
    ] {
        gate_failed(
            r.fill(&f).await,
            &format!("price_ohlcv_1m__new/202401: {msg}"),
        );
    }
    let failed =
        format!("SELECT count() FROM {db}.{LOG_TABLE} WHERE step = 'fill' AND status = 'failed'");
    assert_eq!(count(&c, &failed).await, 3);
    // A failed partition is refilled on the next run.
    assert_eq!(
        r.fill(&Fill::table("price_ohlcv_1m")).await.unwrap(),
        copied(2, 0)
    );

    // A verified partition that loses a row is refilled, even after a merge.
    exec(
        &c,
        &format!(
            "ALTER TABLE {db}.price_ohlcv_1m__new DELETE WHERE timestamp = '2024-01-01 00:01:00' \
             SETTINGS mutations_sync = 2"
        ),
    )
    .await;
    exec(
        &c,
        &format!("OPTIMIZE TABLE {db}.price_ohlcv_1m__new FINAL"),
    )
    .await;
    assert_eq!(
        r.fill(&Fill::table("price_ohlcv_1m")).await.unwrap(),
        copied(1, 1)
    );

    let none = written_rows(
        &c,
        "rekey0139-no-such-query",
        2,
        std::time::Duration::from_millis(50),
    )
    .await
    .unwrap();
    assert_eq!(none, None, "a missing query_log row fails closed");
    drop_db(&c, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn fill_refuses_new_space_sources_and_a_backup_fills_after_the_alter() {
    let (c, db) = seed("fill_refuse").await;
    let r = Rekey::new(c.clone(), &db, true);
    refused(
        r.fill(&Fill::table("price_ohlcv_1d")).await,
        "does not hold UInt64 ids",
    );
    r.create().await.unwrap();
    refused(r.fill(&Fill::table("price_ohlcv_1d")).await, "no map");
    r.map().await.unwrap();

    let new_source = Fill {
        source: "price_ohlcv_1m__new".into(),
        target: "price_ohlcv_1m__new".into(),
        ..Fill::default()
    };
    refused(r.fill(&new_source).await, "already in the new id space");

    // GA1's rewrite: an old-shape backup, filled through the map after alter-assets.
    exec(
        &c,
        &format!("CREATE TABLE {db}.bak_1d AS {db}.price_ohlcv_1d"),
    )
    .await;
    exec(
        &c,
        &format!("INSERT INTO {db}.bak_1d SELECT * FROM {db}.price_ohlcv_1d"),
    )
    .await;
    exec(
        &c,
        &format!("CREATE TABLE {db}.bak_1d__new AS {db}.price_ohlcv_1d__new"),
    )
    .await;
    exec(&c, &format!("SYSTEM START MERGES {db}.assets")).await;
    exec(
        &c,
        &format!(
            "ALTER TABLE {db}.assets MODIFY COLUMN asset_id UInt64 MATERIALIZED {}",
            id_expr("asset_code", "issuer_address", "contract_address")
        ),
    )
    .await;
    refused(r.map().await, MAP_FINAL);
    let backup = Fill {
        source: "bak_1d".into(),
        target: "bak_1d__new".into(),
        ..Fill::default()
    };
    assert_eq!(r.fill(&backup).await.unwrap(), copied(1, 0));
    let xlm = fetch_id(&c, "XLM", "", "").await;
    let usdc = fetch_id(&c, "USDC", "GUSDC", "").await;
    assert_eq!(
        ids_of(
            &c,
            &format!("SELECT asset_id, quote_asset_id FROM {db}.bak_1d__new")
        )
        .await,
        vec![(xlm, usdc)]
    );

    // After the swap the default source is the new table: refused.
    exec(
        &c,
        &format!("EXCHANGE TABLES {db}.price_ohlcv_1d AND {db}.price_ohlcv_1d__new"),
    )
    .await;
    refused(
        r.fill(&Fill::table("price_ohlcv_1d")).await,
        "already in the new id space",
    );
    drop_db(&c, &db).await;
}
