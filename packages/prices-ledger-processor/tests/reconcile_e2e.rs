//! End-to-end reconcile test against the bundled real Galexie fixtures.
//!
//! Drives the production pipeline (`prices_ingest_core` decode → extract →
//! bucket) over the three contiguous fixture ledgers 62460540–62460542 using a
//! local-disk fetcher and an in-memory counting sink (no ClickHouse). Proves the
//! doorbell-cursor loop decodes real XDR, advances the cursor to the last
//! contiguous ledger, stops at the gap, and is idempotent on re-run.
//!
//! Fixtures are gitignored (large binary Galexie objects, copied locally), so
//! each test **self-skips** when they are absent — matching the repo's
//! self-skipping integration-test convention (`prices-clickhouse` mtls smoke).

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use prices_ingest_core::{AssetRegistry, OhlcvCandle, OracleSample, Registries};
use prices_ledger_processor::{
    cursor::{Cursor, StubFileCursor},
    object_fetcher::LocalDiskFetcher,
    reconcile::Reconciler,
    sink::{CandleSink, CountingSink, SinkError},
};
use tempfile::tempdir;

const FIRST_FIXTURE: u64 = 62_460_540;
const LAST_FIXTURE: u64 = 62_460_542;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/ledgers")
}

/// The first fixture file must be present, else the test self-skips.
fn fixtures_present() -> bool {
    let key = format!("FC47D9FF--62400000-62463999/FC46ED83--{FIRST_FIXTURE}.xdr.zst");
    fixtures_dir().join(key).exists()
}

macro_rules! skip_if_no_fixtures {
    () => {
        if !fixtures_present() {
            eprintln!(
                "skipping: no local fixtures under packages/prices-ledger-processor/fixtures/"
            );
            return;
        }
    };
}

fn reconciler(
    fixtures: PathBuf,
    cursor: StubFileCursor,
) -> Reconciler<LocalDiskFetcher, StubFileCursor, CountingSink> {
    Reconciler::new(
        LocalDiskFetcher::new(fixtures),
        cursor,
        CountingSink::default(),
        AssetRegistry::from_existing(Vec::new()),
        Registries::new(),
    )
}

/// Fault-injecting sink: fails the first `write_new_assets` call, succeeds after.
/// Candles/oracle always succeed. Shared counters via `Arc` so a clone handed to
/// the reconciler and a clone kept by the test observe the same state.
#[derive(Clone, Default)]
struct FailFirstAssetSink {
    fail_next_asset_write: Arc<AtomicBool>,
    assets_written: Arc<AtomicU64>,
    candles_written: Arc<AtomicU64>,
}

impl CandleSink for FailFirstAssetSink {
    async fn write_candles(&self, candles: &[OhlcvCandle], _source: &str) -> Result<(), SinkError> {
        self.candles_written
            .fetch_add(candles.len() as u64, Ordering::Relaxed);
        Ok(())
    }

    async fn write_oracle(&self, _samples: &[OracleSample]) -> Result<(), SinkError> {
        Ok(())
    }

    async fn write_new_assets(
        &self,
        registry: &AssetRegistry,
        since: u32,
    ) -> Result<(), SinkError> {
        if self.fail_next_asset_write.swap(false, Ordering::Relaxed) {
            return Err(SinkError::Write("injected asset-write failure".to_string()));
        }
        let n = registry.assets_since(since).count() as u64;
        self.assets_written.fetch_add(n, Ordering::Relaxed);
        Ok(())
    }
}

/// Regression for task 0132 / code-review finding 1: the registry is warm across
/// invocations, so a run that interns new assets and then fails a later write
/// must NOT strand those assets below the next run's watermark. The durable
/// persisted watermark only advances after a successful asset write, so the
/// retry re-writes them instead of orphaning the candles that reference them.
#[tokio::test]
async fn assets_from_a_failed_run_are_written_on_the_next_run() {
    skip_if_no_fixtures!();
    let dir = tempdir().unwrap();
    let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
    cursor.write(FIRST_FIXTURE - 1).await.unwrap();

    let sink = FailFirstAssetSink::default();
    sink.fail_next_asset_write.store(true, Ordering::Relaxed);

    // Empty starting registry → the fixtures intern brand-new assets this run.
    let reconciler = Reconciler::new(
        LocalDiskFetcher::new(fixtures_dir()),
        cursor,
        sink.clone(),
        AssetRegistry::from_existing(Vec::new()),
        Registries::new(),
    );

    // Run 1: interns new assets, then the (first) asset write fails → the run
    // errors and the cursor is never advanced (the doorbell would redeliver).
    let first = reconciler.run(16).await;
    assert!(
        first.is_err(),
        "injected asset-write failure should fail the run"
    );
    assert_eq!(
        sink.assets_written.load(Ordering::Relaxed),
        0,
        "nothing persisted when the asset write fails"
    );

    // Run 2 on the SAME warm reconciler: the registry still holds the interned
    // assets (next_id advanced), but the durable watermark did NOT advance — so
    // those assets are re-offered and written, not skipped.
    let second = reconciler.run(16).await.expect("retry run should succeed");
    assert!(
        second.ledgers_persisted > 0,
        "run 2 reprocesses the fixtures"
    );
    assert!(
        sink.assets_written.load(Ordering::Relaxed) > 0,
        "assets interned by the failed run must be written on retry, not orphaned"
    );
}

#[tokio::test]
async fn contiguous_run_decodes_real_fixtures_and_advances_cursor() {
    skip_if_no_fixtures!();
    let dir = tempdir().unwrap();
    let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
    cursor.write(FIRST_FIXTURE - 1).await.unwrap();

    let stats = reconciler(fixtures_dir(), cursor)
        .run(16)
        .await
        .expect("real-fixture reconcile run should succeed");

    assert_eq!(stats.start_cursor, FIRST_FIXTURE - 1);
    assert_eq!(
        stats.end_cursor, LAST_FIXTURE,
        "cursor should advance to the last contiguous fixture ledger"
    );
    assert_eq!(
        stats.ledgers_persisted, 3,
        "all three contiguous fixtures should be processed"
    );

    // Cursor file persisted at the last ledger → next invocation resumes here.
    let resumed = StubFileCursor::new(dir.path().join("cursor.txt"));
    assert_eq!(resumed.read().await.unwrap(), LAST_FIXTURE);
}

#[tokio::test]
async fn gap_stop_when_no_new_ledger() {
    skip_if_no_fixtures!();
    let dir = tempdir().unwrap();
    let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
    // Start past the last fixture → next key (62460543) is a miss → gap stop.
    cursor.write(LAST_FIXTURE).await.unwrap();

    let stats = reconciler(fixtures_dir(), cursor).run(16).await.unwrap();

    assert_eq!(stats.ledgers_persisted, 0);
    assert_eq!(stats.end_cursor, LAST_FIXTURE);
    assert_eq!(stats.rows_emitted, 0);
}

#[tokio::test]
async fn idempotent_on_re_run_from_same_cursor() {
    skip_if_no_fixtures!();
    let run = || async {
        let dir = tempdir().unwrap();
        let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
        cursor.write(FIRST_FIXTURE - 1).await.unwrap();
        reconciler(fixtures_dir(), cursor).run(16).await.unwrap()
    };

    let first = run().await;
    let second = run().await;

    assert_eq!(first.start_cursor, second.start_cursor);
    assert_eq!(first.end_cursor, second.end_cursor);
    assert_eq!(first.ledgers_persisted, second.ledgers_persisted);
    assert_eq!(
        first.rows_emitted, second.rows_emitted,
        "row count must be deterministic across identical runs"
    );
}
