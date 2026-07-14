//! Lambda entrypoint — SQS doorbell handler (built only with `--features lambda`).
//!
//! Cold start mirrors BE's indexer: eager config + connectivity validation, then
//! one shared [`Reconciler`] reused across invocations. The SQS message body is
//! **ignored** — production doorbells arrive via SNS fan-out
//! (`S3 ObjectCreated → SNS (BE-owned) → prices-ingest-{env} SQS + DLQ → here`,
//! 2026-06-10 cross-team decision); raw or SNS-wrapped, the handler just runs
//! the doorbell-cursor reconcile loop. `reservedConcurrency = 1` (set in CDK)
//! keeps runs serial, which is the ordering guarantee.
//!
//! Transport here is production: S3 object fetch + ClickHouse over mTLS (task
//! 0052). The cursor is durable in `prices.ingest_cursor` (task 0064), sharing
//! the sink's mTLS client, so it survives execution-environment recycles instead
//! of rewinding to the `INITIAL_CURSOR` seed on every cold start.

use std::sync::Arc;

use aws_lambda_events::sqs::{BatchItemFailure, SqsBatchResponse, SqsEvent};
use lambda_runtime::{Error, LambdaEvent, run, service_fn};
use prices_ledger_processor::{
    cursor::{ClickHouseCursor, Cursor, CursorError},
    object_fetcher::S3Fetcher,
    reconcile::Reconciler,
    sink::ClickHouseSink,
};
use tracing::{error, info};

const ENV_BUCKET: &str = "BUCKET_NAME";
const ENV_INITIAL_CURSOR: &str = "INITIAL_CURSOR";
const ENV_MAX_ITERATIONS: &str = "MAX_ITERATIONS";
const DEFAULT_MAX_ITERATIONS: usize = 16;
/// Logical consumer key for this processor's row in `prices.ingest_cursor`.
const CURSOR_ID: &str = "ledger-processor";

type R = Reconciler<S3Fetcher, ClickHouseCursor, ClickHouseSink>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    // Eager cold-start init — a missing env / unreachable cluster should be a
    // Lambda Init error, not a per-event panic.
    let bucket = std::env::var(ENV_BUCKET)
        .map_err(|_| Error::from(format!("{ENV_BUCKET} env var is required")))?;
    let max_iterations: usize = std::env::var(ENV_MAX_ITERATIONS)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_ITERATIONS);

    // Build the S3 fetcher and the mTLS sink concurrently — they are
    // independent (ambient AWS config load vs. Secrets-extension fetch +
    // mTLS handshake), so joining them shaves their latency off cold start.
    let (fetcher, sink) = tokio::join!(
        S3Fetcher::from_env(&bucket),
        ClickHouseSink::from_lambda_env()
    );
    let sink = sink?;

    // Durable cursor in `prices.ingest_cursor` (task 0064), sharing the sink's
    // mTLS client. Unlike the old `/tmp` file, it survives execution-environment
    // recycles — so the loop resumes from the stored ledger instead of rewinding
    // to INITIAL_CURSOR every cold start (the freeze this task fixes).
    let cursor = ClickHouseCursor::new(sink.client().clone(), CURSOR_ID);
    match cursor.read().await {
        // Populated → resume where we left off.
        Ok(at) => info!(at, "resumed cursor from ClickHouse"),
        // Genuinely empty table (true first run) → seed once from INITIAL_CURSOR.
        Err(CursorError::Empty) => {
            match std::env::var(ENV_INITIAL_CURSOR)
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
            {
                Some(seed) => {
                    cursor.write(seed).await?;
                    info!(
                        seed,
                        "seeded cursor from INITIAL_CURSOR (empty ingest_cursor)"
                    );
                }
                // No seed configured: leave the table empty; the first reconcile
                // run's read errors and DLQs (recoverable once a seed is set).
                None => {
                    info!("ingest_cursor empty and no INITIAL_CURSOR set — first doorbell will DLQ")
                }
            }
        }
        // Any OTHER read error (transient CH failure, or a missing table) must
        // NOT be treated as empty: seeding here would clobber a healthy durable
        // cursor with the floor value and re-freeze the frontier. Fail Init loudly
        // instead — the retry cold-starts and reads cleanly once CH recovers, and
        // a truly missing table stays a loud, alarmable Init failure.
        Err(e) => {
            return Err(Error::from(format!(
                "ingest_cursor read failed at init: {e}"
            )));
        }
    }
    // Two independent ClickHouse reads on the cold path — joined so `try_join!`
    // shaves a round-trip off Lambda Init (same reasoning as the fetcher+sink join
    // above). Either failing still surfaces an unreachable cluster (or missing
    // schema) as a Lambda Init error, so no separate preflight `SELECT 1` is needed.
    //
    // - load_registry: existing asset surrogate ids from `prices.assets`.
    // - load_pool_registry: discovered AMM pool classification from
    //   `prices.pool_registry` (task 0078). The processor only READS this table —
    //   it never applies schema — so the table MUST already exist (created by
    //   init.sql, applied out-of-band; task 0076). Present-but-empty (registry not
    //   yet seeded by the 0053 backfill) is fine: AMM swaps for pre-existing pools
    //   stay unresolved but SDEX is unaffected. ABSENT is NOT fine: the read errors
    //   and init fails (taking SDEX down too), so schema must be applied before deploy.
    let (registry, pool_registry) =
        tokio::try_join!(sink.load_registry(), sink.load_pool_registry())?;

    info!(
        %bucket,
        max_iterations,
        cursor_id = CURSOR_ID,
        "prices-ledger-processor cold start ready"
    );

    let reconciler: Arc<R> = Arc::new(Reconciler::new(
        fetcher,
        cursor,
        sink,
        registry,
        pool_registry,
    ));

    run(service_fn(move |event: LambdaEvent<SqsEvent>| {
        let r = reconciler.clone();
        async move { handler(event, r, max_iterations).await }
    }))
    .await
}

async fn handler(
    event: LambdaEvent<SqsEvent>,
    reconciler: Arc<R>,
    max_iterations: usize,
) -> Result<SqsBatchResponse, Error> {
    let (payload, _ctx) = event.into_parts();
    let mut batch_item_failures = Vec::new();

    for msg in &payload.records {
        let message_id = msg.message_id.clone().unwrap_or_default();
        match reconciler.run(max_iterations).await {
            Ok(stats) => info!(
                message_id = %message_id,
                start = stats.start_cursor,
                end = stats.end_cursor,
                persisted = stats.ledgers_persisted,
                rows = stats.rows_emitted,
                "doorbell processed"
            ),
            Err(e) => {
                error!(
                    message_id = %message_id,
                    error = %e,
                    "reconcile failed — will redeliver doorbell"
                );
                batch_item_failures.push(BatchItemFailure {
                    item_identifier: message_id,
                });
            }
        }
    }

    Ok(SqsBatchResponse {
        batch_item_failures,
    })
}
