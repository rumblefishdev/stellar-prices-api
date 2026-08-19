//! USD-value correctness on the USDT quote leg (task 0204, gap 4).
//!
//! Every other alarm in this crate watches **liveness**: is data arriving, is
//! there disk. This one watches **correctness** — a `close_usd` that is present,
//! recent and wrong looks perfectly healthy to all of them. Task 0204's gap 3
//! makes the same point about a drifted materialized view, which "produces data
//! perfectly well while producing the wrong numbers".
//!
//! ## Why the writer tests cannot be the guard
//!
//! `close_usd` has now been wrong on production through **three different
//! doors**, and only one of them goes through the writer:
//!
//! | how it broke | task | caught by a writer test? |
//! |---|---|---|
//! | USDT valued at a $1 peg it no longer held | 0172 | ✅ |
//! | Reflector rows mis-attributed to the USDT identity | 0196, and 0168 before it | ❌ |
//! | a repair epoch 19 h below its pivot reference | 0182 | ❌ |
//!
//! Two of the three never touch the writer at all, so the regression tests task
//! 0172 already owns would have caught one of them. **The condition lives in the
//! data, so the check has to as well.**
//!
//! ## The two failure directions, and why both are needed
//!
//! - **Peg applied** — `close_usd ≈ close`, i.e. the leg valued at $1. The
//!   original 0172 defect, and what 0182 corrected across 567,760 candles.
//! - **Stranded** — `close_usd = 0` on a candle whose `close` is large enough to
//!   produce a representable value. ⚠️ Added because **0182's own repair caused
//!   this**: on 2026-08-19 an epoch 19 hours below the pivot's first reference
//!   candle left 157 rows zeroed with nothing to refill them. A check for only
//!   the first direction would have passed while that damage stood.
//!
//! ## Three ways this check could be wrong, all designed around
//!
//! 1. **Scope it to the quote leg, never "all candles".** Exotic-quoted rows sit
//!    at `close_usd = 0` *by design* — no USD reference exists and no enrichment
//!    tier can price them (0182 measured ~74M such rows on `_1h` alone). A check
//!    counting every zero would breach permanently on healthy data, and a
//!    permanently-firing alarm gets muted, which is the state task 0204 exists
//!    to end.
//! 2. **Give the stranded direction a grace period.** Enrichment fills
//!    `close_usd` asynchronously, so a freshly written candle is *legitimately*
//!    zero until the sweep reaches it. Without [`STRANDED_GRACE_SECONDS`] this
//!    metric would never read zero. See that constant for why the grace is 48 h
//!    specifically and not an arbitrary round number.
//! 3. **Bound the window.** An unbounded scan of the OHLCV tables every 15
//!    minutes is task 0111's outage, reintroduced as a health check. The window
//!    is [`LOOKBACK_SECONDS`] and prunes by partition.
//!
//! ⚠️ **This is a re-introduction guard, not a historical audit.** It watches
//! recent writes, because that is where a regression shows up. A frozen
//! historical corruption inside the window would latch the alarm rather than
//! re-notify — see [`SanityCounts`] on why the counts still climb in the case
//! this actually guards against.
//!
//! ⚠️ **If USDT ever returns to par, the peg-applied direction stops being
//! diagnostic** and this check must be revisited. It reads a ratio near 1.0 as
//! evidence the peg path was applied, which is only sound while the asset
//! genuinely trades away from $1 (measured ~0.13-0.15 since the June 2022
//! break, task 0172). That assumption is load-bearing and is why the tolerance
//! below is tight rather than generous.

use prices_clickhouse::USDT_ISSUER;

/// Count of USDT-quoted candles whose `close_usd` looks like the $1 peg was
/// applied. Watched by the `prices-{env}-usd-peg-applied` alarm ladder.
pub const PEG_APPLIED_METRIC: &str = "UsdtPegAppliedCandles";

/// Count of USDT-quoted candles left at `close_usd = 0` past the grace period
/// despite a representable `close`. Watched by the
/// `prices-{env}-usd-stranded` alarm ladder.
pub const STRANDED_METRIC: &str = "UsdtStrandedCandles";

/// How far back the check looks, in seconds.
///
/// Seven days. Long enough that a regression cannot slip through between runs
/// or hide behind a weekend, short enough that the scan prunes to one or two
/// monthly partitions. ⚠️ Do not widen this to "all history" — that is the
/// full-table scan of task 0111, which caused a four-day production outage, and
/// re-introducing it inside a 15-minute probe would be worse than the defect
/// this alarm watches for.
pub const LOOKBACK_SECONDS: i64 = 7 * 86_400;

/// How old a candle must be before a `close_usd` of zero counts as *stranded*
/// rather than *not yet enriched*.
///
/// **48 hours, and the number is not arbitrary.** It is the width of the window
/// the downstream consumer actually reads: BE take the last `close_usd > 0`
/// within 48 h for pool TVL and render `--` when there is none (task 0182, BE
/// answers 2026-08-13). So a row still at zero after 48 h is precisely a row
/// that has begun costing a consumer a value — which is the moment this is worth
/// waking someone for, and not before.
///
/// ⚠️ It must also comfortably exceed real enrichment lag or the metric never
/// reads zero. Measured 2026-08-19: USDT-quoted candles were sitting unpriced
/// for **17 h+** (task 0209, still open). 48 h clears that with room, but if
/// 0209 turns out to be a widening gap rather than ordinary lag, this bound
/// needs re-checking against it — do not simply raise it to silence the alarm.
pub const STRANDED_GRACE_SECONDS: i64 = 2 * 86_400;

/// How close `close_usd / close` must sit to 1.0 to count as "the peg was
/// applied".
///
/// 2%. USDT has traded at ~0.13-0.15 since the June 2022 break (task 0172), so
/// a genuine value is nowhere near this band and the tolerance exists only to
/// absorb rounding at `Decimal(38, 14)`, not to make a judgement call. Widening
/// it does not make the check more sensitive — it makes it start catching real
/// prices.
pub const PEG_RATIO_TOLERANCE: f64 = 0.02;

/// Below this `close`, `rate × close` rounds to zero at `Decimal(38, 14)` and a
/// `close_usd` of 0 is arithmetic, not damage.
///
/// ⚠️ **Anchored on the arithmetic, not on what looks small.** Task 0182's first
/// attempt at this bound used `1e-11` — three orders of magnitude too high — and
/// counted dust rows that had priced perfectly well. With a rate of ~0.15,
/// `rate × close` rounds to zero below ~`3.3e-14`; `5e-14` sits just above that
/// and is the value 0182's verification settled on.
pub const REPRESENTABLE_CLOSE_FLOOR: &str = "0.00000000000005";

/// Which OHLCV granularity the check reads.
///
/// `price_ohlcv_1d`. The defect appears on every tier — 0182 corrected all five
/// — so any one of them is diagnostic, and this is the cheapest that still
/// reacts promptly: a day's bucket exists from its first trade, so a regression
/// surfaces within one probe interval rather than a day later. `_1h` would scan
/// ~24× the rows for the same answer.
pub const SANITY_TABLE: &str = "price_ohlcv_1d";

/// One reading of the USDT quote leg's USD-value health.
///
/// `resolved_legs` and `scanned` are the two guards against the silent
/// all-clear. If the USDT identity cannot be resolved — the registry moved, an
/// issuer changed, task 0139 renumbered something — the quote-leg filter matches
/// nothing, both counts come back `0`, and the alarm scores a **healthy** result
/// for a check that did not run. `resolved_legs` catches the identity being
/// missing; `scanned` catches it resolving to an id the candles no longer carry,
/// which the first guard cannot see. Carrying both in the same row makes each
/// state detectable; [`sanity_metrics`] refuses them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct SanityCounts {
    /// Number of `assets` rows matching the canonical USDT identity. Must be 1.
    pub resolved_legs: u64,
    /// USDT-quoted candles in the window whose `close_usd / close` sits within
    /// [`PEG_RATIO_TOLERANCE`] of 1.0.
    pub peg_applied: u64,
    /// USDT-quoted candles older than [`STRANDED_GRACE_SECONDS`] still at
    /// `close_usd = 0` with a `close` above [`REPRESENTABLE_CLOSE_FLOOR`].
    pub stranded: u64,
    /// USDT-quoted candles examined.
    ///
    /// Not alarmed on, but **guarded on**: zero means the query matched nothing,
    /// so the two counts above are zero for want of data rather than for want of
    /// defects. See [`SanityRefusal::EmptyScan`].
    pub scanned: u64,
}

/// One CloudWatch datum about USD-value correctness. Counts, so
/// `StandardUnit::Count` — mirrored without dragging the AWS SDK into the
/// default build, exactly as [`crate::disk::DiskMetric`] does.
#[derive(Debug, Clone, PartialEq)]
pub struct SanityMetric {
    pub name: &'static str,
    pub value: f64,
}

/// The one-row query behind [`SanityCounts`].
///
/// The USDT identity is resolved **inline by code and issuer** rather than
/// hard-coded as an `asset_id`. The numeric id is not a stable contract while
/// task 0139 is open, and a probe that silently watches the wrong leg is worse
/// than one that fails — see [`SanityCounts::resolved_legs`].
///
/// Tables are unqualified: the probe binds its client to the `prices` database,
/// the same convention as [`crate::freshness_query`] and [`crate::disk::disk_query`].
///
/// ⚠️ `FINAL` is required. These are `ReplacingMergeTree` tables and a repair
/// re-inserts corrected rows at a higher `version`; without `FINAL` this would
/// read superseded rows and report a defect that has already been fixed —
/// alarming on history rather than on state.
///
/// ⚠️ **`resolved_legs` is wrapped in `toUInt64(ifNull(…))` and must stay that
/// way.** A bare scalar subquery — `(SELECT count() FROM usdt)` — comes back as
/// **`Nullable(UInt64)`**, and in RowBinary a nullable column is a one-byte null
/// flag followed by the value. Deserializing that into a plain `u64` does not
/// error: it consumes the flag as the low byte and silently returns **256** for
/// a true count of 1. Measured on 26.3.10.60 while writing the integration
/// tests. That is worse than the `backfill-freshness-probe` regression this
/// crate's IT file was written for (PR #97), which at least failed loudly — here
/// the probe would refuse every healthy run as "ambiguous identity" and the
/// gap-4 alarms would never publish.
pub fn sanity_query() -> String {
    format!(
        "WITH usdt AS ( \
             SELECT asset_id FROM assets FINAL \
             WHERE asset_code = 'USDT' AND issuer_address = '{usdt}' \
         ) \
         SELECT \
             toUInt64(ifNull((SELECT count() FROM usdt), 0)) AS resolved_legs, \
             countIf(close_usd > 0 AND abs(close_usd / close - 1) < {tol}) AS peg_applied, \
             countIf( \
                 close_usd = 0 \
                 AND close > {floor} \
                 AND timestamp < now() - INTERVAL {grace} SECOND \
             ) AS stranded, \
             count() AS scanned \
         FROM {table} FINAL \
         WHERE quote_asset_id IN (SELECT asset_id FROM usdt) \
           AND close > 0 \
           AND timestamp >= now() - INTERVAL {lookback} SECOND",
        usdt = USDT_ISSUER,
        tol = PEG_RATIO_TOLERANCE,
        floor = REPRESENTABLE_CLOSE_FLOOR,
        grace = STRANDED_GRACE_SECONDS,
        table = SANITY_TABLE,
        lookback = LOOKBACK_SECONDS,
    )
}

/// Why a reading was refused instead of published.
///
/// Both variants describe the **same hazard from different distances**: a query
/// that matched nothing still returns two perfectly publishable zeros, and the
/// gap-4 alarms are `treatMissingData: NOT_BREACHING`, so those zeros are scored
/// as a clean bill of health forever. The variants are separate because the
/// operator's first move differs — one points at the asset registry, the other
/// at the candles — and an alarm that names the wrong one costs an hour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanityRefusal {
    /// The canonical USDT identity did not resolve to exactly one `assets` row.
    UnresolvableLeg { resolved_legs: u64 },
    /// The leg resolved, but matched **no candles at all** in the window.
    EmptyScan,
}

impl std::fmt::Display for SanityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnresolvableLeg { resolved_legs } => write!(
                f,
                "the canonical USDT identity resolved to {resolved_legs} assets, expected \
                 exactly 1 — check prices.assets for the USDT code/issuer pair"
            ),
            Self::EmptyScan => write!(
                f,
                "the USDT identity resolved, but no USDT-quoted candles were found in the \
                 last {days} days of {table} — the asset_id in the registry no longer matches \
                 the quote_asset_id stored in the candles (task 0139), or the leg has stopped \
                 trading entirely",
                days = LOOKBACK_SECONDS / 86_400,
                table = SANITY_TABLE,
            ),
        }
    }
}

impl std::error::Error for SanityRefusal {}

/// Shape one [`SanityCounts`] reading into the CloudWatch data to publish.
///
/// `Err` when the check **did not actually run**, which is not a clean bill of
/// health and must never be published as one. The caller turns an `Err` into a
/// failed invocation so the probe's own `-errors` alarm carries it — the same
/// contract [`crate::disk::disk_metrics`] uses for an unreadable capacity.
///
/// ⚠️ **Two guards, not one, and the second is the subtler.** `resolved_legs`
/// catches an identity that cannot be found at all. But an identity that
/// resolves to an `asset_id` no longer present in the candles — precisely the
/// renumber risk task 0139 is open for, and the reason the leg is resolved by
/// issuer rather than by id — passes that guard and then matches zero rows.
/// Both counts publish `0` and every gap-4 alarm reads healthy forever. A scan
/// that examined nothing has measured nothing.
pub fn sanity_metrics(counts: &SanityCounts) -> Result<Vec<SanityMetric>, SanityRefusal> {
    if counts.resolved_legs != 1 {
        return Err(SanityRefusal::UnresolvableLeg {
            resolved_legs: counts.resolved_legs,
        });
    }
    if counts.scanned == 0 {
        return Err(SanityRefusal::EmptyScan);
    }
    Ok(vec![
        SanityMetric {
            name: PEG_APPLIED_METRIC,
            value: counts.peg_applied as f64,
        },
        SanityMetric {
            name: STRANDED_METRIC,
            value: counts.stranded as f64,
        },
    ])
}

/// Publish USD-sanity counts to CloudWatch under [`crate::METRIC_NAMESPACE`],
/// tagged with the `Environment` dimension. One `PutMetricData` call.
///
/// Rides in `Prices/Rollup` for the same reason the disk metrics do — the
/// probe role's `PutMetricData` grant is conditioned on that namespace in
/// `eventbridge-stack.ts`, and that stack owns `CleanupRule`, whose template
/// still asserts `State: ENABLED` while the live rule is DISABLED. See
/// [`crate::disk::publish_disk`] for the full reasoning; it applies unchanged.
#[cfg(feature = "lambda")]
pub async fn publish_sanity(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    metrics: &[SanityMetric],
) -> Result<(), aws_sdk_cloudwatch::Error> {
    use aws_sdk_cloudwatch::types::{Dimension, MetricDatum, StandardUnit};

    if metrics.is_empty() {
        return Ok(());
    }

    let env_dim = Dimension::builder()
        .name("Environment")
        .value(environment)
        .build();

    let data = metrics
        .iter()
        .map(|m| {
            MetricDatum::builder()
                .metric_name(m.name)
                .value(m.value)
                .unit(StandardUnit::Count)
                .dimensions(env_dim.clone())
                .build()
        })
        .collect::<Vec<_>>();

    client
        .put_metric_data()
        .namespace(crate::METRIC_NAMESPACE)
        .set_metric_data(Some(data))
        .send()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn healthy() -> SanityCounts {
        SanityCounts {
            resolved_legs: 1,
            peg_applied: 0,
            stranded: 0,
            scanned: 4_503,
        }
    }

    /// The trap this whole module is scoped around. Exotic-quoted candles sit at
    /// `close_usd = 0` by design (~74M on `_1h` alone, task 0182), so a check
    /// that did not filter by quote leg would breach permanently on healthy
    /// data.
    #[test]
    fn the_query_is_scoped_to_the_usdt_quote_leg() {
        let sql = sanity_query();
        assert!(sql.contains("WHERE quote_asset_id IN (SELECT asset_id FROM usdt)"));
    }

    /// The numeric `asset_id` is not a stable contract while task 0139 is open,
    /// so the leg is resolved by code and issuer at query time.
    #[test]
    fn the_usdt_leg_is_resolved_by_issuer_not_a_hardcoded_id() {
        let sql = sanity_query();
        assert!(sql.contains(USDT_ISSUER));
        assert!(sql.contains("asset_code = 'USDT'"));
    }

    /// Both directions must be counted. 0182's repair caused the second one, so
    /// a check for only the peg would have passed while 157 candles sat
    /// destroyed.
    #[test]
    fn the_query_counts_both_failure_directions() {
        let sql = sanity_query();
        assert!(sql.contains("AS peg_applied"));
        assert!(sql.contains("AS stranded"));
    }

    /// Without the grace period the stranded count can never read zero:
    /// enrichment fills `close_usd` asynchronously, so the newest candles are
    /// legitimately at zero on every single run.
    #[test]
    fn the_stranded_count_excludes_candles_too_young_to_be_enriched() {
        let sql = sanity_query();
        assert!(sql.contains(&format!(
            "timestamp < now() - INTERVAL {STRANDED_GRACE_SECONDS} SECOND"
        )));
    }

    /// An unbounded scan every 15 minutes is task 0111's outage wearing a health
    /// check's clothes.
    #[test]
    fn the_scan_is_bounded_to_a_recent_window() {
        let sql = sanity_query();
        assert!(sql.contains(&format!(
            "timestamp >= now() - INTERVAL {LOOKBACK_SECONDS} SECOND"
        )));
    }

    /// `ReplacingMergeTree` + a repair that re-inserts at a higher `version`.
    /// Without `FINAL` this reads superseded rows and alarms on a defect that
    /// has already been corrected.
    #[test]
    fn the_query_reads_final() {
        let sql = sanity_query();
        assert!(sql.contains(&format!("FROM {SANITY_TABLE} FINAL")));
        assert!(sql.contains("FROM assets FINAL"));
    }

    /// The dust bound is the arithmetic one. 0182's first attempt used `1e-11`
    /// and counted rows that had priced perfectly well.
    #[test]
    fn the_representable_floor_is_the_underflow_bound_not_a_round_number() {
        assert!(sanity_query().contains(REPRESENTABLE_CLOSE_FLOOR));
        let floor: f64 = REPRESENTABLE_CLOSE_FLOOR.parse().expect("parses");
        // Comfortably below 1e-11, and above where a ~0.15 rate underflows.
        assert!(floor < 1e-13);
        assert!(floor > 1e-14);
    }

    /// ⚠️ Regression pin for a silent-corruption trap, not a style preference.
    /// A bare `(SELECT count() FROM usdt)` is `Nullable(UInt64)`; RowBinary
    /// prefixes a nullable with a null-flag byte, and deserializing that into
    /// `u64` yields **256** for a count of 1 without any error. Measured on
    /// 26.3.10.60. The probe would then reject every healthy run.
    #[test]
    fn the_resolved_leg_count_is_forced_non_nullable() {
        let sql = sanity_query();
        assert!(sql.contains("toUInt64(ifNull((SELECT count() FROM usdt), 0)) AS resolved_legs"));
    }

    #[test]
    fn a_healthy_reading_publishes_two_zeroes() {
        let m = sanity_metrics(&healthy()).expect("leg resolved");
        assert_eq!(
            m,
            vec![
                SanityMetric {
                    name: PEG_APPLIED_METRIC,
                    value: 0.0
                },
                SanityMetric {
                    name: STRANDED_METRIC,
                    value: 0.0
                },
            ]
        );
    }

    #[test]
    fn counts_are_published_as_they_are_read() {
        let m = sanity_metrics(&SanityCounts {
            peg_applied: 44_657,
            stranded: 157,
            ..healthy()
        })
        .expect("leg resolved");
        assert_eq!(m[0].value, 44_657.0);
        assert_eq!(m[1].value, 157.0);
    }

    /// The silent all-clear this module's `resolved_legs` column exists to
    /// prevent: an unresolvable leg matches no candles, both counts read zero,
    /// and a `NOT_BREACHING` alarm scores it healthy forever.
    #[test]
    fn an_unresolvable_usdt_leg_is_refused_rather_than_reported_healthy() {
        assert_eq!(
            sanity_metrics(&SanityCounts {
                resolved_legs: 0,
                ..healthy()
            }),
            Err(SanityRefusal::UnresolvableLeg { resolved_legs: 0 })
        );
    }

    /// Two rows for one identity means the registry is ambiguous — the counts
    /// would be a union across legs and the alarm would be reading something
    /// nobody designed. Refuse that too, rather than picking one.
    #[test]
    fn an_ambiguous_usdt_identity_is_refused() {
        assert_eq!(
            sanity_metrics(&SanityCounts {
                resolved_legs: 2,
                ..healthy()
            }),
            Err(SanityRefusal::UnresolvableLeg { resolved_legs: 2 })
        );
    }

    /// The guard `resolved_legs` cannot provide. The identity resolves cleanly,
    /// so the first guard passes — but the `asset_id` it resolves to is not the
    /// `quote_asset_id` the candles carry (task 0139 renumbering, a registry
    /// rewrite), so the scan matches nothing. Both counts are zero because
    /// nothing was examined, and `NOT_BREACHING` would score that healthy for as
    /// long as it lasted.
    #[test]
    fn a_resolved_leg_that_matches_no_candles_is_refused() {
        assert_eq!(
            sanity_metrics(&SanityCounts {
                scanned: 0,
                ..healthy()
            }),
            Err(SanityRefusal::EmptyScan)
        );
    }

    /// The two refusals must not read alike: one sends the operator to the asset
    /// registry, the other to the candles.
    #[test]
    fn the_two_refusals_name_different_first_moves() {
        let unresolvable = SanityRefusal::UnresolvableLeg { resolved_legs: 0 }.to_string();
        let empty = SanityRefusal::EmptyScan.to_string();
        assert!(unresolvable.contains("prices.assets"));
        assert!(empty.contains("quote_asset_id"));
        assert_ne!(unresolvable, empty);
    }

    /// The peg tolerance is a rounding allowance, not a judgement call. USDT
    /// trades at ~0.13-0.15, so a real value must sit far outside the band.
    #[test]
    fn the_peg_tolerance_cannot_reach_a_real_usdt_price() {
        let measured_2026 = 0.15_f64;
        assert!((measured_2026 - 1.0).abs() > PEG_RATIO_TOLERANCE);
    }
}
