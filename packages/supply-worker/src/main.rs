//! Supply worker Lambda entrypoint (task 0039).
//!
//! EventBridge `rate(1 hour)` → this binary. Each run loads classic assets,
//! fetches their total supply from Horizon (best-effort), and writes
//! `prices.asset_supply` over mTLS.
//!
//!     cargo lambda build -p supply-worker --release --arm64
//!
//! Requires the `lambda` feature (default build/test exercises the parse +
//! ClickHouse paths without the AWS runtime / mTLS stack).

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

    // Cold start: mTLS ClickHouse client + a Horizon HTTP client, built once.
    let ch = Arc::new(prices_clickhouse::mtls::client_from_lambda_env("prices").await?);
    ch.query("SELECT 1").execute().await?;
    let http = Arc::new(
        reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("stellar-prices-supply-worker/0.1")
            .build()?,
    );
    let base_url = prices_clickhouse::env::env_or("HORIZON_URL", supply_worker::DEFAULT_HORIZON);
    let cfg = supply_worker::SupplyRunConfig::from_env();
    tracing::info!(
        %base_url,
        time_budget_secs = cfg.time_budget.as_secs(),
        max_assets = cfg.max_assets,
        "supply-worker cold start ready"
    );

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let ch = ch.clone();
        let http = http.clone();
        let base_url = base_url.clone();
        async move {
            let stats = supply_worker::run_supply(&ch, &http, &base_url, &cfg).await?;
            tracing::info!(
                considered = stats.considered,
                written = stats.written,
                skipped = stats.skipped,
                deferred = stats.deferred,
                deadline_hit = stats.deadline_hit,
                "supply-worker run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "considered": stats.considered,
                "written": stats.written,
                "skipped": stats.skipped,
                "deferred": stats.deferred,
                "deadline_hit": stats.deadline_hit,
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "supply-worker: build with `--features lambda` (or `cargo lambda build -p \
         supply-worker --release --arm64`) for the AWS Lambda entrypoint."
    );
}
