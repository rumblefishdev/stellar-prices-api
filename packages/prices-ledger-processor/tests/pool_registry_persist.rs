//! Task 0291: the live processor persists the AMM pools it learns from factory
//! events, so a cold start does not forget them.
//!
//! Before this, `prices.pool_registry` was written only by the history backfill.
//! A pool created later was known only to a warm container that had seen its
//! factory event; after a cold start every trade in it was silently dropped. On
//! production that was 27 Aquarius, 14 Soroswap and 1 Phoenix pool created after
//! 2026-07-06. The last test covers the counter that makes such a drop visible.
//!
//! The ledgers here are synthetic — built from `Default` with the events under
//! test injected — so, unlike the fixture-gated reconcile tests, these run
//! everywhere.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use prices_ingest_core::{
    AssetRegistry, OhlcvCandle, OracleSample, PoolRegistryRow, Registries, process_ledger,
};
use prices_ledger_processor::{
    cursor::{Cursor, StubFileCursor},
    galexie_key::ledger_s3_key,
    object_fetcher::{FetchError, ObjectFetcher},
    reconcile::Reconciler,
    sink::{CandleSink, SinkError},
};
use stellar_xdr::{
    ContractEvent, ContractEventBody, ContractEventType, ContractEventV0, ContractId, Hash,
    Int128Parts, LedgerCloseMeta, LedgerCloseMetaBatch, LedgerCloseMetaV2, Limits, OperationMetaV2,
    ScAddress, ScMap, ScMapEntry, ScString, ScSymbol, ScVal, TimePoint, TransactionMeta,
    TransactionMetaV4, TransactionResultMetaV1, WriteXdr,
};
use tempfile::tempdir;

const FIRST: u64 = 70_000_000;
const BASE_MINUTE: u64 = 1_800_000_000 / 60 * 60;

const PAIR: [u8; 32] = [0x11; 32];
const TOKEN_0: [u8; 32] = [0x22; 32];
const TOKEN_1: [u8; 32] = [0x33; 32];

fn strkey(bytes: [u8; 32]) -> String {
    ContractId(Hash(bytes)).to_string()
}

fn contract(bytes: [u8; 32]) -> ScVal {
    ScVal::Address(ScAddress::Contract(ContractId(Hash(bytes))))
}

fn sym(s: &str) -> ScVal {
    ScVal::Symbol(ScSymbol(s.try_into().unwrap()))
}

/// The Soroswap factory's pair-creation event, in its real shape: the action
/// symbol sits in topic[1], behind a name String in topic[0].
fn new_pair_event() -> ContractEvent {
    let entry = |k: &str, v: ScVal| ScMapEntry {
        key: sym(k),
        val: v,
    };
    let data = ScVal::Map(Some(ScMap(
        vec![
            entry("new_pairs_length", ScVal::U32(1)),
            entry("pair", contract(PAIR)),
            entry("token_0", contract(TOKEN_0)),
            entry("token_1", contract(TOKEN_1)),
        ]
        .try_into()
        .unwrap(),
    )));
    ContractEvent {
        ext: Default::default(),
        contract_id: Some(ContractId(Hash([0x44; 32]))),
        type_: ContractEventType::Contract,
        body: ContractEventBody::V0(ContractEventV0 {
            topics: vec![
                ScVal::String(ScString("SoroswapFactory".try_into().unwrap())),
                sym("new_pair"),
            ]
            .try_into()
            .unwrap(),
            data,
        }),
    }
}

/// An Aquarius pool `trade`, as a concentrated or constant-product pool emits
/// it, from a contract no registry holds.
fn unregistered_aquarius_trade() -> ContractEvent {
    ContractEvent {
        ext: Default::default(),
        contract_id: Some(ContractId(Hash([0x55; 32]))),
        type_: ContractEventType::Contract,
        body: ContractEventBody::V0(ContractEventV0 {
            topics: vec![
                sym("trade"),
                contract(TOKEN_0),
                contract(TOKEN_1),
                contract([0x66; 32]),
            ]
            .try_into()
            .unwrap(),
            data: ScVal::Vec(Some(
                vec![
                    ScVal::I128(Int128Parts {
                        hi: 0,
                        lo: 1_000_000,
                    }),
                    ScVal::I128(Int128Parts {
                        hi: 0,
                        lo: 2_000_000,
                    }),
                    ScVal::I128(Int128Parts { hi: 0, lo: 3_000 }),
                ]
                .try_into()
                .unwrap(),
            )),
        }),
    }
}

/// One ledger closing at `close_time`, optionally carrying the factory event in
/// its single transaction. Encoded as Galexie writes it.
fn ledger(seq: u64, close_time: u64, with_factory_event: bool) -> Vec<u8> {
    let events = if with_factory_event {
        vec![new_pair_event()]
    } else {
        Vec::new()
    };
    ledger_with(seq, close_time, events)
}

fn ledger_with(seq: u64, close_time: u64, events: Vec<ContractEvent>) -> Vec<u8> {
    let batch = LedgerCloseMetaBatch {
        start_sequence: seq as u32,
        end_sequence: seq as u32,
        ledger_close_metas: vec![lcm_with(seq, close_time, events)].try_into().unwrap(),
    };
    let xdr = batch.to_xdr(Limits::none()).unwrap();
    zstd::encode_all(&xdr[..], 0).unwrap()
}

/// The same ledger, undecorated — for driving `process_ledger` directly.
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

/// Records every pool row written; can fail the next pool write once.
#[derive(Clone, Default)]
struct PoolSink {
    pools: Arc<Mutex<Vec<PoolRegistryRow>>>,
    fail_next_pool_write: Arc<AtomicBool>,
}

impl PoolSink {
    fn pools(&self) -> Vec<PoolRegistryRow> {
        self.pools.lock().unwrap().clone()
    }
}

impl CandleSink for PoolSink {
    async fn write_candles(&self, _: &[OhlcvCandle], _: &str) -> Result<(), SinkError> {
        Ok(())
    }

    async fn write_oracle(&self, _: &[OracleSample]) -> Result<(), SinkError> {
        Ok(())
    }

    async fn write_new_assets(&self, _: &AssetRegistry, _: u32) -> Result<(), SinkError> {
        Ok(())
    }

    async fn write_pool_rows(&self, rows: &[PoolRegistryRow]) -> Result<(), SinkError> {
        if self.fail_next_pool_write.swap(false, Ordering::Relaxed) {
            return Err(SinkError::Write("injected pool-write failure".to_string()));
        }
        self.pools.lock().unwrap().extend_from_slice(rows);
        Ok(())
    }
}

type Harness = Reconciler<MemoryFetcher, StubFileCursor, PoolSink>;

async fn harness(
    dir: &std::path::Path,
    cursor_at: u64,
    fetcher: &MemoryFetcher,
    registries: Registries,
) -> (Harness, PoolSink) {
    let cursor = StubFileCursor::new(dir.join("cursor.txt"));
    cursor.write(cursor_at).await.unwrap();
    let sink = PoolSink::default();
    let reconciler = Reconciler::new(
        fetcher.clone(),
        cursor,
        sink.clone(),
        AssetRegistry::from_existing(Vec::new()),
        registries,
    );
    (reconciler, sink)
}

/// The cold-start preload (task 0078) and factory learning (this task) both
/// rest on one invariant: learning a pool ADDS to a preloaded registry, it
/// never rebuilds it. Task 0256 deleted asset-discovery's
/// `register_ledger_assets_preserves_preseeded_pools` along with the ledger
/// scan; that test passed no ledgers, so it never reached `process_ledger`.
/// This one does.
#[test]
fn a_preloaded_pool_survives_a_ledger_that_teaches_another() {
    let preloaded = PoolRegistryRow {
        contract_id: strkey([0x77; 32]),
        venue: "soroswap".to_string(),
        token0: strkey(TOKEN_0),
        token1: strkey(TOKEN_1),
        pool_type: 0,
        wasm_hash: String::new(),
    };
    let mut reg = Registries::new();
    reg.load_pool_rows(std::slice::from_ref(&preloaded));
    let mut assets = AssetRegistry::from_existing(Vec::new());

    // A ledger with events but no factory event, then one that teaches a pool:
    // neither kind of ledger may cost the registry what it was loaded with.
    let quiet = lcm_with(FIRST, BASE_MINUTE, vec![unregistered_aquarius_trade()]);
    let teaching = lcm_with(FIRST + 1, BASE_MINUTE + 60, vec![new_pair_event()]);
    for lcm in [&quiet, &teaching] {
        let _ = process_ledger(lcm, &mut reg, &mut assets);
    }

    let rows = reg.to_pool_rows();
    assert_eq!(rows.len(), 2, "the factory event adds a pool: {rows:?}");
    assert!(rows.contains(&preloaded), "the preloaded pool must stay");
    assert!(rows.iter().any(|r| r.contract_id == strkey(PAIR)));
}

#[tokio::test]
async fn a_learned_pool_is_written_once_and_survives_a_cold_start() {
    let dir = tempdir().unwrap();
    let fetcher = MemoryFetcher::default();
    fetcher.publish(FIRST, ledger(FIRST, BASE_MINUTE, true));
    fetcher.publish(FIRST + 1, ledger(FIRST + 1, BASE_MINUTE + 60, false));

    // Run 1 learns the pool and moves the cursor past its factory event.
    let (warm, sink) = harness(dir.path(), FIRST - 1, &fetcher, Registries::new()).await;
    let stats = warm.run(32).await.unwrap();
    assert_eq!(
        stats.end_cursor, FIRST,
        "the factory ledger's minute is complete"
    );
    assert_eq!(stats.pools_persisted, 1);
    assert_eq!(
        sink.pools(),
        vec![PoolRegistryRow {
            contract_id: strkey(PAIR),
            venue: "soroswap".to_string(),
            token0: strkey(TOKEN_0),
            token1: strkey(TOKEN_1),
            pool_type: 0,
            wasm_hash: String::new(),
        }]
    );

    // Run 2, same warm container: the pool is already durable — nothing more.
    fetcher.publish(FIRST + 2, ledger(FIRST + 2, BASE_MINUTE + 120, false));
    let stats = warm.run(32).await.unwrap();
    assert_eq!(stats.end_cursor, FIRST + 1);
    assert_eq!(stats.pools_persisted, 0, "steady state writes no pool rows");
    assert_eq!(sink.pools().len(), 1);

    // Cold start: the new container rebuilds its registry from what was written,
    // exactly as `load_pool_registry` does. The factory ledger is behind the
    // cursor and will never be read again, so this is the only way it knows.
    let mut reloaded = Registries::new();
    reloaded.load_pool_rows(&sink.pools());
    assert!(reloaded.venue.contains_key(&strkey(PAIR)));
    assert!(reloaded.soroswap.contains(&strkey(PAIR)));

    let cold_dir = tempdir().unwrap();
    fetcher.publish(FIRST + 3, ledger(FIRST + 3, BASE_MINUTE + 180, false));
    let (cold, cold_sink) = harness(cold_dir.path(), FIRST + 1, &fetcher, reloaded).await;
    let stats = cold.run(32).await.unwrap();
    assert_eq!(stats.end_cursor, FIRST + 2);
    assert_eq!(
        stats.pools_persisted, 0,
        "a pool loaded at cold start is already durable and is not rewritten"
    );
    assert!(cold_sink.pools().is_empty());
}

#[tokio::test]
async fn a_failed_pool_write_holds_the_cursor_and_is_retried() {
    let dir = tempdir().unwrap();
    let fetcher = MemoryFetcher::default();
    fetcher.publish(FIRST, ledger(FIRST, BASE_MINUTE, true));
    fetcher.publish(FIRST + 1, ledger(FIRST + 1, BASE_MINUTE + 60, false));

    let (reconciler, sink) = harness(dir.path(), FIRST - 1, &fetcher, Registries::new()).await;
    sink.fail_next_pool_write.store(true, Ordering::Relaxed);

    assert!(reconciler.run(32).await.is_err());
    let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
    assert_eq!(
        cursor.read().await.unwrap(),
        FIRST - 1,
        "the cursor must not pass a factory event whose pool is not durable"
    );
    assert!(sink.pools().is_empty());

    // The warm registry still holds the pool, and the snapshot did not move, so
    // the retry writes it — even though re-reading the ledger is not needed.
    let stats = reconciler.run(32).await.unwrap();
    assert_eq!(stats.end_cursor, FIRST);
    assert_eq!(stats.pools_persisted, 1);
    assert_eq!(sink.pools().len(), 1);
    assert_eq!(sink.pools()[0].contract_id, strkey(PAIR));
}

#[tokio::test]
async fn a_pool_learned_in_a_held_back_minute_is_still_written() {
    // The only ledger sits in the still-open minute, so the run holds back and
    // writes no candles. The pool is a dimension row and is written anyway.
    let dir = tempdir().unwrap();
    let fetcher = MemoryFetcher::default();
    fetcher.publish(FIRST, ledger(FIRST, BASE_MINUTE, true));

    let (reconciler, sink) = harness(dir.path(), FIRST - 1, &fetcher, Registries::new()).await;
    let stats = reconciler.run(32).await.unwrap();
    assert_eq!(stats.end_cursor, FIRST - 1, "held back");
    assert_eq!(stats.pools_persisted, 1);
    assert_eq!(sink.pools().len(), 1);

    // Re-reading the held-back ledger re-learns the same pool: no second write.
    fetcher.publish(FIRST + 1, ledger(FIRST + 1, BASE_MINUTE + 60, false));
    let stats = reconciler.run(32).await.unwrap();
    assert_eq!(stats.end_cursor, FIRST);
    assert_eq!(stats.pools_persisted, 0);
    assert_eq!(sink.pools().len(), 1);
}

#[tokio::test]
async fn unregistered_pool_trades_are_counted_once_when_their_minute_is_written() {
    // Two ledgers in one minute each carry a trade from an unregistered
    // Aquarius-shaped pool. Held-back runs re-read them, so the count must
    // follow the WRITTEN minute: 2, reported once.
    let dir = tempdir().unwrap();
    let fetcher = MemoryFetcher::default();
    let (reconciler, _sink) = harness(dir.path(), FIRST - 1, &fetcher, Registries::new()).await;

    for (seq, close) in [(FIRST, BASE_MINUTE), (FIRST + 1, BASE_MINUTE + 5)] {
        fetcher.publish(
            seq,
            ledger_with(seq, close, vec![unregistered_aquarius_trade()]),
        );
        let held = reconciler.run(32).await.unwrap();
        assert_eq!(held.end_cursor, FIRST - 1, "the minute is still open");
        assert!(held.unregistered_pool_events.is_empty());
    }

    // The next minute's first ledger carries a trade too. This run decodes it
    // but holds it back, so it must not be counted yet: 2, not 3.
    fetcher.publish(
        FIRST + 2,
        ledger_with(
            FIRST + 2,
            BASE_MINUTE + 60,
            vec![unregistered_aquarius_trade()],
        ),
    );
    let written = reconciler.run(32).await.unwrap();
    assert_eq!(written.end_cursor, FIRST + 1);
    assert_eq!(
        written
            .unregistered_pool_events
            .into_iter()
            .collect::<Vec<_>>(),
        vec![("aquarius", 2)]
    );

    fetcher.publish(FIRST + 3, ledger(FIRST + 3, BASE_MINUTE + 120, false));
    let next = reconciler.run(32).await.unwrap();
    assert_eq!(next.end_cursor, FIRST + 2);
    assert_eq!(
        next.unregistered_pool_events
            .into_iter()
            .collect::<Vec<_>>(),
        vec![("aquarius", 1)],
        "only the minute this run wrote — the first two are not re-reported"
    );
}
