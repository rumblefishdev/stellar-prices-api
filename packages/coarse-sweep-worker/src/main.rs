//! Standalone coarse-table sweep Lambda entrypoint (task 0218).
//!
//!     cargo lambda build -p enrichment-worker --release --arm64 --features lambda
//!
//! ## Why this is its own Lambda
//!
//! The sweep used to run as the last stage of the enrichment worker, after
//! `let stats = pass.run().await?` (`main.rs`). That placement gave it two
//! failure modes it could not escape, and it **never executed in production**
//! under either:
//!
//! 1. **Skipped.** The `?` propagates, so any error in the 1m pass returned
//!    before the sweep was reached. Task 0215 had the pass erroring on *every*
//!    invocation for 26 days; the sweep simply never ran.
//! 2. **Starved.** Once the pass stopped erroring it consumed the whole
//!    invocation, so the sweep's budget — the Lambda deadline minus what the
//!    pass left — saturated at zero.
//!
//! Task 0111 fixed the starvation by bounding the 1m pass, and the sweep began
//! running for the first time. That is the symptom, not the cause: any stage
//! placed after an unbounded stage in one fixed budget is starved by
//! construction, and the next growth in the 1m pass silently starves it again.
//!
//! ⚠️ **The `?` is the deciding reason for a separate function, not the budget.**
//! Reserving the sweep's time up front inside the one Lambda would fix the
//! starvation and leave the skipping intact. Only a separate invocation removes
//! the dependency on the 1m pass *succeeding*.
//!
//! A separate function also makes the absence of a run **observable**: a
//! zero-invocations alarm fires on a function that never ran, which is
//! impossible for a stage buried inside another function's handler — the three
//! states *ran and found nothing*, *ran and failed*, and *never reached* were
//! indistinguishable from outside. That is task 0218's AC 2.
//!
//! ## What it does NOT change
//!
//! The sweep logic itself is [`repair::run_coarse_sweep`], called unchanged, and
//! the table list is built by [`repair::sweep_config_from_env`] — the same
//! function the enrichment worker uses. There is one implementation and one
//! table guard, so the two entrypoints cannot disagree about scope.
//!
//! ⚠️ **`price_ohlcv_1m` is refused** by `is_coarse_table`, so this worker and
//! the enrichment worker write provably disjoint table sets: `_1m` there, the
//! six coarse rollups here. They may safely run concurrently. Note the coarse
//! tables are *already* written every 60 s by `mv_ohlcv_1m_to_15m`, so
//! concurrent writers are the pre-existing steady state, not something this
//! split introduces.

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use enrichment_worker::ch_enrich::{ChEnrichConfig, ChEnrichmentPass};
    use enrichment_worker::repair::sweep_config_from_env;
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use prices_clickhouse::env::{env_or, env_parse_or};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .json()
        .init();

    let env_name = env_or("ENV_NAME", "unknown");

    // The sweep overwrites `table`, `time_window`, `one_shot` and `max_batches`
    // per table/month, so those fields are template values only. Everything else
    // — oracle name, forward-fill and pivot windows, batch size, database — must
    // match the enrichment worker, because both write `close_usd` with the same
    // tiers and a divergence would make the two produce different rows for the
    // same candle.
    let base = ChEnrichConfig {
        // Unused on the mTLS path — the client carries the URL/TLS.
        url: String::new(),
        database: env_or("CLICKHOUSE_DATABASE", "prices"),
        table: env_or("CLICKHOUSE_TABLE", "price_ohlcv_1m"),
        oracle_name: env_or("ORACLE_NAME", "reflector"),
        window_s: env_parse_or("FORWARD_FILL_WINDOW_S", 300),
        pivot_window_s: env_parse_or("PIVOT_WINDOW_S", 86_400),
        recent_window_s: env_parse_or("ENRICH_RECENT_WINDOW_S", 14_400),
        batch_size: env_parse_or("BATCH_SIZE", 10_000),
        max_batches: env_parse_or("COARSE_SWEEP_MAX_BATCHES", 20),
        one_shot: false,
        time_window: None,
        // Never set on a scheduled pass. A reset re-zeroes already-written USD
        // columns so they re-enter the candidate set (task 0182); on a recurring
        // schedule that would re-zero and recompute the same rows forever.
        // Deliberately not env-driven — an operator names the quote leg and
        // epoch explicitly, via the `coarse-repair` binary.
        usd_reset: None,
    };

    // Cold start, mirroring the sibling workers: build and probe the mTLS client
    // once so a missing secret or unreachable endpoint surfaces as a Lambda Init
    // Error rather than a per-event panic.
    let client = prices_clickhouse::mtls::client_from_lambda_env(&base.database).await?;
    ChEnrichmentPass::with_client(client.clone(), base.clone())
        .preflight()
        .await?;

    // Validate the table list ONCE here (see `sweep_config_from_env`). `None`
    // means COARSE_SWEEP_TABLES is empty — the documented off switch. The worker
    // still starts and still reports each invocation, so "configured off" stays
    // distinguishable from "never ran", which is the whole point of task 0218.
    let sweep_cfg = sweep_config_from_env(base);
    let sweep_budget_secs: u64 = env_parse_or("COARSE_SWEEP_TIME_BUDGET_SECS", 120);
    tracing::info!(
        enabled = sweep_cfg.is_some(),
        tables = sweep_cfg.as_ref().map_or(0, |s| s.tables.len()),
        lookback_months = sweep_cfg.as_ref().map_or(0, |s| s.lookback_months),
        max_batches = sweep_cfg.as_ref().map_or(0, |s| s.max_batches),
        time_budget_secs = sweep_budget_secs,
        "coarse sweep config"
    );

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let env_name = Arc::new(env_name);
    let sweep_cfg = Arc::new(sweep_cfg);
    let client = Arc::new(client);
    tracing::info!("coarse-sweep-worker cold start ready");

    run(service_fn(move |event: LambdaEvent<serde_json::Value>| {
        let sweep_cfg = sweep_cfg.clone();
        let client = client.clone();
        let cw = cw.clone();
        let env_name = env_name.clone();
        let lambda_deadline_ms = event.context.deadline;
        async move {
            let Some(cfg) = sweep_cfg.as_ref().as_ref() else {
                // Configured off. Return Ok so this is not counted as an error,
                // but say so explicitly every invocation.
                tracing::warn!(
                    "coarse sweep disabled — COARSE_SWEEP_TABLES is empty; nothing to do"
                );
                return Ok::<serde_json::Value, lambda_runtime::Error>(
                    serde_json::json!({ "enabled": false }),
                );
            };

            // Stop a margin BEFORE the Lambda deadline, so a slow catch-up
            // defers to the next run instead of being hard-killed — a function
            // timeout is an invocation error, not a Rust `Err`, and would escape
            // the best-effort handling below.
            const MARGIN_MS: u64 = 30_000;
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_millis() as u64);
            let remaining_ms = lambda_deadline_ms
                .saturating_sub(now_ms)
                .saturating_sub(MARGIN_MS);
            let budget_ms = sweep_budget_secs.saturating_mul(1_000).min(remaining_ms);
            let started = Instant::now();
            let deadline = started + Duration::from_millis(budget_ms);

            match enrichment_worker::repair::run_coarse_sweep(&client, cfg, Some(deadline)).await {
                Ok(sum) => {
                    tracing::info!(
                        start_month = sum.start_month,
                        end_month = sum.end_month,
                        rows_enriched = sum.total_enriched(),
                        rows_remaining = sum.total_remaining(),
                        tables_swept = sum.tables.len(),
                        tables_failed = sum.failed_tables.len(),
                        tables_skipped = sum.skipped_tables.len(),
                        deadline_hit = sum.deadline_hit,
                        deferred_tables = sum.deferred_tables.len(),
                        duration_ms = started.elapsed().as_millis() as u64,
                        budget_ms,
                        "coarse sweep complete"
                    );
                    // Published on EVERY completed run, including one that swept
                    // nothing and one cut short by its budget — that is what makes
                    // "ran and found nothing" distinguishable from "never reached"
                    // (0218 AC 2) and a starved run visible (AC 4).
                    let sm = enrichment_worker::metrics::sweep_metrics(
                        &sum,
                        started.elapsed().as_millis() as u64,
                    );
                    if let Err(e) = enrichment_worker::metrics::publish(&cw, &env_name, &sm).await {
                        tracing::warn!(error = %e, "coarse sweep metric publish failed (non-fatal)");
                    }
                    Ok(serde_json::to_value(&sum)?)
                }
                Err(e) => {
                    // Fail the invocation. Unlike the old in-handler stage, there
                    // is no 1m pass to protect here, so swallowing the error would
                    // only hide it — and "swept nothing" being indistinguishable
                    // from "failed" is exactly the defect this task exists to fix.
                    tracing::error!(error = %e, budget_ms, "coarse sweep failed");
                    // Publish BEFORE returning Err. A failed run must leave a
                    // datapoint, or it is indistinguishable from a run that never
                    // happened — the Ok-only publishing this replaced was itself
                    // part of the blind spot (0218 AC 2). Best-effort: a metric
                    // failure must not mask the real error below.
                    let fm = enrichment_worker::metrics::sweep_failure_metrics(
                        started.elapsed().as_millis() as u64,
                    );
                    if let Err(pe) = enrichment_worker::metrics::publish(&cw, &env_name, &fm).await {
                        tracing::warn!(error = %pe, "coarse sweep failure-metric publish failed");
                    }
                    Err(lambda_runtime::Error::from(e.to_string()))
                }
            }
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "coarse-sweep-worker: build with `--features lambda` (or `cargo lambda build -p \
         enrichment-worker --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint. The operator-run driver is the `coarse-repair` binary."
    );
}
