//! Task 0242: a SAC never becomes a second asset identity on the live path.
//!
//! A SAC's own `transfer`/`mint`/`burn`/`clawback` names its asset (SEP-11) in
//! the last topic. `process_ledger` checks that claim against the emitter's
//! address and resolves the SAC to its classic identity, wherever the event sits
//! in the transaction. A contract BE flags `is_sac` that no proof resolves is
//! skipped and counted, never minted as a `Contract` identity.
//!
//! The ledgers are synthetic, like `pool_registry_persist.rs`, so these run
//! everywhere.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::{Arc, Mutex};

use extractors_core::Venue;
use prices_ingest_core::{
    AssetIdentity, AssetRegistry, LedgerSoroban, OhlcvCandle, OracleSample, PoolRegistryRow,
    Registries, process_ledger,
};
use prices_ledger_processor::{
    cursor::{Cursor, StubFileCursor},
    galexie_key::ledger_s3_key,
    object_fetcher::{FetchError, ObjectFetcher},
    reconcile::Reconciler,
    sink::{CandleSink, SinkError, with_sac_candidates},
};
use stellar_xdr::{
    ContractEvent, ContractEventBody, ContractEventType, ContractEventV0, ContractId, Hash,
    Int128Parts, LedgerCloseMeta, LedgerCloseMetaBatch, LedgerCloseMetaV2, Limits, OperationMetaV2,
    ScAddress, ScString, ScSymbol, ScVal, TimePoint, TransactionMeta, TransactionMetaV4,
    TransactionResultMetaV1, WriteXdr,
};
use tempfile::tempdir;

const FIRST: u64 = 70_000_000;
const BASE_MINUTE: u64 = 1_800_000_000 / 60 * 60;

// Public on-chain identifiers: XCR, its SAC, and the XLM SAC.
const XCR_ISSUER: &str = "GBLJBHWVORDFI4J7CLBDRPECMYT3XO5S6GERXGC74VXOJMZPLI6ZU3S7";
const XCR_SAC: &str = "CDJQXBQO5ICVQUPHZHW7SHOM56K2UNNPPAIXUUSA3XACEI6Q4JQLXNVI";
const XLM_SAC: &str = "CAS3J7GYLGXMF6TDJBBYYSE3HQ6BBSMLNUQ34T6TZMYMW2EVH34XOWMA";

const POOL: [u8; 32] = [0x55; 32];
const IMPOSTOR: [u8; 32] = [0x77; 32];
const GENUINE_TOKEN: [u8; 32] = [0x22; 32];

fn xcr() -> AssetIdentity {
    AssetIdentity::Credit {
        code: "XCR".to_string(),
        issuer: XCR_ISSUER.to_string(),
    }
}

fn xcr_sep11() -> String {
    format!("XCR:{XCR_ISSUER}")
}

fn strkey(bytes: [u8; 32]) -> String {
    ContractId(Hash(bytes)).to_string()
}

fn cid(strkey: &str) -> ContractId {
    ContractId::from_str(strkey).unwrap()
}

fn addr(strkey: &str) -> ScVal {
    ScVal::Address(ScAddress::Contract(cid(strkey)))
}

fn sym(s: &str) -> ScVal {
    ScVal::Symbol(ScSymbol(s.try_into().unwrap()))
}

fn i128v(lo: u64) -> ScVal {
    ScVal::I128(Int128Parts { hi: 0, lo })
}

fn event(emitter: &str, topics: Vec<ScVal>, data: ScVal) -> ContractEvent {
    ContractEvent {
        ext: Default::default(),
        contract_id: Some(cid(emitter)),
        type_: ContractEventType::Contract,
        body: ContractEventBody::V0(ContractEventV0 {
            topics: topics.try_into().unwrap(),
            data,
        }),
    }
}

/// A SAC-shaped `transfer` from `emitter`, naming `sep11` in its last topic.
fn sac_transfer(emitter: &str, sep11: &str) -> ContractEvent {
    event(
        emitter,
        vec![
            sym("transfer"),
            addr(&strkey([0x66; 32])),
            addr(&strkey(POOL)),
            ScVal::String(ScString(sep11.try_into().unwrap())),
        ],
        i128v(10_000_000),
    )
}

/// An Aquarius pool `trade` on [`POOL`]: `token_in` sold for `token_out`.
fn trade(token_in: &str, token_out: &str) -> ContractEvent {
    event(
        &strkey(POOL),
        vec![
            sym("trade"),
            addr(token_in),
            addr(token_out),
            addr(&strkey([0x66; 32])),
        ],
        ScVal::Vec(Some(
            vec![i128v(10_000_000), i128v(20_000_000), i128v(3_000)]
                .try_into()
                .unwrap(),
        )),
    )
}

/// One ledger whose single transaction carries `events`, in order.
fn lcm_with(seq: u64, close_time: u64, events: Vec<ContractEvent>) -> LedgerCloseMeta {
    let mut v2 = LedgerCloseMetaV2::default();
    v2.ledger_header.header.ledger_seq = seq as u32;
    v2.ledger_header.header.scp_value.close_time = TimePoint(close_time);
    if !events.is_empty() {
        let op = OperationMetaV2 {
            events: events.try_into().unwrap(),
            ..Default::default()
        };
        let meta = TransactionMetaV4 {
            operations: vec![op].try_into().unwrap(),
            ..Default::default()
        };
        v2.tx_processing = vec![TransactionResultMetaV1 {
            tx_apply_processing: TransactionMeta::V4(meta),
            ..Default::default()
        }]
        .try_into()
        .unwrap();
    }
    LedgerCloseMeta::V2(v2)
}

/// The same ledger, encoded as Galexie writes it.
fn ledger_with(seq: u64, close_time: u64, events: Vec<ContractEvent>) -> Vec<u8> {
    let batch = LedgerCloseMetaBatch {
        start_sequence: seq as u32,
        end_sequence: seq as u32,
        ledger_close_metas: vec![lcm_with(seq, close_time, events)].try_into().unwrap(),
    };
    let xdr = batch.to_xdr(Limits::none()).unwrap();
    zstd::encode_all(&xdr[..], 0).unwrap()
}

fn pool_registries() -> Registries {
    let mut reg = Registries::new();
    reg.venue.insert(strkey(POOL), Venue::Aquarius);
    reg
}

/// Through the production step, so every case here runs the live arming.
fn with_candidates(assets: AssetRegistry) -> AssetRegistry {
    with_sac_candidates(assets, HashSet::from([XCR_SAC.to_string()]))
}

fn run(events: Vec<ContractEvent>, assets: &mut AssetRegistry) -> LedgerSoroban {
    process_ledger(
        &lcm_with(FIRST, BASE_MINUTE, events),
        &mut pool_registries(),
        assets,
    )
}

fn pending(assets: &AssetRegistry) -> Vec<AssetIdentity> {
    assets.pending_new().cloned().collect()
}

fn holds_contract(assets: &AssetRegistry, addr: &str) -> bool {
    assets
        .assets()
        .any(|a| *a == AssetIdentity::Contract(addr.to_string()))
}

/// `main.rs` needs the `lambda` feature and AWS, so no test runs it. Its cold
/// start must build the registry through `load_ingest_registry`: a plain
/// `load_registry` leaves the candidate set empty, and every unproven SAC would
/// mint a `Contract` identity again with all other tests green.
#[test]
fn the_lambda_cold_start_arms_the_sac_candidates() {
    let main = include_str!("../src/main.rs");
    assert!(main.contains("sink.load_ingest_registry()"));
    assert!(
        !main.contains(".load_registry()"),
        "a registry without candidates"
    );
    let sink = include_str!("../src/sink/mod.rs");
    assert!(sink.contains("Ok(with_sac_candidates(registry?, sac_contracts?))"));
}

/// The fixture constants are pinned by our own derivation, not trusted.
#[test]
fn the_fixture_sacs_are_the_derived_ones() {
    let assets = AssetRegistry::from_existing(vec![]);
    assert_eq!(assets.sac_address_of(&xcr()).as_deref(), Some(XCR_SAC));
    assert_eq!(
        assets.sac_address_of(&AssetIdentity::Native).as_deref(),
        Some(XLM_SAC)
    );
}

/// T2 and T4: the proof resolves the SAC whether it comes before or after the
/// trade in the transaction, on an empty registry.
#[test]
fn a_sac_proven_in_its_transaction_trades_as_its_classic_in_either_order() {
    let cases = [
        (
            "proof first",
            vec![sac_transfer(XCR_SAC, &xcr_sep11()), trade(XLM_SAC, XCR_SAC)],
        ),
        (
            "proof after",
            vec![trade(XLM_SAC, XCR_SAC), sac_transfer(XCR_SAC, &xcr_sep11())],
        ),
    ];
    for (name, events) in cases {
        for candidates in [false, true] {
            let mut assets = AssetRegistry::from_existing(vec![]);
            if candidates {
                assets = with_candidates(assets);
            }
            let out = run(events.clone(), &mut assets);
            assert_eq!(out.amm_ticks.len(), 1, "{name}");
            let (source, tick) = &out.amm_ticks[0];
            assert_eq!(*source, "aquarius", "{name}");
            assert_eq!(tick.base, xcr(), "{name}");
            assert_eq!(tick.quote, AssetIdentity::Native, "{name}");
            assert!(out.sac_unproven.is_empty(), "{name}");
            assert!(pending(&assets).contains(&xcr()), "{name}");
            assert!(!holds_contract(&assets, XCR_SAC), "{name}: no Contract");
        }
    }
}

/// T3: one identity whichever path sees the asset first.
#[test]
fn classic_and_sac_meet_on_one_identity_in_either_order() {
    // (a) The SDEX path interns the classic; the SAC then resolves through it,
    // with no proof in the transaction.
    let mut assets = with_candidates(AssetRegistry::from_existing(vec![]));
    assert!(assets.intern(&xcr()));
    let out = run(vec![trade(XLM_SAC, XCR_SAC)], &mut assets);
    assert_eq!(out.amm_ticks.len(), 1);
    assert_eq!(out.amm_ticks[0].1.base, xcr());
    assert!(out.sac_unproven.is_empty());
    assert_eq!(pending(&assets).iter().filter(|a| **a == xcr()).count(), 1);
    assert!(!holds_contract(&assets, XCR_SAC));

    // (b) The SAC trades first with its proof; the classic is then known.
    let mut assets = with_candidates(AssetRegistry::from_existing(vec![]));
    let out = run(
        vec![sac_transfer(XCR_SAC, &xcr_sep11()), trade(XLM_SAC, XCR_SAC)],
        &mut assets,
    );
    assert_eq!(out.amm_ticks[0].1.base, xcr());
    assert!(!assets.intern(&xcr()), "already interned by the trade");
    assert_eq!(pending(&assets).iter().filter(|a| **a == xcr()).count(), 1);
    assert!(!holds_contract(&assets, XCR_SAC));
}

/// T6-live and the impostor: an `is_sac` leg with no valid proof is skipped,
/// counted, and interns nothing.
#[test]
fn an_unproven_sac_trade_is_skipped_and_interns_nothing() {
    let impostor = strkey(IMPOSTOR);
    let cases = [
        ("no proof", vec![trade(XLM_SAC, XCR_SAC)]),
        (
            "impostor proof",
            vec![
                sac_transfer(&impostor, &xcr_sep11()),
                trade(XLM_SAC, XCR_SAC),
            ],
        ),
    ];
    for (name, events) in cases {
        let mut assets = with_candidates(AssetRegistry::from_existing(vec![]));
        let out = run(events, &mut assets);
        assert!(out.amm_ticks.is_empty(), "{name}");
        assert_eq!(out.sac_unproven, vec![XCR_SAC.to_string()], "{name}");
        assert!(pending(&assets).is_empty(), "{name}: nothing interned");
        assert_eq!(assets.resolve_sac(XCR_SAC), None, "{name}");
        assert_eq!(assets.resolve_sac(&impostor), None, "{name}");
    }
}

/// D2: a token BE does not flag `is_sac` keeps its `Contract` identity.
#[test]
fn a_genuine_soroban_token_keeps_its_contract_identity() {
    let token = strkey(GENUINE_TOKEN);
    let mut assets = with_candidates(AssetRegistry::from_existing(vec![]));
    let out = run(vec![trade(XLM_SAC, &token)], &mut assets);
    assert_eq!(out.amm_ticks.len(), 1);
    assert_eq!(
        out.amm_ticks[0].1.base,
        AssetIdentity::Contract(token.clone())
    );
    assert!(out.sac_unproven.is_empty());
    assert!(holds_contract(&assets, &token));
}

#[derive(Clone, Default)]
struct MemoryFetcher {
    objects: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

impl MemoryFetcher {
    fn publish(&self, seq: u64, bytes: Vec<u8>) {
        self.objects
            .lock()
            .unwrap()
            .insert(ledger_s3_key(seq as i64), bytes);
    }
}

impl ObjectFetcher for MemoryFetcher {
    async fn fetch(&self, key: &str) -> Result<Option<Vec<u8>>, FetchError> {
        Ok(self.objects.lock().unwrap().get(key).cloned())
    }
}

#[derive(Clone, Default)]
struct NullSink;

impl CandleSink for NullSink {
    async fn write_candles(&self, _: &[OhlcvCandle], _: &str) -> Result<(), SinkError> {
        Ok(())
    }

    async fn write_oracle(&self, _: &[OracleSample]) -> Result<(), SinkError> {
        Ok(())
    }

    async fn write_new_assets(&self, _: &AssetRegistry) -> Result<(), SinkError> {
        Ok(())
    }

    async fn write_pool_rows(&self, _: &[PoolRegistryRow]) -> Result<(), SinkError> {
        Ok(())
    }
}

/// T6-live through the `Reconciler`: the count follows the WRITTEN minute, as
/// `UnregisteredPoolEvents` does, so a held-back ledger is not counted twice.
#[tokio::test]
async fn unproven_sac_trades_are_counted_once_when_their_minute_is_written() {
    let dir = tempdir().unwrap();
    let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
    cursor.write(FIRST - 1).await.unwrap();
    let fetcher = MemoryFetcher::default();
    let reconciler = Reconciler::new(
        fetcher.clone(),
        cursor,
        NullSink,
        with_candidates(AssetRegistry::from_existing(vec![])),
        pool_registries(),
    );
    let unproven = || vec![trade(XLM_SAC, XCR_SAC)];

    for (seq, close) in [(FIRST, BASE_MINUTE), (FIRST + 1, BASE_MINUTE + 5)] {
        fetcher.publish(seq, ledger_with(seq, close, unproven()));
        let held = reconciler.run(32).await.unwrap();
        assert_eq!(held.end_cursor, FIRST - 1, "the minute is still open");
        assert_eq!(held.sac_unproven_skipped, 0);
    }

    // The next minute's ledger is decoded but held back: 2, not 3.
    fetcher.publish(
        FIRST + 2,
        ledger_with(FIRST + 2, BASE_MINUTE + 60, unproven()),
    );
    let written = reconciler.run(32).await.unwrap();
    assert_eq!(written.end_cursor, FIRST + 1);
    assert_eq!(written.sac_unproven_skipped, 2);

    fetcher.publish(FIRST + 3, ledger_with(FIRST + 3, BASE_MINUTE + 120, vec![]));
    let next = reconciler.run(32).await.unwrap();
    assert_eq!(next.end_cursor, FIRST + 2);
    assert_eq!(
        next.sac_unproven_skipped, 1,
        "only the minute this run wrote"
    );
}
