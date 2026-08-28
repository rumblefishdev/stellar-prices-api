//! Oracle worker Lambda entrypoint (task 0039).
//!
//! EventBridge `rate(5 minutes)` → this binary. Each run polls Reflector via
//! Soroban RPC `simulateTransaction` and writes `prices.oracle_prices`.
//!
//!     cargo lambda build -p oracle-worker --release --arm64
//!
//! Requires the `lambda` feature (default build/test exercises the SEP-40
//! encode/parse + RPC paths without the AWS runtime / mTLS stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use std::sync::Arc;
    use std::time::Duration;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let ch = prices_clickhouse::mtls::client_from_lambda_env("prices").await?;
    let writer = Arc::new(prices_ingest_core::OhlcvWriter::new(ch));
    writer.preflight().await?;
    let http = Arc::new(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?,
    );
    let rpc_url =
        prices_clickhouse::env::env_or("SOROBAN_RPC_URL", oracle_worker::DEFAULT_SOROBAN_RPC);
    let contract =
        prices_clickhouse::env::env_or("REFLECTOR_CONTRACT", oracle_worker::REFLECTOR_CEX_DEX);

    // CloudWatch client for the task 0231 metrics. Built once at cold start;
    // publish is best-effort per invocation, so a CloudWatch failure never fails
    // the poll — oracle_prices is the deliverable, the metric is the witness.
    //
    // `ENV_NAME` becomes the `Environment` dimension and is therefore part of
    // the metric's identity: if it disagrees with what `infra/` declares, the
    // alarms watch a series that does not exist and return 0 datapoints rather
    // than an error (task 0204). Logged at cold start so that mismatch is
    // visible without guessing.
    let env_name = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    tracing::info!(
        %rpc_url,
        %contract,
        env_name = %env_name,
        metric_namespace = oracle_worker::metrics::METRIC_NAMESPACE,
        "oracle-worker cold start ready"
    );

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let writer = writer.clone();
        let http = http.clone();
        let rpc_url = rpc_url.clone();
        let contract = contract.clone();
        let cw = cw.clone();
        let env_name = env_name.clone();
        async move {
            // Deliberately NOT `?` on the pass: a run that fails before
            // producing stats must still publish `OracleRuns=1` +
            // `OracleFailedRuns=1`, or a failed run is indistinguishable from an
            // invocation that never happened — both emit nothing (task 0218's
            // lesson, applied here before it could cost a second incident).
            let outcome = oracle_worker::run_oracle(&writer, &http, &rpc_url, &contract).await;

            let metrics = match &outcome {
                Ok(stats) => oracle_worker::metrics::pass_metrics(stats),
                // A failed pass still reports the rejections it had already
                // counted: they happen in the per-symbol loop, ahead of the
                // ClickHouse writes that are the likely reason it died.
                Err(failure) => oracle_worker::metrics::failure_metrics(failure.timestamp_rejected),
            };
            if let Err(e) = oracle_worker::metrics::publish(&cw, &env_name, &metrics).await {
                tracing::warn!(error = %e, "cloudwatch metric publish failed (non-fatal)");
            }

            let stats = outcome?;
            tracing::info!(
                queried = stats.queried,
                written = stats.written,
                skipped = stats.skipped,
                timestamp_rejected = stats.timestamp_rejected,
                rates_snapshotted = stats.rates_snapshotted,
                "oracle-worker run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "queried": stats.queried,
                "written": stats.written,
                "skipped": stats.skipped,
                "timestamp_rejected": stats.timestamp_rejected,
                "rates_snapshotted": stats.rates_snapshotted,
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "oracle-worker: build with `--features lambda` (or `cargo lambda build -p \
         oracle-worker --release --arm64`) for the AWS Lambda entrypoint."
    );
}
