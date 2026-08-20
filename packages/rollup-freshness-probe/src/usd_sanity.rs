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
//!    zero until the sweep reaches it. Without
//!    [`crate::usd_sanity::STRANDED_GRACE_SECONDS`] this
//!    metric would never read zero. See that constant for why the grace is 48 h
//!    specifically and not an arbitrary round number.
//! 3. **Bound the window.** An unbounded scan of the OHLCV tables every 15
//!    minutes is task 0111's outage, reintroduced as a health check. The
//!    windows are [`crate::usd_sanity::STRANDED_LOOKBACK_SECONDS`] and
//!    [`crate::usd_sanity::PEG_LOOKBACK_SECONDS`].
//!
//!    ⚠️ **A window bound is not a read bound.** Both tables are
//!    `PARTITION BY toYYYYMM(timestamp)` with `ORDER BY (asset_id,
//!    quote_asset_id, source, timestamp)`, so `timestamp` is not a primary-key
//!    prefix: a window prunes to whole *monthly partitions*, and a 48 h window
//!    spanning a month boundary reads two of them. Shortening a window
//!    therefore does **not** proportionally shorten the scan. ⛔ Size these on
//!    what the query READS (`EXPLAIN` / `read_rows`), never on the span it
//!    names — the same rule the tier choice was decided by.
//!
//! ⚠️ **This is a re-introduction guard, not a historical audit.** It watches
//! recent writes, because that is where a regression shows up. A frozen
//! historical corruption inside the window would latch the alarm rather than
//! re-notify — see [`crate::usd_sanity::ScanGuards`] on why a reading that
//! examined nothing is
//! refused rather than published as a zero.
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

/// How far back the **stranded** direction looks, in seconds.
///
/// Seven days. Long enough that a regression cannot slip through between runs
/// or hide behind a weekend, short enough that the scan prunes to one or two
/// monthly partitions. ⚠️ Do not widen this to "all history" — that is the
/// full-table scan of task 0111, which caused a four-day production outage, and
/// re-introducing it inside a 15-minute probe would be worse than the defect
/// this alarm watches for.
///
/// ⚠️ This bound is safe at seven days **because [`STRANDED_TABLE`] is a
/// forever-table**. The peg direction reads a retention-managed tier and cannot
/// inherit it — see [`PEG_LOOKBACK_SECONDS`].
pub const STRANDED_LOOKBACK_SECONDS: i64 = 7 * 86_400;

/// How far back the **peg-applied** direction looks, in seconds.
///
/// **48 hours, and deliberately not the seven days the stranded direction
/// uses.** [`PEG_TABLE`] is `price_ohlcv_1m`, which is retention-managed at
/// seven days, so a seven-day window would sit exactly on the deletion
/// frontier: the oldest hours of the window would be removed out from under the
/// scan while it runs, and the count would move for a reason that is not a
/// defect. 48 h leaves five days of margin, so no cleanup run can ever truncate
/// this window.
///
/// ⚠️ **Retention here is a JOB, not a TTL** — the same trap that produced task
/// 0174. `_1m` is pruned by the cleanup worker, which is currently DISABLED
/// (task 0200), so today the tier holds far more than seven days. This bound is
/// sized to be correct under **both** states rather than under whichever one
/// happens to be live, because 0200 may re-enable it at any time and nothing
/// would fail loudly if it did.
///
/// ⚠️ Shortening the window costs nothing here because this is a
/// **re-introduction guard, not a historical audit** (see the module docs). A
/// writer that has started applying the peg again writes it continuously, so it
/// shows up in the newest rows; it does not need seven days of history to be
/// seen. A frozen historical population is task 0212's job, not this alarm's.
pub const PEG_LOOKBACK_SECONDS: i64 = 2 * 86_400;

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
/// reads zero, and it holds **only on the hourly tier** — see [`STRANDED_TABLE`]
/// for why a coarser tier spends its own bucket width out of this grace before
/// enrichment can even begin.
///
/// 🔴 **THERE IS NO HEADROOM ON THE USDT LEG, AND THERE IS NO LAG TO CLEAR.**
/// An earlier version of this comment claimed a measured ~30 h enrichment
/// ceiling and ~18 h of headroom, from an age distribution taken on 2026-08-19.
/// Re-measured on 2026-08-20 the unpriced frontier had reached **47 h** — about
/// one hour inside this grace — and the cause is not lag at all: the USDT pivot
/// has **never** priced a `price_ohlcv_1m` row (task 0209). Everything from
/// 48 h out is 100% priced only because task 0182's repair wrote the coarse
/// tiers directly, up to its high-water mark of 2026-08-18 12:00.
///
/// ⚠️ So do **not** read this constant as "sized against measured lag". It is
/// sized against BE's 48 h loss window, which is the meaning that still holds.
/// Until 0209 is fixed the stranded metric climbs forever, and that is correct
/// behaviour — the alarm is right and the data is wrong. ⛔ **Do not widen this
/// bound to quiet it.**
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

/// Which OHLCV granularity the **stranded** direction reads.
///
/// `price_ohlcv_1h`, and the choice is about how the tier interacts with
/// [`STRANDED_GRACE_SECONDS`], not about coverage.
///
/// ⚠️ **This was `price_ohlcv_1d` and was changed on measurement (2026-08-19).**
/// The original reasoning — `_1d` is cheapest, `_1h` would scan "~24× the rows"
/// — was wrong on both halves, and the way it was wrong is worth keeping:
///
/// - **The 24× was about how many rows the TABLES HOLD, not what this query
///   touches.** Scoped to one quote leg and 7 days, prod measures `_1d` at
///   984,706 rows / 50.5 MiB / 44-62 ms and `_1h` at ~1.37M / ~70 MiB /
///   41-50 ms — **1.4×, and the same wall time**. Most of the work is the
///   `FINAL` merge and the `assets` lookup, which both tiers pay identically.
/// - **A bucket's `timestamp` is its START, so a coarse tier burns its own
///   width out of the grace before its data even exists.** A `_1d` candle
///   stamped `00:00` is not complete until 24 h later, so half of a 48 h grace
///   is gone before there is anything to enrich. On `_1h` that cost is one hour.
///
/// The bucket-width argument above is the durable half of that reasoning and it
/// still stands. ⚠️ The *measurement* quoted alongside it on 2026-08-19 — "every
/// USDT-quoted `_1h` candle is priced by 30 hours of age" — was an artefact and
/// is corrected in [`STRANDED_GRACE_SECONDS`].
///
/// ⚠️ `_1h` is a forever-table (no retention job, unlike `_1m`/`_15m`), so
/// [`STRANDED_LOOKBACK_SECONDS`] can never outrun what is kept.
///
/// # Why the stranded direction is correct on a derived tier
///
/// A zero rolls up as a zero: `argMaxIf(close_usd, …, close_usd > 0)` has
/// nothing to select, so an unpriced `_1m` row surfaces as an unpriced `_1h`
/// row. Reading the coarse tier therefore detects the condition faithfully —
/// which is how task 0209 was found at all. ⛔ **Do not move this direction to
/// `_1m` alongside the peg direction.** The 48 h grace is calibrated to BE's
/// loss window *on the hourly tier*, and the tier swap would silently change
/// what the alarm asserts.
pub const STRANDED_TABLE: &str = "price_ohlcv_1h";

/// Which OHLCV granularity the **peg-applied** direction reads.
///
/// `price_ohlcv_1m` — **the tier enrichment actually writes**, not one rolled
/// from it. Task 0213; this was `price_ohlcv_1h` and was blind.
///
/// # 🔴 Why a derived tier cannot carry this direction
///
/// A *repaired* value in a coarse tier says nothing about the row it was rolled
/// from. Task 0182's repair wrote the five coarse tables **directly** and never
/// touched `_1m`, so on 2026-08-20 the two tiers disagreed completely:
///
/// | | `_1h` — what this check used to read | `_1m` |
/// |---|---|---|
/// | USDT-quoted rows at the $1 peg | 0 | **1,564,045** |
/// | 2026-08-17, USDT leg | 13 priced / 0 | 0 priced / 16 |
///
/// The check would have published **a confident 0 over 1.5 M wrong values**, and
/// gone on doing so indefinitely.
///
/// ⚠️ **This is task 0204's own founding failure, reproduced inside the guard
/// built against it** — a check scoring healthy because it looked at the surface
/// least able to show the defect. It is the same shape as 0182 being verified
/// against the tiers its own repair had written.
///
/// # Why this is a separate constant and not a repointed `SANITY_TABLE`
///
/// Pointing one shared table name at `_1m` would have been the small change, and
/// it is wrong three times over:
///
/// 1. It sits permanently in ALARM, and a permanently-firing alarm gets muted —
///    the exact end-state task 0204 exists to prevent.
///
///    ⚠️ **Do not restate this as "reads 1,564,045, above every rung".** That
///    figure is the *all-history* peg population (task 0212) and an earlier
///    draft of this comment used it here, which is wrong for the query that
///    actually ships: bounded to [`PEG_LOOKBACK_SECONDS`], the reading is the
///    recent arrival rate — measured at ~16 USDT-quoted `_1m` rows per day on
///    2026-08-17, so tens of rows, not millions. That clears rung 1 and
///    nothing above it. The deploy block stands on "breached at all, forever,
///    until 0209 stops the writer" — not on the magnitude.
/// 2. `_1m` is retention-managed while `_1h` is a forever-table, so the window
///    reasoning has to be redone rather than inherited — see
///    [`PEG_LOOKBACK_SECONDS`].
/// 3. The stranded direction is *correct* on `_1h` and would be made worse by
///    moving, because its grace is calibrated to that tier.
///
/// ⛔ **This alarm must not be deployed until task 0212 has repaired the 1.5 M
/// rows and task 0209 has fixed the writer that produces them.** Deployed
/// before that, the ladder ships permanently breached and earns itself the
/// muting in point 1. Task 0212 carries the same ordering in the other
/// direction ("fix 0209 FIRST"), so the chain is 0111 → 0209 → 0212 → this.
pub const PEG_TABLE: &str = "price_ohlcv_1m";

/// Shared guards against the silent all-clear, carried by every reading.
///
/// If the USDT identity cannot be resolved — the registry moved, an issuer
/// changed, task 0139 renumbered something — the quote-leg filter matches
/// nothing, the counts come back `0`, and the alarm scores a **healthy** result
/// for a check that did not run. `resolved_legs` catches the identity being
/// missing; `scanned` catches it resolving to an id the candles no longer carry,
/// which the first guard cannot see. Carrying both in the same row makes each
/// state detectable; [`stranded_metric`] and [`peg_metric`] refuse them.
///
/// ⚠️ **Each direction carries its own pair, and they are not interchangeable.**
/// The two now read different tables, so `_1m` can match nothing while `_1h`
/// reads fine — a state a single shared guard would score as healthy on the
/// tier that was never examined. That is precisely the defect task 0213 exists
/// to close, so it must not be reintroduced one level up.
pub trait ScanGuards {
    fn resolved_legs(&self) -> u64;
    fn scanned(&self) -> u64;
    /// Which table this reading examined, for the refusal message.
    fn table(&self) -> &'static str;
    /// The window this reading covered, in seconds, for the refusal message.
    fn lookback_seconds(&self) -> i64;
}

/// One reading of the **stranded** direction, from [`STRANDED_TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct StrandedCounts {
    /// Number of `assets` rows matching the canonical USDT identity. Must be 1.
    pub resolved_legs: u64,
    /// USDT-quoted candles older than [`STRANDED_GRACE_SECONDS`] still at
    /// `close_usd = 0` with a `close` above [`REPRESENTABLE_CLOSE_FLOOR`].
    pub stranded: u64,
    /// USDT-quoted candles examined. See [`ScanGuards`].
    pub scanned: u64,
}

impl ScanGuards for StrandedCounts {
    fn resolved_legs(&self) -> u64 {
        self.resolved_legs
    }
    fn scanned(&self) -> u64 {
        self.scanned
    }
    fn table(&self) -> &'static str {
        STRANDED_TABLE
    }
    fn lookback_seconds(&self) -> i64 {
        STRANDED_LOOKBACK_SECONDS
    }
}

/// One reading of the **peg-applied** direction, from [`PEG_TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct PegCounts {
    /// Number of `assets` rows matching the canonical USDT identity. Must be 1.
    pub resolved_legs: u64,
    /// USDT-quoted candles in the window whose `close_usd / close` sits within
    /// [`PEG_RATIO_TOLERANCE`] of 1.0.
    pub peg_applied: u64,
    /// USDT-quoted candles examined. See [`ScanGuards`].
    pub scanned: u64,
}

impl ScanGuards for PegCounts {
    fn resolved_legs(&self) -> u64 {
        self.resolved_legs
    }
    fn scanned(&self) -> u64 {
        self.scanned
    }
    fn table(&self) -> &'static str {
        PEG_TABLE
    }
    fn lookback_seconds(&self) -> i64 {
        PEG_LOOKBACK_SECONDS
    }
}

/// One CloudWatch datum about USD-value correctness. Counts, so
/// `StandardUnit::Count` — mirrored without dragging the AWS SDK into the
/// default build, exactly as [`crate::disk::DiskMetric`] does.
#[derive(Debug, Clone, PartialEq)]
pub struct SanityMetric {
    pub name: &'static str,
    pub value: f64,
}

/// The `WITH` clause both queries open with: the USDT identity, resolved
/// **inline by code and issuer** rather than hard-coded as an `asset_id`.
///
/// The numeric id is not a stable contract while task 0139 is open, and a probe
/// that silently watches the wrong leg is worse than one that fails — see
/// [`ScanGuards`].
fn usdt_cte() -> String {
    format!(
        "WITH usdt AS ( \
             SELECT asset_id FROM assets FINAL \
             WHERE asset_code = 'USDT' AND issuer_address = '{usdt}' \
         ) ",
        usdt = USDT_ISSUER,
    )
}

/// The `resolved_legs` projection, and the reason it looks the way it does.
///
/// ⚠️ **It is wrapped in `toUInt64(ifNull(…))` and must stay that way.** A bare
/// scalar subquery — `(SELECT count() FROM usdt)` — comes back as
/// **`Nullable(UInt64)`**, and in RowBinary a nullable column is a one-byte null
/// flag followed by the value. Deserializing that into a plain `u64` does not
/// error: it consumes the flag as the low byte and silently returns **256** for
/// a true count of 1. Measured on 26.3.10.60 while writing the integration
/// tests. That is worse than the `backfill-freshness-probe` regression this
/// crate's IT file was written for (PR #97), which at least failed loudly — here
/// the probe would refuse every healthy run as "ambiguous identity" and the
/// gap-4 alarms would never publish.
const RESOLVED_LEGS_PROJECTION: &str =
    "toUInt64(ifNull((SELECT count() FROM usdt), 0)) AS resolved_legs";

/// The one-row query behind [`StrandedCounts`], over [`STRANDED_TABLE`].
///
/// Tables are unqualified: the probe binds its client to the `prices` database,
/// the same convention as [`crate::freshness_query`] and [`crate::disk::disk_query`].
///
/// ⚠️ `FINAL` is required. These are `ReplacingMergeTree` tables and a repair
/// re-inserts corrected rows at a higher `version`; without `FINAL` this would
/// read superseded rows and report a defect that has already been fixed —
/// alarming on history rather than on state.
pub fn stranded_query() -> String {
    format!(
        "{cte}\
         SELECT \
             {legs}, \
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
        cte = usdt_cte(),
        legs = RESOLVED_LEGS_PROJECTION,
        floor = REPRESENTABLE_CLOSE_FLOOR,
        grace = STRANDED_GRACE_SECONDS,
        table = STRANDED_TABLE,
        lookback = STRANDED_LOOKBACK_SECONDS,
    )
}

/// The one-row query behind [`PegCounts`], over [`PEG_TABLE`].
///
/// Structurally the sibling of [`stranded_query`], and deliberately a **separate
/// query rather than a second column on one** — see [`PEG_TABLE`] for why the
/// two directions cannot share a tier, and [`ScanGuards`] for why they cannot
/// share a guard either.
///
/// ⚠️ No grace period, and that asymmetry is the point. A stranded row is a row
/// enrichment has **not reached yet**, so it needs time before it means damage.
/// A peg-valued row is a row enrichment has already **written wrongly** — it is
/// wrong the instant it appears and waiting cannot improve it.
///
/// ⚠️ `FINAL` for the same `ReplacingMergeTree` reason as its sibling. It
/// matters more here: task 0212's repair re-inserts corrected rows at a higher
/// `version`, so without `FINAL` this alarm would keep reading the pre-repair
/// rows and stay breached over data that had been fixed.
pub fn peg_query() -> String {
    format!(
        "{cte}\
         SELECT \
             {legs}, \
             countIf(close_usd > 0 AND abs(close_usd / close - 1) < {tol}) AS peg_applied, \
             count() AS scanned \
         FROM {table} FINAL \
         WHERE quote_asset_id IN (SELECT asset_id FROM usdt) \
           AND close > 0 \
           AND timestamp >= now() - INTERVAL {lookback} SECOND",
        cte = usdt_cte(),
        legs = RESOLVED_LEGS_PROJECTION,
        tol = PEG_RATIO_TOLERANCE,
        table = PEG_TABLE,
        lookback = PEG_LOOKBACK_SECONDS,
    )
}

/// Why a reading was refused instead of published.
///
/// Both variants describe the **same hazard from different distances**: a query
/// that matched nothing still returns a perfectly publishable zero, and the
/// gap-4 alarms are `treatMissingData: NOT_BREACHING`, so that zero is scored
/// as a clean bill of health forever. The variants are separate because the
/// operator's first move differs — one points at the asset registry, the other
/// at the candles — and an alarm that names the wrong one costs an hour.
///
/// ⚠️ **Both carry the table.** Since task 0213 the two directions read
/// different tiers, so "the scan matched nothing" is no longer a single fact
/// about one place. A refusal that did not say which tier would send the
/// operator to look at a table that was fine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanityRefusal {
    /// The canonical USDT identity did not resolve to exactly one `assets` row.
    UnresolvableLeg {
        resolved_legs: u64,
        table: &'static str,
    },
    /// The leg resolved, but matched **no candles at all** in the window.
    EmptyScan {
        table: &'static str,
        lookback_seconds: i64,
    },
}

impl std::fmt::Display for SanityRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnresolvableLeg {
                resolved_legs,
                table,
            } => write!(
                f,
                "the canonical USDT identity resolved to {resolved_legs} assets, expected \
                 exactly 1 — check prices.assets for the USDT code/issuer pair (reading {table})"
            ),
            Self::EmptyScan {
                table,
                lookback_seconds,
            } => write!(
                f,
                "the USDT identity resolved, but no USDT-quoted candles were found in the \
                 last {days} days of {table} — the asset_id in the registry no longer matches \
                 the quote_asset_id stored in the candles (task 0139), or the leg has stopped \
                 trading entirely",
                days = lookback_seconds / 86_400,
            ),
        }
    }
}

impl std::error::Error for SanityRefusal {}

/// The two silent-all-clear guards, applied to either direction's reading.
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
/// The count publishes `0` and every gap-4 alarm reads healthy forever. A scan
/// that examined nothing has measured nothing.
fn guard<G: ScanGuards>(counts: &G) -> Result<(), SanityRefusal> {
    if counts.resolved_legs() != 1 {
        return Err(SanityRefusal::UnresolvableLeg {
            resolved_legs: counts.resolved_legs(),
            table: counts.table(),
        });
    }
    if counts.scanned() == 0 {
        return Err(SanityRefusal::EmptyScan {
            table: counts.table(),
            lookback_seconds: counts.lookback_seconds(),
        });
    }
    Ok(())
}

/// Shape one [`StrandedCounts`] reading into the CloudWatch datum to publish.
pub fn stranded_metric(counts: &StrandedCounts) -> Result<SanityMetric, SanityRefusal> {
    guard(counts)?;
    Ok(SanityMetric {
        name: STRANDED_METRIC,
        value: counts.stranded as f64,
    })
}

/// Shape one [`PegCounts`] reading into the CloudWatch datum to publish.
///
/// ⚠️ **Independent of [`stranded_metric`], deliberately.** Before task 0213 one
/// refusal suppressed both metrics, which was harmless while they came from one
/// query. Now they read different tables, and a `_1m` scan that matched nothing
/// says nothing whatever about `_1h` — suppressing a working signal because an
/// unrelated tier failed would be the muting failure again, arrived at from the
/// other side. Each direction publishes or refuses on its own evidence.
pub fn peg_metric(counts: &PegCounts) -> Result<SanityMetric, SanityRefusal> {
    guard(counts)?;
    Ok(SanityMetric {
        name: PEG_APPLIED_METRIC,
        value: counts.peg_applied as f64,
    })
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

    fn healthy_stranded() -> StrandedCounts {
        StrandedCounts {
            resolved_legs: 1,
            stranded: 0,
            scanned: 4_503,
        }
    }

    fn healthy_peg() -> PegCounts {
        PegCounts {
            resolved_legs: 1,
            peg_applied: 0,
            scanned: 4_503,
        }
    }

    /// The trap this whole module is scoped around. Exotic-quoted candles sit at
    /// `close_usd = 0` by design (~74M on `_1h` alone, task 0182), so a check
    /// that did not filter by quote leg would breach permanently on healthy
    /// data. Both directions need it.
    #[test]
    fn both_queries_are_scoped_to_the_usdt_quote_leg() {
        for sql in [stranded_query(), peg_query()] {
            assert!(sql.contains("WHERE quote_asset_id IN (SELECT asset_id FROM usdt)"));
        }
    }

    /// The numeric `asset_id` is not a stable contract while task 0139 is open,
    /// so the leg is resolved by code and issuer at query time.
    #[test]
    fn the_usdt_leg_is_resolved_by_issuer_not_a_hardcoded_id() {
        for sql in [stranded_query(), peg_query()] {
            assert!(sql.contains(USDT_ISSUER));
            assert!(sql.contains("asset_code = 'USDT'"));
        }
    }

    /// Both directions must still be counted. 0182's repair caused the stranded
    /// one, so a check for only the peg would have passed while 157 candles sat
    /// destroyed.
    #[test]
    fn the_two_directions_are_still_both_counted() {
        assert!(stranded_query().contains("AS stranded"));
        assert!(peg_query().contains("AS peg_applied"));
    }

    /// 🔴 **The defect task 0213 exists to close.** `close_usd` is written by
    /// enrichment into `_1m`; every coarse tier rolls from it. Reading `_1h`
    /// measured a *derived* surface that task 0182's repair had written
    /// directly, so the peg direction published a confident 0 over 1,564,045
    /// wrong values (task 0212).
    #[test]
    fn the_peg_direction_reads_the_tier_enrichment_writes_not_a_rolled_one() {
        assert_eq!(PEG_TABLE, "price_ohlcv_1m");
        assert!(peg_query().contains(&format!("FROM {PEG_TABLE} FINAL")));
        assert!(
            !peg_query().contains("price_ohlcv_1h"),
            "the peg direction must not read a tier 0182's repair wrote directly"
        );
    }

    /// ⚠️ The stranded direction is **correct** on the coarse tier and must not
    /// be moved along with the peg direction: a zero rolls up as a zero, and the
    /// 48 h grace is calibrated to BE's loss window on the hourly tier.
    #[test]
    fn the_stranded_direction_stays_on_the_hourly_tier() {
        assert_eq!(STRANDED_TABLE, "price_ohlcv_1h");
        assert!(stranded_query().contains(&format!("FROM {STRANDED_TABLE} FINAL")));
    }

    /// The two directions must not be collapsed back into one scan. Pinned
    /// because "one query, two columns" is the obvious tidy-up and it is exactly
    /// what re-introduces the blind spot.
    #[test]
    fn the_two_directions_read_different_tables() {
        assert_ne!(STRANDED_TABLE, PEG_TABLE);
        assert!(!stranded_query().contains("peg_applied"));
        assert!(!peg_query().contains("AS stranded"));
    }

    /// Without the grace period the stranded count can never read zero:
    /// enrichment fills `close_usd` asynchronously, so the newest candles are
    /// legitimately at zero on every single run.
    #[test]
    fn the_stranded_count_excludes_candles_too_young_to_be_enriched() {
        assert!(stranded_query().contains(&format!(
            "timestamp < now() - INTERVAL {STRANDED_GRACE_SECONDS} SECOND"
        )));
    }

    /// ⚠️ The asymmetry is deliberate. A stranded row is one enrichment has not
    /// reached yet, so it needs time before it means damage. A peg-valued row is
    /// one enrichment has already written **wrongly** — it is wrong the instant
    /// it appears, and a grace period would only delay saying so.
    #[test]
    fn the_peg_count_has_no_grace_because_a_wrong_write_is_wrong_immediately() {
        // The generated SQL never contains Rust identifier text, so asserting
        // the absence of "STRANDED_GRACE" could not fail and proved nothing.
        // Assert the absence of the rendered clause instead.
        assert!(!peg_query().contains(&format!(
            "timestamp < now() - INTERVAL {STRANDED_GRACE_SECONDS} SECOND"
        )));
        assert!(!peg_query().contains("close_usd = 0"));
    }

    /// An unbounded scan every 15 minutes is task 0111's outage wearing a health
    /// check's clothes. Both directions stay bounded.
    #[test]
    fn both_scans_are_bounded_to_a_recent_window() {
        assert!(stranded_query().contains(&format!(
            "timestamp >= now() - INTERVAL {STRANDED_LOOKBACK_SECONDS} SECOND"
        )));
        assert!(peg_query().contains(&format!(
            "timestamp >= now() - INTERVAL {PEG_LOOKBACK_SECONDS} SECOND"
        )));
    }

    /// ⚠️ **The retention interaction, pinned.** `_1m` is pruned by the cleanup
    /// worker at 7 days — a JOB, not a TTL (task 0174's lesson, currently
    /// DISABLED under task 0200). A 7-day peg window would sit exactly on the
    /// deletion frontier and move for reasons that are not defects, so it keeps
    /// a wide margin instead of inheriting the stranded direction's bound.
    #[test]
    fn the_peg_window_stays_clear_of_the_1m_retention_frontier() {
        let retention_seconds = 7 * 86_400_i64;
        assert!(
            PEG_LOOKBACK_SECONDS * 3 <= retention_seconds,
            "the peg window must keep real margin below _1m retention, or a cleanup \
             run truncates the scan and the count moves without a defect"
        );
        // The stranded direction reads a forever-table and needs no such margin.
        assert_eq!(STRANDED_LOOKBACK_SECONDS, retention_seconds);
    }

    /// `ReplacingMergeTree` + a repair that re-inserts at a higher `version`.
    /// Without `FINAL` this reads superseded rows and alarms on a defect that
    /// has already been corrected — which for the peg direction means staying
    /// breached over exactly the rows task 0212 just fixed.
    #[test]
    fn both_queries_read_final() {
        // ⚠️ Assert the CANDLE table's FINAL by name. A bare `contains(" FINAL ")`
        // is already satisfied by `FROM assets FINAL` in the shared CTE, so
        // deleting FINAL from either candle table would leave it green — a test
        // unable to distinguish the thing it asserts, in the module whose whole
        // subject is that failure.
        assert!(stranded_query().contains(&format!("FROM {STRANDED_TABLE} FINAL")));
        assert!(peg_query().contains(&format!("FROM {PEG_TABLE} FINAL")));
        for sql in [stranded_query(), peg_query()] {
            assert!(sql.contains("FROM assets FINAL"));
        }
    }

    /// The dust bound is the arithmetic one. 0182's first attempt used `1e-11`
    /// and counted rows that had priced perfectly well.
    #[test]
    fn the_representable_floor_is_the_underflow_bound_not_a_round_number() {
        assert!(stranded_query().contains(REPRESENTABLE_CLOSE_FLOOR));
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
    fn the_resolved_leg_count_is_forced_non_nullable_in_both_queries() {
        for sql in [stranded_query(), peg_query()] {
            assert!(
                sql.contains("toUInt64(ifNull((SELECT count() FROM usdt), 0)) AS resolved_legs")
            );
        }
    }

    #[test]
    fn healthy_readings_publish_zeroes() {
        assert_eq!(
            stranded_metric(&healthy_stranded()).expect("leg resolved"),
            SanityMetric {
                name: STRANDED_METRIC,
                value: 0.0
            }
        );
        assert_eq!(
            peg_metric(&healthy_peg()).expect("leg resolved"),
            SanityMetric {
                name: PEG_APPLIED_METRIC,
                value: 0.0
            }
        );
    }

    #[test]
    fn counts_are_published_as_they_are_read() {
        assert_eq!(
            peg_metric(&PegCounts {
                peg_applied: 1_564_045,
                ..healthy_peg()
            })
            .expect("leg resolved")
            .value,
            1_564_045.0
        );
        assert_eq!(
            stranded_metric(&StrandedCounts {
                stranded: 157,
                ..healthy_stranded()
            })
            .expect("leg resolved")
            .value,
            157.0
        );
    }

    /// The silent all-clear the `resolved_legs` column exists to prevent: an
    /// unresolvable leg matches no candles, the count reads zero, and a
    /// `NOT_BREACHING` alarm scores it healthy forever.
    #[test]
    fn an_unresolvable_usdt_leg_is_refused_rather_than_reported_healthy() {
        assert_eq!(
            stranded_metric(&StrandedCounts {
                resolved_legs: 0,
                ..healthy_stranded()
            }),
            Err(SanityRefusal::UnresolvableLeg {
                resolved_legs: 0,
                table: STRANDED_TABLE
            })
        );
        assert_eq!(
            peg_metric(&PegCounts {
                resolved_legs: 0,
                ..healthy_peg()
            }),
            Err(SanityRefusal::UnresolvableLeg {
                resolved_legs: 0,
                table: PEG_TABLE
            })
        );
    }

    /// Two rows for one identity means the registry is ambiguous — the counts
    /// would be a union across legs and the alarm would be reading something
    /// nobody designed. Refuse that too, rather than picking one.
    #[test]
    fn an_ambiguous_usdt_identity_is_refused() {
        assert_eq!(
            peg_metric(&PegCounts {
                resolved_legs: 2,
                ..healthy_peg()
            }),
            Err(SanityRefusal::UnresolvableLeg {
                resolved_legs: 2,
                table: PEG_TABLE
            })
        );
    }

    /// The guard `resolved_legs` cannot provide. The identity resolves cleanly,
    /// so the first guard passes — but the `asset_id` it resolves to is not the
    /// `quote_asset_id` the candles carry (task 0139 renumbering, a registry
    /// rewrite), so the scan matches nothing. The count is zero because nothing
    /// was examined, and `NOT_BREACHING` would score that healthy.
    #[test]
    fn a_resolved_leg_that_matches_no_candles_is_refused() {
        assert_eq!(
            peg_metric(&PegCounts {
                scanned: 0,
                ..healthy_peg()
            }),
            Err(SanityRefusal::EmptyScan {
                table: PEG_TABLE,
                lookback_seconds: PEG_LOOKBACK_SECONDS
            })
        );
    }

    /// ⚠️ The whole point of splitting the guards. A `_1m` scan that matched
    /// nothing says nothing about `_1h`; suppressing a working signal because an
    /// unrelated tier failed would be the muting failure arrived at from the
    /// other side.
    #[test]
    fn one_direction_refusing_does_not_suppress_the_other() {
        let peg = peg_metric(&PegCounts {
            scanned: 0,
            ..healthy_peg()
        });
        let stranded = stranded_metric(&StrandedCounts {
            stranded: 42,
            ..healthy_stranded()
        });
        assert!(peg.is_err());
        assert_eq!(stranded.expect("published on its own evidence").value, 42.0);
    }

    /// The two refusals must not read alike: one sends the operator to the asset
    /// registry, the other to the candles.
    #[test]
    fn the_two_refusals_name_different_first_moves() {
        let unresolvable = SanityRefusal::UnresolvableLeg {
            resolved_legs: 0,
            table: PEG_TABLE,
        }
        .to_string();
        let empty = SanityRefusal::EmptyScan {
            table: PEG_TABLE,
            lookback_seconds: PEG_LOOKBACK_SECONDS,
        }
        .to_string();
        assert!(unresolvable.contains("prices.assets"));
        assert!(empty.contains("quote_asset_id"));
        assert_ne!(unresolvable, empty);
    }

    /// ⚠️ Since the directions read different tiers, a refusal that did not name
    /// the table would send the operator to look at a table that was fine.
    #[test]
    fn a_refusal_names_the_tier_it_examined() {
        assert!(
            SanityRefusal::EmptyScan {
                table: PEG_TABLE,
                lookback_seconds: PEG_LOOKBACK_SECONDS,
            }
            .to_string()
            .contains(PEG_TABLE)
        );
        assert!(
            SanityRefusal::UnresolvableLeg {
                resolved_legs: 0,
                table: STRANDED_TABLE,
            }
            .to_string()
            .contains(STRANDED_TABLE)
        );
    }

    /// ⚠️ Pins the stranded tier against a "cheaper coarser table" optimisation,
    /// which is exactly the change that was measured and REVERSED on 2026-08-19.
    ///
    /// The grace is measured from a bucket's `timestamp`, which is its START, so
    /// a coarse tier spends its own width out of the grace before its data even
    /// exists — 24 of a 48 h grace on `_1d`, one hour on `_1h`.
    #[test]
    fn the_stranded_tier_does_not_let_bucket_width_eat_the_grace() {
        assert_eq!(STRANDED_TABLE, "price_ohlcv_1h");

        // The bucket width must stay small relative to the grace. At `_1d` this
        // ratio is 1/2 — half the grace gone before enrichment can start.
        let bucket_width_seconds = 3_600_i64;
        assert!(
            bucket_width_seconds * 8 <= STRANDED_GRACE_SECONDS,
            "a bucket wide enough to consume a meaningful share of the grace makes \
             the stranded metric measure the tier rather than the defect"
        );
    }

    /// The grace is **BE's loss window**, not a lag measurement.
    ///
    /// ⚠️ This test previously asserted a 30 h "measured enrichment ceiling"
    /// with 12 h of headroom, from an age distribution taken on 2026-08-19.
    /// Re-measured a day later the frontier was at 47 h, and the cause turned
    /// out not to be lag at all — the USDT pivot has never priced a `_1m` row
    /// (task 0209). A headroom assertion over a broken pipeline measures
    /// nothing, so it is replaced by the property that is actually invariant:
    /// the grace is the window the downstream consumer reads.
    ///
    /// ⛔ If this alarm is noisy, fix 0209. Do not raise the grace.
    #[test]
    fn the_grace_is_bes_loss_window_not_a_lag_estimate() {
        let bes_loss_window_seconds = 48 * 3_600_i64;
        assert_eq!(
            STRANDED_GRACE_SECONDS, bes_loss_window_seconds,
            "the grace means 'a consumer has begun losing a value'; changing it \
             silently changes what the alarm asserts"
        );
    }

    /// The peg tolerance is a rounding allowance, not a judgement call. USDT
    /// trades at ~0.13-0.15, so a real value must sit far outside the band.
    #[test]
    fn the_peg_tolerance_cannot_reach_a_real_usdt_price() {
        let measured_2026 = 0.15_f64;
        assert!((measured_2026 - 1.0).abs() > PEG_RATIO_TOLERANCE);
    }
}
