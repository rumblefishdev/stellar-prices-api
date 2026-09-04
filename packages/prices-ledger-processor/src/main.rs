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
    metrics,
    object_fetcher::S3Fetcher,
    reconcile::Reconciler,
    sink::ClickHouseSink,
};
use tracing::{error, info, warn};

const ENV_BUCKET: &str = "BUCKET_NAME";
const ENV_INITIAL_CURSOR: &str = "INITIAL_CURSOR";
const ENV_MAX_ITERATIONS: &str = "MAX_ITERATIONS";
/// Deployment environment, set on the function by CDK. Every custom metric in
/// this account carries it as its `Environment` dimension.
const ENV_NAME: &str = "ENV_NAME";
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

    // Three independent ClickHouse reads on the cold path — the durable cursor
    // (task 0064) plus the two registries — joined so they overlap instead of
    // running serially (same reasoning as the fetcher+sink join above). All use
    // the sink's shared client; clients are cheap to clone and pool-backed, so
    // concurrent queries are safe.
    //
    // - cursor.read(): last processed ledger from `prices.ingest_cursor`. Durable,
    //   so the loop resumes across execution-environment recycles instead of
    //   rewinding to INITIAL_CURSOR every cold start (the freeze this task fixes).
    // - load_registry: existing asset surrogate ids from `prices.assets`.
    // - load_pool_registry: discovered AMM pool classification from
    //   `prices.pool_registry` (task 0078). The processor only READS this table —
    //   it never applies schema — so the table MUST already exist (created by
    //   init.sql, applied out-of-band; task 0076). Present-but-empty (registry not
    //   yet seeded by the 0053 backfill) is fine: AMM swaps for pre-existing pools
    //   stay unresolved but SDEX is unaffected. ABSENT is NOT fine: the read errors
    //   and init fails (taking SDEX down too), so schema must be applied before deploy.
    let cursor = ClickHouseCursor::new(sink.client().clone(), CURSOR_ID);
    let (cursor_state, registry, pool_registry) = tokio::join!(
        cursor.read(),
        sink.load_registry(),
        sink.load_pool_registry(),
    );
    let registry = registry?;
    let pool_registry = pool_registry?;

    // Seed only on a genuinely empty table (true first run); any OTHER read error
    // (transient CH failure, or a missing table) must NOT be treated as empty —
    // seeding would clobber a healthy durable cursor with the floor value and
    // re-freeze the frontier. Fail Init loudly instead: the retry cold-starts and
    // reads cleanly once CH recovers, and a truly missing table stays a loud,
    // alarmable Init failure.
    match cursor_state {
        Ok(at) => info!(at, "resumed cursor from ClickHouse"),
        Err(CursorError::Empty) => match std::env::var(ENV_INITIAL_CURSOR)
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
            // No seed configured: leave the table empty; the first reconcile run's
            // read errors and DLQs (recoverable once a seed is set).
            None => {
                info!("ingest_cursor empty and no INITIAL_CURSOR set — first doorbell will DLQ")
            }
        },
        Err(e) => {
            return Err(Error::from(format!(
                "ingest_cursor read failed at init: {e}"
            )));
        }
    }

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

    // CloudWatch client for `ClickHouseWriteLatencyMs` (task 0125). Built once
    // at cold start; the publish itself is best-effort per invocation and runs
    // only after the cursor commit, so a CloudWatch failure never redelivers a
    // doorbell.
    //
    // Bounded on purpose: the publish is best-effort, but without a timeout a
    // stalled CloudWatch endpoint would hold the invocation open until the
    // 60 s Lambda limit — and with reserved concurrency 1 that is ingestion
    // lag, not a dropped metric. Worst case with these numbers is ~6 s
    // (2 attempts x 3 s), well inside the ~5 s ledger cadence's slack.
    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .timeout_config(
            aws_config::timeout::TimeoutConfig::builder()
                .connect_timeout(std::time::Duration::from_secs(1))
                .operation_attempt_timeout(std::time::Duration::from_secs(3))
                .operation_timeout(std::time::Duration::from_secs(6))
                .build(),
        )
        .retry_config(aws_config::retry::RetryConfig::standard().with_max_attempts(2))
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let env_name = Arc::new(std::env::var(ENV_NAME).unwrap_or_else(|_| {
        // Every datapoint would land on `Environment=unknown`, a dimension no
        // widget or alarm reads. Loud, but not fatal: ingestion must not stop
        // over telemetry.
        tracing::error!(
            var = ENV_NAME,
            "environment variable missing — write-latency metrics will be tagged Environment=unknown and no dashboard will show them"
        );
        "unknown".to_string()
    }));

    run(service_fn(move |event: LambdaEvent<SqsEvent>| {
        let r = reconciler.clone();
        let cw = cw.clone();
        let env_name = env_name.clone();
        async move { handler(event, r, max_iterations, cw, env_name).await }
    }))
    .await
}

async fn handler(
    event: LambdaEvent<SqsEvent>,
    reconciler: Arc<R>,
    max_iterations: usize,
    cw: Arc<aws_sdk_cloudwatch::Client>,
    env_name: Arc<String>,
) -> Result<SqsBatchResponse, Error> {
    let (payload, _ctx) = event.into_parts();
    let mut batch_item_failures = Vec::new();

    for msg in &payload.records {
        let message_id = msg.message_id.clone().unwrap_or_default();
        match reconciler.run(max_iterations).await {
            Ok(stats) => {
                info!(
                    message_id = %message_id,
                    start = stats.start_cursor,
                    end = stats.end_cursor,
                    persisted = stats.ledgers_persisted,
                    rows = stats.rows_emitted,
                    "doorbell processed"
                );

                // Task 0125 — publish the candle-write latency. This sits in the
                // `Ok` arm ON PURPOSE: reaching it means `reconcile` already
                // committed the cursor, so the rows are durable and nothing here
                // can reach the `Err` arm below, which is the sole path that
                // pushes a `BatchItemFailure`. Best-effort: a CloudWatch failure
                // is a warning, never a redelivered doorbell.
                let m = metrics::write_latency_metrics(stats.ch_write);
                if let Err(e) = metrics::publish(&cw, &env_name, &m).await {
                    warn!(error = %e, "cloudwatch metric publish failed (non-fatal)");
                }
            }
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
