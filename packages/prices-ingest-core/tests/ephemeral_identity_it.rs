//! Task 0139 — the database derives `asset_id` from the identity, through the
//! real `clickhouse` crate (0.13.3).
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-ingest-core --test ephemeral_identity_it -- --ignored --test-threads=1
//!
//! Writers send identity strings in `EPHEMERAL` columns and no id. ClickHouse
//! fills `asset_id` / `quote_asset_id` from `prices_clickhouse::asset_id`'s
//! expression. This only works if the crate's RowBinary insert can name
//! EPHEMERAL columns, which is what these tests pin; an upgrade of the crate
//! that breaks it fails here.
//!
//! Every expected id comes from the server (`SELECT xxh3(...)` with bound
//! strings). Nothing here hashes in Rust. The DDL is rendered from the shared
//! expression, not typed by hand.
//!
//! Each test owns a scratch database and drops it at the end.

use clickhouse::{Client, Row};
use prices_clickhouse::asset_id::{
    fixture::{self, AssetFixture},
    id_expr, id_of, oracle_id_expr,
};
use serde::{Deserialize, Serialize};

const USDC_ISSUER: &str = "GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN";
const SOROBAN_CONTRACT: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";
const GBXYZ: &str = "GBXYZISSUER";

type Identity = (&'static str, &'static str, &'static str);

const XLM: Identity = ("XLM", "", "");
const USDC: Identity = ("USDC", USDC_ISSUER, "");
const USDC_LOWER: Identity = ("usdc", USDC_ISSUER, "");
const CONTRACT_TOKEN: Identity = ("", "", SOROBAN_CONTRACT);
const CODE12: Identity = ("ABCDEFGHIJKL", GBXYZ, "");
/// What the lossy XDR code decode produces for invalid UTF-8.
const REPLACEMENT: Identity = ("A\u{FFFD}B", GBXYZ, "");
/// Exercises `sql_str` escaping against the server.
const QUOTED: Identity = ("O'B\\X", GBXYZ, "");
const BLANK: Identity = ("", "", "");

/// Every non-XLM identity, priced against XLM in the candle tests.
const BASES: [Identity; 6] = [
    USDC,
    USDC_LOWER,
    CONTRACT_TOKEN,
    CODE12,
    REPLACEMENT,
    QUOTED,
];

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

/// Candle writer row: identity strings, no id.
#[derive(Row, Serialize)]
struct CandleIn {
    timestamp: u32,
    base_code: String,
    base_issuer: String,
    base_contract: String,
    quote_code: String,
    quote_issuer: String,
    quote_contract: String,
    source: String,
    close: i128,
    version: u64,
}

#[derive(Row, Serialize)]
struct OracleIn {
    timestamp: u32,
    asset_code: String,
    issuer_address: String,
    contract_address: String,
    oracle_name: String,
    price_usd: i128,
}

#[derive(Row, Serialize)]
struct AssetIn {
    asset_code: String,
    asset_type: String,
    issuer_address: String,
    contract_address: String,
}

#[derive(Row, Serialize)]
struct AssetWithId {
    asset_id: u64,
    asset_code: String,
    asset_type: String,
    issuer_address: String,
    contract_address: String,
}

#[derive(Row, Deserialize, Debug, PartialEq)]
struct IdPair {
    asset_id: u64,
    quote_asset_id: u64,
}

fn candle(timestamp: u32, base: Identity, quote: Identity) -> CandleIn {
    CandleIn {
        timestamp,
        base_code: base.0.into(),
        base_issuer: base.1.into(),
        base_contract: base.2.into(),
        quote_code: quote.0.into(),
        quote_issuer: quote.1.into(),
        quote_contract: quote.2.into(),
        source: "sdex".into(),
        close: 105_000_000_000_000,
        version: 1,
    }
}

/// The candle rows every writer test sends: each base against XLM, plus XLM
/// against USDC so XLM appears as a base too.
fn candle_rows() -> Vec<(u32, Identity, Identity)> {
    let mut rows: Vec<(u32, Identity, Identity)> = BASES
        .iter()
        .enumerate()
        .map(|(i, b)| (1_790_000_000 + 60 * i as u32, *b, XLM))
        .collect();
    rows.push((1_790_000_000 + 60 * BASES.len() as u32, XLM, USDC));
    rows
}

async fn scratch(name: &str) -> (Client, String) {
    let db = format!("it_ephemeral_identity_{name}");
    let admin = Client::default().with_url(ch_url());
    exec(&admin, &format!("DROP DATABASE IF EXISTS {db} SYNC")).await;
    exec(&admin, &format!("CREATE DATABASE {db}")).await;
    (admin, db)
}

async fn drop_db(admin: &Client, db: &str) {
    exec(admin, &format!("DROP DATABASE IF EXISTS {db} SYNC")).await;
}

async fn exec(c: &Client, sql: &str) {
    c.query(sql)
        .execute()
        .await
        .unwrap_or_else(|e| panic!("query failed: {e}\n{sql}"));
}

/// The id ClickHouse computes for an identity, from bound strings — a path
/// independent of both the column DEFAULT and `id_of`'s literal rendering.
async fn server_id(c: &Client, (code, issuer, contract): Identity) -> u64 {
    c.query("SELECT xxh3(concat(?, ':', ?, ':', ?))")
        .bind(code)
        .bind(issuer)
        .bind(contract)
        .fetch_one()
        .await
        .unwrap()
}

/// `CHECK` on derived ids: never 0, never the id of a blank identity. It must
/// name the stored ids — ClickHouse refuses a CHECK on EPHEMERAL columns.
fn derived_ids_check(name: &str, cols: &[&str]) -> String {
    let blank = id_of("", "", "");
    let conds: Vec<String> = cols
        .iter()
        .map(|c| format!("{c} != 0 AND {c} != {blank}"))
        .collect();
    format!("CONSTRAINT {name} CHECK {}", conds.join(" AND "))
}

/// A candle tier in the target shape. `id_kind` is `DEFAULT` (what the schema
/// uses, so enrichment and MVs may still write ids) or `MATERIALIZED`.
fn candles_ddl(db: &str, table: &str, id_kind: &str) -> String {
    format!(
        "CREATE TABLE {db}.{table} (
            timestamp DateTime,
            base_code String EPHEMERAL, base_issuer String EPHEMERAL, base_contract String EPHEMERAL,
            quote_code String EPHEMERAL, quote_issuer String EPHEMERAL, quote_contract String EPHEMERAL,
            asset_id UInt64 {id_kind} {base},
            quote_asset_id UInt64 {id_kind} {quote},
            source LowCardinality(String),
            close Decimal(38, 14),
            version UInt64,
            {check}
         ) ENGINE = ReplacingMergeTree(version)
         PARTITION BY toYYYYMM(timestamp)
         ORDER BY (asset_id, quote_asset_id, source, timestamp)",
        base = id_expr("base_code", "base_issuer", "base_contract"),
        quote = id_expr("quote_code", "quote_issuer", "quote_contract"),
        check = derived_ids_check("derived_ids", &["asset_id", "quote_asset_id"]),
    )
}

fn assets_ddl(db: &str) -> String {
    format!(
        "CREATE TABLE {db}.assets (
            asset_code String,
            asset_type String,
            issuer_address String DEFAULT '',
            contract_address String DEFAULT '',
            asset_id UInt64 MATERIALIZED {id},
            updated_at DateTime DEFAULT now(),
            {check}
         ) ENGINE = ReplacingMergeTree(updated_at)
         ORDER BY (asset_code, issuer_address, contract_address)",
        id = id_expr("asset_code", "issuer_address", "contract_address"),
        check = derived_ids_check("derived_id", &["asset_id"]),
    )
}

fn oracle_ddl(db: &str) -> String {
    format!(
        "CREATE TABLE {db}.oracle_prices (
            timestamp DateTime,
            asset_code String EPHEMERAL, issuer_address String EPHEMERAL, contract_address String EPHEMERAL,
            asset_id UInt64 DEFAULT {id},
            oracle_name LowCardinality(String),
            price_usd Decimal(38, 14)
         ) ENGINE = ReplacingMergeTree
         PARTITION BY toYYYYMM(timestamp)
         ORDER BY (asset_id, oracle_name, timestamp)",
        id = oracle_id_expr("asset_code", "issuer_address", "contract_address"),
    )
}

async fn expected_pairs(c: &Client, rows: &[(u32, Identity, Identity)]) -> Vec<IdPair> {
    let mut out = Vec::with_capacity(rows.len());
    for (_, base, quote) in rows {
        out.push(IdPair {
            asset_id: server_id(c, *base).await,
            quote_asset_id: server_id(c, *quote).await,
        });
    }
    out
}

async fn stored_pairs(c: &Client, db: &str) -> Vec<IdPair> {
    c.query(&format!(
        "SELECT asset_id, quote_asset_id FROM {db}.candles ORDER BY timestamp"
    ))
    .fetch_all()
    .await
    .unwrap()
}

fn assert_refused<T: std::fmt::Debug>(
    res: Result<T, clickhouse::error::Error>,
    needle: &str,
    what: &str,
) {
    match res {
        Ok(v) => panic!("{what}: expected a refusal containing {needle:?}, got Ok({v:?})"),
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains(needle),
                "{what}: expected {needle:?} in the error, got: {msg}"
            );
        }
    }
}

fn insert_modes(base: &Client) -> [(&'static str, Client); 2] {
    [
        (
            "async_insert=0",
            base.clone().with_option("async_insert", "0"),
        ),
        (
            "async_insert=1,wait_for_async_insert=1",
            base.clone()
                .with_option("async_insert", "1")
                .with_option("wait_for_async_insert", "1"),
        ),
    ]
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn insert_and_inserter_through_ephemeral_columns_land_server_derived_ids() {
    let (admin, db) = scratch("writers").await;
    exec(&admin, &candles_ddl(&db, "candles", "DEFAULT")).await;
    let table = format!("{db}.candles");
    let rows = candle_rows();
    let want = expected_pairs(&admin, &rows).await;

    for (mode, client) in insert_modes(&admin) {
        exec(&admin, &format!("TRUNCATE TABLE {table}")).await;
        let mut ins = client.insert::<CandleIn>(&table).unwrap();
        for (ts, b, q) in &rows {
            ins.write(&candle(*ts, *b, *q)).await.unwrap();
        }
        ins.end()
            .await
            .unwrap_or_else(|e| panic!("Insert<T> [{mode}]: {e}"));
        assert_eq!(stored_pairs(&admin, &db).await, want, "Insert<T> [{mode}]");

        exec(&admin, &format!("TRUNCATE TABLE {table}")).await;
        let mut inserter = client.inserter::<CandleIn>(&table).unwrap();
        for (ts, b, q) in &rows {
            inserter.write(&candle(*ts, *b, *q)).unwrap();
        }
        let q = inserter
            .end()
            .await
            .unwrap_or_else(|e| panic!("Inserter<T> [{mode}]: {e}"));
        assert_eq!(q.rows, rows.len() as u64, "Inserter<T> [{mode}] row count");
        assert_eq!(
            stored_pairs(&admin, &db).await,
            want,
            "Inserter<T> [{mode}]"
        );
    }

    drop_db(&admin, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn identities_get_distinct_nonzero_ids_and_id_of_agrees_with_the_server() {
    let admin = Client::default().with_url(ch_url());
    let mut all = vec![XLM];
    all.extend(BASES);

    let mut ids = Vec::new();
    for ident in &all {
        let bound = server_id(&admin, *ident).await;
        let literal: u64 = admin
            .query(&format!("SELECT {}", id_of(ident.0, ident.1, ident.2)))
            .fetch_one()
            .await
            .unwrap();
        assert_eq!(
            literal, bound,
            "id_of literal vs bound strings for {ident:?}"
        );
        assert_ne!(bound, 0, "{ident:?} derived to 0");
        // Deterministic: the same identity asked twice is the same id.
        assert_eq!(
            server_id(&admin, *ident).await,
            bound,
            "{ident:?} not deterministic"
        );
        ids.push(bound);
    }
    let blank = server_id(&admin, BLANK).await;
    assert!(!ids.contains(&blank), "an identity derived to xxh3('::')");

    let mut distinct = ids.clone();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(
        distinct.len(),
        ids.len(),
        "two identities share an id: {ids:?}"
    );
    assert_ne!(
        server_id(&admin, USDC).await,
        server_id(&admin, USDC_LOWER).await,
        "case must be preserved"
    );
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_blank_identity_is_refused_on_candles_and_is_the_sentinel_on_oracle_prices() {
    let (admin, db) = scratch("blank").await;
    exec(&admin, &candles_ddl(&db, "candles", "DEFAULT")).await;
    exec(&admin, &oracle_ddl(&db)).await;
    let candles = format!("{db}.candles");

    for (mode, client) in insert_modes(&admin) {
        for (label, row) in [
            ("blank base", candle(1_790_000_000, BLANK, XLM)),
            ("blank quote", candle(1_790_000_000, XLM, BLANK)),
        ] {
            let mut ins = client.insert::<CandleIn>(&candles).unwrap();
            ins.write(&row).await.unwrap();
            assert_refused(ins.end().await, "Code: 469", &format!("{label} [{mode}]"));
        }
    }
    let n: u64 = admin
        .query(&format!("SELECT count() FROM {candles}"))
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(n, 0, "a refused row was stored");

    // A CHECK cannot name the EPHEMERAL inputs, so the guard has to sit on the ids.
    let on_inputs = candles_ddl(&db, "candles_c", "DEFAULT").replace(
        &derived_ids_check("derived_ids", &["asset_id", "quote_asset_id"]),
        "CONSTRAINT ident CHECK base_code != '' OR base_contract != ''",
    );
    assert_refused(
        admin.query(&on_inputs).execute().await,
        "Missing columns",
        "CHECK on EPHEMERAL columns",
    );

    let mut ins = admin
        .clone()
        .with_option("async_insert", "0")
        .insert::<OracleIn>(&format!("{db}.oracle_prices"))
        .unwrap();
    for (ts, ident) in [(1_790_000_000u32, BLANK), (1_790_000_060, XLM)] {
        ins.write(&OracleIn {
            timestamp: ts,
            asset_code: ident.0.into(),
            issuer_address: ident.1.into(),
            contract_address: ident.2.into(),
            oracle_name: "REDSTONE".into(),
            price_usd: 1,
        })
        .await
        .unwrap();
    }
    ins.end().await.unwrap();
    let got: Vec<u64> = admin
        .query(&format!(
            "SELECT asset_id FROM {db}.oracle_prices ORDER BY timestamp"
        ))
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(got, vec![0, server_id(&admin, XLM).await]);

    drop_db(&admin, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn materialized_ids_refuse_any_writer_that_names_them() {
    let (admin, db) = scratch("materialized").await;
    exec(&admin, &assets_ddl(&db)).await;
    exec(&admin, &candles_ddl(&db, "candles", "DEFAULT")).await;
    exec(&admin, &candles_ddl(&db, "candles_mat", "MATERIALIZED")).await;
    let client = admin.clone().with_option("async_insert", "0");
    let assets = format!("{db}.assets");

    let mut ins = client.insert::<AssetIn>(&assets).unwrap();
    for ident in [XLM, USDC, REPLACEMENT] {
        ins.write(&AssetIn {
            asset_code: ident.0.into(),
            asset_type: "classic".into(),
            issuer_address: ident.1.into(),
            contract_address: ident.2.into(),
        })
        .await
        .unwrap();
    }
    ins.end().await.expect("a registry writer without asset_id");

    let mut ins = client.insert::<AssetWithId>(&assets).unwrap();
    ins.write(&AssetWithId {
        asset_id: 42,
        asset_code: "BAD".into(),
        asset_type: "classic".into(),
        issuer_address: GBXYZ.into(),
        contract_address: String::new(),
    })
    .await
    .unwrap();
    assert_refused(
        ins.end().await,
        "ILLEGAL_COLUMN",
        "registry writer naming asset_id",
    );

    let mut ins = client.insert::<AssetIn>(&assets).unwrap();
    ins.write(&AssetIn {
        asset_code: String::new(),
        asset_type: "classic".into(),
        issuer_address: String::new(),
        contract_address: String::new(),
    })
    .await
    .unwrap();
    assert_refused(
        ins.end().await,
        "Code: 469",
        "blank identity in the registry",
    );

    for ident in [XLM, USDC, REPLACEMENT] {
        let got: u64 = admin
            .query(&format!(
                "SELECT asset_id FROM {assets} FINAL \
                 WHERE asset_code = ? AND issuer_address = ? AND contract_address = ?"
            ))
            .bind(ident.0)
            .bind(ident.1)
            .bind(ident.2)
            .fetch_one()
            .await
            .unwrap();
        assert_eq!(
            got,
            server_id(&admin, ident).await,
            "registry read-back for {ident:?}"
        );
    }

    // The enrichment / MV path writes ids it read: legal into DEFAULT ...
    let ids_insert = |table: &str| {
        format!(
            "INSERT INTO {db}.{table} (timestamp, asset_id, quote_asset_id, source, close, version) \
             SELECT toDateTime(1790000000), {}, {}, 'enrich', 1, 2",
            id_of(USDC.0, USDC.1, USDC.2),
            id_of(XLM.0, XLM.1, XLM.2),
        )
    };
    exec(&client, &ids_insert("candles")).await;
    let got: Vec<IdPair> = admin
        .query(&format!(
            "SELECT asset_id, quote_asset_id FROM {db}.candles"
        ))
        .fetch_all()
        .await
        .unwrap();
    assert_eq!(
        got,
        vec![IdPair {
            asset_id: server_id(&admin, USDC).await,
            quote_asset_id: server_id(&admin, XLM).await,
        }]
    );
    // ... and refused into a MATERIALIZED variant.
    assert_refused(
        client.query(&ids_insert("candles_mat")).execute().await,
        "ILLEGAL_COLUMN",
        "INSERT ... SELECT naming asset_id into MATERIALIZED",
    );

    drop_db(&admin, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn a_u64_reader_reads_a_uint32_table_through_to_uint64() {
    let (admin, db) = scratch("reader").await;
    exec(
        &admin,
        &format!(
            "CREATE TABLE {db}.old32 (asset_id UInt32, quote_asset_id UInt32) \
             ENGINE = MergeTree ORDER BY asset_id"
        ),
    )
    .await;
    exec(
        &admin,
        &format!("INSERT INTO {db}.old32 VALUES (4, 3), (111, 3), (7, 4)"),
    )
    .await;

    let got: Vec<IdPair> = admin
        .query(&format!(
            "SELECT toUInt64(asset_id) AS asset_id, toUInt64(quote_asset_id) AS quote_asset_id \
             FROM {db}.old32 ORDER BY asset_id"
        ))
        .fetch_all()
        .await
        .unwrap();
    let want = [(4, 3), (7, 4), (111, 3)].map(|(a, q)| IdPair {
        asset_id: a,
        quote_asset_id: q,
    });
    assert_eq!(got, want);

    // A u64 bind against the UInt32 column compares, it does not error.
    let n: u64 = admin
        .query(&format!(
            "SELECT count() FROM {db}.old32 WHERE asset_id = ?"
        ))
        .bind(server_id(&admin, USDC).await)
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(n, 0);

    drop_db(&admin, &db).await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_fixture_helpers_work_against_todays_init_sql() {
    let (admin, db) = scratch("fixture").await;
    let schema = prices_clickhouse::INIT_SQL
        .replace("prices.", &format!("{db}."))
        .replace("IF NOT EXISTS prices", &format!("IF NOT EXISTS {db}"));
    prices_clickhouse::apply_sql(&admin, &schema)
        .await
        .expect("apply init schema");

    let mut idents = vec![XLM];
    idents.extend(BASES);
    let rows: Vec<AssetFixture<'_>> = idents
        .iter()
        .map(|i| AssetFixture {
            code: i.0,
            asset_type: "classic",
            issuer: i.1,
            contract: i.2,
            sac: "",
        })
        .collect();
    exec(&admin, &fixture::assets_insert(&db, &rows)).await;

    for ident in &idents {
        let stored: u64 = admin
            .query(&format!(
                "SELECT toUInt64(asset_id) FROM {db}.assets FINAL \
                 WHERE asset_code = ? AND issuer_address = ? AND contract_address = ?"
            ))
            .bind(ident.0)
            .bind(ident.1)
            .bind(ident.2)
            .fetch_one()
            .await
            .unwrap();
        let fetched = fixture::fetch_id(&admin, ident.0, ident.1, ident.2).await;
        assert_eq!(stored, fetched, "fixture id for {ident:?}");
        let truncated: u64 = admin
            .query("SELECT toUInt64(toUInt32(xxh3(concat(?, ':', ?, ':', ?))))")
            .bind(ident.0)
            .bind(ident.1)
            .bind(ident.2)
            .fetch_one()
            .await
            .unwrap();
        assert_eq!(
            fetched, truncated,
            "fixture id is the UInt32 form for {ident:?}"
        );
    }
    let distinct: u64 = admin
        .query(&format!(
            "SELECT countDistinct(asset_id) FROM {db}.assets FINAL"
        ))
        .fetch_one()
        .await
        .unwrap();
    assert_eq!(distinct, idents.len() as u64);

    drop_db(&admin, &db).await;
}
