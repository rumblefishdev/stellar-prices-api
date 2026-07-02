//! Backfill freshness probe Lambda entrypoint (task 0056).
//!
//! EventBridge `rate(15 minutes)` → this binary. Each run reads the push age of
//! every `prices.backfill_progress` stream over the 0052 mTLS ClickHouse client
//! and republishes it as the `Prices/Backfill` `PushAgeSeconds` CloudWatch
//! metric that the SDEX push-freshness alarm watches.
//!
//!     cargo lambda build -p backfill-freshness-probe --release --arm64 --features lambda
//!
//! Requires the `lambda` feature (the default build/test exercises the pure
//! metric-shaping in `lib.rs` without the AWS runtime / mTLS stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use backfill_freshness_probe::{AGE_QUERY, StreamAge, age_metrics, publish};
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use std::sync::Arc;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    // Cold start: build the CH + CloudWatch clients once and probe CH
    // connectivity, so a bad secret/endpoint surfaces as a Lambda Init error
    // rather than a per-event failure. The `ingestion` mTLS identity
    // (prices_writer) has SELECT on prices.* — sufficient for the read.
    let ch = Arc::new(prices_clickhouse::mtls::client_from_lambda_env("prices").await?);
    ch.query("SELECT 1").execute().await?;

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(environment = %environment, "backfill-freshness-probe cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let ch = ch.clone();
        let cw = cw.clone();
        let environment = environment.clone();
        async move {
            let rows = ch.query(AGE_QUERY).fetch_all::<StreamAge>().await?;
            let metrics = age_metrics(&rows);

            // Best-effort publish: a transient CloudWatch error must not fail
            // the invocation (that would trip the worker's own error alarm and
            // muddy the signal); the next 15-min run re-publishes.
            if let Err(err) = publish(&cw, &environment, &metrics).await {
                tracing::warn!(error = %err, "PutMetricData failed; skipping this cycle");
            }

            let published: Vec<_> = metrics
                .iter()
                .map(|m| serde_json::json!({ "stream": m.stream, "age_seconds": m.value }))
                .collect();
            tracing::info!(
                streams = metrics.len(),
                "backfill-freshness-probe run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "published": published,
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "backfill-freshness-probe: build with `--features lambda` (or `cargo lambda build -p \
         backfill-freshness-probe --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint."
    );
}
