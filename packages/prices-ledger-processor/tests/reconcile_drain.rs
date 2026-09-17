//! Multi-run reconcile tests: the loop-level property task 0282 depends on.
//!
//! `reconcile_e2e.rs` covers what ONE run decides at a minute boundary. What it
//! cannot cover is the composition across successive runs, because its three
//! fixture ledgers all close inside one minute. This file fabricates a longer
//! chain: it takes one real fixture ledger as a template, re-stamps its
//! `ledger_seq` and `close_time`, and serves the copies from memory. Every copy
//! carries the same trades, so the correct `trade_count` for a minute is the
//! template's own count times the number of ledgers in that minute.
//!
//! The properties pinned here, for both the steady state (one new ledger per
//! doorbell) and a restart after an outage (a backlog drained in one go):
//!
//! 1. every complete minute is written **exactly once** per `(source, pair)`;
//! 2. that one write carries the **whole** minute's trades;
//! 3. after every run the cursor sits on a **minute boundary**;
//! 4. the drain **terminates**, leaving only the still-open minute unwritten.
//!
//! Before task 0282 the steady-state test fails on (1) and (2): each doorbell
//! wrote its own slice of the minute, and ReplacingMergeTree kept only the last.
//!
//! Same fixture convention as `reconcile_e2e.rs`: the template is a gitignored
//! Galexie object, so these tests **self-skip** when it is absent.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use prices_ingest_core::{AssetRegistry, OhlcvCandle, OracleSample, Registries, decode_object};
use prices_ledger_processor::{
    cursor::{Cursor, StubFileCursor},
    galexie_key::ledger_s3_key,
    object_fetcher::{FetchError, ObjectFetcher},
    reconcile::{Reconciler, RunStats},
    sink::{CandleSink, SinkError},
};
use stellar_xdr::{
    LedgerCloseMeta, LedgerCloseMetaBatch, LedgerHeader, Limits, TimePoint, WriteXdr,
};
use tempfile::tempdir;

const TEMPLATE_LEDGER: u64 = 62_460_540;
/// First fabricated ledger. Far from the template's own sequence so no key can
/// collide with a real fixture.
const FIRST: u64 = 70_000_000;
/// A whole minute, well clear of the template's real close time.
const BASE_MINUTE: u64 = 1_800_000_000 / 60 * 60;

fn template_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/ledgers")
        .join(ledger_s3_key(TEMPLATE_LEDGER as i64))
}

macro_rules! skip_if_no_fixtures {
    () => {
        if !template_path().exists() {
            eprintln!(
                "skipping: no local fixtures under packages/prices-ledger-processor/fixtures/"
            );
            return;
        }
    };
}

fn template() -> LedgerCloseMeta {
    let bytes = std::fs::read(template_path()).expect("read template fixture");
    decode_object(&bytes)
        .expect("decode template fixture")
        .into_iter()
        .next()
        .expect("template object holds one ledger")
}

fn header_mut(lcm: &mut LedgerCloseMeta) -> &mut LedgerHeader {
    match lcm {
        LedgerCloseMeta::V0(v) => &mut v.ledger_header.header,
        LedgerCloseMeta::V1(v) => &mut v.ledger_header.header,
        LedgerCloseMeta::V2(v) => &mut v.ledger_header.header,
    }
}

/// The template re-stamped as `seq`, closing at `close_time`, encoded exactly as
/// Galexie writes it: a one-ledger `LedgerCloseMetaBatch`, zstd-compressed.
fn fabricate(template: &LedgerCloseMeta, seq: u64, close_time: u64) -> Vec<u8> {
    let mut lcm = template.clone();
    let header = header_mut(&mut lcm);
    header.ledger_seq = seq as u32;
    header.scp_value.close_time = TimePoint(close_time);
    let batch = LedgerCloseMetaBatch {
        start_sequence: seq as u32,
        end_sequence: seq as u32,
        ledger_close_metas: vec![lcm].try_into().expect("one ledger fits"),
    };
    let xdr = batch.to_xdr(Limits::none()).expect("encode batch");
    zstd::encode_all(&xdr[..], 0).expect("compress batch")
}

/// An in-memory S3: ledgers become visible only once `publish`ed, so a test can
/// replay the feed one doorbell at a time.
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

/// One candle write as the sink received it.
#[derive(Clone, Debug)]
struct Write {
    source: String,
    minute: u32,
    pair: (u32, u32),
    trade_count: u32,
}

#[derive(Clone, Default)]
struct RecordingSink {
    writes: Arc<Mutex<Vec<Write>>>,
}

impl RecordingSink {
    fn writes(&self) -> Vec<Write> {
        self.writes.lock().unwrap().clone()
    }
}

impl CandleSink for RecordingSink {
    async fn write_candles(&self, candles: &[OhlcvCandle], source: &str) -> Result<(), SinkError> {
        self.writes
            .lock()
            .unwrap()
            .extend(candles.iter().map(|c| Write {
                source: source.to_string(),
                minute: c.minute_start,
                pair: (c.asset_id, c.quote_asset_id),
                trade_count: c.trade_count,
            }));
        Ok(())
    }

    async fn write_oracle(&self, _samples: &[OracleSample]) -> Result<(), SinkError> {
        Ok(())
    }

    async fn write_new_assets(
        &self,
        _registry: &AssetRegistry,
        _since: u32,
    ) -> Result<(), SinkError> {
        Ok(())
    }
}

type Harness = Reconciler<MemoryFetcher, StubFileCursor, RecordingSink>;

async fn harness(dir: &std::path::Path) -> (Harness, MemoryFetcher, RecordingSink) {
    let cursor = StubFileCursor::new(dir.join("cursor.txt"));
    cursor.write(FIRST - 1).await.unwrap();
    let fetcher = MemoryFetcher::default();
    let sink = RecordingSink::default();
    let reconciler = Reconciler::new(
        fetcher.clone(),
        cursor,
        sink.clone(),
        AssetRegistry::from_existing(Vec::new()),
        Registries::new(),
    );
    (reconciler, fetcher, sink)
}

/// A fabricated chain: `(seq, close_time)` for `n` ledgers, `close_ms` apart.
fn chain(n: u64, close_ms: u64) -> Vec<(u64, u64)> {
    (0..n)
        .map(|i| (FIRST + i, BASE_MINUTE + i * close_ms / 1000))
        .collect()
}

fn minute_of(close_time: u64) -> u32 {
    (close_time / 60 * 60) as u32
}

/// Trades per source in ONE template ledger, measured through the real pipeline:
/// a lone ledger in a terminal run is written once, whole.
async fn per_ledger_totals(template: &LedgerCloseMeta) -> BTreeMap<String, u64> {
    let dir = tempdir().unwrap();
    let (reconciler, fetcher, sink) = harness(dir.path()).await;
    fetcher.publish(FIRST, fabricate(template, FIRST, BASE_MINUTE));
    reconciler.run_terminal(32).await.unwrap();
    let mut totals = BTreeMap::new();
    for w in sink.writes() {
        *totals.entry(w.source).or_insert(0) += w.trade_count as u64;
    }
    assert!(
        totals.values().sum::<u64>() > 0,
        "the template ledger must carry trades, or every assertion below is 0 == 0"
    );
    totals
}

/// The cursor after a run must be the last ledger of its minute: the next
/// ledger in the chain, if any, closes in a later minute.
fn assert_on_minute_boundary(stats: &RunStats, closes: &BTreeMap<u64, u64>) {
    let end = stats.end_cursor;
    if end == FIRST - 1 {
        return; // never advanced: nothing to check
    }
    let this = minute_of(closes[&end]);
    if let Some(next) = closes.get(&(end + 1)) {
        assert_ne!(
            this,
            minute_of(*next),
            "cursor {end} parked INSIDE minute {this}: the next run would split it"
        );
    }
}

/// Properties (1), (2) and (4) over everything the sink received.
fn assert_each_minute_written_once_and_whole(
    sink: &RecordingSink,
    chain: &[(u64, u64)],
    per_ledger: &BTreeMap<String, u64>,
) {
    let mut ledgers_in_minute: BTreeMap<u32, u64> = BTreeMap::new();
    for (_, close) in chain {
        *ledgers_in_minute.entry(minute_of(*close)).or_insert(0) += 1;
    }
    let open_minute = *ledgers_in_minute.keys().last().unwrap();

    let writes = sink.writes();
    let mut per_key: HashMap<(String, u32, (u32, u32)), u32> = HashMap::new();
    let mut per_minute: BTreeMap<(String, u32), u64> = BTreeMap::new();
    for w in &writes {
        *per_key
            .entry((w.source.clone(), w.minute, w.pair))
            .or_insert(0) += 1;
        *per_minute.entry((w.source.clone(), w.minute)).or_insert(0) += w.trade_count as u64;
    }

    for ((source, minute, pair), n) in &per_key {
        assert_eq!(
            *n, 1,
            "{source} minute {minute} pair {pair:?} written {n} times — \
             ReplacingMergeTree would keep only the last slice"
        );
    }

    assert!(
        writes.iter().all(|w| w.minute != open_minute),
        "the newest minute may still be filling and must not be written"
    );

    for (minute, n_ledgers) in &ledgers_in_minute {
        if *minute == open_minute {
            continue;
        }
        for (source, per) in per_ledger {
            let got = per_minute
                .get(&(source.clone(), *minute))
                .copied()
                .unwrap_or(0);
            assert_eq!(
                got,
                per * n_ledgers,
                "{source} minute {minute}: {n_ledgers} ledgers x {per} trades each"
            );
        }
    }
}

/// The production shape, and the one that lost ~50% of Aquarius: each doorbell
/// finds exactly one new ledger. A minute spans ~11 of them, so before task 0282
/// it was written ~11 times and only the last write survived.
#[tokio::test]
async fn steady_state_one_ledger_per_doorbell_writes_each_minute_once_and_whole() {
    skip_if_no_fixtures!();
    let template = template();
    let per_ledger = per_ledger_totals(&template).await;

    // 5.5 s closes: 10-11 ledgers per minute, as measured on mainnet.
    let chain = chain(66, 5_500);
    let closes: BTreeMap<u64, u64> = chain.iter().copied().collect();

    let dir = tempdir().unwrap();
    let (reconciler, fetcher, sink) = harness(dir.path()).await;
    for (seq, close) in &chain {
        fetcher.publish(*seq, fabricate(&template, *seq, *close));
        let stats = reconciler.run(32).await.unwrap();
        assert!(!stats.forced_partial_flush, "a healthy feed never forces");
        assert_on_minute_boundary(&stats, &closes);
    }

    assert_each_minute_written_once_and_whole(&sink, &chain, &per_ledger);
}

/// Restart after an outage: the whole backlog is already on S3 and the cursor is
/// far behind. Successive runs must drain it whole-minute by whole-minute and
/// stop at the tip, holding back only the open minute.
#[tokio::test]
async fn a_backlog_drains_across_runs_without_splitting_a_minute() {
    skip_if_no_fixtures!();
    let template = template();
    let per_ledger = per_ledger_totals(&template).await;

    // Both budgets: the production one, and the old 16 that leaves ~5 ledgers
    // of headroom past a whole minute.
    for budget in [32, 16] {
        let chain = chain(130, 5_500);
        let closes: BTreeMap<u64, u64> = chain.iter().copied().collect();

        let dir = tempdir().unwrap();
        let (reconciler, fetcher, sink) = harness(dir.path()).await;
        for (seq, close) in &chain {
            fetcher.publish(*seq, fabricate(&template, *seq, *close));
        }

        let mut runs = 0;
        loop {
            runs += 1;
            assert!(
                runs <= chain.len(),
                "budget {budget}: the drain never ended"
            );
            let stats = reconciler.run(budget).await.unwrap();
            assert!(
                !stats.forced_partial_flush,
                "budget {budget}: nothing to force"
            );
            assert_on_minute_boundary(&stats, &closes);
            if stats.end_cursor == stats.start_cursor {
                break;
            }
        }

        // The drain stopped at the tip: the cursor is the last ledger before the
        // open minute, and everything above it is that minute.
        let open_minute = minute_of(chain.last().unwrap().1);
        let end = StubFileCursor::new(dir.path().join("cursor.txt"))
            .read()
            .await
            .unwrap();
        assert_ne!(minute_of(closes[&end]), open_minute, "budget {budget}");
        assert!(
            closes
                .range(end + 1..)
                .all(|(_, c)| minute_of(*c) == open_minute),
            "budget {budget}: a complete minute was left unwritten above the cursor"
        );

        assert_each_minute_written_once_and_whole(&sink, &chain, &per_ledger);
    }
}

/// The escape hatch, across runs: a minute denser than the budget cannot be
/// seen whole, so the loop must still make progress (and flag it) rather than
/// re-read the same ledgers forever.
#[tokio::test]
async fn a_minute_denser_than_the_budget_still_drains_and_is_flagged() {
    skip_if_no_fixtures!();
    let template = template();

    // 1 s closes: 60 ledgers per minute against a budget of 16.
    let chain = chain(150, 1_000);
    let dir = tempdir().unwrap();
    let (reconciler, fetcher, _sink) = harness(dir.path()).await;
    for (seq, close) in &chain {
        fetcher.publish(*seq, fabricate(&template, *seq, *close));
    }

    let mut forced = 0;
    let mut runs = 0;
    loop {
        runs += 1;
        assert!(runs <= chain.len(), "the dense minute deadlocked the drain");
        let stats = reconciler.run(16).await.unwrap();
        forced += stats.forced_partial_flush as usize;
        if stats.end_cursor == stats.start_cursor {
            break;
        }
    }

    assert!(
        forced > 0,
        "a budget below the ledgers-per-minute rate must be reported, \
         because it re-creates the task-0282 loss for those minutes"
    );
    let end = StubFileCursor::new(dir.path().join("cursor.txt"))
        .read()
        .await
        .unwrap();
    assert!(
        end > FIRST + 100,
        "the drain must reach the tip region, not stall early (cursor {end})"
    );
}
