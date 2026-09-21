//! Coverage sweep probe Lambda entrypoint (task 0100).
//!
//! EventBridge, weekly (Monday 05:17 UTC) → this binary. Each run sweeps the
//! trailing [`SWEEP_WINDOW_LEDGERS`](coverage_sweep_probe::sweep::SWEEP_WINDOW_LEDGERS)
//! ledgers of BE's `soroban_events` for swap/trade-shaped emitters that are in
//! neither `prices.pool_registry` nor the committed allow-list, logs each one,
//! and publishes the residual to `Prices/Coverage` when it is non-zero.
//!
//!     cargo lambda build -p coverage-sweep-probe --release --arm64 --features lambda
//!
//! **Identity (decision D5, option A).** The probe uses the existing
//! `ingestion` mTLS identity, i.e. ClickHouse user `prices_writer`, like every
//! other scheduled worker. That user needs two grants from BE —
//! `SELECT ON default.soroban_events` and `SELECT ON default.soroban_contracts`
//! (docs/runbooks/0100-coverage-sweep-triage.md §4.1). Until they land every
//! run fails with Code 497 ACCESS_DENIED and pages through the probe's
//! `-errors` alarm. That is intended: a swallowed error would publish nothing,
//! and the NOT_BREACHING unclassified alarm would read green forever.

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use coverage_sweep_probe::sweep::run_sweep;
    use coverage_sweep_probe::{
        AllowList, BE_DATABASE, PRICES_DATABASE, SWEEP_MAX_EXECUTION_SECS, publish,
        unclassified_metrics,
    };
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use std::sync::Arc;

    prices_clickhouse::observability::init_tracing();

    // A list that fails to parse fails Init, and the -errors alarm fires.
    let allow = Arc::new(AllowList::embedded()?);

    // `client_from_lambda_env` already applies `with_readable_errors`. The
    // execution bound is added here because prices_writer's profile carries
    // none (prices-clickhouse `with_execution_bound`): a ClickHouse
    // TIMEOUT_EXCEEDED is a real, logged error, while a Lambda kill at the
    // 120 s timeout is not — and 90 s fires first.
    let ch = prices_clickhouse::mtls::client_from_lambda_env(PRICES_DATABASE).await?;
    let ch = Arc::new(prices_clickhouse::with_execution_bound(
        ch,
        SWEEP_MAX_EXECUTION_SECS,
    ));

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(
        environment = %environment,
        allowlist_contracts = allow.contract.len(),
        allowlist_wasm = allow.wasm.len(),
        "coverage-sweep-probe cold start ready"
    );

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let ch = ch.clone();
        let cw = cw.clone();
        let environment = environment.clone();
        let allow = allow.clone();
        async move {
            // Every error propagates: Code 497 before BE's grant, a timeout, a
            // bad identifier. Nothing is mapped to "found nothing".
            let report = run_sweep(&ch, BE_DATABASE, PRICES_DATABASE, &allow).await?;

            for row in &report.unclassified {
                tracing::warn!(
                    contract = %row.display_id(),
                    wasm = row.wasm.as_deref().unwrap_or(""),
                    events = row.events,
                    txs = row.txs,
                    first_ledger = row.first_ledger,
                    last_ledger = row.last_ledger,
                    top_action = %row.top_action,
                    top_shape = %row.top_shape,
                    "unclassified swap emitter"
                );
            }

            let metrics = unclassified_metrics(&report.unclassified);
            let unclassified_events: u64 = report.unclassified.iter().map(|r| r.events).sum();
            // Logged on every run, so "ran and found nothing" can be told
            // apart from "never ran" (there is no liveness alarm).
            tracing::info!(
                max_ledger = report.max_ledger,
                lo = report.lo,
                hi = report.hi,
                rows = report.rows_total,
                allowlisted = report.allowlisted.len(),
                unclassified = report.unclassified.len(),
                unclassified_events,
                "coverage sweep complete"
            );

            // Propagated: a failed PutMetricData leaves the NOT_BREACHING alarm
            // green, so it must fail the invocation and fire -errors instead.
            publish(&cw, &environment, &metrics).await?;

            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "max_ledger": report.max_ledger,
                "lo": report.lo,
                "hi": report.hi,
                "rows": report.rows_total,
                "allowlisted": report.allowlisted.len(),
                "unclassified": report.unclassified.len(),
                "unclassified_events": unclassified_events,
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "coverage-sweep-probe: build with `--features lambda` (or `cargo lambda build -p \
         coverage-sweep-probe --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint."
    );
}
