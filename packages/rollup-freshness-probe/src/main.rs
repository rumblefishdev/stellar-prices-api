//! Rollup freshness probe Lambda entrypoint (task 0137).
//!
//! EventBridge `rate(15 minutes)` → this binary. Each run reads the rollup lag of
//! every OHLCV granularity over the 0052 mTLS ClickHouse client and republishes
//! it as the `Prices/Rollup` `RollupLagSeconds` CloudWatch metric that the
//! per-tier rollup freshness alarms watch.
//!
//!     cargo lambda build -p rollup-freshness-probe --release --arm64 --features lambda
//!
//! Requires the `lambda` feature (the default build/test exercises the pure
//! metric-shaping and query construction in `lib.rs` without the AWS runtime /
//! mTLS stack).

#[cfg(feature = "lambda")]
#[tokio::main]
async fn main() -> Result<(), lambda_runtime::Error> {
    use lambda_runtime::{LambdaEvent, run, service_fn};
    use rollup_freshness_probe::disk::{DiskUsage, disk_metrics, disk_query, publish_disk};
    use rollup_freshness_probe::mv_drift::{
        MV_DRIFT_CRITICAL_METRIC, MV_DRIFT_METRIC, describe, drift_metrics, publish_drift,
        visible_objects_query,
    };
    use rollup_freshness_probe::usd_sanity::{
        SanityCounts, publish_sanity, sanity_metrics, sanity_query,
    };
    use rollup_freshness_probe::{TableLag, freshness_query, lag_metrics, publish};
    use std::sync::Arc;

    prices_clickhouse::observability::init_tracing();

    // Cold start: build the CH + CloudWatch clients and the query once. A bad
    // secret/endpoint surfaces on the first query (the invocation fails and the
    // probe's error alarm fires) — no separate `SELECT 1` liveness probe is
    // needed. The `ingestion` mTLS identity (prices_writer) has SELECT on
    // prices.*, which is all this probe needs.
    //
    // ⚠️ This comment used to claim the probe touches no `system.*` table. Since
    // task 0204 gap 3 it reads `system.tables`, and that is fine — `system.tables`
    // is grant-FILTERED (a prices-only user sees the prices objects, measured at
    // 32 on 26.3.10.60), not DENIED like `system.disks`, which is why gap 1 reads
    // filesystem *functions* instead. No new grant is required for either.
    let ch = Arc::new(prices_clickhouse::mtls::client_from_lambda_env("prices").await?);
    let query = Arc::new(freshness_query());
    let sanity_query = Arc::new(sanity_query());

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(environment = %environment, "rollup-freshness-probe cold start ready");

    run(service_fn(move |_event: LambdaEvent<serde_json::Value>| {
        let ch = ch.clone();
        let cw = cw.clone();
        let query = query.clone();
        let sanity_query = sanity_query.clone();
        let environment = environment.clone();
        async move {
            // ⚠️ EVERY CHECK RUNS, AND A FAILURE IN ONE MUST NOT SUPPRESS
            // ANOTHER. This is the invariant the whole invocation is built
            // around, and it is NOT the same as ordering the checks carefully —
            // that was the earlier design and it was wrong.
            //
            // The four checks used to `?` on failure, so each aborted every
            // check below it. Their comments each claimed to run "last", which
            // could not be true of more than one of them, and the consequence
            // was concrete: an unresolvable USDT identity (a documented,
            // plausible state — see `SanityRefusal`) failed the invocation
            // before the MV-drift read ran at all. Every alarm in this crate is
            // `treatMissingData: NOT_BREACHING`, so a missing datum reads as
            // healthy — meaning a materialized view that had lost APPEND and
            // was destroying history on every refresh would have shown OK in
            // Slack for as long as the USDT lookup stayed broken. One
            // correctness check silently disabling another is precisely the
            // false-OK failure task 0204 exists to end.
            //
            // So: each check records its own failure and the next one still
            // runs. The invocation fails at the END if any of them did, which
            // still trips the probe's own `-errors` alarm — the intended
            // dead-probe signal — but only after everything that COULD publish
            // has published.
            let mut failures: Vec<String> = Vec::new();

            // ---- 1. Rollup freshness (task 0137) --------------------------
            let mut published: Vec<serde_json::Value> = Vec::new();
            let mut tiers = 0usize;
            match ch.query(&query).fetch_all::<TableLag>().await {
                Ok(rows) => {
                    let metrics = lag_metrics(&rows);
                    tiers = metrics.len();
                    published = metrics
                        .iter()
                        .map(|m| serde_json::json!({ "table": m.table, "lag_seconds": m.value }))
                        .collect();
                    if let Err(e) = publish(&cw, &environment, &metrics).await {
                        failures.push(format!("rollup publish: {e}"));
                    }
                }
                Err(e) => failures.push(format!("rollup read: {e}")),
            }

            // ---- 2. ClickHouse disk headroom (task 0204, gap 1) -----------
            let mut disk_reading: Option<DiskUsage> = None;
            let mut free_percent: Option<f64> = None;
            match ch.query(disk_query()).fetch_one::<DiskUsage>().await {
                Ok(usage) => {
                    disk_reading = Some(usage);
                    // `None` means capacity read as zero — a broken reading, not
                    // a full disk. Publishing 0.0 would page falsely; publishing
                    // nothing would let NOT_BREACHING score an unreadable disk as
                    // healthy. Record it as a failure instead.
                    match disk_metrics(&usage) {
                        Some(disk) => {
                            free_percent = disk.first().map(|m| m.value);
                            if let Err(e) = publish_disk(&cw, &environment, &disk).await {
                                failures.push(format!("disk publish: {e}"));
                            }
                        }
                        None => failures.push(format!(
                            "disk: ClickHouse reported filesystemCapacity() = 0 (available {} B) \
                             — disk headroom is unreadable, not zero",
                            usage.available_bytes
                        )),
                    }
                }
                Err(e) => failures.push(format!("disk read: {e}")),
            }

            // ---- 3. USD-value correctness (task 0204, gap 4) --------------
            let mut sanity_counts: Option<SanityCounts> = None;
            match ch.query(&sanity_query).fetch_one::<SanityCounts>().await {
                Ok(counts) => {
                    sanity_counts = Some(counts);
                    // An `Err` here is a check that did NOT RUN — an
                    // unresolvable USDT identity, or one that resolved to an id
                    // the candles no longer carry. Either way its two zeros must
                    // not be published, because NOT_BREACHING would score them
                    // as a clean bill of health.
                    match sanity_metrics(&counts) {
                        Ok(sanity) => {
                            if let Err(e) = publish_sanity(&cw, &environment, &sanity).await {
                                failures.push(format!("usd-sanity publish: {e}"));
                            }
                        }
                        Err(refusal) => failures.push(format!("usd-sanity: {refusal}")),
                    }
                }
                Err(e) => failures.push(format!("usd-sanity read: {e}")),
            }

            // ---- 4. Materialized-view drift (task 0204, gap 3) ------------
            //
            // This is the only read in the invocation that touches `system.*`.
            // `system.tables` is grant-FILTERED rather than denied, so it needs
            // no grant the probe does not already hold — but a narrowed grant or
            // a metadata hiccup here still must not cost the three checks above,
            // which is what the failure collection above guarantees.
            let mut drift_critical = 0.0_f64;
            let mut drift_count = 0.0_f64;
            let mut visible_objects: Option<u64> = None;
            let mut drift_detail = String::new();
            match ch
                .query(&visible_objects_query("prices"))
                .fetch_one::<u64>()
                .await
            {
                Ok(visible) => {
                    visible_objects = Some(visible);
                    match prices_clickhouse::drift::check_rollup_drift(&ch, "prices").await {
                        Ok(reports) => {
                            let drift = drift_metrics(&reports, visible);
                            let value_of = |name: &str| {
                                drift
                                    .iter()
                                    .find(|m| m.name == name)
                                    .map(|m| m.value)
                                    .unwrap_or_default()
                            };
                            drift_critical = value_of(MV_DRIFT_CRITICAL_METRIC);
                            drift_count = value_of(MV_DRIFT_METRIC);
                            // Named per-MV, so an alarm can be diagnosed from the
                            // log line without re-running the drift CLI by hand.
                            drift_detail = describe(&reports);
                            if let Err(e) = publish_drift(&cw, &environment, &drift).await {
                                failures.push(format!("mv-drift publish: {e}"));
                            }
                        }
                        Err(e) => failures.push(format!("mv-drift check: {e}")),
                    }
                }
                Err(e) => failures.push(format!("mv-drift visibility read: {e}")),
            }

            // Log before deciding the invocation's fate: on a partial failure
            // this line is the only record of what the healthy checks measured.
            tracing::info!(
                tiers,
                checks_failed = failures.len(),
                disk_free_percent = free_percent.unwrap_or_default(),
                disk_available_bytes = disk_reading.map(|u| u.available_bytes).unwrap_or_default(),
                usd_peg_applied = sanity_counts.map(|c| c.peg_applied).unwrap_or_default(),
                usd_stranded = sanity_counts.map(|c| c.stranded).unwrap_or_default(),
                usd_scanned = sanity_counts.map(|c| c.scanned).unwrap_or_default(),
                mv_drift_critical = drift_critical,
                mv_drift = drift_count,
                mv_visible_objects = visible_objects.unwrap_or_default(),
                mv_detail = %drift_detail,
                "rollup-freshness-probe run complete"
            );

            if !failures.is_empty() {
                return Err(lambda_runtime::Error::from(format!(
                    "{} of 4 probe checks failed (the rest published normally): {}",
                    failures.len(),
                    failures.join("; ")
                )));
            }

            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "published": published,
                "disk": {
                    "free_percent": free_percent,
                    "available_bytes": disk_reading.map(|u| u.available_bytes),
                    "capacity_bytes": disk_reading.map(|u| u.capacity_bytes),
                },
                "usd_sanity": {
                    "peg_applied": sanity_counts.map(|c| c.peg_applied),
                    "stranded": sanity_counts.map(|c| c.stranded),
                    "scanned": sanity_counts.map(|c| c.scanned),
                },
                "mv_drift": {
                    "critical": drift_critical,
                    "drifted": drift_count,
                    "visible_objects": visible_objects,
                    "detail": drift_detail,
                },
            }))
        }
    }))
    .await
}

#[cfg(not(feature = "lambda"))]
fn main() {
    eprintln!(
        "rollup-freshness-probe: build with `--features lambda` (or `cargo lambda build -p \
         rollup-freshness-probe --release --arm64 --features lambda`) for the AWS Lambda \
         entrypoint."
    );
}
