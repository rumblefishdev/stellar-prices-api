//! Rollup MVs stuck behind a dependency, or switched off (task 0203, task 0143).
//!
//! Since task 0143 the rollup chain runs in dependency order: every fast MV but
//! `mv_ohlcv_1m_to_15m`, and every reconciliation MV but
//! `mv_reconcile_1m_to_15m`, is declared `DEPENDS ON` the MV that writes its
//! source tier. The cost of that ordering is a new silent failure, verified on
//! ClickHouse 26.3.10.60 (BRIEF §2): a dependency that is **stopped, failing
//! after its retries, or missing** leaves every dependent in
//! `system.view_refreshes.status = 'WaitingForDependencies'` **forever, with no
//! error**. Nothing else sees it — the freshness alarm only notices once the
//! tip of a coarse tier ages past its bound, which for `1w`/`1M` takes days,
//! and a stuck *reconciliation* MV never ages any tip at all.
//!
//! This module turns `system.view_refreshes` into three `Prices/Rollup`
//! counts over the twelve views the generator declares
//! ([`prices_clickhouse::rollup_sql::rollup_views`]):
//!
//! - **[`MV_REFRESH_WAITING_METRIC`]** — views in `WaitingForDependencies` for
//!   longer than their **own** refresh period. A dependent waits briefly on
//!   every slot while its dependency runs; only a wait that outlives a whole
//!   period means the dependency did not run for that slot. While waiting,
//!   `next_refresh_time` stays at the slot being waited for, so the wait age is
//!   `now - next_refresh_time`.
//! - **[`MV_REFRESH_DISABLED_METRIC`]** — views that were `SYSTEM STOP VIEW`ed.
//!   The re-ingest runbooks STOP the six reconciliation MVs on purpose; a STOP
//!   that is never undone is otherwise silent (and a STOPped view makes its
//!   dependents wait, which the waiting count reports separately so that a
//!   deliberate STOP cannot mask a real dependency stall, nor the reverse).
//! - **[`MV_REFRESH_UNREADABLE_METRIC`]** — `1` when the probe could not read
//!   the table, in which case the two counts are **not published at all**.
//!
//! ## ⚠️ `system.view_refreshes` is DENIED, not filtered
//!
//! Unlike `system.tables` (grant-filtered — see [`crate::mv_drift`]), a user
//! holding only `GRANT SELECT ON prices.*` gets `Code: 497 … Not enough
//! privileges … SELECT ON system.view_refreshes. (ACCESS_DENIED)` for this
//! read (measured on 26.3.10.60). The probe maps that error to
//! [`unreadable_metrics`] — **never to 0**: a zero would be a healthy reading
//! of a table the probe cannot see, the false-OK this crate keeps refusing.
//! Any other read error is an ordinary check failure.
//!
//! Whether production's probe identity (`prices_writer`, XML-managed on BE's
//! side — task 0477 per runbook 0100, against the older comment at
//! `current_prices.rs` that it holds only `SELECT ON prices.*`) carries the
//! grant is a **rollout-checklist item** (`SHOW GRANTS FOR prices_writer`),
//! not something code can settle: until it does, this check publishes
//! unreadable and its alarm says so.
//!
//! ## Seeing none of the twelve is also unreadable
//!
//! A readable table that lists none of the declared views does not mean the
//! chain is healthy — it means the probe is blind to it (a schema that was
//! never applied, a narrowed row policy). Same reasoning as
//! [`crate::mv_drift`]'s `visible_objects == 0`: unreadable, counts suppressed.

use crate::mv_drift::DriftMetric;
use prices_clickhouse::rollup_sql::rollup_views;

/// Count of declared rollup/reconcile MVs waiting on a dependency for longer
/// than their own period. Watched by `prices-{env}-mv-refresh-waiting`.
pub const MV_REFRESH_WAITING_METRIC: &str = "MvRefreshWaitingCount";

/// Count of declared rollup/reconcile MVs that are `SYSTEM STOP VIEW`ed.
/// Watched by `prices-{env}-mv-refresh-disabled`.
pub const MV_REFRESH_DISABLED_METRIC: &str = "MvRefreshDisabledCount";

/// `1` when `system.view_refreshes` could not be read or showed none of the
/// declared views, so the two counts above were not published. Watched by
/// `prices-{env}-mv-refresh-unreadable`.
pub const MV_REFRESH_UNREADABLE_METRIC: &str = "MvRefreshUnreadable";

/// `system.view_refreshes.status` of a dependent whose dependency has not
/// refreshed for the slot it is waiting on.
const WAITING: &str = "WaitingForDependencies";

/// `system.view_refreshes.status` of a `SYSTEM STOP VIEW`ed view.
const DISABLED: &str = "Disabled";

/// One declared view's refresh state, as [`refresh_waits_query`] reads it.
///
/// Both times come from the SAME server clock (`db_now_unix` is `now()` in the
/// same query), so the wait age is immune to Lambda/ClickHouse clock skew — the
/// same reason [`crate::freshness_query`] computes its lag server-side.
#[derive(Debug, Clone, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct ViewRefreshRow {
    pub view: String,
    pub status: String,
    /// `next_refresh_time`, or `now()` when ClickHouse reports none.
    pub next_refresh_unix: i64,
    /// The server's `now()` at the time of the read.
    pub db_now_unix: i64,
}

/// Read the refresh state of the twelve declared views in `database`.
///
/// `database` is the probe's constant `prices` or a test's scratch name; the
/// `IN (…)` list is rendered from [`rollup_views`] and nothing else, so no
/// caller-supplied text reaches the view names.
pub fn refresh_waits_query(database: &str) -> String {
    let names = rollup_views()
        .iter()
        .map(|(name, _)| format!("'{name}'"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT view, toString(status) AS status, \
         toInt64(toUnixTimestamp(ifNull(next_refresh_time, now()))) AS next_refresh_unix, \
         toInt64(toUnixTimestamp(now())) AS db_now_unix \
         FROM system.view_refreshes \
         WHERE database = '{database}' AND view IN ({names}) \
         ORDER BY view"
    )
}

/// Classify the rows of [`refresh_waits_query`] into the three metrics.
///
/// - a `WaitingForDependencies` view counts as waiting only when
///   `now_unix - next_refresh_unix` is **greater than** its own period
///   (exactly one period is still an ordinary wait);
/// - a `Disabled` view counts as disabled, and only there;
/// - `Scheduled` / `Running` (and any other status) count as neither;
/// - rows for views outside [`rollup_views`] are ignored;
/// - no declared view among the rows → only [`MV_REFRESH_UNREADABLE_METRIC`]
///   `= 1` (see the module docs).
///
/// `now_unix` is the server clock of the same read ([`ViewRefreshRow::
/// db_now_unix`]) in production. A test that drives the scheduler with
/// `SYSTEM TEST VIEW … SET FAKE TIME` passes the FAKE clock instead: fake time
/// moves only the view's scheduler, not `now()`, whereas a real stall reaches
/// the same state simply by waiting out a period of wall time.
pub fn refresh_wait_metrics(rows: &[ViewRefreshRow], now_unix: i64) -> Vec<DriftMetric> {
    let views = rollup_views();
    let mut seen = 0usize;
    let mut waiting = 0usize;
    let mut disabled = 0usize;

    for row in rows {
        let Some(&(_, period)) = views.iter().find(|(name, _)| *name == row.view) else {
            continue;
        };
        seen += 1;
        match row.status.as_str() {
            WAITING if now_unix - row.next_refresh_unix > period as i64 => waiting += 1,
            DISABLED => disabled += 1,
            _ => {}
        }
    }

    if seen == 0 {
        return unreadable_metrics();
    }

    vec![
        DriftMetric {
            name: MV_REFRESH_UNREADABLE_METRIC,
            value: 0.0,
        },
        DriftMetric {
            name: MV_REFRESH_WAITING_METRIC,
            value: waiting as f64,
        },
        DriftMetric {
            name: MV_REFRESH_DISABLED_METRIC,
            value: disabled as f64,
        },
    ]
}

/// Whether a read error is ClickHouse refusing the grant (Code 497,
/// `ACCESS_DENIED`) — the one error this check turns into
/// [`unreadable_metrics`] instead of a failure.
pub fn is_access_denied(error_text: &str) -> bool {
    error_text.contains("ACCESS_DENIED") || error_text.contains("Code: 497")
}

/// What an unreadable `system.view_refreshes` publishes: the unreadable flag
/// and NOTHING else — no waiting or disabled count, because a zero would read
/// as a healthy chain.
pub fn unreadable_metrics() -> Vec<DriftMetric> {
    vec![DriftMetric {
        name: MV_REFRESH_UNREADABLE_METRIC,
        value: 1.0,
    }]
}

/// The whole outcome of one [`refresh_waits_query`] read, as the probe
/// publishes it — the ONE place the three branches live, so the IT exercises
/// exactly what `main.rs` does:
///
/// - rows → [`refresh_wait_metrics`] against the server clock of the same read
///   (no rows → unreadable, see the module docs);
/// - a grant refusal ([`is_access_denied`]) → [`unreadable_metrics`], and the
///   read is NOT a failure: the unreadable alarm is the signal;
/// - any other error → returned, for the probe to record as a failed check.
pub fn metrics_for_read<E: std::fmt::Display>(
    read: Result<Vec<ViewRefreshRow>, E>,
) -> Result<Vec<DriftMetric>, E> {
    match read {
        Ok(rows) => {
            let now = rows.first().map(|r| r.db_now_unix).unwrap_or_default();
            Ok(refresh_wait_metrics(&rows, now))
        }
        Err(e) if is_access_denied(&e.to_string()) => Ok(unreadable_metrics()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_790_000_000;

    fn row(view: &str, status: &str, next_refresh_unix: i64) -> ViewRefreshRow {
        ViewRefreshRow {
            view: view.into(),
            status: status.into(),
            next_refresh_unix,
            db_now_unix: NOW,
        }
    }

    fn value_of(metrics: &[DriftMetric], name: &str) -> Option<f64> {
        metrics.iter().find(|m| m.name == name).map(|m| m.value)
    }

    /// A healthy chain: every declared view scheduled.
    fn all_scheduled() -> Vec<ViewRefreshRow> {
        rollup_views()
            .iter()
            .map(|(name, period)| row(name, "Scheduled", NOW + *period as i64))
            .collect()
    }

    fn with(
        mut rows: Vec<ViewRefreshRow>,
        view: &str,
        status: &str,
        next: i64,
    ) -> Vec<ViewRefreshRow> {
        let r = rows
            .iter_mut()
            .find(|r| r.view == view)
            .expect("a declared view");
        r.status = status.into();
        r.next_refresh_unix = next;
        rows
    }

    #[test]
    fn a_healthy_chain_publishes_zero_waiting_zero_disabled_and_readable() {
        let m = refresh_wait_metrics(&all_scheduled(), NOW);
        assert_eq!(value_of(&m, MV_REFRESH_UNREADABLE_METRIC), Some(0.0));
        assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), Some(0.0));
        assert_eq!(value_of(&m, MV_REFRESH_DISABLED_METRIC), Some(0.0));
    }

    /// The daily MV's period is 4 h; a wait of one second more is stuck, a
    /// wait of exactly the period is an ordinary slot.
    #[test]
    fn a_wait_longer_than_the_views_own_period_counts_and_exactly_one_period_does_not() {
        let stuck = with(
            all_scheduled(),
            "mv_ohlcv_4h_to_1d",
            WAITING,
            NOW - 14_400 - 1,
        );
        let m = refresh_wait_metrics(&stuck, NOW);
        assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), Some(1.0));

        let boundary = with(all_scheduled(), "mv_ohlcv_4h_to_1d", WAITING, NOW - 14_400);
        let m = refresh_wait_metrics(&boundary, NOW);
        assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), Some(0.0));
    }

    /// Each view is judged by ITS period: 61 s of waiting is stuck for the
    /// 1-minute MV and nothing for an hourly reconcile MV.
    #[test]
    fn the_threshold_is_each_views_own_period() {
        let rows = with(
            with(all_scheduled(), "mv_ohlcv_1m_to_15m", WAITING, NOW - 61),
            "mv_reconcile_15m_to_1h",
            WAITING,
            NOW - 61,
        );
        let m = refresh_wait_metrics(&rows, NOW);
        assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), Some(1.0));
    }

    #[test]
    fn scheduled_and_running_views_never_count_as_waiting() {
        let rows = with(
            with(
                all_scheduled(),
                "mv_ohlcv_1h_to_4h",
                "Running",
                NOW - 999_999,
            ),
            "mv_ohlcv_1d_to_1w",
            "Scheduled",
            NOW - 999_999,
        );
        let m = refresh_wait_metrics(&rows, NOW);
        assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), Some(0.0));
        assert_eq!(value_of(&m, MV_REFRESH_DISABLED_METRIC), Some(0.0));
    }

    /// A STOPped view counts once, as disabled — not as waiting, however old
    /// its slot.
    #[test]
    fn a_disabled_view_counts_only_as_disabled() {
        let rows = with(
            all_scheduled(),
            "mv_reconcile_1d_to_1w",
            DISABLED,
            NOW - 999_999,
        );
        let m = refresh_wait_metrics(&rows, NOW);
        assert_eq!(value_of(&m, MV_REFRESH_DISABLED_METRIC), Some(1.0));
        assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), Some(0.0));
        assert_eq!(value_of(&m, MV_REFRESH_UNREADABLE_METRIC), Some(0.0));
    }

    #[test]
    fn views_outside_the_declared_twelve_are_ignored() {
        let mut rows = all_scheduled();
        rows.push(row("mv_current_prices", WAITING, NOW - 999_999));
        rows.push(row("mv_someone_elses", DISABLED, NOW));
        let m = refresh_wait_metrics(&rows, NOW);
        assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), Some(0.0));
        assert_eq!(value_of(&m, MV_REFRESH_DISABLED_METRIC), Some(0.0));
    }

    /// Blind is not healthy: none of the twelve visible → only the unreadable
    /// flag, and no count that could read as a clean chain.
    #[test]
    fn no_declared_view_among_the_rows_publishes_only_unreadable() {
        for rows in [vec![], vec![row("mv_current_prices", "Scheduled", NOW)]] {
            let m = refresh_wait_metrics(&rows, NOW);
            assert_eq!(
                m,
                vec![DriftMetric {
                    name: MV_REFRESH_UNREADABLE_METRIC,
                    value: 1.0
                }]
            );
        }
    }

    #[test]
    fn the_unreadable_shape_carries_no_count() {
        assert_eq!(
            unreadable_metrics(),
            vec![DriftMetric {
                name: MV_REFRESH_UNREADABLE_METRIC,
                value: 1.0
            }]
        );
    }

    /// The exact 26.3.10.60 text for a `SELECT ON prices.*` user, and the
    /// errors that must stay ordinary failures.
    #[test]
    fn only_the_grant_refusal_reads_as_access_denied() {
        let denied = "bad response: Code: 497. DB::Exception: rollup_probe_it: Not enough \
                      privileges. To execute this query, it's necessary to have the grant \
                      SELECT ON system.view_refreshes. (ACCESS_DENIED) (version 26.3.10.60 \
                      (official build))";
        assert!(is_access_denied(denied));
        assert!(!is_access_denied(
            "bad response: Code: 159. DB::Exception: Timeout exceeded: elapsed 10.0 seconds, \
             maximum: 10. (TIMEOUT_EXCEEDED)"
        ));
        assert!(!is_access_denied(
            "network error: error trying to connect: tcp connect error: Connection refused \
             (os error 111)"
        ));
    }

    /// `main.rs`'s three branches: rows are classified on the server clock, a
    /// refusal is the unreadable flag alone, anything else stays an error.
    #[test]
    fn a_read_maps_to_counts_a_refusal_to_unreadable_and_other_errors_through() {
        let stuck = with(all_scheduled(), "mv_ohlcv_4h_to_1d", WAITING, NOW - 14_401);
        let m = metrics_for_read::<String>(Ok(stuck)).expect("rows publish");
        assert_eq!(value_of(&m, MV_REFRESH_WAITING_METRIC), Some(1.0));

        let m = metrics_for_read::<String>(Ok(vec![])).expect("no rows publish");
        assert_eq!(m, unreadable_metrics());

        let m = metrics_for_read(Err("Code: 497. DB::Exception: … (ACCESS_DENIED)"))
            .expect("a refusal publishes");
        assert_eq!(m, unreadable_metrics());

        assert_eq!(
            metrics_for_read(Err("Code: 159. Timeout exceeded (TIMEOUT_EXCEEDED)")),
            Err("Code: 159. Timeout exceeded (TIMEOUT_EXCEEDED)")
        );
    }

    /// The names come from the generator, and the query is scoped to one
    /// database and reads both clocks from the server.
    #[test]
    fn the_query_lists_exactly_the_declared_views_in_one_database() {
        let sql = refresh_waits_query("prices");
        assert!(sql.contains("FROM system.view_refreshes"));
        assert!(sql.contains("WHERE database = 'prices' AND view IN ("));
        assert!(sql.contains("ifNull(next_refresh_time, now())"));
        assert!(sql.contains("toUnixTimestamp(now())) AS db_now_unix"));
        for (name, _) in rollup_views() {
            assert_eq!(sql.matches(&format!("'{name}'")).count(), 1, "{name}");
        }
        assert_eq!(sql.matches("'mv_").count(), 12);
    }
}
