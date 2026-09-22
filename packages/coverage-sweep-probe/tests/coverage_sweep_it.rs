//! Integration tests for the coverage sweep (task 0100) against a real
//! ClickHouse (local 26.3.10.60).
//!
//! The unit tests pin the rendered SQL's *shape*; they cannot catch a query
//! that fails to **execute or deserialize** (a `Nullable` column into a
//! non-`Option` field shipped a broken probe once, PR #97), nor prove that the
//! filters select what they claim. These tests run the production statement,
//! through the production client wrapping (`with_readable_errors` +
//! `with_execution_bound`), as a user shaped like `prices_writer` after BE's
//! D5-option-A change: `SELECT, INSERT, OPTIMIZE` on the prices database plus
//! `SELECT` on exactly the two BE tables.
//!
//! ```text
//! cargo test -p coverage-sweep-probe --test coverage_sweep_it -- --ignored --test-threads=1
//! ```
//!
//! ⚠️ **Destructive.** Each test DROPs and CREATEs scratch databases
//! (`cov_it_<case>_be`, `cov_it_<case>_prices`) and CREATEs/DROPs a user on the
//! server `CLICKHOUSE_URL` names (default `http://localhost:8123`). **Never
//! point it at a shared or production cluster.** The real `default` database
//! is never touched: the SQL is rendered against the scratch names.

use clickhouse::Client;
use coverage_sweep_probe::sweep::{SweepError, SweepReport, run_sweep};
use coverage_sweep_probe::{AllowList, SWEEP_MAX_EXECUTION_SECS, unclassified_metrics};

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn admin() -> Client {
    prices_clickhouse::with_readable_errors(Client::default().with_url(ch_url()))
}

async fn exec(c: &Client, sql: &str) {
    c.query(sql).execute().await.expect(sql);
}

/// A valid `C…` strkey for fixture contract `n`.
fn strkey(n: u8) -> String {
    stellar_strkey::Contract([n; 32]).to_string()
}

const AQUARIUS_ROUTER: &str = "CBQDHNBFBZYE4MKPWBSJOPIYLW4SFSXAXUTSXJN76GNKYVYPCKWC6QUK";
const SUSHI_V3_WASM: &str = "003710b383f9da7d650a7f719a7be479110266427817ebbed61d924505fcd7c7";
const AQUARIUS_WASM: &str = "06f4207b0c9ef78cc595e075ded8fa40e73fdb8346e5f9281068a2d4ba1e5037";
/// Stand-in for the unclassified `8abc2891…` POOL/swap family (full hash not
/// needed: it is on no list).
const COMET_LIKE_WASM: &str = "8abc289100000000000000000000000000000000000000000000000000000000";
const SOROSWAP_PAIR_WASM: &str = "aa00000000000000000000000000000000000000000000000000000000000000";

const POOL_SWAP: &str = r#"[{"type":"sym","value":"POOL"},{"type":"sym","value":"swap"}]"#;
const SOROSWAP_PAIR_SWAP: &str =
    r#"[{"type":"string","value":"SoroswapPair"},{"type":"sym","value":"swap"}]"#;
const ROUTER_SWAP_VEC: &str = r#"[{"type":"sym","value":"swap"},{"type":"vec","value":[]}]"#;
const SWAP: &str = r#"[{"type":"sym","value":"swap"}]"#;
const DATA_MAP: &str = r#"{"type":"map","value":[]}"#;

/// Scratch BE + prices databases for one test case.
struct Fixture {
    be: String,
    prices: String,
    user: String,
}

impl Fixture {
    /// DROP/CREATE `cov_it_<case>_be` (BE's two tables, DDL mirrored from BE's
    /// `crates/db-clickhouse/schema/init.sql`: `soroban_events` as re-keyed by
    /// BE task 0541 (@1c3f0603, live on production 2026-09-22),
    /// `soroban_contracts` as of @129ab5d1, the columns the sweep reads) and
    /// `cov_it_<case>_prices` (`pool_registry` from our `init.sql`).
    async fn new(case: &str) -> Self {
        let f = Fixture {
            be: format!("cov_it_{case}_be"),
            prices: format!("cov_it_{case}_prices"),
            user: format!("cov_it_{case}_writer"),
        };
        let a = admin();
        for db in [&f.be, &f.prices] {
            exec(&a, &format!("DROP DATABASE IF EXISTS {db}")).await;
            exec(&a, &format!("CREATE DATABASE {db}")).await;
        }
        exec(
            &a,
            &format!(
                "CREATE TABLE {}.soroban_events (
                    contract_id        Int64,
                    ledger_sequence    Int64,
                    transaction_index  UInt32,
                    operation_index    UInt16,
                    event_index        UInt32,
                    application_order  Int16,
                    event_type         Int16,
                    signature          LowCardinality(Nullable(String)),
                    topics_xdr         String CODEC(ZSTD(3)),
                    data_xdr           String CODEC(ZSTD(3))
                )
                ENGINE = ReplacingMergeTree
                PARTITION BY intDiv(ledger_sequence, 500000)
                ORDER BY (contract_id, ledger_sequence, transaction_index, operation_index, event_index)",
                f.be
            ),
        )
        .await;
        exec(
            &a,
            &format!(
                "CREATE TABLE {}.soroban_contracts (
                    id                       Int64,
                    contract_id              String,
                    wasm_hash                Nullable(FixedString(32)),
                    wasm_uploaded_at_ledger  Int64 DEFAULT 0,
                    deployer_id              Nullable(Int64),
                    deployed_at_ledger       Nullable(Int64),
                    contract_type            Nullable(Int16),
                    is_sac                   Bool
                )
                ENGINE = ReplacingMergeTree(wasm_uploaded_at_ledger)
                ORDER BY (contract_id)",
                f.be
            ),
        )
        .await;
        exec(
            &a,
            &format!(
                "CREATE TABLE {}.pool_registry (
                    contract_id   String,
                    venue         LowCardinality(String),
                    token0        String        DEFAULT '',
                    token1        String        DEFAULT '',
                    pool_type     UInt32        DEFAULT 0,
                    wasm_hash     String        DEFAULT '',
                    updated_at    DateTime      DEFAULT now()
                )
                ENGINE = ReplacingMergeTree(updated_at)
                ORDER BY (contract_id)",
                f.prices
            ),
        )
        .await;
        f
    }

    /// One event of contract surrogate `id` at `ledger`, in the transaction at
    /// position `tx` of that ledger (BE 0541: a transaction is
    /// `(ledger_sequence, transaction_index)`).
    async fn event(&self, id: i64, ledger: i64, tx: i64, topics: &str) {
        self.event_of_type(id, ledger, tx, topics, 1).await;
    }

    /// Same, with BE's `event_type` (0 system, 1 contract, 2 diagnostic).
    async fn event_of_type(&self, id: i64, ledger: i64, tx: i64, topics: &str, event_type: i16) {
        exec(
            &admin(),
            &format!(
                "INSERT INTO {}.soroban_events \
                 (contract_id, ledger_sequence, transaction_index, operation_index, event_index, application_order, event_type, signature, topics_xdr, data_xdr) \
                 VALUES ({id}, {ledger}, {tx}, 0, 0, {tx}, {event_type}, NULL, '{topics}', '{DATA_MAP}')",
                self.be,
            ),
        )
        .await;
    }

    /// `n` events of contract `id`, one per ledger from `from_ledger`.
    async fn events(&self, id: i64, from_ledger: i64, n: i64, topics: &str) {
        for i in 0..n {
            self.event(id, from_ledger + i, 0, topics).await;
        }
    }

    /// BE's contract row for surrogate `id`.
    async fn contract(&self, id: i64, strkey: &str, wasm: Option<&str>) {
        let wasm = wasm.map_or("NULL".to_string(), |w| format!("unhex('{w}')"));
        exec(
            &admin(),
            &format!(
                "INSERT INTO {}.soroban_contracts (id, contract_id, wasm_hash, is_sac) \
                 VALUES ({id}, '{strkey}', {wasm}, false)",
                self.be
            ),
        )
        .await;
    }

    async fn register(&self, strkey: &str, venue: &str) {
        exec(
            &admin(),
            &format!(
                "INSERT INTO {}.pool_registry (contract_id, venue) VALUES ('{strkey}', '{venue}')",
                self.prices
            ),
        )
        .await;
    }

    /// Run the sweep as a `prices_writer`-shaped user through the production
    /// client wrapping. `be_grants = false` is production before BE ships the
    /// two SELECTs. The user is dropped BEFORE the caller asserts anything
    /// (ohlcv_it pattern): a red assertion must not leave a passwordless
    /// account behind.
    async fn sweep_as_writer(
        &self,
        allow: &AllowList,
        be_grants: bool,
    ) -> Result<SweepReport, SweepError> {
        let a = admin();
        let u = &self.user;
        exec(&a, &format!("DROP USER IF EXISTS {u}")).await;
        exec(&a, &format!("CREATE USER {u} IDENTIFIED WITH no_password")).await;
        exec(
            &a,
            &format!("GRANT SELECT, INSERT, OPTIMIZE ON {}.* TO {u}", self.prices),
        )
        .await;
        if be_grants {
            exec(
                &a,
                &format!("GRANT SELECT ON {}.soroban_events TO {u}", self.be),
            )
            .await;
            exec(
                &a,
                &format!("GRANT SELECT ON {}.soroban_contracts TO {u}", self.be),
            )
            .await;
        }

        let client = prices_clickhouse::with_execution_bound(
            prices_clickhouse::with_readable_errors(
                Client::default()
                    .with_url(ch_url())
                    .with_user(u.as_str())
                    .with_database(self.prices.as_str()),
            ),
            SWEEP_MAX_EXECUTION_SECS,
        );
        let result = run_sweep(&client, &self.be, &self.prices, allow).await;

        exec(&a, &format!("DROP USER IF EXISTS {u}")).await;
        result
    }

    async fn drop(self) {
        let a = admin();
        exec(&a, &format!("DROP USER IF EXISTS {}", self.user)).await;
        for db in [&self.be, &self.prices] {
            exec(&a, &format!("DROP DATABASE IF EXISTS {db}")).await;
        }
    }
}

fn ids(rows: &[coverage_sweep_probe::SweepRow]) -> Vec<String> {
    rows.iter().map(|r| r.display_id()).collect()
}

/// The tracer: four emitters in the window, one of each fate.
///
/// (A) unregistered POOL/swap, wasm 8abc2891…      → unclassified (3 events)
/// (B) registered SoroswapPair/swap pair             → not returned by SQL
/// (C) Aquarius router, a `[[contract]]` entry       → returned, allow-listed
/// (D) SushiSwap V3 pool, a `[[wasm]]` family entry  → returned, allow-listed
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn unclassified_contract_is_reported_and_the_rest_are_not() {
    let f = Fixture::new("tracer").await;
    let (a, b, d) = (strkey(1), strkey(2), strkey(4));
    f.contract(1, &a, Some(COMET_LIKE_WASM)).await;
    f.contract(2, &b, Some(SOROSWAP_PAIR_WASM)).await;
    f.contract(3, AQUARIUS_ROUTER, Some(AQUARIUS_WASM)).await;
    f.contract(4, &d, Some(SUSHI_V3_WASM)).await;
    f.register(&b, "soroswap").await;

    f.events(1, 1_000_000, 3, POOL_SWAP).await;
    f.events(2, 1_000_000, 5, SOROSWAP_PAIR_SWAP).await;
    f.events(3, 1_000_000, 4, ROUTER_SWAP_VEC).await;
    f.events(4, 1_000_000, 2, SWAP).await;

    let allow = AllowList::embedded().unwrap();
    let result = f.sweep_as_writer(&allow, true).await;
    let report = result.expect("sweep runs as a prices_writer-shaped user");

    assert_eq!(report.max_ledger, 1_000_004);
    assert_eq!((report.lo, report.hi), (1_000_004 - 221_178, 1_000_004));
    assert_eq!(report.rows_total, 3, "B is registered: {report:?}");
    assert_eq!(ids(&report.unclassified), vec![a.clone()]);
    let row = &report.unclassified[0];
    assert_eq!(row.events, 3);
    assert_eq!(row.txs, 3);
    assert_eq!(row.wasm.as_deref(), Some(COMET_LIKE_WASM));
    assert_eq!(row.top_action, "POOL / swap");
    assert_eq!(row.top_shape, "sym|sym -> map");

    let mut allowlisted: Vec<(String, String)> = report
        .allowlisted
        .iter()
        .map(|(r, k)| (r.display_id(), k.clone()))
        .collect();
    allowlisted.sort();
    let mut expected = vec![
        (
            AQUARIUS_ROUTER.to_string(),
            format!("contract:{AQUARIUS_ROUTER}"),
        ),
        (d.clone(), format!("wasm:{SUSHI_V3_WASM}")),
    ];
    expected.sort();
    assert_eq!(allowlisted, expected);

    let metrics = unclassified_metrics(&report.unclassified);
    assert_eq!(metrics.len(), 2);
    assert_eq!(
        (metrics[0].name, metrics[0].value),
        ("UnclassifiedSwapContracts", 1.0)
    );
    assert_eq!(
        (metrics[1].name, metrics[1].value),
        ("UnclassifiedSwapEvents", 3.0)
    );

    f.drop().await;
}

/// (1) An Address topic that spells SWAP is not an action: only `sym`/`string`
/// topics may match. The fixture is the real production row
/// (`GAA574…SWAPN…`) that pulled SAC events into the first draft.
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn an_address_topic_spelling_swap_is_not_reported() {
    let f = Fixture::new("addr").await;
    let (sac, control) = (strkey(10), strkey(11));
    f.contract(10, &sac, None).await;
    f.contract(11, &control, Some(COMET_LIKE_WASM)).await;
    f.events(
        10,
        2_000_000,
        4,
        r#"[{"type":"sym","value":"fee"},{"type":"address","value":"GAA574QTUD4JAFCXNI2HKFTSWAPN5KQTR2UPB5OFRCYHEPCP7IGZ73DD"}]"#,
    )
    .await;
    f.events(11, 2_000_000, 1, POOL_SWAP).await;

    let result = f
        .sweep_as_writer(&AllowList::embedded().unwrap(), true)
        .await;
    let report = result.expect("sweep runs");
    assert_eq!(ids(&report.unclassified), vec![control]);
    assert_eq!(report.rows_total, 1);
    f.drop().await;
}

/// (1b) Only contract events count. A diagnostic `fn_return` names the invoked
/// function at topic[1], so a call to `swap` on a contract that emits no swap
/// event must not be reported (review of PR #332; BE's table may store type 2).
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn a_diagnostic_event_naming_swap_is_not_reported() {
    let f = Fixture::new("diag").await;
    let (caller, control) = (strkey(12), strkey(13));
    f.contract(12, &caller, None).await;
    f.contract(13, &control, Some(COMET_LIKE_WASM)).await;
    let fn_return = r#"[{"type":"sym","value":"fn_return"},{"type":"sym","value":"swap"}]"#;
    for i in 0..3 {
        f.event_of_type(12, 2_100_000 + i, 0, fn_return, 2).await;
        f.event_of_type(12, 2_100_000 + i, 1, SWAP, 0).await;
    }
    f.events(13, 2_100_000, 1, POOL_SWAP).await;

    let report = f
        .sweep_as_writer(&AllowList::embedded().unwrap(), true)
        .await
        .expect("sweep runs");
    assert_eq!(ids(&report.unclassified), vec![control]);
    assert_eq!(report.rows_total, 1);
    f.drop().await;
}

/// (2) A `string`-typed topic[0] and an action spelled inside a longer symbol
/// are both matched: SoroswapAggregator(string)/swap and AtomicSwapV2.
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn string_typed_and_compound_actions_are_reported() {
    let f = Fixture::new("strtopic").await;
    let (agg, atomic) = (strkey(20), strkey(21));
    f.contract(20, &agg, Some(SOROSWAP_PAIR_WASM)).await;
    f.contract(21, &atomic, Some(COMET_LIKE_WASM)).await;
    f.events(
        20,
        3_000_000,
        3,
        r#"[{"type":"string","value":"SoroswapAggregator"},{"type":"sym","value":"swap"}]"#,
    )
    .await;
    f.events(
        21,
        3_000_000,
        2,
        r#"[{"type":"sym","value":"AtomicSwapV2"},{"type":"u64","value":"7"}]"#,
    )
    .await;

    let result = f
        .sweep_as_writer(&AllowList::embedded().unwrap(), true)
        .await;
    let report = result.expect("sweep runs");
    // ORDER BY events DESC.
    assert_eq!(ids(&report.unclassified), vec![agg, atomic]);
    assert_eq!(report.unclassified[0].events, 3);
    assert_eq!(
        report.unclassified[0].top_action,
        "SoroswapAggregator / swap"
    );
    assert_eq!(report.unclassified[1].events, 2);
    f.drop().await;
}

/// (3) The window is `[max − SWEEP_WINDOW_LEDGERS, max]`, inclusive, with
/// `max` read from the events table: events below `lo` count nowhere.
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn only_in_window_events_count() {
    let f = Fixture::new("window").await;
    let max = 1_000_000;
    let lo = max - coverage_sweep_probe::sweep::SWEEP_WINDOW_LEDGERS;
    let (old, straddle, edge) = (strkey(30), strkey(31), strkey(32));
    f.contract(30, &old, None).await;
    f.contract(31, &straddle, None).await;
    f.contract(32, &edge, None).await;
    // Only before the window.
    f.events(30, 700_000, 5, SWAP).await;
    // Both sides: two before, two inside (the max ledger among them).
    f.events(31, 700_000, 2, SWAP).await;
    f.event(31, 900_000, 0, SWAP).await;
    f.event(31, max, 0, SWAP).await;
    // One ledger before lo, and exactly on lo (inclusive).
    f.event(32, lo - 1, 0, SWAP).await;
    f.event(32, lo, 0, SWAP).await;

    let result = f
        .sweep_as_writer(&AllowList::embedded().unwrap(), true)
        .await;
    let report = result.expect("sweep runs");
    assert_eq!((report.max_ledger, report.lo, report.hi), (max, lo, max));
    assert_eq!(ids(&report.unclassified), vec![straddle, edge]);
    let s = &report.unclassified[0];
    assert_eq!((s.events, s.first_ledger, s.last_ledger), (2, 900_000, max));
    let e = &report.unclassified[1];
    assert_eq!((e.events, e.first_ledger), (1, lo));
    f.drop().await;
}

/// (4) An emitter BE has not resolved in `soroban_contracts` is still
/// reported, by surrogate, never silently lost.
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn an_unresolved_contract_is_reported_by_surrogate() {
    let f = Fixture::new("unresolved").await;
    f.events(99, 4_000_000, 2, POOL_SWAP).await;

    let result = f
        .sweep_as_writer(&AllowList::embedded().unwrap(), true)
        .await;
    let report = result.expect("sweep runs");
    assert_eq!(report.unclassified.len(), 1, "{report:?}");
    let row = &report.unclassified[0];
    assert_eq!(row.strkey, "");
    assert_eq!(row.wasm, None);
    assert_eq!(row.contract_surrogate, 99);
    assert_eq!(row.display_id(), "unresolved:99");
    f.drop().await;
}

/// (5) An empty events table is a misconfiguration, never a clean run.
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn an_empty_events_table_is_an_error() {
    let f = Fixture::new("empty").await;
    let result = f
        .sweep_as_writer(&AllowList::embedded().unwrap(), true)
        .await;
    assert!(
        matches!(result, Err(SweepError::EmptyEventsTable)),
        "{result:?}"
    );
    f.drop().await;
}

/// (6) Back-test shape (BRIEF phase 5): the SushiSwap V3 family is hidden
/// only by its `[[wasm]]` entry. Without that entry, every pool of the
/// family is reported, which is how the sweep would have caught task 0290.
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn the_sushiswap_family_is_reported_without_its_wasm_entry() {
    let f = Fixture::new("backtest").await;
    for (i, n) in [(40u8, 5i64), (41, 2), (42, 1)] {
        f.contract(i64::from(i), &strkey(i), Some(SUSHI_V3_WASM))
            .await;
        f.events(i64::from(i), 5_000_000, n, SWAP).await;
    }

    let embedded = AllowList::embedded().unwrap();
    let with_entry = f
        .sweep_as_writer(&embedded, true)
        .await
        .expect("sweep runs");
    let without = AllowList {
        wasm: Vec::new(),
        ..embedded.clone()
    };
    let without_entry = f.sweep_as_writer(&without, true).await.expect("sweep runs");

    assert!(with_entry.unclassified.is_empty(), "{with_entry:?}");
    assert_eq!(with_entry.allowlisted.len(), 3);
    assert!(
        with_entry
            .allowlisted
            .iter()
            .all(|(_, k)| k == &format!("wasm:{SUSHI_V3_WASM}"))
    );

    assert_eq!(
        ids(&without_entry.unclassified),
        vec![strkey(40), strkey(41), strkey(42)]
    );
    let m = unclassified_metrics(&without_entry.unclassified);
    assert_eq!(
        m.iter().map(|m| (m.name, m.value)).collect::<Vec<_>>(),
        vec![
            ("UnclassifiedSwapContracts", 3.0),
            ("UnclassifiedSwapEvents", 8.0)
        ]
    );
    f.drop().await;
}

/// (7) Production before BE ships D5 option A: prices_writer without the two
/// SELECTs gets Code 497, and the sweep returns Err — never Ok, which would
/// publish nothing and leave the NOT_BREACHING alarm green forever.
#[tokio::test]
#[ignore = "requires ClickHouse (local 26.3.10.60; cargo test -- --ignored)"]
async fn missing_be_grants_fail_the_sweep() {
    let f = Fixture::new("nogrant").await;
    f.contract(50, &strkey(50), Some(COMET_LIKE_WASM)).await;
    f.events(50, 6_000_000, 2, POOL_SWAP).await;

    let result = f
        .sweep_as_writer(&AllowList::embedded().unwrap(), false)
        .await;
    match result {
        Ok(report) => panic!("a user without the BE grants must not sweep: {report:?}"),
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("497") || msg.contains("ACCESS_DENIED"),
                "expected ACCESS_DENIED, got: {msg}"
            );
        }
    }
    f.drop().await;
}
