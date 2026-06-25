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
//! 0052). The cursor is still a file checkpoint (`CURSOR_FILE`, seeded from
//! `INITIAL_CURSOR`) pending the CH-backed cursor decision (G-note Part D.1).

use std::path::PathBuf;
use std::sync::Arc;

use aws_lambda_events::sqs::{BatchItemFailure, SqsBatchResponse, SqsEvent};
use lambda_runtime::{Error, LambdaEvent, run, service_fn};
use prices_ingest_core::Registries;
use prices_ledger_processor::{
    cursor::{Cursor, StubFileCursor},
    object_fetcher::S3Fetcher,
    reconcile::Reconciler,
    sink::ClickHouseSink,
};
use tracing::{error, info};

const ENV_BUCKET: &str = "BUCKET_NAME";
const ENV_CURSOR_FILE: &str = "CURSOR_FILE";
const ENV_INITIAL_CURSOR: &str = "INITIAL_CURSOR";
const ENV_MAX_ITERATIONS: &str = "MAX_ITERATIONS";
const DEFAULT_CURSOR_FILE: &str = "/tmp/prices-cursor.txt";
const DEFAULT_MAX_ITERATIONS: usize = 16;

type R = Reconciler<S3Fetcher, StubFileCursor, ClickHouseSink>;

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
    let cursor_file = std::env::var(ENV_CURSOR_FILE)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CURSOR_FILE));
    let max_iterations: usize = std::env::var(ENV_MAX_ITERATIONS)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_ITERATIONS);

    let cursor = StubFileCursor::new(&cursor_file);
    // Seed the cursor on a fresh container if it has no checkpoint yet.
    if cursor.read().await.is_err()
        && let Some(seed) = std::env::var(ENV_INITIAL_CURSOR)
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
    {
        cursor.write(seed).await?;
        info!(seed, "seeded cursor from INITIAL_CURSOR");
    }

    // Build the S3 fetcher and the mTLS sink concurrently — they are
    // independent (ambient AWS config load vs. Secrets-extension fetch +
    // mTLS handshake), so joining them shaves their latency off cold start.
    let (fetcher, sink) = tokio::join!(
        S3Fetcher::from_env(&bucket),
        ClickHouseSink::from_lambda_env()
    );
    let sink = sink?;
    // `load_registry` is the first ClickHouse round-trip, so it already
    // surfaces an unreachable cluster as a Lambda Init error — a separate
    // preflight `SELECT 1` would just be a redundant extra round-trip on the
    // cold path.
    let registry = sink.load_registry().await?;

    info!(
        %bucket,
        cursor_file = %cursor_file.display(),
        max_iterations,
        "prices-ledger-processor cold start ready"
    );

    let reconciler: Arc<R> = Arc::new(Reconciler::new(
        fetcher,
        cursor,
        sink,
        registry,
        Registries::new(),
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
