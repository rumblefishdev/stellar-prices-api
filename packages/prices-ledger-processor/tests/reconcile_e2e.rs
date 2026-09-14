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
    // MODIFIED for task 0282, intentional, not a regression: the three fixtures
    // all share one minute, so run 2 decodes them, writes their assets, then
    // holds the candles back rather than emitting a partial minute. "Reprocessed
    // the fixtures" is therefore counted by `ledgers_held_back`, not
    // `ledgers_persisted`. The assertion this test exists for — that the failed
    // run's interned assets are re-offered and written — is unchanged below.
    assert_eq!(
        second.ledgers_held_back, 3,
        "run 2 reprocesses the fixtures (held back: all three share one minute)"
    );
    assert!(
        sink.assets_written.load(Ordering::Relaxed) > 0,
        "assets interned by the failed run must be written on retry, not orphaned"
    );
}

/// MODIFIED for task 0282, intentional, not a regression. This test used to
/// assert the cursor advanced to `LAST_FIXTURE` and all three ledgers were
/// persisted. All three fixtures (62460540-62460542) close inside the SAME
/// minute, so under the pre-0282 contract the run wrote that minute as a
/// candle — a partial one, which the next run's higher-`version` write then
/// REPLACED rather than summed, silently dropping the earlier slice. That is
/// the defect, and this test was pinning it.
///
/// The contract now: decode everything, write the assets, and hold the candles
/// back until a ledger from the FOLLOWING minute proves the minute is closed.
/// So the cursor must NOT move here, and nothing may be written.
///
/// The advance case cannot be exercised from these fixtures — there is no
/// ledger from a later minute among them — so it is covered by the
/// `run_boundary` unit tests in `reconcile.rs` instead.
#[tokio::test]
async fn a_run_inside_one_minute_decodes_but_holds_the_cursor() {
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
        stats.end_cursor,
        FIRST_FIXTURE - 1,
        "cursor must NOT advance: every fixture ledger is in the still-open minute"
    );
    assert_eq!(
        stats.ledgers_held_back, 3,
        "all three fixtures decoded, all three held back"
    );
    assert_eq!(
        stats.ledgers_persisted, 0,
        "nothing may be persisted from an open minute"
    );
    assert_eq!(stats.rows_emitted, 0, "no partial candle may be written");

    // Cursor file unchanged → the next invocation re-reads these ledgers and
    // completes the minute in a single write once it turns over.
    let resumed = StubFileCursor::new(dir.path().join("cursor.txt"));
    assert_eq!(resumed.read().await.unwrap(), FIRST_FIXTURE - 1);
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

/// MODIFIED for task 0282 / review of PR #313 finding 4. This used to call
/// `run(16)`, which after the hold-back change took the early return on both
/// passes — so every assertion compared `0 == 0` and the test would have kept
/// passing with `flush_older_than`, the cursor write, or row emission broken
/// outright. It now drives `run_terminal`, which flushes, so the comparison is
/// between two runs that actually wrote something.
#[tokio::test]
async fn idempotent_on_re_run_from_same_cursor() {
    skip_if_no_fixtures!();
    let run = || async {
        let dir = tempdir().unwrap();
        let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
        cursor.write(FIRST_FIXTURE - 1).await.unwrap();
        reconciler(fixtures_dir(), cursor)
            .run_terminal(16)
            .await
            .unwrap()
    };

    let first = run().await;
    let second = run().await;

    assert!(
        first.rows_emitted > 0,
        "the comparison below is worthless unless the run actually wrote candles"
    );
    assert_eq!(first.start_cursor, second.start_cursor);
    assert_eq!(first.end_cursor, second.end_cursor);
    assert_eq!(first.ledgers_persisted, second.ledgers_persisted);
    assert_eq!(
        first.rows_emitted, second.rows_emitted,
        "row count must be deterministic across identical runs"
    );
}

/// Review of PR #313 finding 2, end to end: a run whose whole iteration budget
/// lands inside one minute must still advance. Without the escape hatch the
/// cursor never moves and the same ledgers are re-read forever, silently.
///
/// The three fixtures share a minute, so `max_iterations = 3` reproduces the
/// deadlock shape exactly: budget spent, no complete minute.
#[tokio::test]
async fn a_budget_exhausted_inside_one_minute_still_advances() {
    skip_if_no_fixtures!();
    let dir = tempdir().unwrap();
    let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
    cursor.write(FIRST_FIXTURE - 1).await.unwrap();

    let stats = reconciler(fixtures_dir(), cursor)
        .run(3)
        .await
        .expect("forced-progress run should succeed");

    assert_eq!(
        stats.end_cursor, LAST_FIXTURE,
        "budget exhausted inside one minute: flush the partial minute and advance"
    );
    assert!(
        stats.rows_emitted > 0,
        "the forced flush must actually write the partial minute, not drop it"
    );
    assert_eq!(
        stats.ledgers_held_back, 0,
        "a forced run holds nothing back — everything was flushed"
    );

    // And the cursor is durable, so the next invocation resumes past it.
    let resumed = StubFileCursor::new(dir.path().join("cursor.txt"));
    assert_eq!(resumed.read().await.unwrap(), LAST_FIXTURE);
}

/// Review of PR #313 finding 3: `bin/cli.rs` calls the reconciler once and then
/// exits, so a held-back minute would never be written by anyone. The terminal
/// variant flushes it.
#[tokio::test]
async fn a_terminal_run_flushes_the_open_minute() {
    skip_if_no_fixtures!();
    let dir = tempdir().unwrap();
    let cursor = StubFileCursor::new(dir.path().join("cursor.txt"));
    cursor.write(FIRST_FIXTURE - 1).await.unwrap();

    let stats = reconciler(fixtures_dir(), cursor)
        .run_terminal(16)
        .await
        .expect("terminal run should succeed");

    assert_eq!(stats.end_cursor, LAST_FIXTURE);
    assert!(
        stats.rows_emitted > 0,
        "a one-shot run must not drop its final minute — nothing re-reads it"
    );
}
