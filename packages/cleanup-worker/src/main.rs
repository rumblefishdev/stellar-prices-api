//! Cleanup worker Lambda entrypoint (task 0039).
//!
//! EventBridge `cron(0 2 * * *)` → this binary. Each run drops monthly
//! partitions past their retention window (§3.6) over mTLS to ClickHouse.
//!
//!     cargo lambda build -p cleanup-worker --release --arm64
//!
//! Requires the `lambda` feature (default build/test exercises `run_cleanup`
//! without the AWS runtime / mTLS stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use std::sync::Arc;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    // Cold start: build the mTLS client (MTLS_SECRET_NAME + CH_DOMAIN) and
    // probe connectivity. Failures here surface as a CloudWatch Init error.
    let client = Arc::new(prices_clickhouse::mtls::client_from_lambda_env("prices").await?);
    client.query("SELECT 1").execute().await?;
    tracing::info!("cleanup-worker cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let client = client.clone();
        async move {
            let stats = cleanup_worker::run_cleanup(&client).await?;
            tracing::info!(dropped = stats.dropped.len(), "cleanup run complete");
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "dropped": stats.dropped,
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "cleanup-worker: build with `--features lambda` (or `cargo lambda build -p \
         cleanup-worker --release --arm64`) for the AWS Lambda entrypoint."
    );
}
