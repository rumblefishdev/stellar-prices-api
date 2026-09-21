//! Asset Discovery Lambda entrypoint (task 0054).
//!
//! EventBridge `rate(1 hour)` → this binary. Each invocation:
//!   1. resolves `symbol()` for Soroban contracts that have no
//!      `prices.asset_symbol` row yet (`symbols::run_symbols`, task 0210),
//!   2. ensures the seed assets exist (`ensure_seed`).
//!
//! New assets and AMM pools are registered by the live ledger processor as it
//! ingests; the hourly ledger scan this worker was designed with never ran in
//! production and was removed (task 0256).
//!
//! Build the deployable with:
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

    // Cold start: build the mTLS client (MTLS_SECRET_NAME + CH_DOMAIN), probe
    // connectivity, and parse the seed once. Failures here surface as a
    // CloudWatch Init error, not a per-invocation error.
    let client = prices_clickhouse::mtls::client_from_lambda_env("prices").await?;
    let writer = Arc::new(prices_ingest_core::OhlcvWriter::new(client));
    writer.preflight().await?;

    // Task 0210 symbol stage. `reqwest::Client` is internally an Arc, so the
    // per-invocation clone is cheap and the connection pool is shared.
    let http = asset_discovery::symbols::http_client();
    let rpc_url = std::env::var("SOROBAN_RPC_URL")
        .unwrap_or_else(|_| asset_discovery::symbols::DEFAULT_SOROBAN_RPC.to_string());

    let seed = Arc::new(asset_discovery::seed_identities()?);

    tracing::info!(
        seed = seed.len(),
        rpc_url,
        "asset-discovery cold start ready"
    );

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let writer = writer.clone();
        let seed = seed.clone();
        let http = http.clone();
        let rpc_url = rpc_url.clone();
        async move {
            // 1. Resolve Soroban token symbols (task 0210).
            //
            // First, and NOT behind the seed's `?`. Bounded at
            // `MAX_CONTRACTS_PER_RUN × RPC_TIMEOUT_SECS`. Task 0218's lesson is
            // that a stage sitting behind another stage's `?` is skippable and
            // starvable and cannot be watched, so both stages run and both
            // report — a symbol failure is logged and surfaced in the response
            // rather than aborting the seed.
            let symbols = asset_discovery::symbols::run_symbols(
                writer.client(),
                &http,
                &rpc_url,
                asset_discovery::symbols::MAX_CONTRACTS_PER_RUN,
            )
            .await;
            if let Err(err) = &symbols {
                tracing::error!(error = %err, "symbol stage failed");
            }

            // 2. Seed (idempotent).
            let seeded = asset_discovery::ensure_seed(&writer, &seed).await?;

            tracing::info!(
                symbols_considered = symbols.as_ref().map(|s| s.considered).unwrap_or(0),
                symbols_resolved = symbols.as_ref().map(|s| s.resolved).unwrap_or(0),
                symbols_absent = symbols.as_ref().map(|s| s.absent).unwrap_or(0),
                symbols_skipped = symbols.as_ref().map(|s| s.skipped).unwrap_or(0),
                // Total rows in the asset registry, not rows written — a
                // steady-state run writes none.
                assets_total = seeded,
                "asset-discovery run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                // `null` when the symbol stage failed — its error is logged
                // above and does not abort the run (task 0218).
                "symbols": symbols.ok(),
                "assets_total": seeded,
            }))
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
