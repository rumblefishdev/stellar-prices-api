//! Asset Discovery Lambda entrypoint (task 0054).
//!
//! EventBridge `rate(1 hour)` → this binary. On each invocation it ensures the
//! seed assets exist in `prices.assets` (Increment 1). Organic ledger-scan
//! discovery is Increment 2. Build the deployable with:
//!
//!     cargo lambda build -p asset-discovery --release --arm64
//!
//! Requires the `lambda` feature (default `cargo build`/`cargo test` exercise
//! the lib + seed logic without the AWS runtime / mTLS stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use std::sync::Arc;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    // Cold start: build the mTLS client (MTLS_SECRET_NAME + CH_DOMAIN env),
    // probe connectivity, and parse the seed once. A failure here surfaces as
    // a CloudWatch Init error rather than a per-invocation error.
    let client = prices_clickhouse::mtls::client_from_lambda_env("prices").await?;
    let writer = Arc::new(prices_ingest_core::OhlcvWriter::new(client));
    writer.preflight().await?;
    let seed = Arc::new(asset_discovery::seed_identities()?);
    tracing::info!(seed = seed.len(), "asset-discovery cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let writer = writer.clone();
        let seed = seed.clone();
        async move {
            let assets = asset_discovery::ensure_seed(&writer, &seed).await?;
            tracing::info!(assets, "asset-discovery run complete");
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({ "assets": assets }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "asset-discovery: build with `--features lambda` (or `cargo lambda build -p \
         asset-discovery --release --arm64`) for the AWS Lambda entrypoint."
    );
}
