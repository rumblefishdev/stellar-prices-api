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
            let rows = ch.query(&query).fetch_all::<TableLag>().await?;
            let metrics = lag_metrics(&rows);

            // Propagate a publish failure so the invocation errors (mirrors the
            // backfill-freshness-probe). The rollup alarms are treatMissingData:
            // NOT_BREACHING, so if PutMetricData fails, no RollupLagSeconds datum
            // lands and the alarms silently stay OK — a frozen rollup would go
            // undetected, which is precisely the 0136 failure this task exists to
            // end. Failing the invocation instead trips the probe's own `-errors`
            // alarm, which is the intended dead-probe signal. A transient blip
            // self-heals on the next 15-min run.
            publish(&cw, &environment, &metrics).await?;

            let published: Vec<_> = metrics
                .iter()
                .map(|m| serde_json::json!({ "table": m.table, "lag_seconds": m.value }))
                .collect();

            // ⚠️ ORDER IS LOAD-BEARING: the disk read runs AFTER the rollup
            // metrics are already published, and must never move above it.
            // Both halves propagate their errors, so whichever runs second can
            // only ever cost itself. Reading the disk first would mean a disk
            // failure (a revoked grant, a CH hiccup) aborts the invocation
            // before any RollupLagSeconds datum lands — the alarms are
            // treatMissingData: NOT_BREACHING, so all seven would score healthy
            // while a rollup sat frozen. That is the exact 0136 blind spot task
            // 0137 was filed to close, and it would have been reintroduced by an
            // unrelated feature.
            let usage = ch.query(disk_query()).fetch_one::<DiskUsage>().await?;
            // `None` means capacity read as zero — a broken reading, not a full
            // disk. Fail the invocation so the probe's own `-errors` alarm
            // carries it; publishing nothing would let NOT_BREACHING score an
            // unreadable disk as healthy.
            let disk = disk_metrics(&usage).ok_or_else(|| {
                lambda_runtime::Error::from(format!(
                    "ClickHouse reported filesystemCapacity() = 0 (available {} B) — disk \
                     headroom is unreadable, not zero",
                    usage.available_bytes
                ))
            })?;
            publish_disk(&cw, &environment, &disk).await?;

            // ⚠️ SAME ORDERING RULE AS THE DISK READ, one step further down:
            // the USD-sanity read runs LAST and must never move above either of
            // the publishes above it. It is the newest and least proven of the
            // three, it scans OHLCV data rather than reading a function, and it
            // has the most ways to fail (an unresolvable asset identity, a
            // registry change, a slow FINAL scan). Every one of those would
            // otherwise abort the invocation before the rollup and disk data
            // landed — and those alarms are treatMissingData: NOT_BREACHING, so
            // they would all score healthy. A correctness check must not be able
            // to blind the liveness checks it sits beside.
            let counts = ch.query(&sanity_query).fetch_one::<SanityCounts>().await?;
            // `None` means the USDT identity did not resolve to exactly one
            // asset — the check did not run. Fail the invocation rather than
            // publishing two zeros, which NOT_BREACHING would score as a clean
            // bill of health for a query that matched nothing.
            let sanity = sanity_metrics(&counts).ok_or_else(|| {
                lambda_runtime::Error::from(format!(
                    "the canonical USDT identity resolved to {} assets, expected exactly 1 — \
                     the USD-sanity check did not run and its counts are meaningless",
                    counts.resolved_legs
                ))
            })?;
            publish_sanity(&cw, &environment, &sanity).await?;

            // ⚠️ LAST, by the same rule as the two reads above: every check runs
            // after the previous one has already published, so a failure can
            // only ever cost itself. This one goes last because it is the only
            // read in the invocation that touches `system.*` — a narrowed grant
            // or a metadata hiccup here must not be able to abort the rollup,
            // disk and USD publishes, all of which sit behind NOT_BREACHING
            // alarms that would score healthy on missing data.
            let visible: u64 = ch
                .query(&visible_objects_query("prices"))
                .fetch_one::<u64>()
                .await?;
            let reports = prices_clickhouse::drift::check_rollup_drift(&ch, "prices").await?;
            let drift = drift_metrics(&reports, visible);
            publish_drift(&cw, &environment, &drift).await?;

            let value_of = |name: &str| {
                drift
                    .iter()
                    .find(|m| m.name == name)
                    .map(|m| m.value)
                    .unwrap_or_default()
            };
            let free_percent = disk.first().map(|m| m.value).unwrap_or_default();
            tracing::info!(
                tiers = metrics.len(),
                disk_free_percent = free_percent,
                disk_available_bytes = usage.available_bytes,
                usd_peg_applied = counts.peg_applied,
                usd_stranded = counts.stranded,
                usd_scanned = counts.scanned,
                mv_drift_critical = value_of(MV_DRIFT_CRITICAL_METRIC),
                mv_drift = value_of(MV_DRIFT_METRIC),
                mv_visible_objects = visible,
                // Named per-MV, so an alarm can be diagnosed from the log line
                // without re-running the drift CLI by hand.
                mv_detail = %describe(&reports),
                "rollup-freshness-probe run complete"
            );
            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "published": published,
                "disk": {
                    "free_percent": free_percent,
                    "available_bytes": usage.available_bytes,
                    "capacity_bytes": usage.capacity_bytes,
                },
                "usd_sanity": {
                    "peg_applied": counts.peg_applied,
                    "stranded": counts.stranded,
                    "scanned": counts.scanned,
                },
                "mv_drift": {
                    "critical": value_of(MV_DRIFT_CRITICAL_METRIC),
                    "drifted": value_of(MV_DRIFT_METRIC),
                    "visible_objects": visible,
                    "detail": describe(&reports),
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
