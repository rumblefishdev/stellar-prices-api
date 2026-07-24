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
    use enrichment_worker::repair::CoarseSweepConfig;
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
        // The scheduled Lambda always runs the unbounded hourly pass over
        // price_ohlcv_1m; the partition-bounded window is only set by the 0114
        // coarse-repair driver (operator-run), never here.
        time_window: None,
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

    // Recurring coarse-table sweep (task 0114). The rollup MVs re-aggregate only
    // a bounded recent window, so any 1m row enriched *after* that window closes
    // (enrichment lag / stalls) leaves its coarse counterpart frozen at zero
    // forever. Rather than a second Lambda to repair that, this same enrichment
    // worker also re-sweeps the recent coarse partitions each run — one owner of
    // close_usd across 1m AND the rollups. Disabled unless COARSE_SWEEP_TABLES is
    // set, so the code ships inert until the CDK env turns it on. It runs AFTER
    // the 1m pass, bounded (one_shot = false) and best-effort (see the handler).
    let sweep_cfg: Option<CoarseSweepConfig> = {
        let tables: Vec<String> = env_or("COARSE_SWEEP_TABLES", "")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if tables.is_empty() {
            None
        } else {
            Some(CoarseSweepConfig {
                // Shares oracle name / windows / batch size / database with the 1m
                // pass; the sweep overwrites `table` + `max_batches` per table.
                base: cfg.clone(),
                tables,
                lookback_months: env_parse_or("COARSE_SWEEP_LOOKBACK_MONTHS", 2),
                max_batches: env_parse_or("COARSE_SWEEP_MAX_BATCHES", 20),
            })
        }
    };
    tracing::info!(
        enabled = sweep_cfg.is_some(),
        tables = sweep_cfg.as_ref().map_or(0, |s| s.tables.len()),
        lookback_months = sweep_cfg.as_ref().map_or(0, |s| s.lookback_months),
        max_batches = sweep_cfg.as_ref().map_or(0, |s| s.max_batches),
        "coarse sweep config"
    );
    let sweep = Arc::new(sweep_cfg);

    // Cold start: build the mTLS client (MTLS_SECRET_NAME + CH_DOMAIN) and probe
    // connectivity. Failures here surface as a CloudWatch Init error.
    let client = prices_clickhouse::mtls::client_from_lambda_env(&cfg.database).await?;
    // A cheap clone (Arc-backed handle) the coarse sweep reuses per invocation;
    // the pass takes the original by value.
    let sweep_client = client.clone();
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
        let sweep = sweep.clone();
        let sweep_client = sweep_client.clone();
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

            // Recurring coarse-table sweep (task 0114), AFTER the critical-path 1m
            // pass and strictly best-effort: any failure is logged and swallowed so
            // a coarse hiccup can never fail the invocation or regress 1m
            // enrichment. Bounded per run, so it also cannot blow the timeout.
            if let Some(sweep_cfg) = sweep.as_ref() {
                match enrichment_worker::repair::run_coarse_sweep(&sweep_client, sweep_cfg).await {
                    Ok(sum) => {
                        tracing::info!(
                            start_month = sum.start_month,
                            end_month = sum.end_month,
                            rows_enriched = sum.total_enriched(),
                            rows_remaining = sum.total_remaining(),
                            tables_swept = sum.tables.len(),
                            tables_failed = sum.failed_tables.len(),
                            "coarse sweep complete"
                        );
                        let sm = enrichment_worker::metrics::sweep_metrics(&sum);
                        if let Err(e) =
                            enrichment_worker::metrics::publish(&cw, &env_name, &sm).await
                        {
                            tracing::warn!(error = %e, "coarse sweep metric publish failed (non-fatal)");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "coarse sweep failed (non-fatal — 1m pass unaffected)");
                    }
                }
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
