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
    tracing::info!(%rpc_url, %contract, "oracle-worker cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let writer = writer.clone();
        let http = http.clone();
        let rpc_url = rpc_url.clone();
        let contract = contract.clone();
        async move {
            let stats = oracle_worker::run_oracle(&writer, &http, &rpc_url, &contract).await?;
            tracing::info!(
                queried = stats.queried,
                written = stats.written,
                skipped = stats.skipped,
                rates_snapshotted = stats.rates_snapshotted,
                "oracle-worker run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "queried": stats.queried,
                "written": stats.written,
                "skipped": stats.skipped,
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
