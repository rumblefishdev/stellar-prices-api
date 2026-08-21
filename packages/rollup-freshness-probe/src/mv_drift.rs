//! Materialized-view drift, on a schedule (task 0204, gap 3).
//!
//! Task 0142 built the comparison — `prices_clickhouse::drift` reads the six
//! rollup MVs' live definitions and diffs them against `schema/rollups.sql`.
//! **Nothing ran it.** A check nobody runs is a check that does not exist, and
//! this one covers a condition no other alarm here can see: task 0137 watches
//! whether the rollups are *producing data*, which a drifted MV does perfectly
//! well while producing the wrong numbers.
//!
//! This module is the scheduled half — it turns that report into two counts the
//! ops alarms can watch, and rides the probe that already runs every 15 minutes.
//!
//! ## Two severities, because they are not the same emergency
//!
//! `drift.rs` returns one status per MV and the CLI collapses everything to
//! `exit 1`. That throws away the distinction that decides who gets woken:
//!
//! - **[`MV_DRIFT_CRITICAL_METRIC`]** — an MV that is no longer `APPEND`.
//!   History is being **destroyed on every refresh**; the loss is unrecoverable
//!   and it compounds while nobody looks. This is the [task 0095] failure that
//!   wiped the coarse tables.
//! - **[`MV_DRIFT_METRIC`]** — a definition that does not match the file, is
//!   missing, is unparseable, or is an undeclared MV writing into a table it
//!   does not own. Wrong, but static: it stays exactly as wrong until a person
//!   fixes it. Morning work.
//!
//! [task 0095]: ../../../lore/1-tasks/archive/
//!
//! ## ⚠️ Why one alarm transition is enough here, when it was not for the DLQ
//!
//! Task 0204 exists partly because the ingest-DLQ alarm notified once and went
//! quiet while the queue climbed to 91 overnight — CloudWatch notifies on a
//! state *transition*, so a latched alarm is silent. Gap 2 fixed that with a
//! threshold ladder, and gap 4 could reuse the ladder because a count of wrong
//! candles keeps growing.
//!
//! **Drift does not grow.** One drifted MV stays one drifted MV; the situation
//! is not deteriorating while the alarm is quiet, so a latched alarm costs
//! "somebody may forget", not "we are blind to an escalation". That is the
//! operator's accepted trade (2026-08-19), and it is why this ships without the
//! stored `first_seen` state a re-notifying age metric would need.
//!
//! ⚠️ **The exception is exactly the critical severity**, which *does* compound
//! — hence its own metric and its own alarm, so the case that deteriorates is
//! never buried in the case that does not.
//!
//! ## ⚠️ All six missing is a grant gap, not six deleted MVs
//!
//! `system.tables` is **filtered by grant**, not denied: a user with
//! `GRANT SELECT ON prices.*` sees the `prices` objects and nothing else
//! (measured on 26.3.10.60 — 32 tables, 7 `create_table_query` values). So a
//! narrowed grant does not error; every MV simply reports `Missing`, which is
//! indistinguishable from the whole rollup chain having been dropped.
//!
//! Paging "the rollup chain is gone" for a permissions change would be a false
//! alarm of the worst kind — maximum urgency, wrong diagnosis. So this module
//! also carries [`visible_objects_query`], and treats *nothing visible at all*
//! as the discriminator: no `prices` objects visible means the probe cannot see
//! the schema, and the counts are suppressed in favour of
//! [`MV_DRIFT_UNREADABLE_METRIC`]. If objects **are** visible and the MVs are
//! still missing, they really are missing and the ordinary alarm is correct.

use prices_clickhouse::drift::{MvReport, MvStatus};

/// Count of MVs that have lost `APPEND` — history destroyed on every refresh.
/// Watched by `prices-{env}-mv-drift-critical`.
pub const MV_DRIFT_CRITICAL_METRIC: &str = "MvDriftCritical";

/// Count of MVs that need attention for any non-destructive reason. Watched by
/// `prices-{env}-mv-drift`.
pub const MV_DRIFT_METRIC: &str = "MvDriftCount";

/// `1` when the probe cannot see the schema at all, so the two counts above are
/// not evidence of anything. Watched by `prices-{env}-mv-drift-unreadable`.
pub const MV_DRIFT_UNREADABLE_METRIC: &str = "MvDriftUnreadable";

/// One CloudWatch datum about MV drift. A count, like
/// [`crate::usd_sanity::SanityMetric`], and kept free of the AWS SDK for the
/// same reason — the shaping stays unit-testable in the default build.
#[derive(Debug, Clone, PartialEq)]
pub struct DriftMetric {
    pub name: &'static str,
    pub value: f64,
}

/// How many objects the probe can see in its own database.
///
/// Used **only** to tell a grant gap apart from a deleted rollup chain; see the
/// module docs. Reads `system.tables`, which needs no grant beyond the one the
/// probe already holds — it is filtered, not denied.
pub fn visible_objects_query(database: &str) -> String {
    format!("SELECT count() FROM system.tables WHERE database = '{database}'")
}

/// Split a drift report into the two severities the alarms watch.
///
/// `visible_objects` is the count from [`visible_objects_query`]. When it is
/// zero the probe cannot see its schema, so both counts are suppressed and
/// [`MV_DRIFT_UNREADABLE_METRIC`] is raised instead — publishing
/// `MvDriftCount = 6` in that state would page as if the whole rollup chain had
/// been deleted, when the likely cause is a narrowed grant.
///
/// ⚠️ An MV can be **both** drifted and non-`APPEND`. It is counted as critical
/// and *not* also as ordinary drift: the critical alarm is the one that must be
/// acted on, and double-counting would let a single object inflate a number an
/// operator reads as "how many MVs are affected".
pub fn drift_metrics(reports: &[MvReport], visible_objects: u64) -> Vec<DriftMetric> {
    if visible_objects == 0 {
        return vec![
            DriftMetric {
                name: MV_DRIFT_UNREADABLE_METRIC,
                value: 1.0,
            },
            DriftMetric {
                name: MV_DRIFT_CRITICAL_METRIC,
                value: 0.0,
            },
            DriftMetric {
                name: MV_DRIFT_METRIC,
                value: 0.0,
            },
        ];
    }

    let critical = reports
        .iter()
        .filter(|r| r.live.as_ref().is_some_and(|f| !f.is_append()))
        .count();
    let drifted = reports
        .iter()
        .filter(|r| r.needs_attention() && !r.live.as_ref().is_some_and(|f| !f.is_append()))
        .count();

    vec![
        DriftMetric {
            name: MV_DRIFT_UNREADABLE_METRIC,
            value: 0.0,
        },
        DriftMetric {
            name: MV_DRIFT_CRITICAL_METRIC,
            value: critical as f64,
        },
        DriftMetric {
            name: MV_DRIFT_METRIC,
            value: drifted as f64,
        },
    ]
}

/// One-line summary for the invocation log, so a drift that alarms can be
/// diagnosed from CloudWatch Logs without re-running the CLI by hand.
pub fn describe(reports: &[MvReport]) -> String {
    let mut parts: Vec<String> = reports
        .iter()
        .filter(|r| r.needs_attention())
        .map(|r| {
            let kind = match &r.status {
                MvStatus::InSync => "not-append",
                MvStatus::Missing => "missing",
                MvStatus::Drifted(diffs) => {
                    return format!(
                        "{}=drifted({})",
                        r.name,
                        diffs
                            .iter()
                            .map(|d| d.field.as_str())
                            .collect::<Vec<_>>()
                            .join("|")
                    );
                }
                MvStatus::Unparseable(_) => "unparseable",
                MvStatus::Undeclared => "undeclared",
            };
            format!("{}={}", r.name, kind)
        })
        .collect();
    if parts.is_empty() {
        parts.push("all in sync".to_string());
    }
    parts.join(", ")
}

/// Publish drift counts to CloudWatch under [`crate::METRIC_NAMESPACE`], tagged
/// with the `Environment` dimension.
///
/// Rides in `Prices/Rollup` for the same reason the disk and USD metrics do —
/// see [`crate::disk::publish_disk`]. Reusing the namespace is what keeps gap 3
/// inside `observability-stack.ts` and off `eventbridge-stack.ts`, which owns
/// `CleanupRule`.
#[cfg(feature = "lambda")]
pub async fn publish_drift(
    client: &aws_sdk_cloudwatch::Client,
    environment: &str,
    metrics: &[DriftMetric],
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
    use prices_clickhouse::drift::{Difference, DriftField, MvFingerprint};

    fn value_of(metrics: &[DriftMetric], name: &str) -> f64 {
        metrics
            .iter()
            .find(|m| m.name == name)
            .unwrap_or_else(|| panic!("{name} published"))
            .value
    }

    fn fingerprint(append: bool) -> MvFingerprint {
        MvFingerprint {
            name: "prices.mv_ohlcv_15m_to_1h".into(),
            refresh: if append {
                "EVERY 15 MINUTE APPEND".into()
            } else {
                "EVERY 15 MINUTE".into()
            },
            target: "prices.price_ohlcv_1h".into(),
            body: "SELECT 1".into(),
        }
    }

    fn in_sync(name: &str) -> MvReport {
        MvReport {
            name: name.into(),
            status: MvStatus::InSync,
            live: Some(fingerprint(true)),
        }
    }

    fn lost_append(name: &str) -> MvReport {
        MvReport {
            name: name.into(),
            status: MvStatus::InSync,
            live: Some(fingerprint(false)),
        }
    }

    fn drifted(name: &str) -> MvReport {
        MvReport {
            name: name.into(),
            status: MvStatus::Drifted(vec![Difference {
                field: DriftField::Body,
                declared: "a".into(),
                live: "b".into(),
            }]),
            live: Some(fingerprint(true)),
        }
    }

    fn missing(name: &str) -> MvReport {
        MvReport {
            name: name.into(),
            status: MvStatus::Missing,
            live: None,
        }
    }

    #[test]
    fn a_clean_chain_publishes_zeroes() {
        let m = drift_metrics(&[in_sync("a"), in_sync("b")], 32);
        assert_eq!(value_of(&m, MV_DRIFT_CRITICAL_METRIC), 0.0);
        assert_eq!(value_of(&m, MV_DRIFT_METRIC), 0.0);
        assert_eq!(value_of(&m, MV_DRIFT_UNREADABLE_METRIC), 0.0);
    }

    /// The severity split is the whole point of gap 3's design: a lost `APPEND`
    /// destroys history on every refresh and must not be reported through the
    /// same number as a cosmetic definition mismatch.
    #[test]
    fn a_lost_append_counts_as_critical_not_as_ordinary_drift() {
        let m = drift_metrics(&[lost_append("a"), in_sync("b")], 32);
        assert_eq!(value_of(&m, MV_DRIFT_CRITICAL_METRIC), 1.0);
        assert_eq!(value_of(&m, MV_DRIFT_METRIC), 0.0);
    }

    /// ⚠️ An MV can be both. Counting it twice would inflate a number the
    /// operator reads as "how many MVs are affected".
    #[test]
    fn an_mv_that_is_both_drifted_and_non_append_is_counted_once_as_critical() {
        let both = MvReport {
            name: "a".into(),
            status: MvStatus::Drifted(vec![Difference {
                field: DriftField::Body,
                declared: "a".into(),
                live: "b".into(),
            }]),
            live: Some(fingerprint(false)),
        };
        let m = drift_metrics(&[both], 32);
        assert_eq!(value_of(&m, MV_DRIFT_CRITICAL_METRIC), 1.0);
        assert_eq!(value_of(&m, MV_DRIFT_METRIC), 0.0);
    }

    #[test]
    fn every_non_destructive_status_counts_as_ordinary_drift() {
        let m = drift_metrics(
            &[
                drifted("a"),
                missing("b"),
                MvReport {
                    name: "c".into(),
                    status: MvStatus::Unparseable("?".into()),
                    live: None,
                },
                MvReport {
                    name: "d".into(),
                    status: MvStatus::Undeclared,
                    live: Some(fingerprint(true)),
                },
            ],
            32,
        );
        assert_eq!(value_of(&m, MV_DRIFT_METRIC), 4.0);
        assert_eq!(value_of(&m, MV_DRIFT_CRITICAL_METRIC), 0.0);
    }

    /// ⚠️ The false page this module exists to avoid. `system.tables` is
    /// grant-FILTERED, so a narrowed grant makes every MV report `Missing` —
    /// identical to the whole chain having been dropped. Nothing visible at all
    /// is the discriminator.
    #[test]
    fn an_invisible_schema_is_reported_as_unreadable_not_as_six_dead_mvs() {
        let all_missing: Vec<MvReport> = ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|n| missing(n))
            .collect();
        let m = drift_metrics(&all_missing, 0);
        assert_eq!(value_of(&m, MV_DRIFT_UNREADABLE_METRIC), 1.0);
        assert_eq!(
            value_of(&m, MV_DRIFT_METRIC),
            0.0,
            "must not page as if six MVs were deleted"
        );
        assert_eq!(value_of(&m, MV_DRIFT_CRITICAL_METRIC), 0.0);
    }

    /// The other half of that discriminator: if the schema IS visible and the
    /// MVs are still gone, they really are gone and the ordinary alarm is right.
    #[test]
    fn missing_mvs_in_a_visible_schema_really_are_missing() {
        let all_missing: Vec<MvReport> = ["a", "b", "c", "d", "e", "f"]
            .iter()
            .map(|n| missing(n))
            .collect();
        let m = drift_metrics(&all_missing, 32);
        assert_eq!(value_of(&m, MV_DRIFT_METRIC), 6.0);
        assert_eq!(value_of(&m, MV_DRIFT_UNREADABLE_METRIC), 0.0);
    }

    #[test]
    fn the_visible_objects_query_is_scoped_to_the_probes_database() {
        assert_eq!(
            visible_objects_query("prices"),
            "SELECT count() FROM system.tables WHERE database = 'prices'"
        );
    }

    #[test]
    fn describe_names_the_offenders_and_their_shape() {
        let s = describe(&[in_sync("ok"), lost_append("bad"), drifted("odd")]);
        assert!(s.contains("bad=not-append"));
        assert!(s.contains("odd=drifted(select body)"));
        assert!(!s.contains("ok="), "in-sync MVs are noise in an alarm log");
    }

    #[test]
    fn describe_says_so_when_everything_is_in_sync() {
        assert_eq!(describe(&[in_sync("a")]), "all in sync");
    }
}
