//! The single source of the coarse rollup SQL (task 0286, ADR 0287; BRIEF §4.5).
//!
//! Two statement kinds are rendered here, and nowhere else:
//!
//! - [`mv_ddl`] — the six refreshable `APPEND` MVs of `schema/rollups.sql`;
//! - [`rollup_insert`] — the same SELECT as a bounded or full `INSERT`, which
//!   is what `schema/preroll.sql` and `schema/preroll-live-gap.sql` are.
//!
//! **The static files stay the operator and drift-detector copy.** An operator
//! pastes `rollups.sql`; `drift.rs` compares `rollups.sql` to the live
//! definitions. Unit tests in `lib.rs` assert each shipped statement equals
//! this module's output whitespace-normalised, so the two cannot drift — and
//! the mismatch message prints the generator's rendering, which is how the
//! files are regenerated after an edit here.
//!
//! What the rendered SQL means (ADR 0287 §2–§5):
//!
//! - `open`/`close` are the first/last child that HAS a price-forming fill, and
//!   `high`/`low` their extremes (`argMinIf`/`argMaxIf`/`maxIf`/`minIf` on
//!   `t.pf_trade_count > 0`). A dust-only child — every fill too small for its
//!   price to mean anything, so `open = high = low = close = 0` — can no longer
//!   become a coarse `low` of 0 or a coarse `close` of 0.
//! - A bucket whose children are ALL dust-only has no price at all: every
//!   conditional aggregate matches no row and returns the type default 0 (F6c),
//!   which is the same "no price" encoding the 1m tier writes.
//! - `close_usd` is `close` × the latest priced child's **rate**
//!   (`close_usd / close`), never a carried product. This supersedes task
//!   0146's `argMaxIf(close_usd, …)`: carrying the product decoupled `close`
//!   and `close_usd` (they came from different sub-buckets — the consequence
//!   task 0145 accepted); re-pricing the bucket's own close by the latest rate
//!   makes them same-bucket again by construction.
//! - `vwap` stays Σ`volume_quote` / Σ`volume_base` over EVERY fill, price-forming
//!   or not (ADR 0287 §1) — dust trades happened, and they count in volume.
//! - `version` is `sum(t.version)` exactly as before (task 0095): a fuller
//!   aggregation sums more source versions than a partial one, so a complete
//!   bucket always outranks a partial re-roll of itself under `ReplacingMergeTree`.
//!
//! `close_usd` and `vwap` are computed in Float64 and converted with
//! `ifNull(toDecimal128OrZero(toString(…), 14), 0)`. Decimal division silently
//! overflows past a ~1.7e10 dividend on 26.3.10.60 (BRIEF F11) and
//! `divideDecimal` throws on a zero divisor; this pattern does neither. The
//! `vwap` fallback is spelled `toDecimal128(0, 14)` rather than left NULL
//! because `init.sql` declares `vwap Decimal(38, 14)` — NOT Nullable. Today's
//! MVs only land a NULL there because `insert_null_as_default` rewrites it to
//! the column default; making the zero explicit is the "explicit vwap" item of
//! task 0146.
//!
//! Every column inside an aggregate is `t.`-qualified. An unqualified one
//! raises `ILLEGAL_AGGREGATION` (Code 184) once the bucket key is aliased
//! `AS timestamp`, and a bare `timestamp` inside `argMaxIf` resolves to the
//! CONSTANT bucket alias instead of the row's own time (task 0059's trap).
//!
//! **Interpolation (ASVS V5).** These renderers are `pub`, and this workspace
//! already has operator-driven binaries (`coarse-repair`, the pre-roll runners),
//! so the rule cannot live in prose: every interpolation point that is not a
//! crate-controlled `&'static str` (the [`Tier`] fields, `CANDLE_COLUMNS`) is
//! CHECKED, and a rendering that would splice unchecked text returns
//! [`RollupSqlError`] instead of a `String`.
//!
//! - `db` must be a bare SQL identifier (`^[A-Za-z_][A-Za-z0-9_]*$`).
//! - A [`Bounds::Range`] side is a [`Bound`], not free text: either a
//!   ClickHouse bound PARAMETER by name — `{start_ts:DateTime}`, where the
//!   value never enters the SQL at all, which is what the two maintained
//!   pre-roll scripts use — or a literal `YYYY-MM-DD HH:MM:SS` instant, which
//!   is validated character by character and rendered inside `toDateTime(…)`.
//!
//! A CLI `--database` or `--from` threaded into these functions therefore
//! cannot reach a `CREATE MATERIALIZED VIEW`; it fails the call.

use crate::CANDLE_COLUMNS;

/// One coarse tier of the rollup chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tier {
    /// The grain key, e.g. `15m`. Case-sensitive: `1M` is the month and `1m`
    /// is the minute table, which is not a tier at all (nothing rolls into it).
    pub name: &'static str,
    /// The candle table this tier writes.
    pub target: &'static str,
    /// The candle table this tier aggregates.
    pub child: &'static str,
    /// The bucket interval, as a SQL `INTERVAL` expression.
    pub interval: &'static str,
    /// The refreshable MV that maintains `target` on the live path.
    pub mv: &'static str,
    /// How far back that MV re-aggregates on every refresh.
    pub window: &'static str,
    /// The MV's `REFRESH` cadence, without the `APPEND` keyword.
    pub refresh: &'static str,
}

/// The six coarse tiers, fine to coarse.
pub const TIERS: [Tier; 6] = [
    Tier {
        name: "15m",
        target: "price_ohlcv_15m",
        child: "price_ohlcv_1m",
        interval: "INTERVAL 15 MINUTE",
        mv: "mv_ohlcv_1m_to_15m",
        window: "INTERVAL 2 HOUR",
        refresh: "EVERY 1 MINUTE",
    },
    Tier {
        name: "1h",
        target: "price_ohlcv_1h",
        child: "price_ohlcv_15m",
        interval: "INTERVAL 1 HOUR",
        mv: "mv_ohlcv_15m_to_1h",
        window: "INTERVAL 8 HOUR",
        refresh: "EVERY 15 MINUTE",
    },
    Tier {
        name: "4h",
        target: "price_ohlcv_4h",
        child: "price_ohlcv_1h",
        interval: "INTERVAL 4 HOUR",
        mv: "mv_ohlcv_1h_to_4h",
        window: "INTERVAL 1 DAY",
        refresh: "EVERY 1 HOUR",
    },
    Tier {
        name: "1d",
        target: "price_ohlcv_1d",
        child: "price_ohlcv_4h",
        interval: "INTERVAL 1 DAY",
        mv: "mv_ohlcv_4h_to_1d",
        window: "INTERVAL 7 DAY",
        refresh: "EVERY 4 HOUR",
    },
    Tier {
        name: "1w",
        target: "price_ohlcv_1w",
        child: "price_ohlcv_1d",
        interval: "INTERVAL 1 WEEK",
        mv: "mv_ohlcv_1d_to_1w",
        window: "INTERVAL 60 DAY",
        refresh: "EVERY 1 DAY",
    },
    // Task 0286 / BRIEF F10: the month is rolled from the DAY, not the week,
    // and the MV is renamed `mv_ohlcv_1d_to_1M` to say so. A week straddling a
    // month boundary is attributed WHOLLY to the month it starts in
    // (`toStartOfInterval(week_start, INTERVAL 1 MONTH)`), so under the old
    // 1w-fed month a month's close and extremes could come from the next
    // month's trades — and the first days of a month whose 1st is not a Monday
    // reached the PREVIOUS month instead. Reading days makes the boundaries
    // exact. Re-creating this MV is the one step of the rollout runbook that
    // also has to TRUNCATE and re-roll its target
    // (`docs/runbooks/0286-candle-definitions-rollout.md`).
    Tier {
        name: "1M",
        target: "price_ohlcv_1M",
        child: "price_ohlcv_1d",
        interval: "INTERVAL 1 MONTH",
        mv: "mv_ohlcv_1d_to_1M",
        window: "INTERVAL 400 DAY",
        refresh: "EVERY 1 DAY",
    },
];

/// Why a rendering refused to produce SQL. Every variant is an interpolation
/// point that failed its check, never a formatting failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RollupSqlError {
    /// `db` is not a bare SQL identifier.
    #[error("database qualifier must be a bare SQL identifier, got `{0}`")]
    Database(String),
    /// A [`Bound::Param`] name is not a bare SQL identifier, so it could carry
    /// SQL out of the `{name:DateTime}` placeholder.
    #[error("bound-parameter name must be a bare SQL identifier, got `{0}`")]
    BoundParam(String),
    /// A [`Bound::Timestamp`] is not exactly `YYYY-MM-DD HH:MM:SS`.
    #[error("bound must be a `YYYY-MM-DD HH:MM:SS` instant, got `{0}`")]
    BoundTimestamp(String),
}

/// One side of a [`Bounds::Range`] — a checked instant, never free SQL text.
#[derive(Debug, Clone, Copy)]
pub enum Bound<'a> {
    /// A ClickHouse bound PARAMETER, by name: renders `{name:DateTime}`, and
    /// the value is sent out of band — it never enters the statement text.
    /// This is what `schema/preroll-live-gap.sql` carries.
    Param(&'a str),
    /// A literal UTC instant, `YYYY-MM-DD HH:MM:SS`, rendered inside
    /// `toDateTime(…)`. Validated character by character: the only characters
    /// that survive are digits and the five separators, so nothing can close
    /// the quote.
    Timestamp(&'a str),
}

impl Bound<'_> {
    /// The checked SQL expression for this side.
    fn render(&self) -> Result<String, RollupSqlError> {
        match self {
            Bound::Param(name) if is_identifier(name) => Ok(format!("{{{name}:DateTime}}")),
            Bound::Param(name) => Err(RollupSqlError::BoundParam((*name).to_string())),
            Bound::Timestamp(ts) if is_timestamp(ts) => Ok(format!("toDateTime('{ts}', 'UTC')")),
            Bound::Timestamp(ts) => Err(RollupSqlError::BoundTimestamp((*ts).to_string())),
        }
    }
}

/// `^[A-Za-z_][A-Za-z0-9_]*$` — a bare SQL identifier, which needs no quoting
/// and cannot end the token it is spliced into.
fn is_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Exactly `YYYY-MM-DD HH:MM:SS`. Shape only — ClickHouse rejects an impossible
/// date itself; what matters here is that no other character can appear.
fn is_timestamp(s: &str) -> bool {
    s.len() == 19
        && s.bytes().enumerate().all(|(i, b)| match i {
            4 | 7 => b == b'-',
            10 => b == b' ',
            13 | 16 => b == b':',
            _ => b.is_ascii_digit(),
        })
}

/// The database qualifier, checked.
fn qualifier(db: &str) -> Result<&str, RollupSqlError> {
    if is_identifier(db) {
        Ok(db)
    } else {
        Err(RollupSqlError::Database(db.to_string()))
    }
}

/// The range a rollup statement covers.
#[derive(Debug, Clone, Copy)]
pub enum Bounds<'a> {
    /// The MV's own bounded live window, `now() - tier.window`.
    Window,
    /// An explicit half-open range, `[from, to)`.
    Range { from: Bound<'a>, to: Bound<'a> },
    /// Everything. Renders no `WHERE` at all.
    Full,
}

/// Look a tier up by its grain key. **Exact match, never case-folded**: `1M` is
/// the month and `1m` the minute, so a lowercasing lookup would roll the wrong
/// grain.
pub fn tier_by_name(name: &str) -> Option<&'static Tier> {
    TIERS.iter().find(|t| t.name == name)
}

/// The lower bound of a tier's scan, ALIGNED to the tier's own bucket.
///
/// A raw bound falls mid-bucket, so the OLDEST bucket in range would be rebuilt
/// from its in-range slice only — a PARTIAL row which then wins on
/// `sum(version)` against nothing and silently truncates the bucket (task 0095;
/// the S2 v1 review's WR-01/02 is that `Range` needs this just as much as
/// `Window` does, and gets it here rather than at every call site).
fn lower_bound(tier: &Tier, bounds: &Bounds<'_>) -> Result<Option<String>, RollupSqlError> {
    Ok(match bounds {
        Bounds::Window => Some(format!(
            "toStartOfInterval(now() - {}, {})",
            tier.window, tier.interval
        )),
        Bounds::Range { from, .. } => Some(format!(
            "toStartOfInterval({}, {})",
            from.render()?,
            tier.interval
        )),
        Bounds::Full => None,
    })
}

/// The coarse rollup SELECT for one tier — the body of its MV and of every
/// bounded re-roll. See the module docs for what each projection means.
pub fn rollup_select(tier: &Tier, db: &str, bounds: &Bounds<'_>) -> Result<String, RollupSqlError> {
    let Tier {
        child, interval, ..
    } = *tier;
    let db = qualifier(db)?;

    let where_clause = match (lower_bound(tier, bounds)?, bounds) {
        (Some(lb), Bounds::Range { to, .. }) => {
            format!(
                "\nWHERE t.timestamp >= {lb}\n  AND t.timestamp < {}",
                to.render()?
            )
        }
        (Some(lb), _) => format!("\nWHERE t.timestamp >= {lb}"),
        (None, _) => String::new(),
    };

    Ok(format!(
        "SELECT
    toStartOfInterval(t.timestamp, {interval}) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0) AS open,
    maxIf(t.high, t.pf_trade_count > 0) AS high,
    minIf(t.low, t.pf_trade_count > 0) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) \
/ toFloat64(t.close), t.timestamp, t.close_usd > 0 AND t.close > 0)), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) \
/ nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM {db}.{child} AS t FINAL{where_clause}
GROUP BY timestamp, asset_id, quote_asset_id, source"
    ))
}

/// The tier's refreshable `APPEND` MV, exactly as `schema/rollups.sql` ships it.
pub fn mv_ddl(tier: &Tier, db: &str) -> Result<String, RollupSqlError> {
    let body = rollup_select(tier, db, &Bounds::Window)?;
    let db = qualifier(db)?;
    Ok(format!(
        "CREATE MATERIALIZED VIEW IF NOT EXISTS {db}.{mv}\nREFRESH {refresh} APPEND\nTO \
         {db}.{target} AS\n{body}",
        mv = tier.mv,
        refresh = tier.refresh,
        target = tier.target,
    ))
}

/// The same aggregation as a plain `INSERT … SELECT` over `bounds` — what the
/// two maintained pre-roll scripts are.
///
/// Every candle column is NAMED, and the list is [`CANDLE_COLUMNS`] verbatim.
/// A positional `INSERT … SELECT` of fewer columns than the target holds fails
/// with Code 20 (F6d), but a NAMED list that omits one does not: ClickHouse
/// fills it from its DEFAULT, and for the three task-0286 columns that DEFAULT
/// is the pre-0286 "every fill forms price" value (F6b) — a dust-only bucket
/// would come back declaring itself fully price-forming.
pub fn rollup_insert(
    tier: &Tier,
    db: &str,
    bounds: &Bounds<'_>,
    settings: Option<&str>,
) -> Result<String, RollupSqlError> {
    let body = rollup_select(tier, db, bounds)?;
    let db = qualifier(db)?;
    let columns = CANDLE_COLUMNS
        .chunks(4)
        .map(|c| c.join(", "))
        .collect::<Vec<_>>()
        .join(",\n     ");
    let tail = match settings {
        Some(s) => format!("\nSETTINGS {s}"),
        None => String::new(),
    };
    Ok(format!(
        "INSERT INTO {db}.{target}\n    ({columns})\n{body}{tail}",
        target = tier.target,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every statement the generator can render, for the scans below.
    fn every_rendering() -> Vec<(String, String)> {
        let mut out = Vec::new();
        for tier in TIERS {
            out.push((
                format!("mv {}", tier.name),
                mv_ddl(&tier, "prices").expect("a checked rendering"),
            ));
            out.push((
                format!("preroll {}", tier.name),
                rollup_insert(&tier, "prices", &Bounds::Full, None).expect("a checked rendering"),
            ));
            out.push((
                format!("live-gap {}", tier.name),
                rollup_insert(
                    &tier,
                    "prices",
                    &Bounds::Range {
                        from: Bound::Param("start_ts"),
                        to: Bound::Param("end_ts"),
                    },
                    Some("max_threads = 4"),
                )
                .expect("a checked rendering"),
            ));
        }
        out
    }

    /// BRIEF F10 / §4.5: the month rolls from the DAY. A week straddling a month
    /// boundary belongs wholly to the month it STARTS in, so a 1w-fed month took
    /// its close and extremes from whichever month owned the straddling week —
    /// and a month whose 1st is not a Monday lost its first days to the previous
    /// one. Nothing else in the chain may read the week either.
    #[test]
    fn the_month_rolls_from_the_day_and_nothing_rolls_from_the_week() {
        assert_eq!(TIERS.len(), 6);

        let month = TIERS.last().expect("the coarsest tier");
        assert_eq!(month.name, "1M");
        assert_eq!(month.child, "price_ohlcv_1d");
        assert_eq!(month.mv, "mv_ohlcv_1d_to_1M");

        assert!(
            !TIERS.iter().any(|t| t.child == "price_ohlcv_1w"),
            "no tier may aggregate the week — it is a leaf of the chain"
        );
    }

    /// Fine to coarse, each tier reading the previous one's target (except the
    /// month, which reads the day). The pre-roll scripts run in this order and
    /// each level reads what the previous wrote, so the ORDER is load-bearing.
    #[test]
    fn the_tiers_run_fine_to_coarse_each_reading_a_target_written_before_it() {
        let mut written = vec!["price_ohlcv_1m"];
        for tier in TIERS {
            assert!(
                written.contains(&tier.child),
                "{}: reads {} before anything writes it",
                tier.name,
                tier.child
            );
            written.push(tier.target);
        }
        assert_eq!(
            TIERS.map(|t| t.name),
            ["15m", "1h", "4h", "1d", "1w", "1M"],
            "tiers must stay fine-to-coarse"
        );
    }

    #[test]
    fn a_tier_is_looked_up_by_exact_case() {
        assert_eq!(tier_by_name("1M").map(|t| t.target), Some("price_ohlcv_1M"));
        assert_eq!(tier_by_name("15m").map(|t| t.name), Some("15m"));
        // `1m` is the minute TABLE, not a tier: nothing rolls into it.
        assert!(tier_by_name("1m").is_none());
        assert!(tier_by_name("1m ").is_none());
    }

    /// Task 0059/0071: the bucket key must be aliased `AS timestamp` (the MV
    /// routes by name), and that alias SHADOWS the source column — so a bare
    /// `timestamp` inside an aggregate resolves to the constant bucket start,
    /// and an unqualified column raises `ILLEGAL_AGGREGATION` (Code 184).
    /// Every column inside an aggregate is therefore `t.`-qualified.
    #[test]
    fn every_aggregate_reads_a_qualified_column() {
        for (what, sql) in every_rendering() {
            for head in [
                "argMin(",
                "argMax(",
                "argMinIf(",
                "argMaxIf(",
                "max(",
                "min(",
                "maxIf(",
                "minIf(",
                "sum(",
                "sumIf(",
            ] {
                let mut from = 0;
                while let Some(at) = sql[from..].find(head) {
                    let open = from + at + head.len();
                    let rest = &sql[open..];
                    assert!(
                        rest.starts_with("t.") || rest.starts_with("toFloat64(t."),
                        "{what}: `{head}` reads an unqualified column: {}",
                        &rest[..rest.len().min(60)]
                    );
                    from = open;
                }
            }
        }
    }

    /// The price gate, in the exact spelling every downstream reader greps for.
    /// Each of the four price aggregates must be conditional on the child having
    /// a price-forming fill, and each must appear exactly once per statement.
    #[test]
    fn every_price_aggregate_is_gated_on_a_price_forming_child() {
        for (what, sql) in every_rendering() {
            for needle in [
                "argMinIf(t.open, t.timestamp, t.pf_trade_count > 0) AS open",
                "maxIf(t.high, t.pf_trade_count > 0) AS high",
                "minIf(t.low, t.pf_trade_count > 0) AS low",
                "argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0) AS close",
                "sum(t.pf_trade_count) AS pf_trade_count",
                "sum(t.pf_volume) AS pf_volume",
                "sum(t.pf_price_volume) AS pf_price_volume",
                "sum(t.version) AS version",
            ] {
                assert_eq!(
                    sql.matches(needle).count(),
                    1,
                    "{what}: expected `{needle}` exactly once"
                );
            }
            // The ungated forms are what the 1w-era file shipped; none may
            // survive, or a dust-only child's zero reaches a coarse low again.
            for forbidden in [
                "argMin(open,",
                "argMax(close,",
                "max(high)",
                "min(low)",
                "argMax(t.close, t.timestamp) AS close",
            ] {
                assert!(
                    !sql.contains(forbidden),
                    "{what}: ungated price aggregate `{forbidden}` survived"
                );
            }
        }
    }

    /// BRIEF §4.5: `close_usd` is the bucket's own close re-priced by the latest
    /// priced child's RATE. The carried-product form (task 0146) must be gone —
    /// it is what decoupled `close` from `close_usd`.
    #[test]
    fn close_usd_is_a_rate_re_priced_by_this_buckets_close() {
        for (what, sql) in every_rendering() {
            assert_eq!(
                sql.matches(
                    "argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, \
                     t.close_usd > 0 AND t.close > 0)"
                )
                .count(),
                1,
                "{what}: the rate must be taken exactly once"
            );
            assert!(
                sql.contains("toFloat64(close) * argMaxIf("),
                "{what}: the rate must be re-priced by THIS bucket's close"
            );
            assert!(
                !sql.contains("argMaxIf(close_usd"),
                "{what}: the carried-product form survived — task 0146's fix is \
                 superseded by the rate, not kept beside it"
            );
            assert!(
                !sql.contains("argMax(close_usd"),
                "{what}: an unguarded argMax on close_usd (task 0145) survived"
            );
        }
    }

    /// BRIEF F11 + task 0146's "explicit vwap": both derived Decimals go through
    /// the never-throwing Float64 pattern, and `vwap` falls back to an EXPLICIT
    /// zero because `init.sql` declares the column NOT Nullable.
    #[test]
    fn the_derived_decimals_never_throw_and_vwap_is_explicitly_zero() {
        for (what, sql) in every_rendering() {
            assert!(
                sql.contains(
                    "ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) \
                     / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap"
                ),
                "{what}: vwap must be the never-throwing form with an explicit zero"
            );
            assert!(
                !sql.contains("toDecimal128OrNull"),
                "{what}: a NULL cannot be written into the non-Nullable vwap column"
            );
            assert!(
                !sql.contains("volume_quote / nullIf(volume_base, 0) AS vwap"),
                "{what}: the raw Decimal division silently overflows past ~1.7e10 (F11)"
            );
            assert!(
                !sql.contains("divideDecimal"),
                "{what}: divideDecimal throws on a zero divisor"
            );
        }
    }

    /// BRIEF v2 deleted the settle pass, the windowed close, `close_median` and
    /// the clamp. None of it may reappear in a rendering — a clamp in
    /// particular would hide, rather than prevent, an ordering violation.
    #[test]
    fn nothing_settled_windowed_or_clamped_is_rendered() {
        for (what, sql) in every_rendering() {
            for forbidden in [
                "settle",
                "close_median",
                "open_window_fills",
                "close_window_fills",
                "greatest(",
                "least(",
            ] {
                assert!(
                    !sql.contains(forbidden),
                    "{what}: `{forbidden}` is not part of the v2 definition"
                );
            }
        }
    }

    /// Task 0095, and the S2 v1 review's WR-01/02: BOTH bounded forms align
    /// their lower bound to the tier's own bucket, so the oldest bucket in range
    /// is rebuilt COMPLETE. `Full` has no `WHERE` at all.
    #[test]
    fn both_bounded_forms_align_the_lower_bound_to_the_tiers_bucket() {
        let month = tier_by_name("1M").expect("the month");

        let window = rollup_select(month, "prices", &Bounds::Window).expect("a checked rendering");
        assert!(
            window.contains(
                "WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 400 DAY, INTERVAL 1 MONTH)"
            ),
            "window bound not aligned: {window}"
        );

        let range = rollup_select(
            month,
            "prices",
            &Bounds::Range {
                from: Bound::Param("start_ts"),
                to: Bound::Param("end_ts"),
            },
        )
        .expect("a checked rendering");
        assert!(
            range.contains(
                "WHERE t.timestamp >= toStartOfInterval({start_ts:DateTime}, INTERVAL 1 MONTH)"
            ),
            "range bound not aligned — a raw start_ts rebuilds the straddling \
             bucket partial and deletes the rest of it: {range}"
        );
        assert!(
            range.contains("AND t.timestamp < {end_ts:DateTime}"),
            "the range's upper bound must stay exclusive: {range}"
        );

        assert!(
            !rollup_select(month, "prices", &Bounds::Full)
                .expect("a checked rendering")
                .contains("WHERE"),
            "the full-range form must carry no WHERE"
        );
    }

    /// Two existing drift tests mutate this exact literal to prove they can see
    /// an edit (`rollup_drift_it.rs`, `rollup-freshness-probe`'s
    /// `an_edited_declaration_is_detected_as_drift`). If the generator reshapes
    /// it — `toIntervalMinute(15)`, a different alias — both go blind while
    /// still passing.
    #[test]
    fn the_15m_bucket_key_keeps_the_literal_the_drift_tests_mutate() {
        let fifteen = tier_by_name("15m").expect("the 15m tier");
        assert!(
            mv_ddl(fifteen, "prices")
                .expect("a checked rendering")
                .contains("toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp"),
            "the 15m MV body must keep the literal the drift tests edit"
        );
    }

    /// The MV head, in the shape `drift::parse_fingerprint` and the shipped-file
    /// guards both require: `IF NOT EXISTS`, `REFRESH … APPEND`, a `TO` target
    /// on its own line, and ` AS\nSELECT`.
    #[test]
    fn the_mv_ddl_declares_an_append_refreshable_view_with_a_target() {
        for tier in TIERS {
            let ddl = mv_ddl(&tier, "prices").expect("a checked rendering");
            assert!(
                ddl.starts_with(&format!(
                    "CREATE MATERIALIZED VIEW IF NOT EXISTS prices.{}",
                    tier.mv
                )),
                "unexpected head: {}",
                &ddl[..80.min(ddl.len())]
            );
            assert!(
                ddl.contains(&format!("\nREFRESH {} APPEND\n", tier.refresh)),
                "{}: a refreshable MV without APPEND replaces its whole target \
                 on every refresh (task 0090/0095)",
                tier.name
            );
            assert!(ddl.contains(&format!("\nTO prices.{}", tier.target)));
            assert!(ddl.contains(" AS\nSELECT"));
            assert!(
                !ddl.contains("DROP"),
                "{}: no DROP in the apply path",
                tier.name
            );
        }
    }

    /// The INSERT names all eighteen candle columns in `CANDLE_COLUMNS` order.
    /// A named list that OMITS one is not an error — ClickHouse fills it from
    /// its DEFAULT, and `pf_trade_count DEFAULT trade_count` then reports a
    /// dust-only bucket as fully price-forming (F6b).
    #[test]
    fn the_insert_names_every_candle_column_in_ddl_order() {
        let tier = tier_by_name("15m").expect("the 15m tier");
        let sql = rollup_insert(tier, "prices", &Bounds::Full, None).expect("a checked rendering");

        let head = sql.split("SELECT").next().expect("INSERT head");
        let open = head.find('(').expect("column list");
        let close = head.rfind(')').expect("closing paren");
        let got: Vec<String> = head[open + 1..close]
            .split(',')
            .map(|c| c.trim().to_string())
            .collect();
        assert_eq!(got, CANDLE_COLUMNS.to_vec());

        assert!(sql.starts_with("INSERT INTO prices.price_ohlcv_15m\n"));
        assert!(!sql.contains("SETTINGS"), "no settings unless asked for");
        assert!(
            rollup_insert(tier, "prices", &Bounds::Full, Some("max_threads = 4"))
                .expect("a checked rendering")
                .ends_with("\nSETTINGS max_threads = 4")
        );
    }

    /// The database qualifier is rendered, not assumed — the integration suites
    /// point the whole chain at a scratch schema.
    #[test]
    fn the_database_qualifier_is_rendered_everywhere() {
        let tier = tier_by_name("1d").expect("the 1d tier");
        let ddl = mv_ddl(tier, "scratch_42").expect("a checked rendering");
        assert!(!ddl.contains("prices."), "left a prices. qualifier: {ddl}");
        assert_eq!(ddl.matches("scratch_42.").count(), 3);
    }

    /// T-kpi-01 (ASVS V5). These renderers are `pub` in a workspace that has
    /// operator-driven binaries, so the qualifier is CHECKED, not documented:
    /// a `--database` threaded in here fails the call instead of reaching a
    /// `CREATE MATERIALIZED VIEW`.
    #[test]
    fn a_database_qualifier_that_is_not_an_identifier_is_rejected() {
        let tier = tier_by_name("1d").expect("the 1d tier");
        let bad = "prices; DROP TABLE prices.price_ohlcv_1d --";

        assert_eq!(
            mv_ddl(tier, bad),
            Err(RollupSqlError::Database(bad.to_string()))
        );
        assert_eq!(
            rollup_select(tier, bad, &Bounds::Window),
            Err(RollupSqlError::Database(bad.to_string()))
        );
        assert_eq!(
            rollup_insert(tier, bad, &Bounds::Full, None),
            Err(RollupSqlError::Database(bad.to_string()))
        );

        // A leading digit, a dot-qualified name and an empty string are all
        // identifiers a caller might assume work; none of them is bare.
        for bad in ["1prices", "prices.db", "", "pri ces", "\"prices\""] {
            assert!(mv_ddl(tier, bad).is_err(), "accepted `{bad}`");
        }
        assert!(mv_ddl(tier, "scratch_42").is_ok());
        assert!(mv_ddl(tier, "_prices").is_ok());
    }

    /// A range side is a [`Bound`], so operator text has no way in: a parameter
    /// NAME must be a bare identifier (the value stays out of band), and a
    /// literal instant must be exactly `YYYY-MM-DD HH:MM:SS`.
    #[test]
    fn a_range_bound_that_is_not_a_placeholder_or_timestamp_is_rejected() {
        let tier = tier_by_name("1d").expect("the 1d tier");
        let render = |from, to| rollup_select(tier, "prices", &Bounds::Range { from, to });

        let injected = "start_ts:DateTime} UNION ALL SELECT * FROM prices.secrets --";
        assert_eq!(
            render(Bound::Param(injected), Bound::Param("end_ts")),
            Err(RollupSqlError::BoundParam(injected.to_string()))
        );
        // The upper bound is rendered separately and is checked just the same.
        assert_eq!(
            render(Bound::Param("start_ts"), Bound::Param(injected)),
            Err(RollupSqlError::BoundParam(injected.to_string()))
        );

        let quoted = "2026-01-01 00:00:00' OR '1'='1";
        assert_eq!(
            render(Bound::Timestamp(quoted), Bound::Param("end_ts")),
            Err(RollupSqlError::BoundTimestamp(quoted.to_string()))
        );
        for bad in ["2026-01-01", "2026-01-01T00:00:00", "", "2026-1-1 0:0:0"] {
            assert!(
                render(Bound::Timestamp(bad), Bound::Param("end_ts")).is_err(),
                "accepted `{bad}`"
            );
        }

        // And the two accepted shapes render, quoted where they must be.
        let literal = render(
            Bound::Timestamp("2026-01-01 00:00:00"),
            Bound::Timestamp("2026-02-01 00:00:00"),
        )
        .expect("a well-formed instant renders");
        assert!(
            literal.contains(
                "WHERE t.timestamp >= toStartOfInterval(toDateTime('2026-01-01 00:00:00', 'UTC'), \
                 INTERVAL 1 DAY)"
            ),
            "{literal}"
        );
        assert!(
            literal.contains("AND t.timestamp < toDateTime('2026-02-01 00:00:00', 'UTC')"),
            "{literal}"
        );
    }
}
