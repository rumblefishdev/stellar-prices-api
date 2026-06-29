//! Local one-shot driver for the production ClickHouse enrichment pass
//! (`ChEnrichmentPass::run`) — the lib's `main` only exposes it behind the
//! Lambda runtime. Reads the same env knobs as the worker.
//!
//!   CLICKHOUSE_URL=http://localhost:8123 CLICKHOUSE_DATABASE=prices \
//!   BATCH_SIZE=100000 MAX_BATCHES=30 \
//!   cargo run -p enrichment-worker --example run_ch_pass

use enrichment_worker::ch_enrich::{ChEnrichConfig, ChEnrichmentPass};
use prices_clickhouse::env::{env_or, env_parse_or};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = ChEnrichConfig {
        url: env_or("CLICKHOUSE_URL", "http://localhost:8123"),
        database: env_or("CLICKHOUSE_DATABASE", "prices"),
        table: env_or("CLICKHOUSE_TABLE", "price_ohlcv_1m"),
        oracle_name: env_or("ORACLE_NAME", "reflector"),
        window_s: env_parse_or("FORWARD_FILL_WINDOW_S", 300),
        pivot_window_s: env_parse_or("PIVOT_WINDOW_S", 86_400),
        batch_size: env_parse_or("BATCH_SIZE", 10_000),
        max_batches: env_parse_or("MAX_BATCHES", 20),
        one_shot: env_parse_or("ENRICHMENT_ONE_SHOT", false),
    };

    let pass = ChEnrichmentPass::new(cfg);
    pass.preflight().await?;
    let stats = pass.run().await?;
    println!("{stats:?}");
    Ok(())
}
