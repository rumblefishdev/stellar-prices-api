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
    use enrichment_worker::frontier::{HistoricalSweepConfig, run_historical_sweep};
    use enrichment_worker::live_window::{
        DEFAULT_LIVE_PARTITIONS, live_partition_window, month_of,
    };
    use enrichment_worker::repair::{CoarseSweepConfig, sweep_config_from_env};
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use prices_clickhouse::env::{env_or, env_parse_or};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
        // Template only — the scheduled pass sets this per invocation from
        // `live_partition_window` (below), because a warm container must not
        // carry a stale month across a partition boundary. Left `None` here so
        // the coarse sweep's `base` clone (which sets its own per-month window)
        // is unaffected.
        time_window: None,
        // Never on a scheduled path (task 0182). The reset DISCARDS already-
        // written USD values so a corrected tier can recompute them; it is a
        // deliberate one-off operator action via the coarse-repair CLI, and it
        // is not a fixed point across runs, so an hourly Lambda carrying one
        // would re-zero and recompute the same rows forever. Deliberately not
        // env-driven — there is no ENRICHMENT_USD_RESET.
        usd_reset: None,
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
    // Recurring coarse-table sweep (task 0114). Built from the SAME helper the
    // standalone coarse-sweep Lambda uses (task 0218), so the two entrypoints
    // can never disagree about which tables are in scope. Disabled unless
    // COARSE_SWEEP_TABLES is set.
    let sweep_cfg: Option<CoarseSweepConfig> = sweep_config_from_env(cfg.clone());
    // Per-invocation wall-clock budget for the sweep. It stops after this many
    // seconds — further capped by the Lambda deadline minus a margin below — so a
    // slow catch-up defers to the next run instead of being hard-killed by the
    // function timeout (which is an invocation error, not a Rust Err, and would
    // escape the best-effort handling and fail the invocation).
    let sweep_budget_secs: u64 = env_parse_or("COARSE_SWEEP_TIME_BUDGET_SECS", 120);
    tracing::info!(
        enabled = sweep_cfg.is_some(),
        tables = sweep_cfg.as_ref().map_or(0, |s| s.tables.len()),
        lookback_months = sweep_cfg.as_ref().map_or(0, |s| s.lookback_months),
        max_batches = sweep_cfg.as_ref().map_or(0, |s| s.max_batches),
        time_budget_secs = sweep_budget_secs,
        "coarse sweep config"
    );
    let sweep = Arc::new(sweep_cfg);

    /// Milliseconds of useful work left in this invocation: the Lambda deadline
    /// minus a margin, so a sweep stops cleanly instead of being hard-killed by
    /// the function timeout. A timeout is an invocation error, not a Rust `Err`,
    /// so it would escape the best-effort handling around each sweep and fail
    /// the whole invocation.
    ///
    /// Shared by both sweeps rather than inlined twice — two callers computing
    /// the same deadline arithmetic slightly differently is how one of them ends
    /// up without a margin.
    fn remaining_budget_ms(deadline_ms: u64) -> u64 {
        const MARGIN_MS: u64 = 60_000;
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64);
        deadline_ms.saturating_sub(now_ms).saturating_sub(MARGIN_MS)
    }

    // Task 0111 — bound the scheduled pass to the newest `ENRICH_LIVE_PARTITIONS`
    // monthly partitions. Unbounded, every batch re-scanned all 102 partitions
    // (736 M rows / 18.4 GiB) to serve a live window of 17 M, which is what
    // walked `Duration` up to the 300 s timeout. 0 = unbounded (the pre-0111
    // pass), kept as a config-only escape hatch. Widen it if ingest ever lands
    // rows older than the previous month; do NOT widen it to cover the
    // historical drain — that is the frontier-driven sweep's job, and running it
    // here is exactly the coupling this task removes.
    let live_partitions: u32 = env_parse_or("ENRICH_LIVE_PARTITIONS", DEFAULT_LIVE_PARTITIONS);
    tracing::info!(
        live_partitions,
        bounded = live_partitions > 0,
        "scheduled 1m pass partition bound"
    );

    // Task 0111 phase 2 — the frontier-driven historical drain. Ships INERT
    // (opt-in via ENRICH_HISTORICAL_SWEEP), matching how the 0114 coarse sweep
    // landed: the code deploys, the CDK env turns it on separately.
    //
    // Hard-gated on the live pass being bounded. With live_partitions = 0 the
    // scheduled pass still covers the whole table, so a sweep would contend with
    // it for the same partitions and duplicate its work for nothing.
    let historical_enabled: bool =
        env_parse_or("ENRICH_HISTORICAL_SWEEP", false) && live_partitions > 0;
    // Monthly partitions worked per invocation. 1 by default: a month that still
    // has a backlog stays `pending` and is resumed next run, so throughput comes
    // from the hourly cadence rather than from doing many months at once.
    let historical_max_months: u32 = env_parse_or("ENRICH_HISTORICAL_MAX_MONTHS", 1);
    let historical_budget_secs: u64 = env_parse_or("ENRICH_HISTORICAL_TIME_BUDGET_SECS", 120);
    // How long an `exhausted` mark is trusted before it is re-confirmed against
    // the data. Without this the frontier would be a permanent verdict, and a
    // backfill writing into a finished partition (which is exactly what tasks
    // 0088 and 0201 do) would leave rows unenriched forever while the frontier
    // read clean. Default 7 days.
    let historical_recheck_secs: u32 = env_parse_or("ENRICH_HISTORICAL_RECHECK_SECS", 604_800);
    // Re-checks per invocation, capped separately from `max_months` so drift
    // correction can never starve the drain. At 4/run × 24 runs/day the ~102
    // partitions rotate roughly daily, which is the cadence the plan asked for.
    let historical_max_rechecks: u32 = env_parse_or("ENRICH_HISTORICAL_MAX_RECHECKS", 4);
    tracing::info!(
        enabled = historical_enabled,
        max_months = historical_max_months,
        time_budget_secs = historical_budget_secs,
        recheck_after_secs = historical_recheck_secs,
        max_rechecks = historical_max_rechecks,
        "historical sweep config"
    );

    // Cold start: build the mTLS client (MTLS_SECRET_NAME + CH_DOMAIN) and probe
    // connectivity. Failures here surface as a CloudWatch Init error.
    let client = prices_clickhouse::mtls::client_from_lambda_env(&cfg.database).await?;
    // A cheap clone (Arc-backed handle) the coarse sweep reuses per invocation;
    // the pass takes the original by value.
    let sweep_client = client.clone();
    let pass_client = client.clone();
    // Probe with a throwaway pass; the pass the handler runs is rebuilt per
    // invocation so it picks up the current month (see `live_partitions`).
    ChEnrichmentPass::with_client(client, cfg.clone())
        .preflight()
        .await?;
    let base_cfg = Arc::new(cfg);

    // CloudWatch client for the spec §5 metrics. Built once at cold start;
    // publish is best-effort per invocation (a metric failure never fails the
    // enrichment pass).
    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let env_name = Arc::new(env_name);
    tracing::info!("enrichment-worker cold start ready");

    run(service_fn(move |event: LambdaEvent<serde_json::Value>| {
        let base_cfg = base_cfg.clone();
        let pass_client = pass_client.clone();
        let cw = cw.clone();
        let env_name = env_name.clone();
        let sweep = sweep.clone();
        let sweep_client = sweep_client.clone();
        // The Lambda invocation deadline (epoch ms) — bounds the sweep's budget.
        let lambda_deadline_ms = event.context.deadline;
        async move {
            // Per-invocation, not per-cold-start: a warm container that lives
            // across a month boundary must move its window with the calendar,
            // or the new month's candles fall outside the scan entirely.
            let now_unix = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            let mut pass_cfg = (*base_cfg).clone();
            pass_cfg.time_window = live_partition_window(now_unix, live_partitions);
            tracing::info!(
                window_start = pass_cfg.time_window.map(|(s, _)| s),
                window_end = pass_cfg.time_window.map(|(_, e)| e),
                partitions = live_partitions,
                "1m pass window"
            );
            let pass_window = pass_cfg.time_window;
            let pass = ChEnrichmentPass::with_client(pass_client, pass_cfg);

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

            // Task 0111 phase 2 — historical drain, AFTER the live pass and
            // BEFORE the coarse sweep. Order is deliberate: with the live pass
            // now bounded it costs ~1 min rather than the full 300 s, so both
            // sweeps get real budget for the first time (that starvation is
            // task 0218). The drain goes first because it is this task's
            // deliverable and the coarse sweep is explicitly deferrable —
            // its overflow rolls to the next run by design.
            //
            // Best-effort, exactly like the coarse sweep: a drain failure must
            // never fail the invocation or regress the live pass.
            if historical_enabled {
                let budget_ms = remaining_budget_ms(lambda_deadline_ms)
                    .min(historical_budget_secs.saturating_mul(1_000));
                let hcfg = HistoricalSweepConfig {
                    base: (*base_cfg).clone(),
                    // Months at or above the live window belong to the live
                    // pass; the sweep never touches them, so the two cannot
                    // contend for a partition.
                    live_start_month: pass_window
                        .and_then(|(start, _)| month_of(start as i64))
                        .unwrap_or(u32::MAX),
                    max_months: historical_max_months,
                    deadline: Some(Instant::now() + Duration::from_millis(budget_ms)),
                    recheck_after_secs: historical_recheck_secs,
                    max_rechecks: historical_max_rechecks,
                };
                match run_historical_sweep(&sweep_client, &hcfg).await {
                    Ok(sum) => {
                        tracing::info!(
                            months_swept = sum.months.len(),
                            months_pending = sum.months_pending,
                            frontier_month = sum.frontier_month,
                            rows_enriched = sum.total_enriched(),
                            deadline_hit = sum.deadline_hit,
                            months_rechecked = sum.months_rechecked,
                            months_reopened = sum.months_reopened,
                            "historical sweep complete"
                        );
                        let hm = enrichment_worker::metrics::historical_sweep_metrics(&sum);
                        if let Err(e) =
                            enrichment_worker::metrics::publish(&cw, &env_name, &hm).await
                        {
                            tracing::warn!(error = %e, "historical sweep metric publish failed (non-fatal)");
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "historical sweep failed (non-fatal — live pass unaffected)");
                    }
                }
            }

            // Recurring coarse-table sweep (task 0114), AFTER the critical-path 1m
            // pass and strictly best-effort: any failure is logged and swallowed so
            // a coarse hiccup can never fail the invocation or regress 1m
            // enrichment. Time-bounded (below), so it also cannot blow the timeout.
            if let Some(sweep_cfg) = sweep.as_ref() {
                // Stop time = now + budget, capped so the sweep always finishes a
                // margin BEFORE the Lambda deadline. If the 1m pass already
                // consumed most of the budget, `remaining` is small and the sweep
                // does little/nothing this run, deferring to the next — the point
                // is that it never runs into the hard timeout.
                let budget_ms = sweep_budget_secs
                    .saturating_mul(1_000)
                    .min(remaining_budget_ms(lambda_deadline_ms));
                let sweep_deadline = Instant::now() + Duration::from_millis(budget_ms);

                match enrichment_worker::repair::run_coarse_sweep(
                    &sweep_client,
                    sweep_cfg,
                    Some(sweep_deadline),
                )
                .await
                {
                    Ok(sum) => {
                        tracing::info!(
                            start_month = sum.start_month,
                            end_month = sum.end_month,
                            rows_enriched = sum.total_enriched(),
                            rows_remaining = sum.total_remaining(),
                            tables_swept = sum.tables.len(),
                            tables_failed = sum.failed_tables.len(),
                            tables_skipped = sum.skipped_tables.len(),
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
