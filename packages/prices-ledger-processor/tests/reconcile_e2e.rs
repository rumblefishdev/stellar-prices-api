use std::sync::Mutex;

use extractors_core::{SorobanEventRow, VenueRegistry};
use phoenix_extractor::PhoenixPoolRegistry;
use prices_ledger_processor::{
    bucket::OhlcvRow,
    cursor::{Cursor, StubFileCursor},
    galexie_key::ledger_s3_key,
    object_fetcher::LocalDiskFetcher,
    reconcile::{DecodedLedger, LedgerDecoder, Reconciler},
    sink::{OhlcvSink, SinkError},
};
use soroswap_extractor::SoroswapPoolRegistry;
use tempfile::tempdir;

struct CaptureSink {
    rows: Mutex<Vec<OhlcvRow>>,
}

impl OhlcvSink for CaptureSink {
    async fn write(&self, rows: &[OhlcvRow]) -> Result<(), SinkError> {
        self.rows.lock().unwrap().extend_from_slice(rows);
        Ok(())
    }
}

/// Returns one empty `DecodedLedger` for each fetched object, with the
/// ledger sequence parsed back out of the `--{seq}.xdr.zst` suffix in
/// the bytes (so the test can wire decode to fixture content trivially).
/// No event groups → no trades → no rows. Tests cursor + fetcher + loop.
struct EmptyDecoder;

impl LedgerDecoder for EmptyDecoder {
    async fn decode(&self, bytes: &[u8]) -> Result<Vec<DecodedLedger>, String> {
        let seq: u64 = std::str::from_utf8(bytes)
            .map_err(|e| e.to_string())?
            .trim()
            .parse()
            .map_err(|e: std::num::ParseIntError| e.to_string())?;
        Ok(vec![DecodedLedger {
            ledger_sequence: seq,
            closed_at_unix_seconds: 1_700_000_000,
            event_groups: Vec::new(),
        }])
    }
}

/// Returns one `DecodedLedger` with one event group whose first contract
/// is not in any registry → dispatch returns `Ok(vec![])`. Still no
/// trades, but proves the dispatch path executes.
struct SingleEmptyGroupDecoder;

impl LedgerDecoder for SingleEmptyGroupDecoder {
    async fn decode(&self, bytes: &[u8]) -> Result<Vec<DecodedLedger>, String> {
        let seq: u64 = std::str::from_utf8(bytes).unwrap().trim().parse().unwrap();
        Ok(vec![DecodedLedger {
            ledger_sequence: seq,
            closed_at_unix_seconds: 1_700_000_000,
            event_groups: vec![vec![SorobanEventRow {
                contract_id: "C-unknown".into(),
                transaction_id: "T".into(),
                ledger_sequence: seq,
                event_index: 0,
                topics: Vec::new(),
                data: extractors_core::TaggedValue::Null,
            }]],
        }])
    }
}

#[tokio::test]
async fn empty_fixture_dir_no_op_returns_zero_persisted() {
    let dir = tempdir().unwrap();
    let cursor_path = dir.path().join("cursor.txt");
    let cursor = StubFileCursor::new(&cursor_path);
    cursor.write(99).await.unwrap();

    let reconciler = Reconciler {
        fetcher: LocalDiskFetcher::new(dir.path().join("nope")),
        cursor,
        sink: CaptureSink {
            rows: Mutex::new(Vec::new()),
        },
        decoder: EmptyDecoder,
        venue_registry: VenueRegistry::new(),
        phoenix_registry: PhoenixPoolRegistry::default(),
        soroswap_registry: SoroswapPoolRegistry::new(),
    };

    let stats = reconciler.run(8).await.unwrap();
    assert_eq!(stats.start_cursor, 99);
    assert_eq!(stats.end_cursor, 99);
    assert_eq!(stats.ledgers_persisted, 0);
    assert_eq!(stats.rows_emitted, 0);
}

#[tokio::test]
async fn contiguous_run_advances_cursor_until_gap() {
    let dir = tempdir().unwrap();
    let fixtures = dir.path().join("ledgers");
    // Seed three contiguous "ledgers" 100, 101, 102, then a gap at 103.
    for seq in [100u64, 101, 102] {
        let key = ledger_s3_key(seq as i64);
        let path = fixtures.join(&key);
        tokio::fs::create_dir_all(path.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&path, format!("{seq}")).await.unwrap();
    }

    let cursor_path = dir.path().join("cursor.txt");
    let cursor = StubFileCursor::new(&cursor_path);
    cursor.write(99).await.unwrap();

    let reconciler = Reconciler {
        fetcher: LocalDiskFetcher::new(&fixtures),
        cursor,
        sink: CaptureSink {
            rows: Mutex::new(Vec::new()),
        },
        decoder: EmptyDecoder,
        venue_registry: VenueRegistry::new(),
        phoenix_registry: PhoenixPoolRegistry::default(),
        soroswap_registry: SoroswapPoolRegistry::new(),
    };

    let stats = reconciler.run(8).await.unwrap();
    assert_eq!(stats.start_cursor, 99);
    assert_eq!(stats.end_cursor, 102);
    assert_eq!(stats.ledgers_persisted, 3);
    assert_eq!(stats.rows_emitted, 0);

    // Cursor file ends up at 102 — next invocation resumes here.
    let cursor = StubFileCursor::new(&cursor_path);
    assert_eq!(cursor.read().await.unwrap(), 102);
}

#[tokio::test]
async fn unknown_contract_dispatch_does_not_fail() {
    let dir = tempdir().unwrap();
    let fixtures = dir.path().join("ledgers");
    let key = ledger_s3_key(200);
    let path = fixtures.join(&key);
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&path, "200").await.unwrap();

    let cursor_path = dir.path().join("cursor.txt");
    let cursor = StubFileCursor::new(&cursor_path);
    cursor.write(199).await.unwrap();

    let reconciler = Reconciler {
        fetcher: LocalDiskFetcher::new(&fixtures),
        cursor,
        sink: CaptureSink {
            rows: Mutex::new(Vec::new()),
        },
        decoder: SingleEmptyGroupDecoder,
        venue_registry: VenueRegistry::new(),
        phoenix_registry: PhoenixPoolRegistry::default(),
        soroswap_registry: SoroswapPoolRegistry::new(),
    };

    let stats = reconciler.run(2).await.unwrap();
    assert_eq!(stats.ledgers_persisted, 1);
    assert_eq!(stats.end_cursor, 200);
}

#[tokio::test]
async fn idempotent_on_re_run_from_same_cursor() {
    let dir = tempdir().unwrap();
    let fixtures = dir.path().join("ledgers");
    let key = ledger_s3_key(50);
    let path = fixtures.join(&key);
    tokio::fs::create_dir_all(path.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&path, "50").await.unwrap();

    let cursor_path = dir.path().join("cursor.txt");

    let run = || async {
        let cursor = StubFileCursor::new(&cursor_path);
        cursor.write(49).await.unwrap();
        let sink = CaptureSink {
            rows: Mutex::new(Vec::new()),
        };
        let reconciler = Reconciler {
            fetcher: LocalDiskFetcher::new(&fixtures),
            cursor,
            sink,
            decoder: EmptyDecoder,
            venue_registry: VenueRegistry::new(),
            phoenix_registry: PhoenixPoolRegistry::default(),
            soroswap_registry: SoroswapPoolRegistry::new(),
        };
        reconciler.run(8).await.unwrap()
    };

    let first = run().await;
    let second = run().await;
    assert_eq!(first.start_cursor, second.start_cursor);
    assert_eq!(first.end_cursor, second.end_cursor);
    assert_eq!(first.ledgers_persisted, second.ledgers_persisted);
    assert_eq!(first.rows_emitted, second.rows_emitted);
}
