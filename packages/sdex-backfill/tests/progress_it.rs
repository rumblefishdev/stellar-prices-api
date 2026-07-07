//! Integration test for the dual `backfill_progress` writer (task 0053 /
//! decision 6), against a local Docker ClickHouse with the `prices` schema
//! applied. Validates the three behaviours the unit tests cannot: the
//! `toUnixTimestamp(Nullable(DateTime))` round-trip, the `'running'` string →
//! `Enum8` insert coercion, and the ReplacingMergeTree read-modify-write
//! (preserve `started_at`, honour `Current::Keep`, monotonic window).
//!
//!     docker compose up -d clickhouse
//!     cargo test -p sdex-backfill --test progress_it -- --ignored --nocapture
//!
//! Destructive to the local `prices.backfill_progress` table (truncates it);
//! never run against a shared/prod cluster.

use clickhouse::Client;
use sdex_backfill::ingest::ExtractMode;
use sdex_backfill::progress::{Observed, Phase, SDEX_ARCHIVE, SOROBAN_AMM, progress_updates};
use sdex_backfill::sink::Sink;

const ACTIVATION: u32 = 50_457_424;
const TIP: u32 = 55_000_000;
const T0: u32 = 1_700_000_000; // 2023-11-14
const T1: u32 = 1_720_000_000; // 2024-07-03

fn ch_url() -> String {
    std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".to_string())
}

fn client() -> Client {
    Client::default().with_url(ch_url())
}

async fn u64_of(c: &Client, sql: &str) -> u64 {
    c.query(sql).fetch_one::<u64>().await.expect("scalar u64")
}
async fn u32_of(c: &Client, sql: &str) -> u32 {
    c.query(sql).fetch_one::<u32>().await.expect("scalar u32")
}
async fn str_of(c: &Client, sql: &str) -> String {
    c.query(sql)
        .fetch_one::<String>()
        .await
        .expect("scalar str")
}

/// `current_ledger` for a stream (latest via FINAL).
async fn current(c: &Client, task: &str) -> u64 {
    u64_of(
        c,
        &format!(
            "SELECT current_ledger FROM prices.backfill_progress FINAL WHERE task_name='{task}'"
        ),
    )
    .await
}
async fn status(c: &Client, task: &str) -> String {
    str_of(
        c,
        &format!(
            "SELECT toString(status) FROM prices.backfill_progress FINAL WHERE task_name='{task}'"
        ),
    )
    .await
}
/// Unix seconds of a Nullable(DateTime) column; 0 when NULL.
async fn ts(c: &Client, task: &str, col: &str) -> u32 {
    u32_of(
        c,
        &format!(
            "SELECT ifNull(toUInt32(toUnixTimestamp({col})), 0) \
             FROM prices.backfill_progress FINAL WHERE task_name='{task}'"
        ),
    )
    .await
}
async fn completed_at_set(c: &Client, task: &str) -> bool {
    u32_of(
        c,
        &format!(
            "SELECT toUInt32(completed_at IS NOT NULL) \
             FROM prices.backfill_progress FINAL WHERE task_name='{task}'"
        ),
    )
    .await
        == 1
}

async fn write(sink: &Sink, mode: ExtractMode, start: u32, obs: Observed, phase: Phase) {
    for u in progress_updates(mode, start, TIP, ACTIVATION, obs, phase) {
        sink.write_progress(&u).await.expect("write_progress");
    }
}

#[tokio::test]
#[ignore = "requires a local ClickHouse (docker compose up -d clickhouse)"]
async fn combined_then_sdex_progress_end_to_end() {
    let c = client();
    prices_clickhouse::apply_sql(&c, prices_clickhouse::INIT_SQL)
        .await
        .expect("apply init schema");
    c.query("TRUNCATE TABLE prices.backfill_progress")
        .execute()
        .await
        .expect("truncate backfill_progress");

    let sink = Sink::new(&ch_url());

    // --- Phase 1: combined run, mid-run (Running) --------------------------
    write(
        &sink,
        ExtractMode::Combined,
        ACTIVATION,
        Observed {
            highest_indexed: ACTIVATION + 1_000,
            earliest_minute: Some(T0),
            newest_minute: Some(T1),
        },
        Phase::Running,
    )
    .await;

    // soroban_amm advances forward; window + Enum8 round-trip.
    assert_eq!(current(&c, SOROBAN_AMM).await, (ACTIVATION + 1_000) as u64);
    assert_eq!(status(&c, SOROBAN_AMM).await, "running");
    assert_eq!(ts(&c, SOROBAN_AMM, "earliest_data_available").await, T0);
    assert_eq!(ts(&c, SOROBAN_AMM, "newest_data_available").await, T1);
    // sdex_archive: Current::Keep on a fresh row → seeded 0; window still flows.
    assert_eq!(current(&c, SDEX_ARCHIVE).await, 0);
    assert_eq!(status(&c, SDEX_ARCHIVE).await, "running");
    assert_eq!(ts(&c, SDEX_ARCHIVE, "newest_data_available").await, T1);

    let started_soroban = ts(&c, SOROBAN_AMM, "started_at").await;

    // --- Phase 2: combined run, completion ---------------------------------
    write(
        &sink,
        ExtractMode::Combined,
        ACTIVATION,
        Observed {
            highest_indexed: TIP,
            earliest_minute: Some(T0),
            newest_minute: Some(T1),
        },
        Phase::Completed,
    )
    .await;

    assert_eq!(current(&c, SOROBAN_AMM).await, TIP as u64);
    assert_eq!(status(&c, SOROBAN_AMM).await, "completed");
    assert!(
        completed_at_set(&c, SOROBAN_AMM).await,
        "soroban completed_at set"
    );
    // The AC: recent SDEX reflected → oldest reflected = activation; the stream
    // is paused between the two runs (decision 6), not running.
    assert_eq!(current(&c, SDEX_ARCHIVE).await, ACTIVATION as u64);
    assert_eq!(status(&c, SDEX_ARCHIVE).await, "paused");
    // started_at preserved across the read-modify-write.
    assert_eq!(
        ts(&c, SOROBAN_AMM, "started_at").await,
        started_soroban,
        "started_at must be preserved, not reset to now()"
    );

    // --- Phase 3: idempotent re-run — no duplicate rows after FINAL --------
    write(
        &sink,
        ExtractMode::Combined,
        ACTIVATION,
        Observed {
            highest_indexed: TIP,
            earliest_minute: Some(T0),
            newest_minute: Some(T1),
        },
        Phase::Completed,
    )
    .await;
    let rows = u64_of(&c, "SELECT count() FROM prices.backfill_progress FINAL").await;
    assert_eq!(rows, 2, "FINAL must collapse re-runs to one row per stream");
    assert_eq!(current(&c, SOROBAN_AMM).await, TIP as u64);

    // --- Phase 4: window is monotonic — an older/narrower update never shrinks it
    write(
        &sink,
        ExtractMode::Combined,
        ACTIVATION,
        Observed {
            highest_indexed: TIP,
            earliest_minute: Some(T0 - 1_000), // older → should LOWER earliest
            newest_minute: Some(T0),           // older → must NOT lower newest
        },
        Phase::Running,
    )
    .await;
    assert_eq!(
        ts(&c, SOROBAN_AMM, "earliest_data_available").await,
        T0 - 1_000,
        "earliest lowers to the oldest seen"
    );
    assert_eq!(
        ts(&c, SOROBAN_AMM, "newest_data_available").await,
        T1,
        "newest stays at the max seen (monotonic)"
    );

    // --- Phase 5: sdex-only tail completes the archive at genesis ----------
    write(
        &sink,
        ExtractMode::SdexOnly,
        1,
        Observed {
            highest_indexed: ACTIVATION - 1,
            earliest_minute: Some(T0 - 500_000),
            newest_minute: Some(T0),
        },
        Phase::Completed,
    )
    .await;
    assert_eq!(
        current(&c, SDEX_ARCHIVE).await,
        1,
        "oldest reflected = genesis"
    );
    assert_eq!(status(&c, SDEX_ARCHIVE).await, "completed");
    assert!(
        completed_at_set(&c, SDEX_ARCHIVE).await,
        "sdex completed_at set"
    );
    // soroban_amm untouched by the sdex-only run.
    assert_eq!(current(&c, SOROBAN_AMM).await, TIP as u64);

    // --- Phase 6: a later combined pass must NOT regress or un-complete the
    // archive the sdex-only run already carried down to genesis (the
    // chronological-order bug: combined completion used to overwrite
    // sdex_archive back to current=activation, status='running').
    write(
        &sink,
        ExtractMode::Combined,
        ACTIVATION,
        Observed {
            highest_indexed: TIP,
            earliest_minute: Some(T0 - 500_000),
            newest_minute: Some(T1),
        },
        Phase::Completed,
    )
    .await;
    assert_eq!(
        current(&c, SDEX_ARCHIVE).await,
        1,
        "backward current must not regress from genesis back up to activation"
    );
    assert_eq!(
        status(&c, SDEX_ARCHIVE).await,
        "completed",
        "a stored 'completed' must never be downgraded to 'running'"
    );
}
