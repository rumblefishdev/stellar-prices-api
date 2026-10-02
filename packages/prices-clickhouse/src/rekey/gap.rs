//! Task 0139 after the swap: `verify`, `gap-backfill`, `gap-verify`, and the
//! post-check SQL the runbook and the agent run.
//!
//! The writer-stop gap runs from `last_live_1m_ts` (logged by `swap`) through
//! the catch-up after writers resume. Catch-up writes 1m rows with their
//! ledger timestamps, and `mv_ohlcv_1m_to_15m` re-reads only the last 2 h, so
//! older catch-up rows never reach 15m (or anything above it). Prod has no
//! `mv_reconcile_*` to repair that. `gap-backfill` re-aggregates every tier
//! fine to coarse over `[last_live_1m_ts - 2 h, now)` with the generator's
//! bounded INSERT, the statements `schema/preroll-live-gap.sql` holds, on the
//! swapped tables only. Colliding blends and orphans stay out, as the copy
//! left them.
//!
//! The checks compare a tier with its child from the gap's first bucket up to
//! the buckets that have closed and been refreshed since (`settled_bound`),
//! and fail on an empty range: a gap inside one day, week or month is checked
//! once that bucket settles, never passed unchecked.

use std::time::Duration;

use super::swap::uint32_ids_where;
use super::*;
use crate::rollup_sql::{Bound, Bounds, TIERS, Tier, rollup_insert};

/// The three post-check sets of the runbook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Postcheck {
    /// After W14, the window session.
    Window,
    /// W16, the next day: the window set plus 4h and 1d.
    NextDay,
    /// W17, once the gap's week and month have closed and settled: 1w, 1M.
    PeriodClose,
    /// Each month task 13 finishes; takes `{m:UInt32}` (YYYYMM).
    Month,
}

impl Postcheck {
    /// The runbook marker name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::NextDay => "next-day",
            Self::PeriodClose => "period-close",
            Self::Month => "month",
        }
    }

    /// The set named `name` (the runbook marker, `gap-verify --set`).
    pub fn parse(name: &str) -> Option<Self> {
        [Self::Window, Self::NextDay, Self::PeriodClose, Self::Month]
            .into_iter()
            .find(|s| s.name() == name)
    }

    /// The tiers whose gap the set checks.
    pub fn gap_tiers(self) -> &'static [Tier] {
        match self {
            Self::Window => &TIERS[..2],
            Self::NextDay => &TIERS[..4],
            Self::PeriodClose => &TIERS[4..],
            Self::Month => &[],
        }
    }

    /// The fewest lines the set may render: the Task 9, 12 and 13 guards.
    pub fn min_lines(self) -> usize {
        match self {
            Self::Window => 6,
            Self::NextDay => 8,
            Self::PeriodClose => 2,
            Self::Month => 3,
        }
    }
}

const GAP_ROW: &str = "step = 'gap-backfill' AND status = 'ok'";

/// The gap's bound `col` (`range_from` or `range_to`) of the last
/// `gap-backfill`, aligned to `interval`.
fn gap_bound(db: &str, col: &str, interval: &str) -> String {
    format!(
        "toStartOfInterval((SELECT {col} FROM {db}.{LOG_TABLE} WHERE {GAP_ROW} \
         ORDER BY at DESC LIMIT 1), {interval})"
    )
}

/// The end of the buckets of `tier` that have settled: closed, their child
/// refreshed past them, then the tier itself refreshed, with 15 minutes for
/// the refresh to run. 15m 16 min, 1h 31 min, 4h 1 h 30, 1d 5 h 15, 1w and
/// 1M 28 h 15.
pub fn settled_bound(tier: &Tier) -> String {
    let child = TIERS
        .iter()
        .find(|t| t.target == tier.child)
        .map_or(0, |t| t.refresh_seconds);
    format!(
        "toStartOfInterval(now() - INTERVAL {} SECOND, {})",
        tier.refresh_seconds + child + 900,
        tier.interval
    )
}

/// One post-check line: `ifNull`, because a scalar subquery is Nullable
/// and a NULL must read as a failure, not as RowBinary's null flag.
fn check(name: &str, expr: &str, from: &str) -> (String, String) {
    (
        name.to_string(),
        format!("SELECT ifNull({expr}, 0) AS {name}{from}"),
    )
}

/// `sum(trade_count), sum(volume_base)` of `tier` and of its child agree from
/// the last `gap-backfill`'s first bucket to the settled ones; false if none
/// ran or no bucket of the gap has settled yet.
fn gap_totals(db: &str, tier: &Tier) -> (String, String) {
    let (lb, ub) = (
        gap_bound(db, "range_from", tier.interval),
        settled_bound(tier),
    );
    let totals = |t: &str| {
        format!(
            "(SELECT (sum(trade_count), sum(volume_base)) FROM {db}.{t} FINAL \
             WHERE timestamp >= {lb} AND timestamp < {ub})"
        )
    };
    check(
        &format!("gap_{}", tier.name),
        &format!(
            "(SELECT count() FROM {db}.{LOG_TABLE} WHERE {GAP_ROW}) > 0 AND {lb} < {ub} AND {} = {}",
            totals(tier.target),
            totals(tier.child)
        ),
        "",
    )
}

/// The post-check SELECTs of `set`: one line each, `1` on pass, no
/// `SETTINGS` (dev_read is readonly). Gap bounds come from the log and
/// `settled_bound`.
pub fn postcheck_sql(set: Postcheck, db: &str) -> Vec<(String, String)> {
    let gaps = |tiers: &[Tier]| tiers.iter().map(|t| gap_totals(db, t)).collect::<Vec<_>>();
    let assets_unique = check(
        "assets_unique",
        "count() = uniqExact(asset_id)",
        &format!(" FROM {db}.assets FINAL"),
    );
    let window = || {
        let mut v = vec![
            assets_unique.clone(),
            check(
                "no_uint32_ids",
                "count() = 0",
                &format!(" FROM system.columns WHERE {}", uint32_ids_where(db)),
            ),
            check(
                "current_price_usd_one_row",
                &format!(
                    "(SELECT count() FROM {db}.current_price_usd) = \
                     (SELECT count() FROM {db}.current_prices FINAL)"
                ),
                "",
            ),
            check(
                "cross_check_0129",
                &format!(
                    "(SELECT count() FROM {db}.price_ohlcv_1h WHERE toYYYYMM(timestamp) \
                     BETWEEN 202402 AND 202607 AND volume_quote > 0) = (SELECT count() FROM \
                     {db}.price_ohlcv_1h AS c INNER JOIN {db}.assets AS a FINAL \
                     ON a.asset_id = c.quote_asset_id WHERE toYYYYMM(c.timestamp) \
                     BETWEEN 202402 AND 202607 AND c.volume_quote > 0)"
                ),
                "",
            ),
        ];
        v.extend(gaps(Postcheck::Window.gap_tiers()));
        v
    };
    match set {
        Postcheck::Window => window(),
        Postcheck::NextDay => {
            let mut v = window();
            v.extend(gaps(&TIERS[2..4]));
            v
        }
        Postcheck::PeriodClose => gaps(set.gap_tiers()),
        Postcheck::Month => {
            // A new id that some clean old id also maps to already has copied
            // rows, so it cannot show the colliding rows came back.
            let colliding = format!(
                "SELECT new_id FROM {db}.{MAP_TABLE} GROUP BY new_id \
                 HAVING countIf(status = '{STATUS_COLLIDING}') > 0 \
                 AND countIf(status IN ('{STATUS_MAPPED}', '{STATUS_SENTINEL}')) = 0"
            );
            vec![
                assets_unique,
                check(
                    "colliding_restored",
                    &format!(
                        "(SELECT count() FROM {db}.{MONTHS_TABLE} WHERE month = {{m:UInt32}} \
                         AND colliding_rows > 0) = 0 OR (SELECT count() FROM {db}.price_ohlcv_1m \
                         WHERE toYYYYMM(timestamp) = {{m:UInt32}} AND (asset_id IN ({colliding}) \
                         OR quote_asset_id IN ({colliding}))) > 0"
                    ),
                    "",
                ),
                check(
                    "month_1d_equals_1m",
                    &format!(
                        "(SELECT sum(trade_count) FROM {db}.price_ohlcv_1d FINAL \
                         WHERE toYYYYMM(timestamp) = {{m:UInt32}}) = (SELECT sum(trade_count) \
                         FROM {db}.price_ohlcv_1m FINAL WHERE toYYYYMM(timestamp) = {{m:UInt32}})"
                    ),
                    "",
                ),
            ]
        }
    }
}

/// The six bounded rollup INSERTs over `[from, to)`, fine to coarse: the
/// statements of `schema/preroll-live-gap.sql` when the bounds are its two
/// parameters on `prices`.
pub fn gap_backfill_sql(
    db: &str,
    from: Bound<'_>,
    to: Bound<'_>,
) -> std::result::Result<Vec<String>, crate::rollup_sql::RollupSqlError> {
    TIERS
        .iter()
        .map(|t| rollup_insert(t, db, &Bounds::Range { from, to }, Some("max_threads = 4")))
        .collect()
}

/// Per `(asset_id, quote_asset_id, source, bucket)` of `tier` over the closed
/// buckets of `[lb, ub)`: `trade_count` and `volume_base` disagree with the
/// child, or one side is missing. One row, the count.
pub fn gap_mismatch_sql(db: &str, tier: &Tier, from: &str, to: &str) -> String {
    let lb = format!(
        "toStartOfInterval(toDateTime('{from}', 'UTC'), {})",
        tier.interval
    );
    let ub = format!(
        "toStartOfInterval(toDateTime('{to}', 'UTC'), {})",
        tier.interval
    );
    format!(
        "SELECT count() FROM (SELECT toDateTime(toStartOfInterval(timestamp, {i})) AS b, \
         asset_id, quote_asset_id, source, sum(trade_count) AS tc, sum(volume_base) AS vb \
         FROM {db}.{child} FINAL WHERE timestamp >= {lb} AND timestamp < {ub} \
         GROUP BY b, asset_id, quote_asset_id, source) AS c FULL OUTER JOIN \
         (SELECT toDateTime(timestamp) AS b, asset_id, quote_asset_id, source, \
         trade_count AS tc, volume_base AS vb FROM {db}.{target} FINAL \
         WHERE timestamp >= {lb} AND timestamp < {ub}) AS p \
         USING (b, asset_id, quote_asset_id, source) WHERE c.tc != p.tc OR c.vb != p.vb",
        i = tier.interval,
        child = tier.child,
        target = tier.target,
    )
}

/// `YYYY-MM-DD HH:MM:SS`, the only shape a gap bound may take.
pub fn is_instant(s: &str) -> bool {
    s.len() == 19
        && s.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            10 => b == b' ',
            13 | 16 => b == b':',
            _ => b.is_ascii_digit(),
        })
}

impl Rekey {
    async fn instant(&self, unix: u32) -> Result<String> {
        Ok(self
            .client
            .query("SELECT formatDateTime(toDateTime(?, 'UTC'), '%Y-%m-%d %H:%i:%S', 'UTC')")
            .bind(unix)
            .fetch_one()
            .await?)
    }

    async fn unix(&self, instant: &str) -> Result<u32> {
        Ok(self
            .client
            .query("SELECT toUInt32(toDateTime(?, 'UTC'))")
            .bind(instant)
            .fetch_one()
            .await?)
    }

    /// `(last_live_1m_ts, swap_at)` of a completed swap; refused otherwise.
    async fn swapped(&self) -> Result<(u32, u32)> {
        match self.last_logged("swap").await? {
            Some((_, s, from, to)) if s == "ok" => Ok((from, to)),
            _ => Err(RekeyError::Refused("swap has not run".into())),
        }
    }

    /// Every post-check of `set` returns 1; `m` for the month set.
    pub async fn postcheck(&self, set: Postcheck, m: Option<u32>) -> Result<Vec<String>> {
        let mut lines = Vec::new();
        let mut failed = Vec::new();
        for (name, sql) in postcheck_sql(set, &self.db) {
            let mut q = self.client.query(&sql);
            if let Some(m) = m {
                q = q.param("m", m);
            }
            let ok: u8 = q.fetch_one().await?;
            lines.push(format!("{name}: {ok}"));
            if ok != 1 {
                failed.push(name);
            }
        }
        if failed.is_empty() {
            Ok(lines)
        } else {
            Err(RekeyError::Gate(format!(
                "post-check {}: {} failed\n{}",
                set.name(),
                failed.join(", "),
                lines.join("\n")
            )))
        }
    }

    /// W12: the window post-checks that need no gap, the type gate, no id
    /// outside `assets` in any swapped table, XLM/USDC resolve in
    /// `usd_reference_1h`, and every MV refreshed since the swap.
    pub async fn verify(&self) -> Result<Vec<String>> {
        let (_, swap_at) = self.swapped().await?;
        let mut lines = Vec::new();
        let mut bad = Vec::new();
        if let Err(e) = self.type_gate().await {
            bad.push(e.to_string());
        }
        for (name, sql) in postcheck_sql(Postcheck::Window, &self.db)
            .into_iter()
            .filter(|(n, _)| !n.starts_with("gap_"))
        {
            let ok: u8 = self.client.query(&sql).fetch_one().await?;
            lines.push(format!("{name}: {ok}"));
            if ok != 1 {
                bad.push(name);
            }
        }
        for t in COPIED_TABLES {
            for c in id_columns(t) {
                let sentinel = if t == "oracle_prices" {
                    " AND s.asset_id != 0"
                } else {
                    ""
                };
                let orphans = self
                    .count(&format!(
                        "SELECT count() FROM {} AS s WHERE s.{c} NOT IN \
                         (SELECT asset_id FROM {}){sentinel}",
                        self.t(t),
                        self.t("assets")
                    ))
                    .await?;
                if orphans > 0 {
                    bad.push(format!("{t}.{c}: {orphans} rows on an id assets lacks"));
                }
            }
        }
        lines.push("swapped tables: every id has an assets row".into());
        let refs = self
            .count(&format!(
                "SELECT count() FROM {} WHERE bucket >= now() - INTERVAL 7 DAY",
                self.t("usd_reference_1h")
            ))
            .await?;
        lines.push(format!(
            "usd_reference_1h: {refs} XLM/USDC buckets in 7 days"
        ));
        if refs == 0 {
            bad.push("usd_reference_1h resolves no XLM/USDC bucket in 7 days".into());
        }
        let stale: Vec<String> = self
            .client
            .query(
                "SELECT view FROM system.view_refreshes WHERE database = ? \
                 AND (last_success_time IS NULL OR last_success_time < toDateTime(?)) ORDER BY view",
            )
            .bind(&self.db)
            .bind(swap_at)
            .fetch_all()
            .await?;
        if !stale.is_empty() {
            bad.push(format!(
                "no refresh since the swap: {} (SYSTEM REFRESH VIEW, then SYSTEM WAIT VIEW)",
                stale.join(", ")
            ));
        }
        if bad.is_empty() {
            Ok(lines)
        } else {
            Err(RekeyError::Gate(format!(
                "verify: {}\n{}",
                bad.join("; "),
                lines.join("\n")
            )))
        }
    }

    /// W14: re-aggregate every tier over `[last_live_1m_ts - 2 h, to)`. `to`
    /// is now, refused while the newest 1m row is older than 15 minutes
    /// (catch-up not finished) unless given; never earlier than that row.
    pub async fn gap_backfill(&self, to: Option<&str>) -> Result<Vec<String>> {
        let (last_live, _) = self.swapped().await?;
        if self.id_type("price_ohlcv_1m").await?.as_deref() != Some("UInt64") {
            return Err(RekeyError::Refused(
                "price_ohlcv_1m is not UInt64: on the old tables use schema/preroll-live-gap.sql"
                    .into(),
            ));
        }
        let tz: String = self
            .client
            .query("SELECT serverTimezone()")
            .fetch_one()
            .await?;
        if tz != "UTC" {
            return Err(RekeyError::Refused(format!(
                "server timezone is {tz}: the rollup bounds are UTC"
            )));
        }
        let (newest, now): (u32, u32) = self
            .client
            .query(&format!(
                "SELECT toUInt32(max(timestamp)), toUInt32(now()) FROM {}",
                self.t("price_ohlcv_1m")
            ))
            .fetch_one()
            .await?;
        let to_unix = match to {
            None if newest + 900 < now => {
                return Err(RekeyError::Refused(format!(
                    "the newest price_ohlcv_1m row ({}) is older than 15 minutes: catch-up \
                     is not finished (or pass --to)",
                    self.instant(newest).await?
                )));
            }
            None => now,
            Some(t) if !is_instant(t) => {
                return Err(RekeyError::Refused(format!(
                    "--to {t}: YYYY-MM-DD HH:MM:SS"
                )));
            }
            Some(t) => {
                let u = self.unix(t).await?;
                if u < newest {
                    return Err(RekeyError::Refused(format!(
                        "--to {t} is earlier than the newest price_ohlcv_1m row ({})",
                        self.instant(newest).await?
                    )));
                }
                u
            }
        };
        let from_unix = last_live.saturating_sub(7200);
        let (from, to) = (self.instant(from_unix).await?, self.instant(to_unix).await?);
        let stmts = gap_backfill_sql(&self.db, Bound::Timestamp(&from), Bound::Timestamp(&to))
            .map_err(|e| RekeyError::Refused(e.to_string()))?;
        self.ensure_tool_tables().await?;
        let mut lines = vec![format!("gap [{from}, {to}) UTC")];
        let attempt = if self.execute {
            self.count(&format!(
                "SELECT count() FROM {} WHERE step = 'gap-backfill' AND status = 'ok'",
                self.t(LOG_TABLE)
            ))
            .await?
                + 1
        } else {
            0
        };
        let start = std::time::Instant::now();
        for (tier, sql) in TIERS.iter().zip(&stmts) {
            if !self.execute {
                println!("{sql};");
                continue;
            }
            let query_id = format!("rekey0139-{}-gap-{}-{attempt}", self.db, tier.name);
            let t0 = std::time::Instant::now();
            self.client
                .query(sql)
                .with_option("query_id", &query_id)
                .execute()
                .await?;
            let written = written_rows(&self.client, &query_id, 5, Duration::from_secs(2))
                .await?
                .unwrap_or(0);
            let took = format!("{:.1}s", t0.elapsed().as_secs_f64());
            self.log(Log {
                step: "gap-backfill",
                target: tier.target,
                status: "tier",
                query_id: &query_id,
                written,
                detail: &took,
                range_from: from_unix,
                range_to: to_unix,
                ..Log::default()
            })
            .await?;
            lines.push(format!("{}: {written} rows in {took}", tier.target));
        }
        let took = format!("{:.1}s", start.elapsed().as_secs_f64());
        self.log(Log {
            step: "gap-backfill",
            status: "ok",
            detail: &took,
            range_from: from_unix,
            range_to: to_unix,
            ..Log::default()
        })
        .await?;
        lines.push(format!("total {took}"));
        Ok(lines)
    }

    /// Read-only. Per tier of `set` against its child, from the gap's first
    /// bucket to the settled ones, every key's `trade_count` and
    /// `volume_base` agree; a tier with no settled bucket fails. Then the
    /// set's gap post-checks. With explicit bounds (`pre0139`: the restored
    /// UInt32 tables after a rollback) every tier over those bounds instead.
    pub async fn gap_verify(
        &self,
        pre0139: bool,
        bounds: Option<(&str, &str)>,
        set: Postcheck,
    ) -> Result<Vec<String>> {
        let want = if pre0139 { "UInt32" } else { "UInt64" };
        if self.id_type("price_ohlcv_1m").await?.as_deref() != Some(want) {
            return Err(RekeyError::Refused(format!(
                "price_ohlcv_1m is not {want} (--schema pre0139 is for the restored tables)"
            )));
        }
        let (from, to) = match bounds {
            Some((f, t)) if is_instant(f) && is_instant(t) => (f.to_string(), t.to_string()),
            Some((f, t)) => {
                return Err(RekeyError::Refused(format!(
                    "--from {f} --to {t}: YYYY-MM-DD HH:MM:SS"
                )));
            }
            None if pre0139 => {
                return Err(RekeyError::Refused(
                    "--schema pre0139 needs --from and --to".into(),
                ));
            }
            None => match self.last_logged("gap-backfill").await? {
                Some((_, s, f, t)) if s == "ok" => (self.instant(f).await?, self.instant(t).await?),
                _ => {
                    return Err(RekeyError::Refused(
                        "no gap-backfill logged (or pass --from and --to)".into(),
                    ));
                }
            },
        };
        let mut lines = vec![format!("gap [{from}, {to}) UTC")];
        let mut bad = Vec::new();
        let ranges: Vec<(&Tier, String, String)> = if bounds.is_some() {
            TIERS
                .iter()
                .map(|t| (t, from.clone(), to.clone()))
                .collect()
        } else {
            let mut v = Vec::new();
            for t in set.gap_tiers() {
                let instant = |e: String| {
                    format!("formatDateTime(toDateTime({e}, 'UTC'), '%Y-%m-%d %H:%i:%S', 'UTC')")
                };
                let (lb, ub): (String, String) = self
                    .client
                    .query(&format!(
                        "SELECT {}, {}",
                        instant(format!(
                            "toStartOfInterval(toDateTime('{from}', 'UTC'), {})",
                            t.interval
                        )),
                        instant(settled_bound(t))
                    ))
                    .fetch_one()
                    .await?;
                v.push((t, lb, ub));
            }
            v
        };
        for (tier, lb, ub) in ranges {
            if bounds.is_none() && lb >= ub {
                lines.push(format!(
                    "{}: no bucket of the gap has settled yet [{lb}, {ub})",
                    tier.target
                ));
                bad.push(format!("{}: not settled", tier.target));
                continue;
            }
            let n = self
                .count(&gap_mismatch_sql(&self.db, tier, &lb, &ub))
                .await?;
            lines.push(format!(
                "{} vs {} [{lb}, {ub}): {n} mismatched buckets",
                tier.target, tier.child
            ));
            if n > 0 {
                bad.push(format!("{}: {n}", tier.target));
            }
        }
        if !pre0139 && bounds.is_none() {
            for (name, sql) in postcheck_sql(set, &self.db)
                .into_iter()
                .filter(|(n, _)| n.starts_with("gap_"))
            {
                let ok: u8 = self.client.query(&sql).fetch_one().await?;
                lines.push(format!("{name}: {ok}"));
                if ok != 1 {
                    bad.push(name);
                }
            }
        }
        if bad.is_empty() {
            Ok(lines)
        } else {
            Err(RekeyError::Gate(format!(
                "gap-verify: {}\n{}",
                bad.join(", "),
                lines.join("\n")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUNBOOK: &str = include_str!("../../../../docs/runbooks/0139-asset-id-migration.md");

    fn squash(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn the_gap_backfill_is_the_live_gap_preroll_of_the_generator() {
        let ours =
            gap_backfill_sql("prices", Bound::Param("start_ts"), Bound::Param("end_ts")).unwrap();
        let shipped = crate::split_statements(crate::PREROLL_LIVE_GAP_SQL);
        assert_eq!(ours.len(), 6);
        assert_eq!(
            ours.iter().map(|s| squash(s)).collect::<Vec<_>>(),
            shipped.iter().map(|s| squash(s)).collect::<Vec<_>>()
        );
        let lit = gap_backfill_sql(
            "d",
            Bound::Timestamp("2026-10-01 00:00:00"),
            Bound::Timestamp("2026-10-01 05:00:00"),
        )
        .unwrap();
        assert!(lit[0].starts_with("INSERT INTO d.price_ohlcv_15m"));
        assert!(lit[5].starts_with("INSERT INTO d.price_ohlcv_1M"));
        assert!(gap_backfill_sql("d", Bound::Timestamp("now()"), Bound::Param("e")).is_err());
    }

    #[test]
    fn post_checks_are_one_line_selects_without_settings() {
        for set in [
            Postcheck::Window,
            Postcheck::NextDay,
            Postcheck::PeriodClose,
            Postcheck::Month,
        ] {
            let v = postcheck_sql(set, "prices");
            assert!(v.len() >= set.min_lines(), "{set:?}");
            for (name, sql) in &v {
                assert!(sql.starts_with("SELECT "), "{name}");
                assert!(!sql.contains('\n'), "{name} is one line");
                assert!(!sql.to_uppercase().contains("SETTINGS"), "{name}");
                assert!(sql.starts_with("SELECT ifNull("), "{name} is never NULL");
                assert!(sql.contains(&format!(", 0) AS {name}")), "{name}");
            }
        }
        let month = postcheck_sql(Postcheck::Month, "prices");
        assert!(
            month
                .iter()
                .all(|(_, s)| !s.contains("{m:") || s.contains("{m:UInt32}"))
        );
        let gaps = |set| {
            postcheck_sql(set, "prices")
                .into_iter()
                .map(|(n, _)| n)
                .filter(|n| n.starts_with("gap_"))
                .collect::<Vec<_>>()
        };
        assert_eq!(gaps(Postcheck::Window), ["gap_15m", "gap_1h"]);
        assert_eq!(
            gaps(Postcheck::NextDay),
            ["gap_15m", "gap_1h", "gap_4h", "gap_1d"]
        );
        assert_eq!(gaps(Postcheck::PeriodClose), ["gap_1w", "gap_1M"]);
        let next = postcheck_sql(Postcheck::NextDay, "prices");
        assert!(
            next[4]
                .1
                .contains("step = 'gap-backfill' AND status = 'ok') > 0")
        );
    }

    /// The runbook's three marked blocks are the renderings, verbatim.
    #[test]
    fn the_runbook_blocks_are_the_renderings() {
        for set in [
            Postcheck::Window,
            Postcheck::NextDay,
            Postcheck::PeriodClose,
            Postcheck::Month,
        ] {
            let (open, close) = (
                format!("<!-- 0139-postcheck:{} -->", set.name()),
                format!("<!-- /0139-postcheck:{} -->", set.name()),
            );
            let start = RUNBOOK.find(&open).unwrap_or_else(|| panic!("no {open}"));
            let end = RUNBOOK.find(&close).unwrap_or_else(|| panic!("no {close}"));
            let block: Vec<&str> = RUNBOOK[start..end]
                .lines()
                .filter(|l| l.starts_with("SELECT "))
                .collect();
            let want: Vec<String> = postcheck_sql(set, "prices")
                .into_iter()
                .map(|(_, s)| s)
                .collect();
            assert_eq!(
                block,
                want,
                "block {} — paste:\n{}",
                set.name(),
                want.join("\n")
            );
        }
    }

    /// A gap inside one bucket is never compared over an empty range: the
    /// range ends at the settled buckets, and an empty one fails.
    #[test]
    fn a_gap_check_ends_at_the_settled_buckets_and_fails_when_none_has() {
        let lag = |t: &Tier| {
            settled_bound(t)
                .split("INTERVAL ")
                .nth(1)
                .and_then(|x| x.split(' ').next())
                .and_then(|x| x.parse::<u32>().ok())
                .unwrap()
        };
        let lags: Vec<u32> = TIERS.iter().map(lag).collect();
        assert_eq!(lags, [960, 1_860, 5_400, 18_900, 101_700, 101_700]);
        for set in [
            Postcheck::Window,
            Postcheck::NextDay,
            Postcheck::PeriodClose,
        ] {
            for (t, (name, sql)) in set.gap_tiers().iter().zip(
                postcheck_sql(set, "prices")
                    .into_iter()
                    .filter(|(n, _)| n.starts_with("gap_")),
            ) {
                assert_eq!(name, format!("gap_{}", t.name));
                assert!(!sql.contains("range_to"), "{name} ends at now, not the gap");
                assert!(
                    sql.contains(&format!(
                        "{} < {}",
                        gap_bound("prices", "range_from", t.interval),
                        settled_bound(t)
                    )),
                    "{name} fails on an empty range"
                );
            }
        }
        assert_eq!(
            Postcheck::parse("period-close"),
            Some(Postcheck::PeriodClose)
        );
        assert_eq!(Postcheck::parse("daily"), None);
    }

    #[test]
    fn a_gap_bound_is_an_instant_and_the_mismatch_is_bucket_by_bucket() {
        assert!(is_instant("2026-10-01 12:00:00"));
        for bad in ["2026-10-01", "2026-10-01T12:00:00", "2026-10-01 12:00:0'"] {
            assert!(!is_instant(bad), "{bad}");
        }
        let sql = gap_mismatch_sql("d", &TIERS[0], "2026-10-01 00:00:00", "2026-10-01 05:00:00");
        assert!(sql.contains("FROM d.price_ohlcv_1m FINAL"));
        assert!(sql.contains("FROM d.price_ohlcv_15m FINAL"));
        assert!(sql.contains("FULL OUTER JOIN"));
        assert!(sql.contains("USING (b, asset_id, quote_asset_id, source)"));
        assert!(sql.contains("c.tc != p.tc OR c.vb != p.vb"));
    }
}
