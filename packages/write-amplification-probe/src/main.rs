//! Write-amplification probe Lambda entrypoint (task 0133).
//!
//! EventBridge `rate(1 hour)` → this binary. Each run reads the rows written to
//! every `prices.*` table in the trailing hour from `system.part_log` over the
//! 0052 mTLS ClickHouse client, and republishes the max as the `Prices/Ingest`
//! `MaxRowsWrittenPerHour` CloudWatch metric that the write-amplification alarm
//! watches.
//!
//!     cargo lambda build -p write-amplification-probe --release --arm64 --features lambda
//!
//! Reads as the **`prices_reader`** identity (the `api` mTLS bundle, set on the
//! Function env by infra), which is granted `SELECT ON system.part_log` (task
//! 0133 prerequisite) in addition to its `prices.*` read. Requires the `lambda`
//! feature (the default build/test exercises the pure metric-shaping in `lib.rs`
//! without the AWS runtime / mTLS stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use std::sync::Arc;
    use write_amplification_probe::{TableWrite, WRITE_VOLUME_QUERY, max_rows_written, publish};

    prices_clickhouse::observability::init_tracing();

    // Cold start: build the CH + CloudWatch clients once. A bad secret/endpoint
    // or a missing part_log grant surfaces on the first query (the invocation
    // fails and the probe's own `-errors` alarm fires) — no separate liveness
    // probe needed. Reads as `prices_reader` (the `api` bundle, wired by infra),
    // which has the task-0133 `SELECT ON system.part_log` grant.
    let ch = Arc::new(prices_clickhouse::mtls::client_from_lambda_env("prices").await?);

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(environment = %environment, "write-amplification-probe cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let ch = ch.clone();
        let cw = cw.clone();
        let environment = environment.clone();
        async move {
            let rows = ch
                .query(WRITE_VOLUME_QUERY)
                .fetch_all::<TableWrite>()
                .await?;
            let max_rows = max_rows_written(&rows);

            // Propagate a publish failure so the invocation errors (mirrors the
            // freshness / notafter probes). The alarm treats missing data as
            // NOT_BREACHING, so a swallowed PutMetricData failure would blind it;
            // failing instead trips the probe's own `-errors` alarm — the
            // intended dead-probe signal. A transient blip self-heals next hour.
            publish(&cw, &environment, max_rows).await?;

            // Log the top writers so a breach is immediately diagnosable from the
            // invocation log without a manual part_log query.
            let top: Vec<_> = rows
                .iter()
                .take(5)
                .map(|r| serde_json::json!({ "table": r.table, "rows_written": r.rows_written }))
                .collect();
            tracing::info!(
                max_rows,
                tables = rows.len(),
                top = %serde_json::Value::Array(top),
                "write-amplification-probe run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "max_rows_written": max_rows,
                "tables_written": rows.len(),
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "write-amplification-probe: build with `--features lambda` (or `cargo lambda build -p \
         write-amplification-probe --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint."
    );
}
