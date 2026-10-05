//! Backfill freshness probe Lambda entrypoint (task 0056).
//!
//! EventBridge `rate(15 minutes)` → this binary. Each run reads the push age of
//! every `prices.backfill_progress` stream over the 0052 mTLS ClickHouse client
//! and republishes it as the `Prices/Backfill` `PushAgeSeconds` CloudWatch
//! metric that the SDEX push-freshness alarm watches.
//!
//! A weekly rule sends `{"check":"reconcile"}` instead (task 0272, see
//! `reconcile.rs`); any other `check` value fails the invocation.
//!
//!     cargo lambda build -p backfill-freshness-probe --release --arm64 --features lambda
//!
//! Requires the `lambda` feature (the default build/test exercises the pure
//! metric-shaping in `lib.rs` without the AWS runtime / mTLS stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use backfill_freshness_probe::{
        AGE_QUERY, Check, METRIC_NAME, StreamAge, age_metrics, check_from_event, publish,
    };
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use std::sync::Arc;

    prices_clickhouse::observability::init_tracing();

    // Cold start: build the CH + CloudWatch clients once. A bad secret/endpoint
    // surfaces on the first `AGE_QUERY` (the invocation fails and the probe's
    // error alarm fires) — no separate `SELECT 1` liveness probe is needed. The
    // `ingestion` mTLS identity (prices_writer) has SELECT on prices.*.
    let ch = Arc::new(prices_clickhouse::mtls::client_from_lambda_env("prices").await?);

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(environment = %environment, "backfill-freshness-probe cold start ready");

    run(service_fn(move |event: LambdaEvent<serde_json::Value>| {
        let ch = ch.clone();
        let cw = cw.clone();
        let environment = environment.clone();
        async move {
            if check_from_event(&event.payload)? == Check::Reconcile {
                return reconcile(&ch, &cw, &environment).await;
            }
            let rows = ch.query(AGE_QUERY).fetch_all::<StreamAge>().await?;
            let metrics = age_metrics(&rows);

            // Propagate a publish failure so the invocation errors (mirrors the
            // mtls-notafter-probe). The freshness alarm is treatMissingData:
            // NOT_BREACHING, so if PutMetricData fails, no PushAgeSeconds datum
            // lands and the alarm silently stays OK — a stalled push would go
            // undetected. Failing the invocation instead trips the probe's own
            // `-errors` alarm, which is the intended dead-probe signal. A
            // transient blip self-heals on the next 15-min run; a persistent
            // failure (bad grant, sustained throttle) is a real fault that must
            // page rather than be swallowed.
            publish(&cw, &environment, METRIC_NAME, &metrics).await?;

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

/// The weekly `{"check":"reconcile"}` run (task 0272).
#[cfg(feature = "lambda")]
async fn reconcile(
    ch: &clickhouse::Client,
    cw: &aws_sdk_cloudwatch::Client,
    environment: &str,
) -> Result<serde_json::Value, lambda_runtime::Error> {
    use backfill_freshness_probe::publish;
    use backfill_freshness_probe::reconcile::{
        OVERCLAIM_METRIC_NAME, RECONCILE_QUERY, StreamOverclaim, missing_streams, overclaim_metrics,
    };

    let rows = ch
        .query(RECONCILE_QUERY)
        .fetch_all::<StreamOverclaim>()
        .await?;
    for stream in missing_streams(&rows) {
        // Under treatMissingData IGNORE the stream's alarm keeps its last state.
        tracing::warn!(stream, "no claim to reconcile; no datum published");
    }
    // As on the push-age path, a publish failure fails the invocation.
    publish(
        cw,
        environment,
        OVERCLAIM_METRIC_NAME,
        &overclaim_metrics(&rows),
    )
    .await?;

    let published: Vec<_> = rows
        .iter()
        .map(|r| serde_json::json!({ "stream": r.task_name, "overclaim_seconds": r.overclaim_seconds }))
        .collect();
    tracing::info!(?published, "backfill reconcile complete");
    Ok(serde_json::json!({ "check": "reconcile", "published": published }))
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "backfill-freshness-probe: build with `--features lambda` (or `cargo lambda build -p \
         backfill-freshness-probe --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint."
    );
}
