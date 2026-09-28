//! The single source of the coarse rollup SQL (task 0286, ADR 0287; BRIEF §4.5).
//!
//! These statement kinds are rendered here, and nowhere else:
//!
//! - [`mv_ddl`] — the six refreshable `APPEND` MVs of `schema/rollups.sql`,
//!   each waiting (`DEPENDS ON`) for the MV that writes the table it reads
//!   ([`dependency`], task 0143), and [`mv_modify_refresh`], the in-place
//!   `ALTER` that lands that refresh clause on a live MV;
//! - [`reconcile_mv_ddl`] — the six hourly reconciliation MVs that follow them
//!   in `schema/rollups.sql` (task 0203), and [`reconcile_mismatch_select`],
//!   the read-only count of what they would rewrite, for the freshness probe;
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
//!   [`PRICE_FORMING_CHILD`] — `pf_trade_count > 0 AND close >= 1e-12`, because on a
//!   pre-0286 row the first term is a DEFAULT and says nothing). A dust-only
//!   child — every fill too small for its price to mean anything, so
//!   `open = high = low = close = 0` — can no longer become a coarse `low` of 0
//!   or a coarse `close` of 0.
//! - A bucket whose children are ALL dust-only has no price at all: every
//!   conditional aggregate matches no row and returns the type default 0 (F6c),
//!   which is the same "no price" encoding the 1m tier writes.
//! - `close_usd` is `close` × the latest priced child's **rate**
//!   (`close_usd / close`), never a carried product
//!   (`argMaxIf(close_usd, …)`): carrying the product decoupled `close` and
//!   `close_usd` (they came from different sub-buckets — the consequence
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
//! the column default; the explicit zero makes the value independent of that
//! server setting (task 0171's review, PR #312).
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
    /// [`Tier::refresh`] in seconds — the MV's own period, which the probe
    /// uses to tell a view that is merely between slots from one stuck
    /// `WaitingForDependencies` (task 0203). A unit test ties it to the text.
    pub refresh_seconds: u64,
    /// The hourly reconciliation MV that rebuilds any of `target`'s CLOSED
    /// buckets (past [`MISMATCH_GRACE`]) that disagree with `child` over
    /// [`RECONCILE_WINDOW`] (task 0203).
    ///
    /// `mv_reconcile_<src>_to_<dst>`: greppable as `mv_reconcile_`, and it
    /// contains no fast MV name as a substring, so a `contains(tier.mv)`
    /// statement lookup can never match the reconcile statement instead.
    pub reconcile_mv: &'static str,
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
        refresh_seconds: 60,
        reconcile_mv: "mv_reconcile_1m_to_15m",
    },
    Tier {
        name: "1h",
        target: "price_ohlcv_1h",
        child: "price_ohlcv_15m",
        interval: "INTERVAL 1 HOUR",
        mv: "mv_ohlcv_15m_to_1h",
        window: "INTERVAL 8 HOUR",
        refresh: "EVERY 15 MINUTE",
        refresh_seconds: 900,
        reconcile_mv: "mv_reconcile_15m_to_1h",
    },
    Tier {
        name: "4h",
        target: "price_ohlcv_4h",
        child: "price_ohlcv_1h",
        interval: "INTERVAL 4 HOUR",
        mv: "mv_ohlcv_1h_to_4h",
        window: "INTERVAL 1 DAY",
        refresh: "EVERY 1 HOUR",
        refresh_seconds: 3_600,
        reconcile_mv: "mv_reconcile_1h_to_4h",
    },
    Tier {
        name: "1d",
        target: "price_ohlcv_1d",
        child: "price_ohlcv_4h",
        interval: "INTERVAL 1 DAY",
        mv: "mv_ohlcv_4h_to_1d",
        window: "INTERVAL 7 DAY",
        refresh: "EVERY 4 HOUR",
        refresh_seconds: 14_400,
        reconcile_mv: "mv_reconcile_4h_to_1d",
    },
    Tier {
        name: "1w",
        target: "price_ohlcv_1w",
        child: "price_ohlcv_1d",
        interval: "INTERVAL 1 WEEK",
        mv: "mv_ohlcv_1d_to_1w",
        window: "INTERVAL 60 DAY",
        refresh: "EVERY 1 DAY",
        refresh_seconds: 86_400,
        reconcile_mv: "mv_reconcile_1d_to_1w",
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
        refresh_seconds: 86_400,
        reconcile_mv: "mv_reconcile_1d_to_1M",
    },
];

/// How far back every reconciliation MV compares a tier with its source
/// (task 0203, BRIEF §3.2) — the ONE place the width is spelled.
///
/// Seven days is wider than any stall this system has had (0202's 11.5 h,
/// 0111's four days) and is bounded by `_1m` retention (task 0200): no pass can
/// heal from source rows that are gone. An outage longer than this needs
/// `schema/preroll-live-gap.sql` (runbook 0142). The bound is ALIGNED to each
/// tier's bucket (see `lower_bound`), so every compared bucket is whole.
pub const RECONCILE_WINDOW: &str = "INTERVAL 7 DAY";

/// The cadence of every reconciliation MV — the ONE place it is spelled.
///
/// Hourly is a backstop's cadence, not a live one: the fast MVs keep the tip
/// fresh, and this pass only repairs what they missed. It never touches an
/// OPEN bucket, or one closed less than [`MISMATCH_GRACE`] ago (review WR-06):
/// those belong to the fast MVs, so on a live system a pass that finds nothing
/// missed writes nothing. No `OFFSET`: its benefit (keeping off the `:00`
/// minute) is unmeasured, its drift round-trip is unverified, and equal slots
/// keep the reconcile `DEPENDS ON` chain aligned.
pub const RECONCILE_REFRESH: &str = "EVERY 1 HOUR";

/// [`RECONCILE_REFRESH`] in seconds — every reconciliation MV's own period
/// (see [`Tier::refresh_seconds`]). A unit test ties it to the text.
pub const RECONCILE_REFRESH_SECONDS: u64 = 3_600;

/// How old a bucket's END must be before the reconciliation pass may rewrite
/// it ([`reconcile_select`]) — and therefore before a disagreement counts as a
/// mismatch ([`reconcile_mismatch_select`] counts that same SELECT). ONE bound
/// for both, so the metric is exactly what the next pass would write.
///
/// The open bucket of every tier always disagrees with its source for a
/// while: the fast MVs roll it on their own cadence (1h every 15 min, 1d every
/// 4 h, 1w/1M daily), and a just-closed one may still lag by ingest delay.
/// Without this bound the reconcile pass would rewrite every tier's open
/// bucket every hour and become the live writer of 1d/1w/1M (review WR-06).
/// Two hours is the fast 15m window, so every 15m bucket is covered by one
/// writer or the other; every coarser fast window is wider still, and carries
/// a repaired child into the open parent on its own next slot.
pub const MISMATCH_GRACE: &str = "INTERVAL 2 HOUR";

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
    /// The reconciliation window, `now() - RECONCILE_WINDOW`, aligned to the
    /// tier's bucket like every other lower bound. No upper bound.
    Reconcile,
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
        Bounds::Reconcile => Some(format!(
            "toStartOfInterval(now() - {RECONCILE_WINDOW}, {})",
            tier.interval
        )),
    })
}

/// The price gate of every coarse aggregate, spelled ONCE (task 0286 S5,
/// review B WR-01/WR-02, C1).
///
/// ⚠️ `pf_trade_count > 0` alone is not a price gate. It is exactly right for a
/// row the post-0286 ingest wrote — such a row has a price iff it had a
/// price-forming fill — and it is worth nothing on every row written before,
/// because the migration gives those `pf_trade_count DEFAULT trade_count`. A
/// legacy candle whose price underflowed `Decimal(38, 14)` and stored as `0`
/// therefore reads `pf_trade_count = 2, close = 0` and passes the one-term
/// gate: it becomes the parent's `low` (0), and whenever it lands last its
/// `close` (0) as well, on every tier. That row shape is on disk today — one
/// per tier in the local verification database — and stays there until phase 3
/// re-ingests the history.
///
/// A floor on `close` closes it, and costs nothing on a post-0286 row, where
/// `pf_trade_count > 0` already implies a printable price. The four price
/// columns are written together, so gating them all on `close` is the same
/// question asked once rather than four nearly-identical questions.
///
/// ⚠️ The floor is [`crate::PRICE_FLOOR_SQL`] (`1e-12`), NOT `> 0` (review
/// WR-03). `/ohlcv` has always refused a price under that line as quantisation
/// noise, and the coarse gate used to sit at `> 0`, so a 1m child with
/// `close = 1.86e-12, low = 9e-14` passed here, gave the parent its `low` via
/// `minIf`, and was then published — the coarse row's own `close` cleared the
/// read path's floor, so nothing downstream looked at the `low` again. 24 `1d`
/// rows on the verification database carried exactly that shape. One line, in
/// all three languages: here, in `queries_ch.rs` and in
/// `price::price_survives_column_scale`.
///
/// The literal is spelled out rather than built from `PRICE_FLOOR_SQL`, because
/// a `const` cannot be concatenated from another; `the_price_gate_uses_the_shared_floor`
/// pins them together.
pub const PRICE_FORMING_CHILD: &str =
    "t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)";

/// The predicate a child must pass to LEND ITS RATE (`close_usd / close`) to the
/// coarse `close_usd` — both legs at the same floor as [`PRICE_FORMING_CHILD`].
///
/// It used to be `t.close_usd > 0 AND t.close > 0`, which skips the un-enriched
/// sentinel and nothing else. A ratio of two values a few ticks wide is not a
/// rate: the prod row `PRICE_FLOOR_SQL`'s doc quotes (`close = 5e-14,
/// close_usd = 4e-14`) reads as 0.8, and as the latest "priced" child it
/// re-priced a healthy parent close — on every tier above it, since the wrong
/// `close_usd` then carries the same rate upward. The floor is on `close_usd` as
/// well as `close` because a real price beside a four-tick USD value is the same
/// defect from the other side; `/ohlcv` refuses to convert either
/// (`queries_ch.rs`, `convertible`).
///
/// No `pf_trade_count` term: a post-0286 child with none has `close = 0` and
/// fails the floor, and a legacy child's DEFAULT says nothing either way.
pub const RATE_BEARING_CHILD: &str = "t.close_usd >= toDecimal128('0.000000000001', 14) \
AND t.close >= toDecimal128('0.000000000001', 14)";

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
    argMinIf(t.open, t.timestamp, {PRICE_FORMING_CHILD}) AS open,
    maxIf(t.high, {PRICE_FORMING_CHILD}) AS high,
    minIf(t.low, {PRICE_FORMING_CHILD}) AS low,
    argMaxIf(t.close, t.timestamp, {PRICE_FORMING_CHILD}) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) \
/ toFloat64(t.close), t.timestamp, {RATE_BEARING_CHILD})), 14), 0) AS close_usd,
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

/// The tier whose MV WRITES this tier's child table — the fast MV this tier's
/// fast MV must wait for (task 0143, BRIEF §3.1).
///
/// Derived from [`TIERS`], never hand-kept: `None` for 15m (its child,
/// `price_ohlcv_1m`, is written by ingest, not by a rollup), and the day tier
/// for BOTH the week and the month, which both read `price_ohlcv_1d`.
pub fn dependency(tier: &Tier) -> Option<&'static Tier> {
    TIERS.iter().find(|t| t.target == tier.child)
}

/// The text after `REFRESH` in a fast MV's head: its cadence, the fully
/// qualified MV it waits for (if any), and `APPEND` — the ONE spelling both
/// [`mv_ddl`] and [`mv_modify_refresh`] render.
///
/// Task 0143: every refreshable MV fired on its own clock, so at 00:00 the
/// two dailies could read `_1d` before `mv_ohlcv_4h_to_1d` had written the day
/// that just closed. `DEPENDS ON` makes a dependent's slot wait until its
/// dependency has refreshed for the same slot. The name is FULLY qualified
/// because ClickHouse stores an unqualified one qualified (BRIEF §2), and the
/// drift check compares the stored text.
fn refresh_clause(tier: &Tier, db: &str) -> Result<String, RollupSqlError> {
    let db = qualifier(db)?;
    Ok(match dependency(tier) {
        Some(dep) => format!("{} DEPENDS ON {db}.{} APPEND", tier.refresh, dep.mv),
        None => format!("{} APPEND", tier.refresh),
    })
}

/// The tier's refreshable `APPEND` MV, exactly as `schema/rollups.sql` ships it.
pub fn mv_ddl(tier: &Tier, db: &str) -> Result<String, RollupSqlError> {
    let body = rollup_select(tier, db, &Bounds::Window)?;
    let refresh = refresh_clause(tier, db)?;
    let db = qualifier(db)?;
    Ok(format!(
        "CREATE MATERIALIZED VIEW IF NOT EXISTS {db}.{mv}\nREFRESH {refresh}\nTO \
         {db}.{target} AS\n{body}",
        mv = tier.mv,
        target = tier.target,
    ))
}

/// The in-place rollout of [`mv_ddl`]'s refresh clause onto a live fast MV
/// (BRIEF §6): `ALTER TABLE … MODIFY REFRESH <the same clause>`.
///
/// `MODIFY REFRESH` REPLACES every refresh parameter, so the full clause is
/// repeated — omitting `DEPENDS ON` would drop it, and ClickHouse refuses to
/// add or remove `APPEND` this way (Code 48). `None` for the 15m tier, whose
/// clause has no dependency and so never changes. Runbook
/// `docs/runbooks/0142-rollup-mv-reapply.md` quotes these statements.
pub fn mv_modify_refresh(tier: &Tier, db: &str) -> Result<Option<String>, RollupSqlError> {
    let refresh = refresh_clause(tier, db)?;
    let db = qualifier(db)?;
    Ok(dependency(tier).map(|_| {
        format!(
            "ALTER TABLE {db}.{mv} MODIFY REFRESH {refresh}",
            mv = tier.mv
        )
    }))
}

/// The reconciliation SELECT for one tier (task 0203): the tier's rollup over
/// [`Bounds::Reconcile`], keeping ONLY the rows the target does not already
/// hold with the same `trade_count` and `volume_base`.
///
/// - The aggregation is [`rollup_select`] verbatim, once — there is one
///   definition of a coarse candle (ADR 0287 §4), and the reconcile row is
///   exactly the row the fast MV would have written.
/// - The comparison is on `trade_count` and `volume_base` and NEVER on
///   `version`: enrichment re-inserts a `_1m` child at `version + 1` and the
///   coarse sweep bumps coarse rows `+1`, so a version difference says nothing
///   about completeness. A back-dated or late child changes the count/volume.
/// - `NOT IN` over a tuple, not a `JOIN`: one predicate covers both a missing
///   and a disagreeing bucket, and a `LEFT JOIN` would read a missing target
///   row as `trade_count = 0` (ClickHouse's default-value join) rather than
///   NULL. No top-level `WITH` (the shipped-file guards and
///   `drift::parse_fingerprint` forbid it).
/// - Source and target are bounded by the SAME aligned lower bound, computed
///   once, so the two sides can never compare different buckets.
/// - ONLY CLOSED buckets whose END is at least [`MISMATCH_GRACE`] old are
///   emitted (review WR-06). The open bucket is the fast MVs' job; without
///   this bound the pass rewrote every tier's open bucket hourly whenever its
///   child had moved since the fast MV last ran — the hourly live writer of
///   1d/1w/1M, with a non-zero `written_rows` on every pass.
///
/// The emitted row wins in the `ReplacingMergeTree(version)` target because
/// its `sum(version)` covers a superset of the children the stale row summed
/// (BRIEF §3.3); nothing new is versioned here.
pub fn reconcile_select(tier: &Tier, db: &str) -> Result<String, RollupSqlError> {
    let body = rollup_select(tier, db, &Bounds::Reconcile)?;
    let db = qualifier(db)?;
    let lb = lower_bound(tier, &Bounds::Reconcile)?
        .expect("the reconcile bound always has a lower side");
    let columns = CANDLE_COLUMNS
        .chunks(6)
        .map(|c| c.join(", "))
        .collect::<Vec<_>>()
        .join(",\n    ");
    Ok(format!(
        "SELECT\n    {columns}\nFROM (\n{body}\n) AS s\n\
         WHERE timestamp + {interval} <= now() - {MISMATCH_GRACE}\n  \
         AND (timestamp, asset_id, quote_asset_id, source, trade_count, volume_base) NOT IN (\n    \
         SELECT d.timestamp, d.asset_id, d.quote_asset_id, d.source, d.trade_count, d.volume_base\n    \
         FROM {db}.{target} AS d FINAL\n    \
         WHERE d.timestamp >= {lb}\n)",
        interval = tier.interval,
        target = tier.target,
    ))
}

/// The tier's hourly reconciliation MV (task 0203), exactly as
/// `schema/rollups.sql` ships it: [`reconcile_select`] as a refreshable
/// `APPEND` MV writing into the SAME target as the tier's fast MV.
///
/// It waits (`DEPENDS ON`) for the reconcile MV of the tier it reads, so one
/// hourly pass carries a repair 15m → 1h → 4h → 1d → {1w, 1M}. The lowest one
/// depends on NOTHING — in particular not on the fast `1m → 15m` MV: a stopped
/// or failing dependency blocks its dependents silently and forever, and this
/// backstop exists for exactly the case where the fast path failed. Both read
/// `_1m FINAL` and append into the same RMT, and `sum(version)` resolves any
/// overlap, so the ordering would buy nothing. No FAST MV ever depends on a
/// reconcile MV: the backstop must never delay fresh data.
pub fn reconcile_mv_ddl(tier: &Tier, db: &str) -> Result<String, RollupSqlError> {
    let select = reconcile_select(tier, db)?;
    let db = qualifier(db)?;
    let depends = match dependency(tier) {
        Some(dep) => format!(" DEPENDS ON {db}.{}", dep.reconcile_mv),
        None => String::new(),
    };
    Ok(format!(
        "CREATE MATERIALIZED VIEW IF NOT EXISTS {db}.{mv}\n\
         REFRESH {RECONCILE_REFRESH}{depends} APPEND\n\
         TO {db}.{target} AS\n{select}",
        mv = tier.reconcile_mv,
        target = tier.target,
    ))
}

/// Read-only: how many CLOSED buckets of `tier` disagree with their source
/// right now — one row, one `UInt64` column `mismatched` (task 0203 AC 4).
///
/// It is [`reconcile_select`] verbatim, counted, so the probe measures exactly
/// what the reconcile MV would rewrite — including its [`MISMATCH_GRACE`]
/// bound: the open bucket always disagrees for a while and is not counted.
pub fn reconcile_mismatch_select(tier: &Tier, db: &str) -> Result<String, RollupSqlError> {
    let select = reconcile_select(tier, db)?;
    Ok(format!(
        "SELECT count() AS mismatched FROM (\n{select}\n) AS m"
    ))
}

/// Every refreshable view `schema/rollups.sql` declares, with its own period
/// in seconds: the six fast MVs fine to coarse, then the six reconciliation
/// MVs in the same order (task 0203).
///
/// The probe's waiting-for-dependencies read takes its `IN (…)` list and each
/// view's period from here, never from input — so a view added to [`TIERS`]
/// is watched without a probe change, and no caller-supplied text reaches the
/// query.
pub fn rollup_views() -> Vec<(&'static str, u64)> {
    TIERS
        .iter()
        .map(|t| (t.mv, t.refresh_seconds))
        .chain(
            TIERS
                .iter()
                .map(|t| (t.reconcile_mv, RECONCILE_REFRESH_SECONDS)),
        )
        .collect()
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
            out.push((
                format!("reconcile {}", tier.name),
                reconcile_mv_ddl(&tier, "prices").expect("a checked rendering"),
            ));
            out.push((
                format!("mismatch {}", tier.name),
                reconcile_mismatch_select(&tier, "prices").expect("a checked rendering"),
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

    /// Review WR-03: the coarse gate and the read path's gate are ONE line.
    #[test]
    fn the_price_gate_uses_the_shared_floor() {
        assert!(
            PRICE_FORMING_CHILD.contains(crate::PRICE_FLOOR_SQL),
            "the coarse gate must carry `{}`, got `{PRICE_FORMING_CHILD}`",
            crate::PRICE_FLOOR_SQL
        );
        assert!(
            PRICE_FORMING_CHILD.contains("t.close >="),
            "a floor, not a `> 0` presence check"
        );
    }

    /// The price gate, in the exact spelling every downstream reader greps for.
    /// Each of the four price aggregates must be conditional on the child having
    /// a price-forming fill, and each must appear exactly once per statement.
    #[test]
    fn every_price_aggregate_is_gated_on_a_price_forming_child() {
        for (what, sql) in every_rendering() {
            for needle in [
                "argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open",
                "maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high",
                "minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low",
                "argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close",
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
                // The one-term gate: true on every legacy row, because
                // `pf_trade_count` DEFAULTs to `trade_count` there.
                "maxIf(t.high, t.pf_trade_count > 0)",
                "minIf(t.low, t.pf_trade_count > 0)",
                // The `> 0` floor: true on a child whose close is a handful of
                // `Decimal(38, 14)` ticks, i.e. quantisation noise (WR-03).
                "t.pf_trade_count > 0 AND t.close > 0",
            ] {
                assert!(
                    !sql.contains(forbidden),
                    "{what}: ungated price aggregate `{forbidden}` survived"
                );
            }
        }
    }

    /// BRIEF §4.5: `close_usd` is the bucket's own close re-priced by the latest
    /// priced child's RATE. The carried-product form must be gone — it is what
    /// decoupled `close` from `close_usd`.
    #[test]
    fn close_usd_is_a_rate_re_priced_by_this_buckets_close() {
        for (what, sql) in every_rendering() {
            let floor = crate::PRICE_FLOOR_SQL;
            assert_eq!(
                sql.matches(&format!(
                    "argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, \
                     t.close_usd >= {floor} AND t.close >= {floor})"
                ))
                .count(),
                1,
                "{what}: the rate must be taken exactly once, from a child whose \
                 close_usd AND close both clear the precision floor"
            );
            assert!(
                sql.contains("toFloat64(close) * argMaxIf("),
                "{what}: the rate must be re-priced by THIS bucket's close"
            );
            assert!(
                !sql.contains("argMaxIf(close_usd"),
                "{what}: the carried-product form survived — the rate replaces \
                 it, it is not kept beside it"
            );
            assert!(
                !sql.contains("argMax(close_usd"),
                "{what}: an unguarded argMax on close_usd (task 0145) survived"
            );
        }
    }

    /// A ratio of two values under the precision floor is quantisation noise (the
    /// prod row `close = 5e-14, close_usd = 4e-14` reads as a rate of 0.8), so
    /// the rate gate draws the SAME line as the price gate — never `> 0`.
    #[test]
    fn no_rendering_takes_a_rate_from_a_value_that_merely_exceeds_zero() {
        for (what, sql) in every_rendering() {
            for loose in ["t.close_usd > 0", "t.close > 0"] {
                assert!(
                    !sql.contains(loose),
                    "{what}: `{loose}` admits a sub-floor child into the close_usd rate"
                );
            }
        }
    }

    /// BRIEF F11 + the explicit `vwap` zero: both derived Decimals go through
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
            let clause = match dependency(&tier) {
                Some(dep) => format!("{} DEPENDS ON prices.{} APPEND", tier.refresh, dep.mv),
                None => format!("{} APPEND", tier.refresh),
            };
            assert!(
                ddl.contains(&format!("\nREFRESH {clause}\n")),
                "{}: a refreshable MV without APPEND replaces its whole target \
                 on every refresh (task 0090/0095), and a dependent must wait \
                 for the MV writing its child (task 0143)",
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
        // The MV, its DEPENDS ON, its TO target and the FROM child.
        assert_eq!(ddl.matches("scratch_42.").count(), 4);
        let alter = mv_modify_refresh(tier, "scratch_42")
            .expect("a checked rendering")
            .expect("the day tier has a dependency");
        assert!(
            !alter.contains("prices."),
            "left a prices. qualifier: {alter}"
        );
        assert_eq!(alter.matches("scratch_42.").count(), 2);
        // The reconcile MV, its DEPENDS ON, its TO target, the FROM child and
        // the target read by the NOT IN.
        let reconcile = reconcile_mv_ddl(tier, "scratch_42").expect("a checked rendering");
        assert!(
            !reconcile.contains("prices."),
            "left a prices. qualifier: {reconcile}"
        );
        assert_eq!(reconcile.matches("scratch_42.").count(), 5);
    }

    /// Task 0143 / BRIEF §3.1: each fast MV waits for the MV that writes the
    /// table it reads. The expected pairs are spelled out here, NOT derived
    /// from [`dependency`] — a test that re-derived them would pass on any
    /// lookup bug.
    #[test]
    fn each_fast_mv_depends_on_the_mv_writing_its_child() {
        let expected: [(&str, Option<&str>); 6] = [
            ("mv_ohlcv_1m_to_15m", None),
            ("mv_ohlcv_15m_to_1h", Some("mv_ohlcv_1m_to_15m")),
            ("mv_ohlcv_1h_to_4h", Some("mv_ohlcv_15m_to_1h")),
            ("mv_ohlcv_4h_to_1d", Some("mv_ohlcv_1h_to_4h")),
            ("mv_ohlcv_1d_to_1w", Some("mv_ohlcv_4h_to_1d")),
            ("mv_ohlcv_1d_to_1M", Some("mv_ohlcv_4h_to_1d")),
        ];
        let got: Vec<(&str, Option<&str>)> = TIERS
            .iter()
            .map(|t| (t.mv, dependency(t).map(|d| d.mv)))
            .collect();
        assert_eq!(got, expected.to_vec());

        for (tier, (_, dep)) in TIERS.iter().zip(expected) {
            let ddl = mv_ddl(tier, "prices").expect("a checked rendering");
            let head = ddl.lines().nth(1).expect("the REFRESH line");
            match dep {
                Some(dep) => assert_eq!(
                    head,
                    format!("REFRESH {} DEPENDS ON prices.{dep} APPEND", tier.refresh),
                    "{}: the dependency must be fully qualified, on the REFRESH line",
                    tier.name
                ),
                None => {
                    assert_eq!(head, format!("REFRESH {} APPEND", tier.refresh));
                    assert!(
                        !ddl.contains("DEPENDS ON"),
                        "{}: _1m is ingest's",
                        tier.name
                    );
                }
            }
        }
    }

    /// Every `DEPENDS ON` in a fast DDL names a FAST MV — the fix must never
    /// delay fresh data behind anything else (Adam, 2026-09-28).
    #[test]
    fn a_fast_mv_depends_only_on_a_fast_mv() {
        let fast: Vec<&str> = TIERS.iter().map(|t| t.mv).collect();
        for tier in TIERS {
            let ddl = mv_ddl(&tier, "prices").expect("a checked rendering");
            for dep in depends_on_names(&ddl) {
                let name = dep.strip_prefix("prices.").expect("a qualified dependency");
                assert!(
                    fast.contains(&name),
                    "{}: fast MV depends on `{dep}`, which is not a fast MV",
                    tier.name
                );
            }
        }
    }

    /// The names every `DEPENDS ON` in `sql` lists (qualified, as rendered).
    fn depends_on_names(sql: &str) -> Vec<&str> {
        sql.match_indices("DEPENDS ON ")
            .map(|(at, needle)| {
                let rest = &sql[at + needle.len()..];
                rest.split([' ', '\n', ',']).next().expect("a name")
            })
            .collect()
    }

    /// Task 0203 / BRIEF §6: the reconcile names are fixed, greppable, and can
    /// never be mistaken for a fast MV by a substring lookup.
    #[test]
    fn the_reconcile_mvs_are_named_apart_from_the_fast_mvs() {
        assert_eq!(
            TIERS.map(|t| t.reconcile_mv),
            [
                "mv_reconcile_1m_to_15m",
                "mv_reconcile_15m_to_1h",
                "mv_reconcile_1h_to_4h",
                "mv_reconcile_4h_to_1d",
                "mv_reconcile_1d_to_1w",
                "mv_reconcile_1d_to_1M",
            ]
        );
        for tier in TIERS {
            assert!(tier.reconcile_mv.starts_with("mv_reconcile_"));
            for other in TIERS {
                assert!(
                    !tier.reconcile_mv.contains(other.mv),
                    "{} contains the fast name {}",
                    tier.reconcile_mv,
                    other.mv
                );
            }
        }
    }

    /// BRIEF §3.2: the reconcile MVs chain among themselves bottom-up, the
    /// lowest waits for nothing (not even the fast 1m → 15m MV), and no FAST MV
    /// ever waits for a reconcile MV — the backstop must never delay fresh data.
    #[test]
    fn a_reconcile_mv_depends_only_on_a_reconcile_mv_and_no_fast_mv_on_one() {
        let expected: [(&str, Option<&str>); 6] = [
            ("mv_reconcile_1m_to_15m", None),
            ("mv_reconcile_15m_to_1h", Some("mv_reconcile_1m_to_15m")),
            ("mv_reconcile_1h_to_4h", Some("mv_reconcile_15m_to_1h")),
            ("mv_reconcile_4h_to_1d", Some("mv_reconcile_1h_to_4h")),
            ("mv_reconcile_1d_to_1w", Some("mv_reconcile_4h_to_1d")),
            ("mv_reconcile_1d_to_1M", Some("mv_reconcile_4h_to_1d")),
        ];
        let reconcile: Vec<&str> = TIERS.iter().map(|t| t.reconcile_mv).collect();
        for (tier, (name, dep)) in TIERS.iter().zip(expected) {
            assert_eq!(tier.reconcile_mv, name);
            let ddl = reconcile_mv_ddl(tier, "prices").expect("a checked rendering");
            let deps = depends_on_names(&ddl);
            match dep {
                Some(dep) => assert_eq!(deps, vec![format!("prices.{dep}").as_str()]),
                None => assert!(deps.is_empty(), "{name} must depend on nothing: {deps:?}"),
            }
            for d in deps {
                let d = d.strip_prefix("prices.").expect("qualified");
                assert!(
                    reconcile.contains(&d),
                    "{name} depends on non-reconcile {d}"
                );
            }

            let fast = mv_ddl(tier, "prices").expect("a checked rendering");
            assert!(
                !fast.contains("mv_reconcile_"),
                "{}: a fast MV must never mention, let alone wait for, a reconcile MV",
                tier.mv
            );
        }
    }

    /// The reconcile DDL: its cadence and window come from the two constants,
    /// the rollup body appears exactly once, both sides are bounded by the SAME
    /// window, and the comparison never looks at `version`.
    #[test]
    fn the_reconcile_ddl_wraps_the_rollup_once_and_compares_counts_not_versions() {
        for tier in TIERS {
            let ddl = reconcile_mv_ddl(&tier, "prices").expect("a checked rendering");
            let body = rollup_select(&tier, "prices", &Bounds::Reconcile).expect("rendered");

            assert!(ddl.starts_with(&format!(
                "CREATE MATERIALIZED VIEW IF NOT EXISTS prices.{}\nREFRESH {RECONCILE_REFRESH}",
                tier.reconcile_mv
            )));
            assert!(
                ddl.lines().nth(1).is_some_and(|l| l.ends_with(" APPEND")),
                "{}: a reconcile MV without APPEND would replace its whole target",
                tier.name
            );
            assert!(ddl.contains(&format!("\nTO prices.{} AS\nSELECT", tier.target)));
            assert_eq!(
                ddl.matches(&body).count(),
                1,
                "{}: the body once",
                tier.name
            );
            assert_eq!(
                ddl.matches(&format!("now() - {RECONCILE_WINDOW}")).count(),
                2,
                "{}: the source and the target side share the one window",
                tier.name
            );
            assert_eq!(
                ddl.matches(&format!(
                    "toStartOfInterval(now() - {RECONCILE_WINDOW}, {})",
                    tier.interval
                ))
                .count(),
                2,
                "{}: both bounds aligned to the tier's bucket",
                tier.name
            );

            let (_, tail) = ddl.split_once(") AS s\n").expect("the outer filter");
            // Review WR-06: only closed buckets past the grace, never the open
            // one — the fast MVs stay its only writer.
            assert!(
                tail.starts_with(&format!(
                    "WHERE timestamp + {} <= now() - {MISMATCH_GRACE}\n  AND (timestamp, asset_id, \
                     quote_asset_id, source, trade_count, volume_base) NOT IN (",
                    tier.interval
                )),
                "{}: the reconcile pass must be bounded to closed buckets past the grace: {tail}",
                tier.name
            );
            assert_eq!(
                ddl.matches(&format!("now() - {MISMATCH_GRACE}")).count(),
                1,
                "{}: one grace bound",
                tier.name
            );
            assert!(tail.contains(&format!("FROM prices.{} AS d FINAL", tier.target)));
            assert!(
                !tail.contains("version"),
                "{}: the comparison must never read version: {tail}",
                tier.name
            );
            assert!(!ddl.contains("JOIN"), "a JOIN reads a missing row as 0");
            assert!(!ddl.contains("WITH "), "no top-level WITH");

            // The outer projection is every candle column, in DDL order, so
            // the MV routes each by name into the target.
            let head = ddl
                .split_once(" AS\nSELECT\n")
                .and_then(|(_, rest)| rest.split_once("\nFROM (\n"))
                .map(|(cols, _)| cols)
                .expect("the outer projection");
            let got: Vec<&str> = head.split(',').map(str::trim).collect();
            assert_eq!(got, CANDLE_COLUMNS.to_vec());
        }
    }

    /// The mismatch count is the reconcile SELECT verbatim, counted — and
    /// NOTHING else: the closed-bucket grace lives in the reconcile SELECT
    /// itself (review WR-06), so the probe measures exactly what the MV would
    /// rewrite, and a second grace here could only make the two disagree.
    #[test]
    fn the_mismatch_select_counts_the_reconcile_select_verbatim() {
        for tier in TIERS {
            let sql = reconcile_mismatch_select(&tier, "prices").expect("a checked rendering");
            let select = reconcile_select(&tier, "prices").expect("a checked rendering");
            assert_eq!(
                sql,
                format!("SELECT count() AS mismatched FROM (\n{select}\n) AS m")
            );
            assert_eq!(sql.matches(&format!("now() - {MISMATCH_GRACE}")).count(), 1);
        }
    }

    /// `EVERY <n> <MINUTE|HOUR|DAY>` → seconds. Lives in the test on purpose:
    /// the API carries the number, and this is what ties it to the text.
    fn every_seconds(refresh: &str) -> u64 {
        let mut words = refresh.split_whitespace();
        assert_eq!(words.next(), Some("EVERY"), "{refresh}");
        let n: u64 = words
            .next()
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("{refresh}: a count"));
        let unit = match words.next() {
            Some("MINUTE") => 60,
            Some("HOUR") => 3_600,
            Some("DAY") => 86_400,
            other => panic!("{refresh}: unsupported unit {other:?}"),
        };
        assert_eq!(words.next(), None, "{refresh}: nothing after the unit");
        n * unit
    }

    /// The probe decides "stuck" by a view's own period, so the number must
    /// be the text — a drifted pair would silently widen or narrow the alarm.
    #[test]
    fn each_refresh_period_in_seconds_is_its_refresh_text() {
        for tier in TIERS {
            assert_eq!(
                tier.refresh_seconds,
                every_seconds(tier.refresh),
                "{}: refresh_seconds vs {:?}",
                tier.name,
                tier.refresh
            );
        }
        assert_eq!(RECONCILE_REFRESH_SECONDS, every_seconds(RECONCILE_REFRESH));
    }

    /// Twelve distinct views, fast then reconcile, each with its own period —
    /// the probe's `IN (…)` list and its thresholds.
    #[test]
    fn the_rollup_views_are_the_twelve_declared_views_with_their_own_periods() {
        let views = rollup_views();
        assert_eq!(views.len(), 12);
        let unique: std::collections::BTreeSet<_> = views.iter().map(|(n, _)| *n).collect();
        assert_eq!(unique.len(), 12, "no view listed twice");
        for (i, tier) in TIERS.iter().enumerate() {
            assert_eq!(views[i], (tier.mv, tier.refresh_seconds));
            assert_eq!(views[6 + i], (tier.reconcile_mv, RECONCILE_REFRESH_SECONDS));
        }
        for (name, _) in &views {
            assert!(
                crate::ROLLUPS_SQL.contains(&format!(
                    "CREATE MATERIALIZED VIEW IF NOT EXISTS prices.{name}\n"
                )),
                "{name} is declared in rollups.sql"
            );
        }
    }

    /// The window, cadence and grace are spelled once, as the constants.
    #[test]
    fn the_reconcile_parameters_are_the_decided_values() {
        assert_eq!(RECONCILE_WINDOW, "INTERVAL 7 DAY");
        assert_eq!(RECONCILE_REFRESH, "EVERY 1 HOUR");
        assert_eq!(MISMATCH_GRACE, "INTERVAL 2 HOUR");
    }

    /// BRIEF §6: the in-place rollout repeats EXACTLY the clause the CREATE
    /// renders (`MODIFY REFRESH` replaces every refresh parameter), and there is
    /// nothing to modify on the 15m tier.
    #[test]
    fn the_modify_refresh_repeats_the_ddls_refresh_clause() {
        for tier in TIERS {
            let ddl = mv_ddl(&tier, "prices").expect("a checked rendering");
            let alter = mv_modify_refresh(&tier, "prices").expect("a checked rendering");
            match dependency(&tier) {
                None => assert_eq!(alter, None, "{}: nothing to modify", tier.name),
                Some(_) => {
                    let alter = alter.expect("a dependent has a MODIFY REFRESH");
                    let clause = ddl
                        .lines()
                        .nth(1)
                        .and_then(|l| l.strip_prefix("REFRESH "))
                        .expect("the REFRESH line");
                    assert_eq!(
                        alter,
                        format!("ALTER TABLE prices.{} MODIFY REFRESH {clause}", tier.mv)
                    );
                    assert!(alter.ends_with(" APPEND"), "CH refuses to drop APPEND");
                }
            }
        }
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
        assert_eq!(
            mv_modify_refresh(tier, bad),
            Err(RollupSqlError::Database(bad.to_string()))
        );
        assert_eq!(
            reconcile_select(tier, bad),
            Err(RollupSqlError::Database(bad.to_string()))
        );
        assert_eq!(
            reconcile_mv_ddl(tier, bad),
            Err(RollupSqlError::Database(bad.to_string()))
        );
        assert_eq!(
            reconcile_mismatch_select(tier, bad),
            Err(RollupSqlError::Database(bad.to_string()))
        );
        // The 15m tier renders no ALTER, and still refuses the qualifier.
        assert_eq!(
            mv_modify_refresh(tier_by_name("15m").expect("the 15m tier"), bad),
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
