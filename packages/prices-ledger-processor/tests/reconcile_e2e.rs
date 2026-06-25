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

use prices_ingest_core::{AssetRegistry, Registries};
use prices_ledger_processor::{
    cursor::{Cursor, StubFileCursor},
    object_fetcher::LocalDiskFetcher,
    reconcile::Reconciler,
    sink::CountingSink,
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
