//! Local one-shot driver for the production ClickHouse enrichment pass
//! (`ChEnrichmentPass::run`) — the lib's `main` only exposes it behind the
//! Lambda runtime. Reads the same env knobs as the worker.
//!
//!   CLICKHOUSE_URL=http://localhost:8123 CLICKHOUSE_DATABASE=prices \
//!   BATCH_SIZE=100000 MAX_BATCHES=30 \
//!   cargo run -p enrichment-worker --example run_ch_pass

use enrichment_worker::ch_enrich::{ChEnrichConfig, ChEnrichmentPass};

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = ChEnrichConfig {
        url: std::env::var("CLICKHOUSE_URL").unwrap_or_else(|_| "http://localhost:8123".into()),
        database: std::env::var("CLICKHOUSE_DATABASE").unwrap_or_else(|_| "prices".into()),
        table: std::env::var("CLICKHOUSE_TABLE").unwrap_or_else(|_| "price_ohlcv_1m".into()),
        oracle_name: std::env::var("ORACLE_NAME").unwrap_or_else(|_| "reflector".into()),
        window_s: env_or("FORWARD_FILL_WINDOW_S", 300),
        pivot_window_s: env_or("PIVOT_WINDOW_S", 86_400),
        batch_size: env_or("BATCH_SIZE", 10_000),
        max_batches: env_or("MAX_BATCHES", 20),
    };

    let pass = ChEnrichmentPass::new(cfg);
    pass.preflight().await?;
    let stats = pass.run().await?;
    println!("{stats:?}");
    Ok(())
}
