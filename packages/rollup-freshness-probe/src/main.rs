//! Rollup freshness probe Lambda entrypoint (task 0137).
//!
//! EventBridge `rate(15 minutes)` → this binary. Each run reads the rollup lag of
//! every OHLCV granularity over the 0052 mTLS ClickHouse client and republishes
//! it as the `Prices/Rollup` `RollupLagSeconds` CloudWatch metric that the
//! per-tier rollup freshness alarms watch. It also reads how long ago
//! `current_prices` was last rewritten (task 0243) and publishes that under the
//! same metric with `Table = current_prices`.
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
    use rollup_freshness_probe::asset_id_uniqueness::{
        AssetIdCounts, OrphanCandleCounts, collisions_metric, collisions_query,
        orphan_candles_metric, orphan_candles_query,
    };
    use rollup_freshness_probe::current_prices::{
        CurrentPricesAge, current_prices_age_query, current_prices_metric,
    };
    use rollup_freshness_probe::disk::{DiskUsage, disk_metrics, disk_query, publish_disk};
    use rollup_freshness_probe::mv_drift::{
        MV_DRIFT_CRITICAL_METRIC, MV_DRIFT_METRIC, describe, drift_metrics, publish_drift,
        visible_objects_query,
    };
    use rollup_freshness_probe::ohlc_band::{
        OHLC_BAND_TIERS, OhlcBandCounts, ohlc_band_detail, ohlc_band_metric, ohlc_band_queries,
    };
    use rollup_freshness_probe::reconcile_mismatch::{
        MISMATCH_MIN_READ_SECS, MISMATCH_RESERVE, MismatchCount, mismatch_metric, mismatch_queries,
        mismatch_read_bound, publish_mismatch,
    };
    use rollup_freshness_probe::refresh_waits::{
        MV_REFRESH_DISABLED_METRIC, MV_REFRESH_FAILING_METRIC, MV_REFRESH_UNREADABLE_METRIC,
        MV_REFRESH_WAITING_METRIC, ViewRefreshRow, describe_failing, failing_detail_for_log,
        metrics_for_read, refresh_waits_query,
    };
    use rollup_freshness_probe::usd_sanity::{
        PegCounts, StrandedCounts, peg_metric, peg_query, publish_sanity, stranded_metric,
        stranded_query,
    };
    use rollup_freshness_probe::zero_invariants::{
        ZeroInvariantCounts, zero_invariant_metric, zero_invariant_query,
    };
    use rollup_freshness_probe::{
        FAILED, SKIPPED_UNREADABLE, TableLag, freshness_query, lag_metrics, publish, reading,
    };
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
    // ⚠️ Two queries since task 0213, not one. The peg direction reads
    // `price_ohlcv_1m` — the tier enrichment writes — while the stranded
    // direction stays on `price_ohlcv_1h`. See `usd_sanity::PEG_TABLE` for why
    // a single shared tier made the peg direction structurally blind.
    let stranded_query = Arc::new(stranded_query());
    let peg_query = Arc::new(peg_query());
    let zero_invariant_query = Arc::new(zero_invariant_query());
    let collisions_query = Arc::new(collisions_query());
    let orphan_candles_query = Arc::new(orphan_candles_query());
    let refresh_waits_query = Arc::new(refresh_waits_query("prices"));
    // The mismatch reads are the only ones here whose cost grows with a week
    // of the child tier (the 15m read scans seven days of `_1m`: 0.7–4.0 s on
    // 26.3.10.60 over ~3 M rows with 8 threads, unmeasured on prod, where the
    // CPU is shared with the BE tenant). Six reads at a 10 s bound would be the
    // whole 60 s Lambda timeout (eventbridge-stack.ts) on their own, after the
    // freshness, disk, USD, drift (~2 round trips per declared MV, 12 now),
    // zero-invariant and asset-id reads. So the per-read bound is NOT fixed:
    // each read gets `min(10 s, time left − 5 s reserve)` from the invocation's
    // deadline, and a read that would get under 2 s is skipped and recorded as
    // a failure — see `reconcile_mismatch` (review WR-04).
    let mismatch_queries = Arc::new(mismatch_queries());
    let ohlc_band_queries = Arc::new(ohlc_band_queries());

    let aws_cfg = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let cw = Arc::new(aws_sdk_cloudwatch::Client::new(&aws_cfg));
    let environment = Arc::new(prices_clickhouse::env::env_or("ENV_NAME", "unknown"));
    tracing::info!(environment = %environment, "rollup-freshness-probe cold start ready");

    run(service_fn(move |event: LambdaEvent<serde_json::Value>| {
        let ch = ch.clone();
        let cw = cw.clone();
        let query = query.clone();
        let stranded_query = stranded_query.clone();
        let peg_query = peg_query.clone();
        let zero_invariant_query = zero_invariant_query.clone();
        let collisions_query = collisions_query.clone();
        let orphan_candles_query = orphan_candles_query.clone();
        let refresh_waits_query = refresh_waits_query.clone();
        let mismatch_queries = mismatch_queries.clone();
        let ohlc_band_queries = ohlc_band_queries.clone();
        let environment = environment.clone();
        let deadline = event.context.deadline();
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

            // ---- 1b. current_prices writer liveness (task 0243) ----------
            //
            // Its own read, not a branch of the tier query: a failure here must
            // not cost the tier metrics, and vice versa. Placed before the slower
            // checks below, because a hard Lambda timeout loses whatever has not
            // been published yet and this datum is one cheap read.
            let mut current_age: Option<CurrentPricesAge> = None;
            match ch
                .query(current_prices_age_query())
                .fetch_one::<CurrentPricesAge>()
                .await
            {
                Ok(age) => {
                    current_age = Some(age);
                    let metric = current_prices_metric(&age);
                    if let Err(e) = publish(&cw, &environment, std::slice::from_ref(&metric)).await
                    {
                        failures.push(format!("current-prices publish: {e}"));
                    }
                }
                Err(e) => failures.push(format!("current-prices read: {e}")),
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
            //
            // ⚠️ TWO INDEPENDENT READS, and the independence is the fix task
            // 0213 shipped. They read different tiers, so a `_1m` scan that
            // matched nothing says nothing whatever about `_1h`. Letting one
            // refusal suppress the other's metric would re-create the
            // suppressed-signal failure at the invocation level, having just
            // removed it at the query level — and it is the same invariant the
            // block comment at the top of this function is about.
            let mut stranded_counts: Option<StrandedCounts> = None;
            match ch
                .query(&stranded_query)
                .fetch_one::<StrandedCounts>()
                .await
            {
                Ok(counts) => {
                    stranded_counts = Some(counts);
                    // An `Err` here is a check that did NOT RUN — an
                    // unresolvable USDT identity, or one that resolved to an id
                    // the candles no longer carry. Either way its zero must not
                    // be published, because NOT_BREACHING would score it as a
                    // clean bill of health.
                    match stranded_metric(&counts) {
                        Ok(metric) => {
                            if let Err(e) =
                                publish_sanity(&cw, &environment, std::slice::from_ref(&metric))
                                    .await
                            {
                                failures.push(format!("usd-stranded publish: {e}"));
                            }
                        }
                        Err(refusal) => failures.push(format!("usd-stranded: {refusal}")),
                    }
                }
                Err(e) => failures.push(format!("usd-stranded read: {e}")),
            }

            let mut peg_counts: Option<PegCounts> = None;
            match ch.query(&peg_query).fetch_one::<PegCounts>().await {
                Ok(counts) => {
                    peg_counts = Some(counts);
                    match peg_metric(&counts) {
                        Ok(metric) => {
                            if let Err(e) =
                                publish_sanity(&cw, &environment, std::slice::from_ref(&metric))
                                    .await
                            {
                                failures.push(format!("usd-peg-applied publish: {e}"));
                            }
                        }
                        Err(refusal) => failures.push(format!("usd-peg-applied: {refusal}")),
                    }
                }
                Err(e) => failures.push(format!("usd-peg-applied read: {e}")),
            }

            // ---- 4. Materialized-view drift (task 0204, gap 3) ------------
            //
            // One of the two reads here that touch `system.*` (the other is
            // 4b, `system.view_refreshes`, which is DENIED rather than filtered
            // and handled there). `system.tables` is grant-FILTERED, so it needs
            // no grant the probe does not already hold — but a narrowed grant or
            // a metadata hiccup here still must not cost the three checks above,
            // which is what the failure collection above guarantees.
            // `None` until a read succeeds, and left `None` when the schema is
            // unreadable (the published 0 is then a suppression, not a count):
            // the log line and the JSON result must never show a healthy 0
            // for a reading the probe does not have (review IN-01).
            let mut drift_critical: Option<f64> = None;
            let mut drift_count: Option<f64> = None;
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
                            let value_of =
                                |name: &str| drift.iter().find(|m| m.name == name).map(|m| m.value);
                            if visible > 0 {
                                drift_critical = value_of(MV_DRIFT_CRITICAL_METRIC);
                                drift_count = value_of(MV_DRIFT_METRIC);
                            }
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

            // ---- 4b. Rollup MVs stuck behind a dependency, or failing ------
            //
            // Since task 0143 the rollup MVs run `DEPENDS ON` their source
            // tier's MV, and a stopped, failing or missing dependency leaves
            // every dependent `WaitingForDependencies` forever with no error.
            // A view that fails itself — above all a leaf, which nothing waits
            // on — is counted from the same read (`exception`,
            // `last_success_time`; review WR-07), and named in the log line.
            // A cheap `system.*` read, so it sits with the drift read.
            //
            // ⚠️ `system.view_refreshes` is DENIED (Code 497), not filtered,
            // to a `SELECT ON prices.*` identity. That refusal publishes the
            // unreadable flag alone — never a waiting/disabled/failing 0, which would
            // read as a healthy chain — and is not a failure of the
            // invocation: the unreadable alarm is the signal. Any other error
            // is an ordinary failure.
            let mut refresh_waiting: Option<f64> = None;
            let mut refresh_disabled: Option<f64> = None;
            let mut refresh_failing: Option<f64> = None;
            // `None` = the read failed; the three counts stay `None` too when
            // the table is unreadable (then nothing but the flag is published).
            let mut refresh_unreadable: Option<f64> = None;
            let read = ch
                .query(&refresh_waits_query)
                .fetch_all::<ViewRefreshRow>()
                .await;
            let refresh_failing_detail = match &read {
                Ok(rows) => Some(describe_failing(
                    rows,
                    rows.first().map(|r| r.db_now_unix).unwrap_or_default(),
                )),
                Err(_) => None,
            };
            let refresh_metrics = match metrics_for_read(read) {
                Ok(metrics) => Some(metrics),
                Err(e) => {
                    failures.push(format!("mv-refresh-waits read: {e}"));
                    None
                }
            };
            if let Some(metrics) = refresh_metrics {
                for m in &metrics {
                    match m.name {
                        MV_REFRESH_WAITING_METRIC => refresh_waiting = Some(m.value),
                        MV_REFRESH_DISABLED_METRIC => refresh_disabled = Some(m.value),
                        MV_REFRESH_FAILING_METRIC => refresh_failing = Some(m.value),
                        MV_REFRESH_UNREADABLE_METRIC => refresh_unreadable = Some(m.value),
                        _ => {}
                    }
                }
                if let Err(e) = publish_drift(&cw, &environment, &metrics).await {
                    failures.push(format!("mv-refresh-waits publish: {e}"));
                }
            }
            let refresh_failing_detail =
                failing_detail_for_log(refresh_failing_detail, refresh_unreadable);

            // ---- 5. The zero sentinel's stored-data invariants (ADR 0292) ---
            //
            // Late on purpose (only 5b, 5c and the mismatch reads, 6, come after it). It is the one unscoped read here: `timestamp` is the
            // fourth sort-key column, so the 48 h window prunes only to the monthly
            // partition, which is then merged `FINAL` across every pair. Measured on
            // production 2026-09-18 it is cheap (0.04 s, 650k rows read) — but it
            // is still the read whose cost grows with the table. A hard Lambda timeout loses whatever has not
            // been published yet, and the MV-drift datum above is `NOT_BREACHING` on
            // missing data — so a slow scan placed before it would turn a lost
            // `APPEND` into a false OK. Placed here, a timeout costs only this
            // check and the mismatch reads after it.
            //
            // Not scoped to a quote leg, unlike the two USD-sanity checks (3): a
            // candle with no price-forming fill carries no price whatever it is
            // quoted in. Independent of them for the same reason they are
            // independent of each other — one refusal says nothing about another.
            let mut zero_counts: Option<ZeroInvariantCounts> = None;
            match ch
                .query(&zero_invariant_query)
                .fetch_one::<ZeroInvariantCounts>()
                .await
            {
                Ok(counts) => {
                    zero_counts = Some(counts);
                    match zero_invariant_metric(&counts) {
                        Ok(metric) => {
                            if let Err(e) =
                                publish_sanity(&cw, &environment, std::slice::from_ref(&metric))
                                    .await
                            {
                                failures.push(format!("zero-invariants publish: {e}"));
                            }
                        }
                        Err(refusal) => failures.push(format!("zero-invariants: {refusal}")),
                    }
                }
                Err(e) => failures.push(format!("zero-invariants read: {e}")),
            }

            // ---- 5b. Asset-id uniqueness (task 0139) ----------------------
            //
            // Two independent reads, after the zero invariants for the same
            // reason: they grow with the data (the registry, and the live tip
            // of `_1m` against it). A refusal is an empty read, recorded as a
            // failure rather than published as a healthy 0.
            let mut id_counts: Option<AssetIdCounts> = None;
            match ch.query(&collisions_query).fetch_one::<AssetIdCounts>().await {
                Ok(counts) => {
                    id_counts = Some(counts);
                    match collisions_metric(&counts) {
                        Ok(metric) => {
                            if let Err(e) =
                                publish_sanity(&cw, &environment, std::slice::from_ref(&metric))
                                    .await
                            {
                                failures.push(format!("asset-id-collisions publish: {e}"));
                            }
                        }
                        Err(refusal) => failures.push(format!("asset-id-collisions: {refusal}")),
                    }
                }
                Err(e) => failures.push(format!("asset-id-collisions read: {e}")),
            }

            let mut orphan_counts: Option<OrphanCandleCounts> = None;
            match ch
                .query(&orphan_candles_query)
                .fetch_one::<OrphanCandleCounts>()
                .await
            {
                Ok(counts) => {
                    orphan_counts = Some(counts);
                    match orphan_candles_metric(&counts) {
                        Ok(metric) => {
                            if let Err(e) =
                                publish_sanity(&cw, &environment, std::slice::from_ref(&metric))
                                    .await
                            {
                                failures.push(format!("asset-id-orphans publish: {e}"));
                            }
                        }
                        Err(refusal) => failures.push(format!("asset-id-orphans: {refusal}")),
                    }
                }
                Err(e) => failures.push(format!("asset-id-orphans read: {e}")),
            }

            // ---- 5c. Stored-candle OHLC band + positive prices, all seven tiers (task 0236) ---
            //
            // Late, for the reason block 5 gives: these reads grow with the
            // tables. Before the mismatch reads (6), which stay last because
            // their budget is what is left of the invocation. Measured on
            // production 2026-10-06 the seven reads together are cheap: 0.29 s
            // of server time, 3.7 M rows read.
            //
            // Seven INDEPENDENT reads, not one `UNION ALL`: a statement fails as a
            // unit, so one dropped tier or lost grant would hide the other six
            // counts, and the alarm is `NOT_BREACHING` on missing data. A failed
            // tier is recorded and the loop moves on.
            //
            // Independent of block 5, although a priced `_1m` row with
            // `close = 0` fires both: that overlap is expected (see `ohlc_band`).
            let mut band_readings: Vec<(&'static str, OhlcBandCounts)> = Vec::new();
            for (table, sql) in ohlc_band_queries.iter() {
                match ch.query(sql).fetch_one::<OhlcBandCounts>().await {
                    Ok(counts) => band_readings.push((*table, counts)),
                    Err(e) => failures.push(format!("ohlc-band read {table}: {e}")),
                }
            }
            match ohlc_band_metric(&band_readings) {
                Ok(metric) => {
                    if let Err(e) =
                        publish_sanity(&cw, &environment, std::slice::from_ref(&metric)).await
                    {
                        failures.push(format!("ohlc-band publish: {e}"));
                    }
                }
                Err(refusal) => failures.push(format!("ohlc-band: {refusal}")),
            }
            // Totals only when every tier was read: a partial sum logged as a
            // number would read as the whole chain. The detail names each tier.
            let band_totals = (band_readings.len() == OHLC_BAND_TIERS.len()).then(|| {
                band_readings.iter().fold((0u64, 0u64, 0u64), |(v, b, n), (_, c)| {
                    (v + c.violations(), b + c.band, n + c.nonpositive)
                })
            });
            let band_scanned_1m = band_readings
                .iter()
                .find(|(t, _)| *t == OHLC_BAND_TIERS[0].0)
                .map(|(_, c)| c.scanned);
            let band_detail = ohlc_band_detail(&band_readings);

            // ---- 6. Coarse buckets disagreeing with their source (0203) ---
            //
            // The new tail, for the reason the zero invariants were last: these
            // are the reads whose cost grows with the data (a GROUP BY over a
            // week of the child tier; the 15m read scans seven days of `_1m`).
            // They run after every other check, 15m FIRST (the heaviest, and
            // where a `_1m` hole shows first), each bounded by what is left of
            // the invocation (review WR-04): a tier that cannot get
            // MISMATCH_MIN_READ_SECS is skipped and recorded as a failure, so
            // the probe's -errors alarm speaks instead of a hard timeout that
            // publishes nothing. One tier's error is recorded and the next
            // tier still runs. A 0 here is a real reading — the query is
            // scoped to closed buckets past the grace, so a healthy chain
            // legitimately reads 0.
            let mut mismatch_detail: Vec<String> = Vec::new();
            for (table, sql) in mismatch_queries.iter() {
                let remaining = deadline
                    .duration_since(std::time::SystemTime::now())
                    .unwrap_or_default();
                let Some(bound) = mismatch_read_bound(remaining) else {
                    mismatch_detail.push(format!("{table}=skipped"));
                    failures.push(format!(
                        "rollup-mismatch {table} skipped: {:.1} s left of the invocation, \
                         under the {} s reserve + {MISMATCH_MIN_READ_SECS} s a read needs",
                        remaining.as_secs_f64(),
                        MISMATCH_RESERVE.as_secs(),
                    ));
                    continue;
                };
                let bounded = prices_clickhouse::with_execution_bound((*ch).clone(), bound);
                // The server bound is checked between blocks; this client-side
                // guard (bound + 2 s, inside the reserve) is what makes the
                // budget hold if the server does not answer at all.
                let read = tokio::time::timeout(
                    std::time::Duration::from_secs(bound + 2),
                    bounded.query(sql).fetch_one::<MismatchCount>(),
                )
                .await;
                match read {
                    Ok(Ok(count)) => {
                        mismatch_detail.push(format!("{table}={}", count.mismatched));
                        let metric = mismatch_metric(table, &count);
                        if let Err(e) =
                            publish_mismatch(&cw, &environment, std::slice::from_ref(&metric)).await
                        {
                            failures.push(format!("rollup-mismatch {table} publish: {e}"));
                        }
                    }
                    Ok(Err(e)) => {
                        mismatch_detail.push(format!("{table}=failed"));
                        failures.push(format!(
                            "rollup-mismatch {table} read (bound {bound} s): {e}"
                        ));
                    }
                    Err(_) => {
                        mismatch_detail.push(format!("{table}=failed"));
                        failures.push(format!(
                            "rollup-mismatch {table} read: no answer within {} s",
                            bound + 2
                        ));
                    }
                }
            }
            let mismatch_detail = mismatch_detail.join(", ");

            // Log before deciding the invocation's fate: on a partial failure
            // this line is the only record of what the healthy checks measured.
            //
            // ⚠️ A reading the probe does not have is logged as `failed` (the
            // read errored) or `unreadable` (suppressed: the grant or the
            // schema is missing), NEVER as 0 — a 0 is the healthy shape, and
            // this line is what the triage paragraphs point to (review IN-01).
            let drift_missing = if visible_objects == Some(0) {
                SKIPPED_UNREADABLE
            } else {
                FAILED
            };
            let refresh_missing = if refresh_unreadable == Some(1.0) {
                SKIPPED_UNREADABLE
            } else {
                FAILED
            };
            tracing::info!(
                tiers,
                current_prices_rows = %reading(current_age.map(|a| a.row_count), FAILED),
                current_prices_age_seconds = %reading(current_age.map(|a| a.age_seconds), FAILED),
                checks_failed = failures.len(),
                disk_free_percent = %reading(free_percent, FAILED),
                disk_available_bytes = %reading(disk_reading.map(|u| u.available_bytes), FAILED),
                usd_peg_applied = %reading(peg_counts.map(|c| c.peg_applied), FAILED),
                usd_peg_scanned = %reading(peg_counts.map(|c| c.scanned), FAILED),
                usd_stranded = %reading(stranded_counts.map(|c| c.stranded), FAILED),
                usd_stranded_scanned = %reading(stranded_counts.map(|c| c.scanned), FAILED),
                zero_invariant_violations = %reading(zero_counts.map(|c| c.violations), FAILED),
                zero_invariant_scanned = %reading(zero_counts.map(|c| c.scanned), FAILED),
                asset_identities = %reading(id_counts.map(|c| c.identities), FAILED),
                asset_ids = %reading(id_counts.map(|c| c.ids), FAILED),
                asset_id_orphan_candles = %reading(orphan_counts.map(|c| c.orphans), FAILED),
                asset_id_orphan_scanned = %reading(orphan_counts.map(|c| c.scanned), FAILED),
                mv_drift_critical = %reading(drift_critical, drift_missing),
                mv_drift = %reading(drift_count, drift_missing),
                mv_visible_objects = %reading(visible_objects, FAILED),
                ohlc_band_violations = %reading(band_totals.map(|t| t.0), FAILED),
                ohlc_band_band = %reading(band_totals.map(|t| t.1), FAILED),
                ohlc_band_nonpositive = %reading(band_totals.map(|t| t.2), FAILED),
                ohlc_band_scanned_1m = %reading(band_scanned_1m, FAILED),
                ohlc_band_tiers_read = band_readings.len(),
                ohlc_band_detail = %band_detail,
                mv_detail = %drift_detail,
                mv_refresh_waiting = %reading(refresh_waiting, refresh_missing),
                mv_refresh_disabled = %reading(refresh_disabled, refresh_missing),
                mv_refresh_failing = %reading(refresh_failing, refresh_missing),
                mv_refresh_failing_views = %reading(refresh_failing_detail.as_deref(), refresh_missing),
                mv_refresh_unreadable = %reading(refresh_unreadable, FAILED),
                rollup_mismatch = %mismatch_detail,
                "rollup-freshness-probe run complete"
            );

            if !failures.is_empty() {
                return Err(lambda_runtime::Error::from(format!(
                    "{} probe check(s) failed (the rest published normally): {}",
                    failures.len(),
                    failures.join("; ")
                )));
            }

            Ok::<serde_json::Value, lambda_runtime::Error>(serde_json::json!({
                "published": published,
                "current_prices": {
                    "rows": current_age.map(|a| a.row_count),
                    "age_seconds": current_age.map(|a| a.age_seconds),
                },
                "disk": {
                    "free_percent": free_percent,
                    "available_bytes": disk_reading.map(|u| u.available_bytes),
                    "capacity_bytes": disk_reading.map(|u| u.capacity_bytes),
                },
                "usd_sanity": {
                    "peg_applied": peg_counts.map(|c| c.peg_applied),
                    "peg_scanned": peg_counts.map(|c| c.scanned),
                    "stranded": stranded_counts.map(|c| c.stranded),
                    "stranded_scanned": stranded_counts.map(|c| c.scanned),
                },
                "zero_invariants": {
                    "violations": zero_counts.map(|c| c.violations),
                    "scanned": zero_counts.map(|c| c.scanned),
                },
                "asset_id_uniqueness": {
                    "identities": id_counts.map(|c| c.identities),
                    "ids": id_counts.map(|c| c.ids),
                    "orphan_candles": orphan_counts.map(|c| c.orphans),
                    "orphan_scanned": orphan_counts.map(|c| c.scanned),
                },
                "ohlc_band": {
                    "violations": band_totals.map(|t| t.0),
                    "tiers": band_readings
                        .iter()
                        .map(|(t, c)| serde_json::json!({
                            "tier": t,
                            "band": c.band,
                            "nonpositive": c.nonpositive,
                            "scanned": c.scanned,
                        }))
                        .collect::<Vec<_>>(),
                },
                "mv_drift": {
                    "critical": drift_critical,
                    "drifted": drift_count,
                    "visible_objects": visible_objects,
                    "detail": drift_detail,
                },
                "mv_refresh": {
                    "waiting": refresh_waiting,
                    "disabled": refresh_disabled,
                    "failing": refresh_failing,
                    "failing_views": refresh_failing_detail,
                    "unreadable": refresh_unreadable,
                },
                "rollup_mismatch": mismatch_detail,
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
