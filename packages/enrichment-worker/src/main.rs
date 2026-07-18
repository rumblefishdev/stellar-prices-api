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
//! `ChEnrichConfig` defaults (reflector / 300s / 86400s / 14400s / 10000 / 20).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use enrichment_worker::ch_enrich::{ChEnrichConfig, ChEnrichmentPass};
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use prices_clickhouse::env::{env_or, env_parse_or};
    use std::sync::Arc;

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let env_name = env_or("ENV_NAME", "unknown");
    let cfg = ChEnrichConfig {
        // Unused on the mTLS path — the client carries the URL/TLS.
        url: String::new(),
        database: env_or("CLICKHOUSE_DATABASE", "prices"),
        table: env_or("CLICKHOUSE_TABLE", "price_ohlcv_1m"),
        oracle_name: env_or("ORACLE_NAME", "reflector"),
        window_s: env_parse_or("FORWARD_FILL_WINDOW_S", 300),
        pivot_window_s: env_parse_or("PIVOT_WINDOW_S", 86_400),
        // Recency window for the EnrichmentRowsRemainingRecent metric the stall
        // alarm watches — must be >= the alarm's 3h sustain window so a fresh
        // stuck candle survives all 3 datapoints (task 0026 finding #5 + #1
        // fix). Default 4 hours.
        recent_window_s: env_parse_or("ENRICH_RECENT_WINDOW_S", 14_400),
        batch_size: env_parse_or("BATCH_SIZE", 10_000),
        max_batches: env_parse_or("MAX_BATCHES", 20),
        // ENRICHMENT_ONE_SHOT=true → drain the whole backlog this invocation
        // (spec §4), ignoring max_batches. An explicit flag, not a MAX_BATCHES
        // sentinel, so MAX_BATCHES keeps its literal meaning.
        one_shot: env_parse_or("ENRICHMENT_ONE_SHOT", false),
    };

    tracing::info!(
        database = %cfg.database,
        table = %cfg.table,
        oracle_name = %cfg.oracle_name,
        window_s = cfg.window_s,
        pivot_window_s = cfg.pivot_window_s,
        recent_window_s = cfg.recent_window_s,
        batch_size = cfg.batch_size,
        max_batches = cfg.max_batches,
        one_shot = cfg.one_shot,
        "enrichment-worker cold start"
    );

    // Cold start: build the mTLS client (MTLS_SECRET_NAME + CH_DOMAIN) and probe
    // connectivity. Failures here surface as a CloudWatch Init error.
    let client = prices_clickhouse::mtls::client_from_lambda_env(&cfg.database).await?;
    let pass = Arc::new(ChEnrichmentPass::with_client(client, cfg));
    pass.preflight().await?;

    // CloudWatch client for the spec §5 metrics. Built once at cold start;
    // publish is best-effort per invocation (a metric failure never fails the
    // enrichment pass).
    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let env_name = Arc::new(env_name);
    tracing::info!("enrichment-worker cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let pass = pass.clone();
        let cw = cw.clone();
        let env_name = env_name.clone();
        async move {
            let stats = pass.run().await?;
            tracing::info!(
                batches = stats.batches,
                candidates_before = stats.candidates_before,
                candidates_after = stats.candidates_after,
                rows_enriched = stats.rows_enriched,
                oracle_misses = stats.oracle_misses,
                rows_remaining_at_volume_zero = stats.rows_remaining_at_volume_zero,
                rows_remaining_recent = stats.rows_remaining_recent,
                duration_ms = stats.duration_ms,
                "enrichment pass complete"
            );

            // Publish the enrichment metrics. Best-effort: log and continue.
            let metrics = enrichment_worker::metrics::pass_metrics(&stats);
            if let Err(e) = enrichment_worker::metrics::publish(&cw, &env_name, &metrics).await {
                tracing::warn!(error = %e, "cloudwatch metric publish failed (non-fatal)");
            }

            // `ChPassStats` derives Serialize, so the response mirrors the
            // struct verbatim — adding a stat field never needs a manual edit here.
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::to_value(&stats)?)
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "enrichment-worker: build with `--features lambda` (or `cargo lambda build -p \
         enrichment-worker --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint. The local prototype driver is the `enrichment-cli` binary."
    );
}
