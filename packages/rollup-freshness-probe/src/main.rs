//! Rollup freshness probe Lambda entrypoint (task 0137).
//!
//! EventBridge `rate(15 minutes)` → this binary. Each run reads the rollup lag of
//! every OHLCV granularity over the 0052 mTLS ClickHouse client and republishes
//! it as the `Prices/Rollup` `RollupLagSeconds` CloudWatch metric that the
//! per-tier rollup freshness alarms watch.
//!
//!     cargo lambda build -p rollup-freshness-probe --release --arm64 --features lambda
//!
//! Requires the `lambda` feature (the default build/test exercises the pure
//! metric-shaping and query construction in `lib.rs` without the AWS runtime /
//! mTLS stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use rollup_freshness_probe::{TableLag, freshness_query, lag_metrics, publish};
    use std::sync::Arc;

    prices_clickhouse::observability::init_tracing();

    // Cold start: build the CH + CloudWatch clients and the query once. A bad
    // secret/endpoint surfaces on the first query (the invocation fails and the
    // probe's error alarm fires) — no separate `SELECT 1` liveness probe is
    // needed. The `ingestion` mTLS identity (prices_writer) has SELECT on
    // prices.*, which is all this probe needs: it reads the OHLCV tables
    // directly and touches no `system.*` table.
    let ch = Arc::new(prices_clickhouse::mtls::client_from_lambda_env("prices").await?);
    let query = Arc::new(freshness_query());

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(environment = %environment, "rollup-freshness-probe cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let ch = ch.clone();
        let cw = cw.clone();
        let query = query.clone();
        let environment = environment.clone();
        async move {
            let rows = ch.query(&query).fetch_all::<TableLag>().await?;
            let metrics = lag_metrics(&rows);

            // Propagate a publish failure so the invocation errors (mirrors the
            // backfill-freshness-probe). The rollup alarms are treatMissingData:
            // NOT_BREACHING, so if PutMetricData fails, no RollupLagSeconds datum
            // lands and the alarms silently stay OK — a frozen rollup would go
            // undetected, which is precisely the 0136 failure this task exists to
            // end. Failing the invocation instead trips the probe's own `-errors`
            // alarm, which is the intended dead-probe signal. A transient blip
            // self-heals on the next 15-min run.
            publish(&cw, &environment, &metrics).await?;

            let published: Vec<_> = metrics
                .iter()
                .map(|m| serde_json::json!({ "table": m.table, "lag_seconds": m.value }))
                .collect();
            tracing::info!(tiers = metrics.len(), "rollup-freshness-probe run complete");
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
        "rollup-freshness-probe: build with `--features lambda` (or `cargo lambda build -p \
         rollup-freshness-probe --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint."
    );
}
