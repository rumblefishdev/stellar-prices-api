//! Lambda entrypoint — SQS doorbell handler.
//!
//! Mirrors BE's indexer cold-start shape (eager config validation,
//! structured JSON tracing, single shared state passed by reference to
//! every invocation). The SQS message body is **ignored**; each
//! invocation just runs the reconcile loop.
//!
//! Doorbell transport (2026-06-10 cross-team decision, spec §C.1):
//! production doorbells reach this Lambda via **SNS fan-out** —
//! `S3 ObjectCreated → SNS (BE-owned) → prices-ingest-{env} SQS + DLQ
//! → this Lambda`. Because the body is ignored, the handler is
//! identical whether the message is raw or SNS-wrapped; the `SqsEvent`
//! envelope is all we deserialise. Failure isolation: the prices queue
//! is prices-owned, so a backlog here never pressures BE's indexer.
//!
//! Phase 2 prototype: the fetcher / cursor / sink are still the
//! local-disk stubs. The Lambda mode exists to prove the
//! `lambda_runtime` event-loop wires up cleanly — a `cargo lambda
//! invoke` against a stub doorbell event runs end-to-end.

use std::path::PathBuf;
use std::sync::Arc;

use aws_lambda_events::sqs::{SqsBatchResponse, SqsEvent};
use extractors_core::VenueRegistry;
use lambda_runtime::{Error, LambdaEvent, service_fn};
use phoenix_extractor::PhoenixPoolRegistry;
use prices_ledger_processor::{
    cursor::StubFileCursor, decode::XdrLedgerDecoder, object_fetcher::LocalDiskFetcher,
    reconcile::Reconciler, sink::StdoutJsonSink,
};
use soroswap_extractor::SoroswapPoolRegistry;
use tracing::{error, info};

const ENV_FIXTURES_DIR: &str = "FIXTURES_DIR";
const ENV_CURSOR_FILE: &str = "CURSOR_FILE";
const ENV_MAX_ITERATIONS: &str = "MAX_ITERATIONS";
const DEFAULT_FIXTURES_DIR: &str = "fixtures/ledgers";
const DEFAULT_CURSOR_FILE: &str = "out/cursor.txt";
const DEFAULT_MAX_ITERATIONS: usize = 16;

type R = Reconciler<LocalDiskFetcher, StubFileCursor, StdoutJsonSink, XdrLedgerDecoder>;

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let fixtures_dir = std::env::var(ENV_FIXTURES_DIR)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_FIXTURES_DIR));
    let cursor_file = std::env::var(ENV_CURSOR_FILE)
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(DEFAULT_CURSOR_FILE));
    let max_iterations: usize = std::env::var(ENV_MAX_ITERATIONS)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_MAX_ITERATIONS);

    info!(
        fixtures_dir = %fixtures_dir.display(),
        cursor_file = %cursor_file.display(),
        max_iterations,
        "prices-ledger-processor cold start"
    );

    let reconciler: Arc<R> = Arc::new(Reconciler {
        fetcher: LocalDiskFetcher::new(&fixtures_dir),
        cursor: StubFileCursor::new(&cursor_file),
        sink: StdoutJsonSink,
        decoder: XdrLedgerDecoder,
        venue_registry: VenueRegistry::new(),
        phoenix_registry: PhoenixPoolRegistry::default(),
        soroswap_registry: SoroswapPoolRegistry::new(),
    });

    lambda_runtime::run(service_fn(move |event: LambdaEvent<SqsEvent>| {
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
                batch_item_failures.push(aws_lambda_events::sqs::BatchItemFailure {
                    item_identifier: message_id,
                });
            }
        }
    }

    Ok(SqsBatchResponse {
        batch_item_failures,
    })
}
