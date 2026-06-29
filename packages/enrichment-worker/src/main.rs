//! Enrichment worker Lambda entrypoint (task 0026).
//!
//! EventBridge cron → this binary. Each run enriches a bounded batch of
//! `prices.price_ohlcv_1m` candidates with `close_usd` / `volume_quote_usd`
//! (oracle → stablecoin-peg → XLM-pivot tiers) over mTLS to ClickHouse, then
//! re-inserts the higher-`version` rows the ReplacingMergeTree collapses on
//! merge.
//!
//!     cargo lambda build -p enrichment-worker --release --arm64 --features lambda
//!
//! Requires the `lambda` feature (default build/test exercises `ch_enrich` and
//! the prototype `enrichment-cli` without the AWS runtime / mTLS stack).
//!
//! Cold-start eager init mirrors the sibling workers: the mTLS client
//! (`MTLS_SECRET_NAME` + `CH_DOMAIN`) is built and probed once, so a missing
//! secret / unreachable endpoint surfaces as a Lambda Init Error rather than a
//! per-event panic. Config is env-driven; unset vars fall back to the
//! `ChEnrichConfig` defaults (reflector / 300s / 86400s / 10000 / 20).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use enrichment_worker::ch_enrich::{ChEnrichConfig, ChEnrichmentPass};
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use std::sync::Arc;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let cfg = ChEnrichConfig {
        // Unused on the mTLS path — the client carries the URL/TLS.
        url: String::new(),
        database: env_or("CLICKHOUSE_DATABASE", "prices"),
        table: env_or("CLICKHOUSE_TABLE", "price_ohlcv_1m"),
        oracle_name: env_or("ORACLE_NAME", "reflector"),
        window_s: env_parse_or("FORWARD_FILL_WINDOW_S", 300),
        pivot_window_s: env_parse_or("PIVOT_WINDOW_S", 86_400),
        batch_size: env_parse_or("BATCH_SIZE", 10_000),
        max_batches: env_parse_or("MAX_BATCHES", 20),
    };

    tracing::info!(
        database = %cfg.database,
        table = %cfg.table,
        oracle_name = %cfg.oracle_name,
        window_s = cfg.window_s,
        pivot_window_s = cfg.pivot_window_s,
        batch_size = cfg.batch_size,
        max_batches = cfg.max_batches,
        "enrichment-worker cold start"
    );

    // Cold start: build the mTLS client (MTLS_SECRET_NAME + CH_DOMAIN) and probe
    // connectivity. Failures here surface as a CloudWatch Init error.
    let client = prices_clickhouse::mtls::client_from_lambda_env(&cfg.database).await?;
    let pass = Arc::new(ChEnrichmentPass::with_client(client, cfg));
    pass.preflight().await?;
    tracing::info!("enrichment-worker cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let pass = pass.clone();
        async move {
            let stats = pass.run().await?;
            tracing::info!(
                batches = stats.batches,
                candidates_before = stats.candidates_before,
                candidates_after = stats.candidates_after,
                rows_enriched = stats.rows_enriched,
                "enrichment pass complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "batches": stats.batches,
                "candidates_before": stats.candidates_before,
                "candidates_after": stats.candidates_after,
                "rows_enriched": stats.rows_enriched,
            }))
        }
    }))
    .await
}

#[cfg(feature = "lambda")]
fn env_or(var: &str, default: &str) -> String {
    std::env::var(var).unwrap_or_else(|_| default.to_string())
}

#[cfg(feature = "lambda")]
fn env_parse_or<T: std::str::FromStr>(var: &str, default: T) -> T {
    std::env::var(var)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "enrichment-worker: build with `--features lambda` (or `cargo lambda build -p \
         enrichment-worker --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint. The local prototype driver is the `enrichment-cli` binary."
    );
}
