//! Task 0242 — the two BE reads behind the SAC resolver: BE's `is_sac` set and
//! the events-backfill proof preload, run as rendered against real tables.
//!
//!     tools/scripts/ignored-tests.sh   # all of them: CI runs exactly this on every Rust PR
//!     cargo test -p prices-ingest-core --test sac_proof_it -- --ignored --test-threads=1
//!
//! Each test DROPs and CREATEs scratch databases `it_0242_<case>_be` (BE's
//! `soroban_events` / `soroban_contracts`, DDL as in coverage_sweep_it.rs) and
//! `it_0242_<case>_prices` (our `init.sql`). The real `default` and `prices`
//! are never touched. Never run against a shared cluster.

use std::collections::HashSet;

use clickhouse::Client;
use prices_clickhouse::USDC_ISSUER;
use prices_clickhouse::asset_id::fixture::{AssetFixture, assets_insert};
use prices_ingest_core::{
    AssetIdentity, AssetRegistry, SacPreload, SacProofRow, apply_sac_proofs, load_sac_contracts,
    load_sac_proofs, preload_sac_resolver,
};

const XCR_ISSUER: &str = "GBLJBHWVORDFI4J7CLBDRPECMYT3XO5S6GERXGC74VXOJMZPLI6ZU3S7";
const XCR_SAC: &str = "CDJQXBQO5ICVQUPHZHW7SHOM56K2UNNPPAIXUUSA3XACEI6Q4JQLXNVI";

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn admin() -> Client {
    Client::default().with_url(ch_url())
}

async fn exec(sql: &str) {
    admin().query(sql).execute().await.expect(sql);
}

fn xcr() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "XCR".to_string(),
        issuer: XCR_ISSUER.to_string(),
    }
}

fn usdc() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "USDC".to_string(),
        issuer: USDC_ISSUER.to_string(),
    }
}

fn strkey(n: u8) -> String {
    stellar_strkey::Contract([n; 32]).to_string()
}

/// `[transfer, from, to, <last>]` in BE's typed-JSON shape.
fn topics(signature: &str, last: &str) -> String {
    let from = stellar_strkey::ed25519::PublicKey([1; 32]).to_string();
    format!(
        r#"[{{"type":"sym","value":"{signature}"}},{{"type":"address","value":"{from}"}},{{"type":"address","value":"{XCR_SAC}"}},{{"type":"string","value":"{last}"}}]"#
    )
}

struct Fixture {
    be: String,
    prices: String,
}

impl Fixture {
    /// Fresh scratch databases, and the contracts every case shares:
    /// - 1 XCR's SAC, is_sac, held as two RMT versions; classic absent from
    ///   `assets`; an `approve` and two `transfer`s.
    /// - 2 USDC's SAC, is_sac; its classic is in `assets` with `sac_address`.
    /// - 3 a non-SAC contract emitting `transfer` with XCR's SEP-11 topic.
    /// - 4 an is_sac contract with no events.
    async fn new(case: &str) -> Self {
        let f = Fixture {
            be: format!("it_0242_{case}_be"),
            prices: format!("it_0242_{case}_prices"),
        };
        for db in [&f.be, &f.prices] {
            exec(&format!("DROP DATABASE IF EXISTS {db}")).await;
            exec(&format!("CREATE DATABASE {db}")).await;
        }
        let init = prices_clickhouse::INIT_SQL
            .replace("prices.", &format!("{}.", f.prices))
            .replace(
                "IF NOT EXISTS prices",
                &format!("IF NOT EXISTS {}", f.prices),
            );
        prices_clickhouse::apply_sql(&admin(), &init)
            .await
            .expect("init schema");
        exec(&format!(
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
        ))
        .await;
        exec(&format!(
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
        ))
        .await;

        let usdc_sac = AssetRegistry::from_existing(vec![])
            .sac_address_of(&usdc())
            .expect("USDC derives a SAC");
        let (genuine, empty) = (strkey(3), strkey(4));
        // Two inserts, two parts: the RMT versions stay unmerged.
        for (id, contract, version, is_sac) in [
            (1, XCR_SAC, 1, true),
            (1, XCR_SAC, 2, true),
            (2, usdc_sac.as_str(), 1, true),
            (3, genuine.as_str(), 1, false),
            (4, empty.as_str(), 1, true),
        ] {
            f.contract(id, contract, version, is_sac).await;
        }
        let xcr_sep11 = format!("XCR:{XCR_ISSUER}");
        f.event(1, 100, "approve", "NOT-A-PROOF").await;
        f.event(1, 101, "transfer", &xcr_sep11).await;
        f.event(1, 102, "transfer", &xcr_sep11).await;
        f.event(2, 100, "transfer", &format!("USDC:{USDC_ISSUER}"))
            .await;
        f.event(3, 100, "transfer", &xcr_sep11).await;

        exec(&assets_insert(
            &f.prices,
            &[AssetFixture::new("USDC", "classic", USDC_ISSUER, "").with_sac(&usdc_sac)],
        ))
        .await;
        f
    }

    async fn contract(&self, id: i64, contract: &str, version: i64, is_sac: bool) {
        exec(&format!(
            "INSERT INTO {}.soroban_contracts (id, contract_id, wasm_uploaded_at_ledger, is_sac) \
             VALUES ({id}, '{contract}', {version}, {is_sac})",
            self.be
        ))
        .await;
    }

    async fn event(&self, id: i64, ledger: i64, signature: &str, last: &str) {
        exec(&format!(
            "INSERT INTO {}.soroban_events \
             (contract_id, ledger_sequence, transaction_index, operation_index, event_index, application_order, event_type, signature, topics_xdr, data_xdr) \
             VALUES ({id}, {ledger}, 0, 0, 0, 0, 1, '{signature}', '{}', '{{\"type\":\"i128\",\"value\":\"1\"}}')",
            self.be,
            topics(signature, last),
        ))
        .await;
    }

    async fn drop(self) {
        for db in [&self.be, &self.prices] {
            exec(&format!("DROP DATABASE IF EXISTS {db}")).await;
        }
    }
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_is_sac_set_holds_each_flagged_contract_once() {
    let f = Fixture::new("sacs").await;
    let got = load_sac_contracts(&admin(), &f.be).await.unwrap();
    let usdc_sac = AssetRegistry::from_existing(vec![])
        .sac_address_of(&usdc())
        .unwrap();
    assert_eq!(
        got,
        HashSet::from([XCR_SAC.to_string(), usdc_sac, strkey(4)]),
        "every is_sac contract once, the non-SAC one never"
    );
    f.drop().await;
}

#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_preload_returns_one_proof_per_sac_the_registry_cannot_resolve() {
    let f = Fixture::new("preload").await;
    let rows = load_sac_proofs(&admin(), &f.be, &f.prices).await.unwrap();
    assert_eq!(
        rows,
        [SacProofRow {
            contract_id: XCR_SAC.to_string(),
            sep11: format!("XCR:{XCR_ISSUER}"),
        }],
        "USDC is resolvable from assets, the non-SAC is no candidate, the eventless SAC has no proof"
    );

    let mut assets = AssetRegistry::from_existing(vec![]);
    assert_eq!(assets.sac_address_of(&xcr()).as_deref(), Some(XCR_SAC));
    assert_eq!(apply_sac_proofs(&mut assets, &rows), (1, 0));
    assert_eq!(assets.resolve_sac(XCR_SAC), Some(xcr()));
    f.drop().await;
}

/// events-backfill's run-start step, end to end: the candidates are BE's
/// `is_sac` set, and the one provable SAC `assets` cannot resolve is learnt.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn the_run_start_preload_arms_candidates_and_learns_the_proofs() {
    let f = Fixture::new("arm").await;
    let mut assets = AssetRegistry::from_existing(vec![]);
    let got = preload_sac_resolver(&admin(), &f.be, &f.prices, &mut assets)
        .await
        .unwrap();
    assert_eq!(
        got,
        SacPreload {
            candidates: 3,
            proofs: 1,
            verified: 1,
            rejected: 0,
        }
    );
    assert_eq!(assets.resolve_sac(XCR_SAC), Some(xcr()));
    assert!(!assets.is_unproven_sac(XCR_SAC));
    assert!(assets.is_unproven_sac(&strkey(4)), "eventless SAC: skipped");
    assert!(!assets.is_unproven_sac(&strkey(3)), "not a SAC: a token");
    f.drop().await;
}

/// IN-04: the `is_sac` read has no `FINAL`. A contract BE corrects to
/// `is_sac = false` stays a candidate (skipped, never minted) until BE's parts
/// merge, and leaves the set once they have. Merges are stopped so the two
/// versions cannot merge before the first read.
#[tokio::test]
#[ignore = "requires ClickHouse — run via tools/scripts/ignored-tests.sh (CI runs it)"]
async fn an_unmerged_is_sac_correction_stays_a_candidate_until_merged() {
    let f = Fixture::new("flip").await;
    let table = format!("{}.soroban_contracts", f.be);
    exec(&format!("SYSTEM STOP MERGES {table}")).await;
    let flipped = strkey(5);
    f.contract(5, &flipped, 1, true).await;
    f.contract(5, &flipped, 1, false).await;
    let got = load_sac_contracts(&admin(), &f.be).await.unwrap();
    assert!(
        got.contains(&flipped),
        "unmerged: either version may answer"
    );

    exec(&format!("SYSTEM START MERGES {table}")).await;
    exec(&format!("OPTIMIZE TABLE {table} FINAL")).await;
    let got = load_sac_contracts(&admin(), &f.be).await.unwrap();
    assert!(!got.contains(&flipped), "merged: the last insert wins");
    assert!(got.contains(XCR_SAC));
    f.drop().await;
}
