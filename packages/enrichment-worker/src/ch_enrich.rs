//! Production enrichment path — the batch ASOF-JOIN form (G-note Part
//! A.2, Option 2), adapted to the real ADR-0007 ClickHouse schema.
//!
//! This is the production swap for the prototype's
//! `CandidateSource` → `OraclePriceLookup` → `EnrichmentSink`
//! pipeline. Those three per-row trait seams **dissolve** here: the
//! whole enrichment is one set-based SQL statement that reads the
//! zero-valued candidates, forward-fills the oracle price via an
//! `ASOF LEFT JOIN`, computes both `volume_quote_usd`
//! (`oracle_usd × volume_quote`) and `close_usd`
//! (`oracle_usd × close`, task 0061), and re-inserts the corrected rows
//! in a single server-side pass.
//!
//! ## USD-close reference tiers (task 0061 §12.1)
//!
//! `close_usd` (and the still-missing `volume_quote_usd`) are filled in three
//! ordered tiers, run inside [`ChEnrichmentPass::run`]. The order is an evidence
//! ranking: a reading we polled, then a reading someone else measured, then an
//! assumption. Each tier only ever touches rows the tiers above left at
//! `close_usd = 0`.
//!
//! 1. **Recent-window oracle tier** ([`ChEnrichmentPass::enrich_batch`]) — the
//!    `ASOF LEFT JOIN oracle_prices`. Sets the USD columns wherever a Reflector
//!    row exists for the candle's `quote_asset_id` within the staleness window.
//!    This is the depeg-aware tier and it wins where it applies.
//! 2. **External tier** ([`ChEnrichmentPass::run_external_tier`], task 0268) —
//!    a USDC-quoted candle is priced from the MEASURED USDC/USD rate task 0267
//!    imports into `prices.usd_rate` (`method = 'external'`), resolved at the
//!    bucket's END. USDC closed at **0.9681** on 2023-03-11, so this is the tier
//!    that stops ~654,291 deep-history candles being stored 3% wrong.
//!
//!    ⚠️ **It is not the oracle tier, and that is deliberate.** The obvious
//!    shortcut — insert the imported series into `oracle_prices` under a pseudo
//!    oracle name and let tier 1 price it — is rejected twice over: task 0247
//!    forbids publishing an import as `oracle`, since `oracle` means "we polled
//!    Reflector"; and `oracle_prices` staying untouched is what preserves the
//!    meaning of the 0182 reset's oracle-shadow guard, which refuses to re-open
//!    a quote leg the oracle can re-price.
//!
//!    The tier writes only the USD columns — candles store no `method`. It is
//!    visible on the wire because `/ohlcv` reconstructs provenance at read time
//!    and labels a scaled pre-epoch USDC leg `external`
//!    (`prices-api`'s `queries_ch::usd_method_expr`).
//! 3. **Peg-pivot tier** ([`ChEnrichmentPass::enrich_peg_pivot_step`]) — the
//!    deep-history backbone for candles the oracle tier left at `close_usd = 0`:
//!    - **peg:** a USDC-quoted candle gets `close_usd = close × $1`, exact and
//!      oracle-free, back to SDEX genesis;
//!    - **pivot:** a candle quoted in a *measured* reference asset gets
//!      `close_usd = close × ref_usd`, where `ref_usd` is that asset's
//!      volume-weighted close against USDC at or before the bucket,
//!      forward-filled by an `ASOF LEFT JOIN`. The reference assets are XLM and
//!      USDT — see [`ReferenceIds::pivot_ids`].
//!
//! ⚠️ **USDT is NOT a $1 peg (task 0172).** It sat in the peg tier until
//! 2026-08-12, which valued every USDT-quoted candle at par; the canonical
//! Stellar USDT depegged in June 2022 and trades at ~$0.13, so that overstated
//! `close_usd` by ~7.4x on 44,657 candles across 495 base assets. It is now
//! priced by measurement through the pivot, exactly like XLM.
//!
//! A USDC-quoted candle with no imported rate for its bucket still reaches the
//! peg tier and is still valued at `close × $1`. That is the correct outcome —
//! it is the best available — and it is also why the 0268 reset joins the
//! external row before zeroing anything: a row no tier can refill must never be
//! re-opened (the 157 candles task 0182 destroyed).
//!
//! Candles whose quote is none of USDC/XLM/USDT (and had no oracle) keep
//! `close_usd = 0` — never a wrong non-NULL value (the view's `no_reference`).
//! The peg-pivot tier preserves any `volume_quote_usd` the oracle tier already
//! set (`if(volume_quote_usd > 0, …)`), so the depeg-aware value is never
//! clobbered by the `$1` peg.
//!
//! ## Why a direct `INSERT … SELECT` (not the staging table)
//!
//! The original G-note sketched a `staging → promote → truncate`
//! dance whose promote step keyed on `_inserted_at = max(_inserted_at)`.
//! That version column never existed: the real schema versions rows by
//! a ledger-derived `version UInt64` (ADR 0007), not a wall clock. With
//! `version = original + 1` the staging indirection buys nothing — a
//! single `INSERT … SELECT` is already idempotent (the
//! `FINAL WHERE volume_quote_usd = 0` read filter stops picking
//! already-enriched rows) and self-healing on partial crashes (fewer
//! rows enriched this pass; the rest roll over to the next). So we
//! INSERT straight into the live table.
//!
//! ## Scope: `price_ohlcv_1m` only
//!
//! Enrichment targets the base 1-minute table. The rolled-up
//! granularities (`_15m … _1M`) are produced by the MV chain (task
//! 0051); how those views re-aggregate a *re-inserted* `_1m` row and
//! what `version` they project onto their `ReplacingMergeTree` targets
//! is a 0051 concern. See the dependency note in the task 0026 G-note.

use clickhouse::Client;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// Canonical Stellar issuers for the USD reference assets, used by the
/// peg-pivot tier to recognise USDC (the $1 peg) and USDT (a *measured*
/// reference, task 0172) quote assets in `prices.assets`.
/// Re-exported from `prices-clickhouse` (the single source of truth, also used by
/// `sdex-backfill`) so the `asset_id` the backfill interns under these issuers
/// matches the `quote_asset_id` enriched here — no hand-synced literal to drift.
use prices_clickhouse::{USDC_ISSUER, USDT_ISSUER};

#[derive(Debug, thiserror::Error)]
pub enum ChEnrichError {
    #[error("clickhouse: {0}")]
    Clickhouse(#[from] clickhouse::error::Error),

    /// A per-partition `FREEZE` snapshot was refused. Carries its own remedy
    /// because the underlying driver error is frequently opaque: the prod run on
    /// 2026-07-23 surfaced this as `Clickhouse(BadResponse(""))` — an empty body
    /// — when the real cause was a missing `ALTER FREEZE PARTITION` grant, only
    /// visible by replaying the statement over curl.
    #[error(
        "FREEZE snapshot failed on {table} partition {month}: {source}\n\
         The repair user most likely lacks `ALTER FREEZE PARTITION ON <db>.*`. Either:\n\
         (a) grant it — note this is impossible for users defined in users.xml, \
         which is read-only storage (ACCESS_STORAGE_READONLY); or\n\
         (b) have an operator FREEZE the partitions as CH admin, verify them under \
         shadow/, then re-run with --skip-snapshot.\n\
         See docs/runbooks/repair-coarse-usd-values.md."
    )]
    FreezeDenied {
        table: String,
        month: u32,
        #[source]
        source: clickhouse::error::Error,
    },

    /// A [`UsdResetSpec`] run was refused because `oracle_prices` still holds
    /// rows for the quote asset being reset. Encodes task 0182's first ordering
    /// constraint as a runtime gate rather than a paragraph in a task file: the
    /// oracle tier runs *before* the peg-pivot tier and wins where it applies, so
    /// resetting while those rows exist re-applies the very value the reset is
    /// meant to remove — and re-labels it `method = 'oracle'`, which a consumer
    /// reads as *more* authoritative than the placeholder it replaced.
    #[error(
        "USD reset refused: prices.oracle_prices holds {rows} row(s) for \
         quote asset_id {quote_asset_id} under oracle '{oracle_name}' inside the \
         reset's own window {window}.\n\
         The oracle tier runs before the peg-pivot tier and wins where it applies, \
         so resetting now would re-apply the oracle's rate to every row this reset \
         zeroes — and label it method='oracle'.\n\
         If the window is all time: purge those rows first (task 0196), verify the \
         count is 0, then re-run.\n\
         If the window is bounded: the overlap is real — narrow --reset-not-after \
         below the first oracle row, or widen --reset-not-before above the last \
         one, so the reset and the oracle tier do not both claim the same span.\n\
         See lore/1-tasks/active/0182_BUG_close-usd-overstated-7x-on-usdt-quoted-candles.md."
    )]
    ResetBlockedByOracleRows {
        quote_asset_id: u32,
        oracle_name: String,
        rows: u64,
        /// The `[not_before, not_after)` span the count was taken over, rendered
        /// for the operator. Without it the message names a number with no
        /// denominator, and "widen or narrow?" is unanswerable.
        window: String,
    },

    /// A [`UsdResetSpec`] asked for `require_external_rate` while
    /// `prices.oracle_prices` holds a canonical USDC reading **below**
    /// `USDC_ORACLE_EPOCH_S` (task 0268 review, WR-09). See
    /// [`ChEnrichmentPass::assert_no_pre_epoch_oracle_rows`].
    #[error(
        "USD reset refused: prices.oracle_prices holds {rows} row(s) for quote \
         asset_id {quote_asset_id} from oracle '{oracle_name}' stamped before \
         USDC_ORACLE_EPOCH_S ({epoch}). The 0268 mode assumes no poll priced USDC \
         before that instant — the wire label, the reset window and the external \
         tier's volume_quote_usd overwrite all key on it — and this count says \
         otherwise. Triage those rows first (docs/runbooks/repair-coarse-usd-values.md, \
         Appendix B, precondition 3); do not work around this.",
        epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S
    )]
    ResetBlockedByPreEpochOracleRows {
        quote_asset_id: u32,
        oracle_name: String,
        rows: u64,
    },

    /// A [`UsdResetSpec`] asked for `require_external_rate` while
    /// `prices.usd_rate` holds **no** `method = 'external'` row at all (task
    /// 0268) — i.e. task 0267's series is not loaded.
    ///
    /// An error, never a `warn!`. With no external rows the day-set predicate
    /// matches nothing, so the reset would zero nothing and the run would report
    /// a clean, healthy, entirely empty repair — the same green all-clear that
    /// hid task 0182 for a month. Worse, if the predicate were ever relaxed, the
    /// rows it zeroed would be refilled with the same `$1` by the peg tier:
    /// `version` churn, a spent FREEZE rollback point, and no change in value.
    #[error(
        "USD reset refused: a rate-gated reset mode was passed \
         (--reset-require-external-rate or --reset-require-pivot-usdc-rate), but \
         prices.usd_rate holds 0 rows with method = 'external' for canonical USDC \
         (quote asset_id {quote_asset_id}).\n\
         Task 0267's measured USDC/USD series is not loaded, so the day-set \
         predicate both modes share matches nothing and this reset can refill \
         nothing: it would re-open rows only for the tier below to write the same \
         $1 back, bumping version and spending the FREEZE rollback point for no \
         change in value.\n\
         Load and verify 0267's series first — see docs/runbooks/repair-coarse-usd-values.md, \
         Appendix B precondition 2 (0268 mode) or Appendix C precondition 1 \
         (0228 mode) — then re-run."
    )]
    ResetRequiresExternalRates { quote_asset_id: u32 },

    /// A `require_external_rate` reset on a SUB-DAILY table while the imported
    /// series holds only daily rows (task 0268). See
    /// [`ChEnrichmentPass::assert_hourly_rates_are_loaded`].
    #[error(
        "USD reset refused on {table}: --reset-require-external-rate was passed, \
         but prices.usd_rate holds no HOURLY `external` row for canonical USDC \
         (every imported row sits at UTC midnight — only the daily file is loaded).\n\
         A sub-daily candle priced now takes the DAY close, and the repair cannot \
         be redone later: the reset re-opens only rows still carrying the $1 \
         signature (close_usd = close), and a row priced at the day close no \
         longer does. Loading the hourly file afterwards would change nothing for \
         these candles — on 2023-03-11 the 12:00 hour would stay at 0.96812 \
         instead of 0.90687, about 7% off.\n\
         Load and promote the hourly file first — docs/runbooks/load-external-usdc-rate.md, \
         section 4½ — then re-run. Daily-grain tables (price_ohlcv_1d/1w/1M) are \
         not affected and do not need it."
    )]
    ResetRequiresHourlyRates { table: String },

    /// `require_external_rate` asked for on a quote leg that is not canonical
    /// USDC (task 0268 review). The external tier can only refill USDC.
    #[error(
        "USD reset refused: --reset-require-external-rate was passed for quote \
         asset_id {quote_asset_id}, which is not canonical USDC (asset_id \
         {usdc_id}; 0 means canonical USDC is not a tracked asset here at all).\n\
         Every part of the external path is pinned to canonical USDC: the day-set \
         predicate, the loaded-rates check and the external tier's own statement. \
         A reset on another quote leg would therefore zero rows on USDC's rate \
         days and leave the external tier unable to refill a single one — the \
         peg tier would write $1 back over every one of them, which is task \
         0182's incident with a different quote asset.\n\
         Reset this leg without --reset-require-external-rate, or target \
         canonical USDC."
    )]
    ResetExternalRateLegIsNotUsdc { quote_asset_id: u32, usdc_id: u32 },

    /// Both reset modes asked for at once (task 0228). They select DIFFERENT
    /// candidate signatures, and the intersection of the two is empty.
    #[error(
        "USD reset refused: --reset-require-external-rate and \
         --reset-require-pivot-usdc-rate are mutually exclusive (quote asset_id \
         {quote_asset_id}).\n\
         The 0268 mode selects rows carrying the peg tier's par signature \
         (close_usd = close) on canonical USDC; the 0228 mode selects PIVOTED \
         rows, which never carry that signature. Asking for both renders both \
         predicates, so the candidate set is empty and the run reports a clean, \
         entirely empty repair — the same green all-clear that hid task 0182 for \
         a month.\n\
         Run one mode per pass: --reset-require-external-rate for the canonical \
         USDC leg, --reset-require-pivot-usdc-rate for an XLM or USDT leg."
    )]
    ResetModesAreMutuallyExclusive { quote_asset_id: u32 },

    /// `require_pivot_usdc_rate` asked for on a quote leg the PIVOT cannot
    /// refill (task 0228) — the mirror image of
    /// [`ChEnrichError::ResetExternalRateLegIsNotUsdc`].
    #[error(
        "USD reset refused: --reset-require-pivot-usdc-rate was passed for quote \
         asset_id {quote_asset_id}, which is not one of this pass's pivot \
         references (pivot: {pivot:?}; canonical USDC is asset_id {usdc_id}).\n\
         This mode re-opens rows the SCALED PIVOT recomputes, and the pivot only \
         runs for a quote leg it has a USDC market to measure against. On any \
         other leg it would zero rows the pivot cannot reach, and the peg tier \
         would write $1 back over them — task 0182's incident with a different \
         quote asset.\n\
         If you meant canonical USDC, that is the 0268 mode: use \
         --reset-require-external-rate instead."
    )]
    ResetPivotRateLegIsNotAPivotReference {
        quote_asset_id: u32,
        usdc_id: u32,
        pivot: Vec<u32>,
    },

    /// A [`UsdResetSpec`] whose `[not_before, not_after)` window is empty (task
    /// 0268 review, WR-05). See [`UsdResetSpec::validate`] for why this is an
    /// error and not a no-op.
    #[error(
        "USD reset refused: --reset-not-before {not_before} is at or above the \
         reset's upper bound {not_after} (quote asset_id {quote_asset_id}). The \
         window [{not_before}, {not_after}) is empty, so the reset would match \
         nothing and report a clean run having touched nothing.\n\
         --reset-not-after defaults to USDC_ORACLE_EPOCH_S ({epoch}) when \
         --reset-require-external-rate is passed; check the two values are in \
         seconds, not milliseconds, and in the right flags.",
        epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S
    )]
    ResetWindowEmpty {
        quote_asset_id: u32,
        not_before: u32,
        not_after: u32,
    },

    /// A [`UsdResetSpec`] named a quote leg that **no tier can price**, so every
    /// row it zeroed would stay at `close_usd = 0` permanently.
    ///
    /// This is the one way the repair can end up strictly worse than the defect
    /// it corrects: a wrong-but-visible number becomes the ambiguous zero that
    /// ~130 unguarded `argMax(close_usd, …)` sites read as a real price. A
    /// mistyped id is enough — `11` for `111` passes the oracle gate, because an
    /// asset with no Reflector rows is exactly what that gate is looking for.
    #[error(
        "USD reset refused: quote asset_id {quote_asset_id} is not a peg or pivot \
         reference (peg: {stable:?}, pivot: {pivot:?}), so no tier can recompute \
         the rows this reset would zero — they would stay at close_usd = 0 \
         permanently, which is worse than the wrong value they hold now.\n\
         Check the id against prices.assets; a mistyped id passes the oracle \
         check because an unknown asset has no oracle rows either."
    )]
    ResetTargetHasNoPricingPath {
        quote_asset_id: u32,
        stable: Vec<u32>,
        pivot: Vec<u32>,
    },

    /// A [`UsdResetSpec`] was combined with a bounded (`one_shot = false`) pass.
    ///
    /// The peg-pivot tier — the only one that can refill a pivoted quote leg — is
    /// gated on the oracle tier draining. A bounded pass that exhausts its batch
    /// budget while still making progress defers that tier to the next run, so
    /// the reset's zeroes would be left published until then.
    #[error(
        "USD reset refused: a reset requires one_shot = true. In a bounded pass the \
         peg-pivot tier can be deferred (it is gated on the oracle tier draining), \
         which would leave the rows this reset zeroes published at close_usd = 0 \
         until a later run."
    )]
    ResetRequiresOneShot { quote_asset_id: u32 },
}

/// Opt-in reset of **already-written** USD columns, so a corrected pricing tier
/// can recompute them (task 0182).
///
/// ## Why this exists
///
/// Enrichment is idempotent because every tier filters on `close_usd = 0` —
/// an already-enriched row is never revisited. That is exactly right in steady
/// state and exactly wrong after a *pricing* defect: [`ReferenceIds::pivot_ids`]
/// documents how USDT-quoted candles were valued at par until 2026-08-12, and
/// those 44,657 rows are inert. Nothing will ever look at them again, because
/// they are non-zero.
///
/// So a repair needs one thing the steady-state pass deliberately lacks: a way to
/// put a row *back* into the candidate set. That is all this does.
///
/// ## Why it is deliberately narrow
///
/// This is the only mechanism in the worker that discards a computed value, so
/// every field here is a bound rather than an option:
///
/// * `quote_asset_id` — one quote leg per run. Never "all rows with a suspect
///   price": the blast radius must be nameable before the statement runs.
/// * `not_before` — the epoch below which the *old* value is correct and must
///   survive. For USDT that is 2021-02-07, when the USDT/USDC market the pivot
///   measures against begins; before it the pivot has no reference, and the `$1`
///   already on disk is right because the asset was genuinely at par (task 0172).
///   Reset those and they stay at `close_usd = 0` forever.
///
/// The statement additionally mirrors the pivot's own `volume_quote > 0` filter,
/// so it will not zero a row the pivot is structurally unable to refill.
///
/// ⚠️ Rows whose reference is missing or stale beyond `pivot_window_s` are still
/// reset and *not* refilled — that residue cannot be predicted from the candidate
/// side alone. Run against a `FREEZE`d partition and check `zeros_after`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsdResetSpec {
    /// The quote `asset_id` whose candles get their USD columns zeroed.
    pub quote_asset_id: u32,
    /// Earliest candle `timestamp` (unix seconds) eligible for reset. Rows older
    /// than this keep whatever they hold — see the epoch note above.
    pub not_before: u32,
    /// **Exclusive** upper bound on candle `timestamp` (task 0268). `None` keeps
    /// the pre-0268 behaviour exactly: unbounded above.
    ///
    /// The reset is scoped BELOW the window where the oracle tier operates, and
    /// that is what makes [`ChEnrichmentPass::assert_reset_not_shadowed_by_oracle`]'s
    /// premise false there. Without it, USDC's live oracle rows from 2026-03-11
    /// onward refuse a reset of 2020-2025 history the oracle cannot reach — an
    /// all-time guard applied to a bounded operation.
    pub not_after: Option<u32>,
    /// When true (task 0268), the candidate set is additionally
    /// `close_usd = close` — the peg tier's EXACT signature, so nothing the
    /// oracle or external tier priced is re-opened — **and** a `usd_rate` row
    /// with `method = 'external'` must exist for the bucket's UTC day.
    ///
    /// ## Why a predicate and not an epoch
    ///
    /// This is task 0182's lesson encoded in the only form that can express it.
    /// 0182's reset epoch sat 19 hours before the reference market's first
    /// candle, and 157 candles were zeroed with nothing able to refill them. An
    /// epoch asserts "a reference exists from here on" — a claim about the whole
    /// span, made once, by hand. A semi-join asks "does a reference exist for
    /// THIS bucket" and cannot be wrong.
    pub require_external_rate: bool,
    /// When true (task 0228), the candidate set is every already-written row of
    /// the named PIVOT quote leg for whose bucket a `method = 'external'` USDC
    /// rate exists — the same [`external_rate_day_pred`] the 0268 mode uses, and
    /// nothing else appended.
    ///
    /// ## Why no par signature, and what that costs
    ///
    /// 0268's candidate carries `close_usd = close`, the peg tier's exact
    /// signature, which is **self-erasing**: once the row is re-priced it stops
    /// matching, so a second run finds zero candidates. A pivoted row has no such
    /// signature — it never equalled its own close. Telling "already scaled" from
    /// "not scaled" would mean comparing `close_usd / close` against the bucket's
    /// own reference vwap: a CORRELATED join, which the one-definition-three-sites
    /// rule forbids outright (see [`external_rate_day_pred`]'s "why uncorrelated"
    /// note), and `version` cannot stand in for it because coarse rollups carry
    /// large summed versions.
    ///
    /// So this mode is **value-idempotent, not a fixed point across runs**: a
    /// second run over a repaired month recomputes identical values at
    /// `version + 2`. That is the framing [`repair_target_pred`] already documents
    /// for the 0182 mode, and it is why this mode is operator-only — the recurring
    /// sweep keeps its `usd_reset: None` pin. Zero-candidate idempotence would
    /// need a stored provenance column, i.e. a schema change.
    ///
    /// ⚠️ Mutually exclusive with [`UsdResetSpec::require_external_rate`], refused
    /// by [`UsdResetSpec::validate`]: the two candidate signatures do not
    /// intersect, so asking for both selects nothing at all.
    pub require_pivot_usdc_rate: bool,
}

impl UsdResetSpec {
    /// Refuse a window that can match nothing (task 0268 review, WR-05).
    ///
    /// `reset_pending_pred` renders `timestamp >= not_before AND timestamp <
    /// not_after`; with `not_before >= not_after` that is unsatisfiable, so
    /// `count_reset_pending` returns 0, [`ChEnrichmentPass`] reports `Ok(0)`, and
    /// the repair driver's month enumeration — which runs BEFORE `reset_step`
    /// and splices the same predicate — finds no month worth visiting. The run
    /// then completes green having discarded and repaired nothing: the exact
    /// "clean, healthy, entirely empty repair" that hid task 0182 for a month.
    ///
    /// A mistyped year, a millisecond timestamp, or the epoch copied into the
    /// wrong flag all produce that shape. Every other footgun in the reset path
    /// is refused rather than warned about; this one is too. Pure, so the CLI
    /// can refuse before it opens a connection and the library refuses even
    /// when driven by something other than the CLI.
    pub fn validate(&self) -> Result<(), ChEnrichError> {
        // Task 0228: the two modes select different candidate signatures, and
        // both predicates are APPENDED, so asking for both ANDs a par signature
        // onto a population that by construction never carries one. The result is
        // an empty candidate set and a green run that touched nothing — the same
        // shape WR-05 refuses below. Checked first because it is the cheaper
        // mistake to make and the more confusing one to diagnose.
        if self.require_external_rate && self.require_pivot_usdc_rate {
            return Err(ChEnrichError::ResetModesAreMutuallyExclusive {
                quote_asset_id: self.quote_asset_id,
            });
        }
        if let Some(na) = self.not_after
            && self.not_before >= na
        {
            return Err(ChEnrichError::ResetWindowEmpty {
                quote_asset_id: self.quote_asset_id,
                not_before: self.not_before,
                not_after: na,
            });
        }
        Ok(())
    }
}

/// The steady-state candidate shape: a row missing either USD column, with
/// volume to price. Shared by [`ChEnrichmentPass::count_candidates`] and the task
/// 0114 repair driver's month enumeration.
pub const CANDIDATE_PRED: &str = "(volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0";

/// Rows a [`UsdResetSpec`] still has work to do on: the named quote leg, at or
/// after the epoch, **still holding a written USD value**.
///
/// The `(close_usd > 0 OR volume_quote_usd > 0)` term is what makes the reset
/// loop terminate — a reset row zeroes both columns and immediately stops
/// matching. It also mirrors the pivot's `volume_quote > 0` so the reset cannot
/// zero a row the pivot is structurally unable to refill.
pub fn reset_pending_pred(db: &str, spec: &UsdResetSpec) -> String {
    let mut pred = format!(
        "quote_asset_id = {q} AND timestamp >= toDateTime({nb}) \
         AND (close_usd > 0 OR volume_quote_usd > 0) AND volume_quote > 0",
        q = spec.quote_asset_id,
        nb = spec.not_before,
    );
    // Both extensions APPEND, so a spec that asks for neither renders exactly the
    // pre-0268 string — the 0182 repair path, which has already run against
    // production, must not change by a byte.
    if let Some(na) = spec.not_after {
        pred.push_str(&format!(" AND timestamp < toDateTime({na})"));
    }
    if spec.require_external_rate {
        pred.push_str(&format!(
            " AND close_usd = close AND {}",
            external_rate_day_pred(db)
        ));
    }
    // Task 0228: the pivot-leg mode appends the SAME rate fragment and NOTHING
    // else. No par signature — a pivoted row never carries one, so including it
    // would select nothing (see `UsdResetSpec::require_pivot_usdc_rate`).
    if spec.require_pivot_usdc_rate {
        pred.push_str(&format!(" AND {}", external_rate_day_pred(db)));
    }
    pred
}

/// **"An imported rate exists for this bucket's UTC day"** — written ONCE, and
/// spliced verbatim into [`reset_pending_pred`], [`reset_sql`] and (through
/// [`repair_target_pred`]) the repair driver's month enumeration.
///
/// One definition because the three must agree. When they did not — the driver
/// enumerating one predicate while the statement acted on another — a `--dry-run`
/// reported "no months with enrichable zeros" over 44,657 wrong values, and task
/// 0182 stayed invisible for a month. The strings cannot drift if there is only
/// one of them.
///
/// ## Why UNCORRELATED
///
/// `repair::months_with_zeros` splices `repair_target_pred` into the bare `WHERE`
/// of its own grouped scan, where no outer alias is in scope. A correlated
/// `EXISTS (… WHERE r.day = p.day)` or a JOIN is not a stylistic preference
/// there — it is a syntax error. An uncorrelated `IN (SELECT …)` is legal in all
/// three contexts unchanged, which is what lets one string serve all of them.
///
/// ## Why the DAY, and why under-reaching is the safe direction
///
/// The grain is the UTC day because task 0267's series is daily. For `_1w` and
/// `_1M` this is deliberately NARROWER than the tier's own rule: the tier
/// resolves at the bucket's END and accepts an anchor up to a bucket old, while
/// this demands a row on the bucket's FIRST day. So a monthly candle whose only
/// covering rate falls mid-month is skipped by the reset even though the tier
/// could have refilled it.
///
/// That asymmetry is chosen, not overlooked. The reset skipping a refillable row
/// costs one stale value that a later run can still fix; the reset zeroing an
/// unrefillable row is the incident. Only one of those two errors is
/// recoverable, so the predicate is biased toward the recoverable one.
///
/// ## Why `'UTC'` is spelled out
///
/// `timestamp` is a bare `DateTime`, so `toDate(timestamp)` uses the SERVER
/// timezone and nothing in this repo pins it. On a non-UTC server a rate
/// stamped 00:00 UTC and a candle at 23:30 UTC land on different local dates,
/// and the day-set slides by the UTC offset. `views.sql`'s position is that
/// correctness must not depend on the server timezone; this predicate takes the
/// same position by naming the zone at the expression.
fn external_rate_day_pred(db: &str) -> String {
    format!(
        "toDate(timestamp, 'UTC') IN (SELECT toDate(timestamp, 'UTC') FROM {db}.usd_rate FINAL \
         WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
           AND issuer_address = '{USDC_ISSUER}' AND contract_address = '' \
           AND method = 'external' AND usd_rate > 0)"
    )
}

/// What makes a month *worth visiting* for the task 0114 repair driver: it holds
/// steady-state zeros, **or** it holds rows a reset is going to re-open.
///
/// This exists because the two halves disagreed before task 0182, and that
/// disagreement is what made the defect invisible. The driver enumerates
/// [`CANDIDATE_PRED`]; every row 0182 targets has `close_usd > 0`; so a
/// `--dry-run` reported *"no months with enrichable zeros"* — a green all-clear
/// over 44,657 wrong values, indistinguishable from a genuinely clean table.
///
/// ⚠️ **Not a fixed point across runs.** Within a run it converges: the reset
/// zeroes the rows, the pivot refills them, and the reset arm stops matching
/// mid-run. But a *second* invocation sees the refilled rows and matches them
/// again, resetting and recomputing values that are already correct. That is
/// value-idempotent (same reference, same result) but it is not free, and it
/// bumps `version` each time. The reset is therefore a deliberate one-off
/// operator action — never wired into the recurring sweep, which pins
/// `usd_reset: None` for exactly this reason.
pub fn repair_target_pred(db: &str, reset: Option<&UsdResetSpec>) -> String {
    match reset {
        None => CANDIDATE_PRED.to_string(),
        Some(spec) => format!("({CANDIDATE_PRED}) OR ({})", reset_pending_pred(db, spec)),
    }
}

/// Config for one production enrichment run. Mirrors the prototype's
/// env-driven knobs (`ORACLE_NAME`, `FORWARD_FILL_WINDOW_S`,
/// `BATCH_SIZE`, `MAX_BATCHES`) plus the CH connection.
#[derive(Debug, Clone)]
pub struct ChEnrichConfig {
    pub url: String,
    pub database: String,
    pub table: String,
    pub oracle_name: String,
    pub window_s: u32,
    /// Max staleness (seconds) for the peg-pivot tier's XLM/USDC pivot: how far
    /// back the `ASOF` join may forward-fill a missing XLM/USDC close. Larger
    /// than `window_s` because deep history is sparser; XLM/USDC is liquid so
    /// gaps are normally small. Default 1 day.
    pub pivot_window_s: u32,
    /// Recency window (seconds) for the `EnrichmentRowsRemainingRecent` metric:
    /// only candles whose `timestamp` is within this window of `now()` count
    /// toward the recency-bounded backlog, so the permanent deep-history
    /// exotic-quote floor (pairs with no oracle/peg reference that will never
    /// enrich) is excluded and an *idle* env reads zero (task 0026 finding #5).
    ///
    /// Must be **≥ the stall alarm's 3×1h sustain window** (default 4h ≥ 3h).
    /// Earlier this was kept *shorter* than the sustain window; that was a bug:
    /// a genuinely stuck *fresh* candle aged out of the window before it could
    /// breach 3 consecutive hourly datapoints, so a real stall in a low-cadence
    /// env never paged. A window ≥ the sustain keeps a fresh stuck candle counted
    /// across all 3 datapoints (real stalls fire again) while the deep-history
    /// floor — candles *years* old — is still excluded, so an idle env still
    /// reads zero. See the decision note in the 0026 task README. Default 4 hours.
    pub recent_window_s: u32,
    pub batch_size: u64,
    pub max_batches: u32,
    /// One-shot historical-drain mode (spec §4): when `true`, each tier loops
    /// until it stops making progress instead of stopping at `max_batches`, so a
    /// single invocation clears the whole post-backfill backlog. An **explicit**
    /// flag, not a `max_batches` sentinel — `max_batches = 0` keeps its literal
    /// meaning (zero batches) so it can never silently become an unbounded drain.
    pub one_shot: bool,
    /// Optional `[start, end)` candle-`timestamp` window (unix seconds) that
    /// bounds every candidate scan to a single monthly partition — the task 0114
    /// coarse-repair driver sets this per month so a pass prunes to one
    /// `toYYYYMM(timestamp)` partition instead of full-scanning the table (task
    /// 0111 option 1). `None` (the default) is the unbounded hourly pass over
    /// `price_ohlcv_1m` and is byte-identical to the pre-0114 behaviour. Only the
    /// **candidate** side is bounded; the pivot tier's inline XLM/USDC reference
    /// still forward-fills from earlier months (a cheap sort-key-prefix scan), so
    /// a month's first buckets keep a valid pivot anchor.
    pub time_window: Option<(u32, u32)>,
    /// Opt-in USD-column reset run **before** the tiers, so already-written
    /// values re-enter the candidate set (task 0182). `None` — the default, and
    /// the only value the scheduled Lambda ever uses — makes a pass
    /// byte-identical to its pre-0182 behaviour; a pass can only discard a
    /// computed value if an operator explicitly names the quote leg and epoch.
    /// See [`UsdResetSpec`].
    pub usd_reset: Option<UsdResetSpec>,
}

impl Default for ChEnrichConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8123".to_string(),
            database: "prices".to_string(),
            table: "price_ohlcv_1m".to_string(),
            oracle_name: "reflector".to_string(),
            window_s: 300,
            pivot_window_s: 86_400,
            recent_window_s: 14_400,
            batch_size: 10_000,
            max_batches: 20,
            one_shot: false,
            time_window: None,
            usd_reset: None,
        }
    }
}

/// Internal `asset_id`s of the USD reference assets, resolved from
/// `prices.assets` at the start of the peg-pivot tier. Any may be absent (e.g. a
/// dataset with no USDT trades), so each is optional.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ReferenceIds {
    xlm: Option<u32>,
    usdc: Option<u32>,
    usdt: Option<u32>,
}

impl ReferenceIds {
    /// Quote `asset_id`s that peg to exactly $1. **USDC only** — USDT is
    /// deliberately absent; see [`ReferenceIds::pivot_ids`] (task 0172).
    fn stable_ids(&self) -> Vec<u32> {
        [self.usdc].into_iter().flatten().collect()
    }

    /// Reference assets whose USD price is **measured** against USDC rather than
    /// assumed, in pivot order: XLM, then USDT. Each is used as the `ref_id` of a
    /// [`pivot_sql`] pass, so candles quoted in it get `close_usd = close × ref_usd`.
    ///
    /// ## Why USDT pivots instead of pegging (task 0172)
    ///
    /// The canonical Stellar USDT (`USDT_ISSUER`) **depegged in June 2022** and
    /// has traded at a deep discount ever since — ~$0.13 through 2026-08. This is
    /// not a data defect; it is confirmed by two markets that share no legs and no
    /// code path (its own USDC pair, and `XLM/USDC ÷ XLM/USDT`, which agree to
    /// within a cent), by four sibling stablecoins that held par through the same
    /// window in the same pipeline, and by `trade_count` collapsing 140,945 →
    /// 805/month as liquidity fled.
    ///
    /// Pegging it to $1 overstated `close_usd` by **~7.4×** on 44,657 candles
    /// across 495 base assets. Removing it from the peg set without adding it here
    /// would be worse than the bug: those candles would fall to `close_usd = 0`,
    /// which in this schema is ambiguous (missing / genuinely zero / not-yet-
    /// enriched) and is read unguarded by ~130 `argMax(close_usd, …)` sites.
    ///
    /// ⚠️ Do **not** "fix" this by sourcing USDT from the oracle. Reflector prices
    /// the *ticker* USDT — Tether's own token, genuinely at par — and we file that
    /// rate under this issuer's address, so `prices.usd_rate` asserts ~$1.00 for an
    /// asset worth $0.13. That mis-attribution is its own defect (task 0173).
    fn pivot_ids(&self) -> Vec<u32> {
        [self.xlm, self.usdt].into_iter().flatten().collect()
    }

    /// A pivot needs a reference asset and the USDC market to measure it against.
    fn can_pivot(&self) -> bool {
        self.usdc.is_some() && !self.pivot_ids().is_empty()
    }

    /// Whether the peg-pivot tier can do anything at all.
    fn has_any(&self) -> bool {
        !self.stable_ids().is_empty() || self.can_pivot()
    }
}

#[derive(Debug, clickhouse::Row, Deserialize)]
struct RefAssetRow {
    asset_id: u32,
    asset_code: String,
    issuer_address: String,
}

/// One `FINAL` scan of the volume-zero backlog, split into the full remainder
/// and the recency-bounded subset (see [`ChEnrichmentPass::count_remaining_at_volume_zero`]).
#[derive(Debug, clickhouse::Row, Deserialize)]
struct RemainingCounts {
    total: u64,
    recent: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ChPassStats {
    pub batches: u32,
    pub candidates_before: u64,
    pub candidates_after: u64,
    pub rows_enriched: u64,
    /// Rows a [`UsdResetSpec`] re-opened this pass (task 0182); 0 whenever
    /// `usd_reset` is `None`, which is every scheduled path.
    ///
    /// Report it alongside `rows_enriched`, never instead of it: a run where
    /// `rows_reset` far exceeds `rows_enriched` means the repair zeroed values it
    /// could not recompute — the one outcome worse than the defect, since a
    /// wrong-but-visible number became an ambiguous zero.
    pub rows_reset: u64,
    /// Candidates the recent-window oracle tier left unenriched this pass — the
    /// count handed down to (or deferred from) the peg-pivot tier. Maps to the
    /// `EnrichmentOracleMiss` CloudWatch metric (spec §5).
    pub oracle_misses: u64,
    /// Candles still at `volume_quote_usd = 0` after the pass — maps to the
    /// `EnrichmentRowsRemainingAtVolumeZero` metric (spec §5). Specifically the
    /// volume-USD backlog, NOT the general `candidates_after` remainder (which
    /// also includes `close_usd = 0` rows).
    pub rows_remaining_at_volume_zero: u64,
    /// The subset of `rows_remaining_at_volume_zero` whose candle `timestamp`
    /// falls within `cfg.recent_window_s` of `now()` — the recency-bounded
    /// backlog that excludes the permanent deep-history exotic-quote floor. Maps
    /// to the `EnrichmentRowsRemainingRecent` metric the stall alarm watches, so
    /// a genuinely idle env (no fresh candles) reads zero instead of latching on
    /// the floor (task 0026 finding #5).
    ///
    /// **Steady-state signal only.** The count mixes two clocks: the population
    /// ceiling is the pass-start `watermark` (frozen), the recency floor is
    /// `now()` at scan time. They only agree when the pass is short. In a
    /// **one-shot drain** longer than `recent_window_s`, `now()` advances past
    /// the frozen `watermark`, the `[now()-window, watermark]` interval goes
    /// empty, and this collapses to 0 regardless of the real fresh backlog. This
    /// is harmless — the alarm gates on the short scheduled pass and on
    /// `enriched < 1`, which a draining one-shot never satisfies — but during a
    /// long one-shot drain read `rows_remaining_at_volume_zero` (`total`), which
    /// stays correct, not this. Anchoring the floor to `watermark` instead would
    /// fix one-shot but re-break finding #5 (an idle env's `watermark` sits on
    /// the floor, so it would read >0 again); the `now()` anchor is the
    /// deliberate trade (task 0026 finding #2, accepted).
    pub rows_remaining_recent: u64,
    /// Wall-clock duration of the **whole pass**, milliseconds — every batch
    /// plus the `FINAL` count scans, not a single batch. Maps to the
    /// `EnrichmentPassDurationMs` CloudWatch metric (renamed from the misleading
    /// `EnrichmentBatchDurationMs`; task 0026 finding #7). `metrics::pass_metrics`
    /// also derives a true per-batch `EnrichmentAvgBatchDurationMs` from this and
    /// `batches`.
    pub duration_ms: u64,
}

pub struct ChEnrichmentPass {
    client: Client,
    cfg: ChEnrichConfig,
}

impl ChEnrichmentPass {
    pub fn new(cfg: ChEnrichConfig) -> Self {
        let client = Client::default()
            .with_url(&cfg.url)
            .with_database(&cfg.database);
        Self { client, cfg }
    }

    /// Build a pass from a pre-constructed client — e.g. the mTLS client from
    /// [`prices_clickhouse::mtls::client_from_lambda_env`] used by the Lambda
    /// entrypoint. The client's URL/TLS are already configured (`cfg.url` is
    /// ignored on this path); only `cfg.database` is re-applied so the pass and
    /// the client agree on the target database.
    pub fn with_client(client: Client, cfg: ChEnrichConfig) -> Self {
        let client = client.with_database(&cfg.database);
        Self { client, cfg }
    }

    /// Cold-start health check — fail Lambda Init, not per-event.
    pub async fn preflight(&self) -> Result<(), ChEnrichError> {
        self.client.query("SELECT 1").execute().await?;
        Ok(())
    }

    /// Per-tier batch budget. In one-shot mode (`cfg.one_shot`, spec §4) each
    /// tier loops until it stops making progress (effective bound `u32::MAX`),
    /// draining the whole backlog in a single invocation instead of the bounded
    /// `MAX_BATCHES × BATCH_SIZE` rows the hourly cron caps at. The no-progress /
    /// drained breaks in each tier guarantee termination regardless of the bound.
    /// `max_batches` keeps its literal meaning in both modes (so `0` = zero
    /// batches, never a hidden unbounded drain).
    fn effective_max_batches(&self) -> u32 {
        if self.cfg.one_shot {
            u32::MAX
        } else {
            self.cfg.max_batches
        }
    }

    /// SQL fragment restricting a candidate scan to `cfg.time_window` — the task
    /// 0114 partition bound. Returns `""` when unset, so the unbounded pass is
    /// byte-for-byte unchanged. The bounds are internal u32 unix timestamps (not
    /// user input), so they are inlined as `toDateTime(N)` literals rather than
    /// bound parameters — that keeps each statement's positional `bind()` order
    /// untouched. Paired with `PARTITION BY toYYYYMM(timestamp)`, the
    /// `col >= start AND col < end` predicate lets ClickHouse prune to the single
    /// month's parts, which is what makes per-pass cost independent of total table
    /// size (task 0111 option 1).
    fn window_pred(&self, col: &str) -> String {
        match self.cfg.time_window {
            Some((start, end)) => {
                format!(" AND {col} >= toDateTime({start}) AND {col} < toDateTime({end})")
            }
            None => String::new(),
        }
    }

    /// The newest candle `timestamp` (unix seconds) at pass start, used as a
    /// snapshot watermark. The pass only counts and enriches candidates at or
    /// before it, so candles the live Ledger Processor (task 0038) inserts *during*
    /// the pass — which carry newer ledger-close timestamps — are excluded from
    /// this pass's population. Without the bound, a concurrent insert can inflate
    /// the candidate count and falsely trip the `after >= remaining` no-progress
    /// break, stopping the pass with enrichable rows left. The newer candles are
    /// picked up by the next scheduled run. Returns 0 on an empty table.
    ///
    /// ⚠️ Deliberately **not** bounded by `cfg.time_window`, and measured, not
    /// assumed (task 0111, 2026-08-21). The suspicion was that this is a third
    /// full scan — `timestamp` is only the 4th sort-key column, so there is no
    /// index prefix to answer `max()` from, and every earlier measurement had
    /// filtered `query_log` on `INSERT INTO` and never looked at it. Prod says
    /// otherwise: **294 rows read, 0.00 s**, because `PARTITION BY
    /// toYYYYMM(timestamp)` gives every part a `minmax_timestamp` index and
    /// ClickHouse answers the aggregate from part metadata alone. Adding a
    /// window predicate here would buy nothing and cost a branch, so this stays
    /// the one statement in the pass that reads the whole table's *extent*.
    async fn watermark(&self) -> Result<u32, ChEnrichError> {
        let sql = format!(
            "SELECT toUnixTimestamp(max(timestamp)) FROM {db}.{tbl}",
            db = self.cfg.database,
            tbl = self.cfg.table,
        );
        Ok(self.client.query(&sql).fetch_one::<u32>().await?)
    }

    /// Count the still-unenriched, enrichable-shaped candidates at or before the
    /// snapshot `watermark` (see [`Self::watermark`]). `FINAL` collapses pending
    /// versions so already-enriched rows (which carry `volume_quote_usd > 0` at
    /// `version + 1`) are excluded even before the background merge runs.
    ///
    /// This is called once per batch to drive the loop's no-progress break, so a
    /// pass does up to `1 + 2·max_batches` of these `FINAL` merge-scans (review
    /// #10, part 2). The cheaper signal would be rows-actually-affected per INSERT,
    /// but the pinned `clickhouse` 0.13 `query().execute()` returns `()` and does
    /// not surface `X-ClickHouse-Summary` (`written_rows`); reading it would mean
    /// bypassing the crate with a raw HTTP call. Left as-is for now — the
    /// `watermark` bound (review #5) at least pins each scan to a fixed population.
    /// The per-batch XLM/USDC re-aggregation (part 1) is fixed separately, by
    /// materializing the reference once in [`Self::run_peg_pivot_tier`].
    async fn count_candidates(&self, watermark: u32) -> Result<u64, ChEnrichError> {
        let sql = format!(
            "SELECT count() FROM {db}.{tbl} FINAL \
             WHERE {pred} \
               AND timestamp <= toDateTime(?){win}",
            db = self.cfg.database,
            tbl = self.cfg.table,
            pred = CANDIDATE_PRED,
            win = self.window_pred("timestamp"),
        );
        Ok(self
            .client
            .query(&sql)
            .bind(watermark)
            .fetch_one::<u64>()
            .await?)
    }

    /// Count candles still at `volume_quote_usd = 0` (with non-zero
    /// `volume_quote`) at or below the watermark — the population the
    /// `EnrichmentRowsRemainingAtVolumeZero` metric (spec §5) is named for.
    /// Distinct from [`Self::count_candidates`], which counts rows missing
    /// *either* USD column (`volume_quote_usd = 0 OR close_usd = 0`): the
    /// `close_usd`-only remainder must not inflate a "volume zero" gauge.
    ///
    /// Returns two figures from a single `FINAL` scan: `total` (the whole
    /// backlog, for the dashboard/forensic metric) and `recent` (only candles
    /// within `cfg.recent_window_s` of the CH server clock — the recency-bounded
    /// backlog the stall alarm watches). `now()` is evaluated server-side in
    /// ClickHouse, not from the Lambda wall clock, so the window is immune to
    /// host clock skew (matching the freshness-probe design in task 0056). The
    /// recency bound excludes the permanent deep-history exotic-quote floor
    /// (quote ∉ {USDC,USDT,XLM}, no oracle) so an *idle* env — one producing no
    /// fresh candles — reports `recent = 0` and cannot false-fire the stall
    /// alarm on the floor alone (task 0026 finding #5).
    async fn count_remaining_at_volume_zero(
        &self,
        watermark: u32,
    ) -> Result<RemainingCounts, ChEnrichError> {
        let sql = format!(
            "SELECT count() AS total, \
                    countIf(timestamp >= now() - ?) AS recent \
             FROM {db}.{tbl} FINAL \
             WHERE volume_quote_usd = 0 AND volume_quote > 0 \
               AND timestamp <= toDateTime(?){win}",
            db = self.cfg.database,
            tbl = self.cfg.table,
            win = self.window_pred("timestamp"),
        );
        Ok(self
            .client
            .query(&sql)
            .bind(self.cfg.recent_window_s)
            .bind(watermark)
            .fetch_one::<RemainingCounts>()
            .await?)
    }

    /// Count rows the configured [`UsdResetSpec`] still has to re-open.
    async fn count_reset_pending(
        &self,
        spec: &UsdResetSpec,
        watermark: u32,
    ) -> Result<u64, ChEnrichError> {
        let sql = format!(
            "SELECT count() FROM {db}.{tbl} FINAL \
             WHERE {pred} AND timestamp <= toDateTime(?){win}",
            db = self.cfg.database,
            tbl = self.cfg.table,
            pred = reset_pending_pred(&self.cfg.database, spec),
            win = self.window_pred("timestamp"),
        );
        Ok(self
            .client
            .query(&sql)
            .bind(watermark)
            .fetch_one::<u64>()
            .await?)
    }

    /// Refuse the reset unless a tier in this pass can actually re-price the
    /// quote leg — i.e. it is the USDC peg or one of the pivot references.
    ///
    /// The reset is the only operation here that discards a value, so "can I put
    /// it back?" has to be answered *before* the write, not discovered after.
    /// `resolve_reference_ids()` otherwise runs inside the tier section, well
    /// after the rows are already zeroed, and a leg with no reference merely logs
    /// `warn!("no USDC/USDT/XLM reference assets …")` and skips — leaving the
    /// zeroes published.
    ///
    /// The realistic trigger is a typo, not an exotic asset: `--reset-quote-asset-id
    /// 11` for `111` sails through the oracle gate, because "no oracle rows" is
    /// precisely what that gate wants to see.
    async fn assert_reset_target_is_priceable(
        &self,
        spec: &UsdResetSpec,
    ) -> Result<(), ChEnrichError> {
        let refs = self.resolve_reference_ids().await?;
        let (stable, pivot) = (refs.stable_ids(), refs.pivot_ids());
        if stable.contains(&spec.quote_asset_id) || pivot.contains(&spec.quote_asset_id) {
            return Ok(());
        }
        Err(ChEnrichError::ResetTargetHasNoPricingPath {
            quote_asset_id: spec.quote_asset_id,
            stable,
            pivot,
        })
    }

    /// Refuse the reset while `oracle_prices` holds rows for the quote leg being
    /// reset **inside the reset's own `[not_before, not_after)` window** — task
    /// 0182's first ordering constraint, as a gate rather than a warning.
    ///
    /// ## Why the window, and not all time (task 0268)
    ///
    /// The guard exists because the oracle tier runs first and wins, so it only
    /// needs to refuse where the oracle tier can actually REACH. An all-time
    /// count answers a different question than the one that matters, and answers
    /// it wrongly in the one case 0268 needs: canonical USDC has held live oracle
    /// rows since 2026-03-11, so an unbounded count refuses every reset of the
    /// 2020-2025 history those rows cannot touch. Counting inside the window
    /// keeps the guard's meaning and drops only the false positives.
    ///
    /// A spec with `not_after = None` still counts to the end of time, so the
    /// 0182 path keeps its all-time refusal ABOVE the floor. The floor itself is
    /// no longer `not_before`: it is one `window_s` below it, because the oracle
    /// tier forward-fills a reading across that window and a row just under the
    /// floor still re-prices candles just over it.
    ///
    /// A warning would not do. The failure is silent and it *looks like success*:
    /// the reset zeroes the rows, the oracle tier (which runs first, and wins
    /// where it applies) immediately re-fills them from the very rate the reset
    /// existed to remove, and the run reports a healthy `rows_enriched` with
    /// `zeros_after = 0`. The operator sees a clean repair over unchanged values,
    /// now labelled `method = 'oracle'`.
    async fn assert_reset_not_shadowed_by_oracle(
        &self,
        spec: &UsdResetSpec,
    ) -> Result<(), ChEnrichError> {
        let upper = match spec.not_after {
            Some(na) => format!(" AND timestamp < toDateTime({na})"),
            None => String::new(),
        };
        // ⚠️ The lower bound is widened by one forward-fill window, and that is
        // not cosmetic. The oracle tier ASOFs `o.timestamp <= p.timestamp` with
        // `(p.timestamp - o.timestamp) <= window_s`, so a reading stamped just
        // BELOW `not_before` still re-prices candles up to `not_before +
        // window_s - 1` — inside the reset's own range. A guard that starts
        // looking at `not_before` cannot see the row that will shadow it, which
        // is the re-apply-and-relabel failure this refusal exists to prevent.
        // Only the UPPER bound needed to become window-scoped for task 0268.
        let nb = spec.not_before.saturating_sub(self.cfg.window_s);
        let sql = format!(
            "SELECT count() FROM {db}.oracle_prices \
             WHERE asset_id = {q} AND oracle_name = ? \
               AND timestamp >= toDateTime({nb}){upper}",
            db = self.cfg.database,
            q = spec.quote_asset_id,
        );
        let rows = self
            .client
            .query(&sql)
            .bind(&self.cfg.oracle_name)
            .fetch_one::<u64>()
            .await?;
        if rows > 0 {
            return Err(ChEnrichError::ResetBlockedByOracleRows {
                quote_asset_id: spec.quote_asset_id,
                oracle_name: self.cfg.oracle_name.clone(),
                rows,
                // ⚠️ The SCANNED band, not the reset's own window: the floor is
                // widened by `window_s` because a reading below it still
                // forward-fills into the reset. Reporting `[not_before, …)`
                // would send the operator to a query returning 0 rows for a
                // refusal that is real.
                window: match spec.not_after {
                    Some(na) => format!(
                        "[{nb}, {na}) (scanned from {nb}, one forward-fill window below --reset-not-before {})",
                        spec.not_before
                    ),
                    None => format!(
                        "[{nb}, all time) (scanned from {nb}, one forward-fill window below --reset-not-before {})",
                        spec.not_before
                    ),
                },
            });
        }
        Ok(())
    }

    /// Refuse a `require_external_rate` reset whose quote leg is not canonical
    /// USDC (task 0268 review).
    ///
    /// Everything on the external path names canonical USDC and nothing else:
    /// [`external_rate_day_pred`]'s day-set, `assert_external_rates_are_loaded`,
    /// and `external_sql`'s own `quote_asset_id` bound, which is filled from
    /// `refs.usdc`. `assert_reset_target_is_priceable` accepts USDT — it is a
    /// stable reference and the PEG tier can price it — so
    /// `--reset-quote-asset-id <USDT> --reset-require-external-rate` was a legal
    /// combination that zeroed USDT-quoted rows on USDC's rate days and left the
    /// external tier, which only ever runs for USDC, unable to refill one of
    /// them. The peg tier then writes $1 back over the lot: task 0182's incident
    /// under a different quote asset.
    ///
    /// Refused here in the library rather than with a CLI `conflicts_with`,
    /// because the CLI is not the only driver.
    async fn assert_external_rate_leg_is_usdc(
        &self,
        spec: &UsdResetSpec,
    ) -> Result<(), ChEnrichError> {
        let refs = self.resolve_reference_ids().await?;
        match refs.usdc {
            Some(usdc_id) if usdc_id == spec.quote_asset_id => Ok(()),
            // `None` means canonical USDC is not a tracked asset at all, which
            // is a different operator error than "wrong leg" — reported as 0,
            // never a real asset_id, and spelled out in the message.
            usdc => Err(ChEnrichError::ResetExternalRateLegIsNotUsdc {
                quote_asset_id: spec.quote_asset_id,
                usdc_id: usdc.unwrap_or(0),
            }),
        }
    }

    /// Refuse a `require_pivot_usdc_rate` reset whose quote leg is not one of the
    /// pass's pivot references (task 0228) — the mirror image of
    /// [`Self::assert_external_rate_leg_is_usdc`].
    ///
    /// The 0228 mode re-opens rows the SCALED PIVOT recomputes, and
    /// [`pivot_sql`] only ever runs for a leg in [`ReferenceIds::pivot_ids`],
    /// against a USDC market. `assert_reset_target_is_priceable` is not enough on
    /// its own: it accepts canonical USDC too, because the peg and external tiers
    /// can price it — so `--reset-quote-asset-id <USDC>
    /// --reset-require-pivot-usdc-rate` would pass that gate while selecting rows
    /// no pivot pass touches. Canonical USDC is named explicitly in the error,
    /// because that mistake has an exact right answer: it is the 0268 mode.
    ///
    /// Refused here in the library rather than with a CLI `conflicts_with`,
    /// because the CLI is not the only driver.
    async fn assert_pivot_rate_leg_is_a_pivot_reference(
        &self,
        spec: &UsdResetSpec,
    ) -> Result<(), ChEnrichError> {
        let refs = self.resolve_reference_ids().await?;
        let pivot = refs.pivot_ids();
        if pivot.contains(&spec.quote_asset_id) {
            return Ok(());
        }
        Err(ChEnrichError::ResetPivotRateLegIsNotAPivotReference {
            quote_asset_id: spec.quote_asset_id,
            // 0 means canonical USDC is not a tracked asset here at all — a
            // different operator error than "wrong leg", and reported as such.
            usdc_id: refs.usdc.unwrap_or(0),
            pivot,
        })
    }

    /// Refuse a `require_external_rate` reset when `prices.usd_rate` holds no
    /// `method = 'external'` row for canonical USDC at all (task 0268).
    ///
    /// The fourth refusal, and the cheapest possible check for the most likely
    /// operator error: running the 0268 mode before task 0267's series has been
    /// loaded. Doing nothing quietly is the failure mode this exists to prevent —
    /// see [`ChEnrichError::ResetRequiresExternalRates`].
    async fn assert_external_rates_are_loaded(
        &self,
        spec: &UsdResetSpec,
    ) -> Result<(), ChEnrichError> {
        let sql = format!(
            "SELECT count() FROM {db}.usd_rate FINAL \
             WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
               AND issuer_address = '{USDC_ISSUER}' AND contract_address = '' \
               AND method = 'external' AND usd_rate > 0",
            db = self.cfg.database,
        );
        let rows = self.client.query(&sql).fetch_one::<u64>().await?;
        if rows == 0 {
            return Err(ChEnrichError::ResetRequiresExternalRates {
                quote_asset_id: spec.quote_asset_id,
            });
        }
        Ok(())
    }

    /// Refuse a `require_external_rate` reset on a sub-daily table unless the
    /// imported series holds HOURLY rows (task 0268).
    ///
    /// The reset is one-shot per row, and that is what makes the ordering matter.
    /// It re-opens only rows still carrying the $1 signature (`close_usd =
    /// close`). With only the daily file loaded, the external tier prices every
    /// hour of a day from that day's single row — the day CLOSE — and the row
    /// leaves the signature for good. Loading the hourly file later cannot reach
    /// it: a second pass finds nothing to re-open. Measured on 2023-03-11: the
    /// 12:00 candle repaired on a daily-only load stayed at 0.96812 through a
    /// later hourly load and a second pass, while Chainlink's 12:00 close was
    /// 0.90687439.
    ///
    /// "Only the daily file" is detected as "no imported row away from UTC
    /// midnight": the hourly pass is the only writer of such rows (the loader's
    /// daily grain refuses a non-midnight timestamp). `'UTC'` is named so a
    /// non-UTC server cannot make every row look non-midnight.
    ///
    /// Daily and coarser tables resolve at the bucket END, where the daily row
    /// and the 23:00 hourly row carry the same close on every covered day, so
    /// they are not gated.
    async fn assert_hourly_rates_are_loaded(&self) -> Result<(), ChEnrichError> {
        let width = bucket_width_s(&self.cfg.table);
        if width == 0 || width >= 86_400 {
            return Ok(());
        }
        let sql = format!(
            "SELECT count() FROM {db}.usd_rate FINAL \
             WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
               AND issuer_address = '{USDC_ISSUER}' AND contract_address = '' \
               AND method = 'external' AND usd_rate > 0 \
               AND timestamp != toStartOfDay(timestamp, 'UTC')",
            db = self.cfg.database,
        );
        let rows = self.client.query(&sql).fetch_one::<u64>().await?;
        if rows == 0 {
            return Err(ChEnrichError::ResetRequiresHourlyRates {
                table: self.cfg.table.clone(),
            });
        }
        Ok(())
    }

    /// Refuse a `require_external_rate` reset while `prices.oracle_prices` holds
    /// a canonical USDC reading below `USDC_ORACLE_EPOCH_S` (task 0268 review,
    /// WR-09).
    ///
    /// This is the precondition of the EXTERNAL tier, not of the reset: the
    /// tier recomputes `volume_quote_usd` unconditionally on every pre-epoch
    /// candidate, and its safety argument is that no oracle reading exists
    /// below the epoch — a premise stated against `usd_rate` while the oracle
    /// tier reads `oracle_prices`. The two are not the same population (the
    /// `usd_rate` copy runs behind a watermark), so the premise is measured here
    /// on the table that matters, and a non-zero count is a refusal: the epoch,
    /// the wire label and the reset window all key on "no poll priced USDC
    /// before this instant", and a reading that says otherwise has to be
    /// triaged (purged as task 0196 did, or the epoch moved) before a campaign
    /// that would relabel and re-price under that assumption.
    ///
    /// The oracle-shadow guard above already counts these rows when
    /// `not_before = 0`; this one does not depend on the operator having chosen
    /// that bound, because the tier's reach does not either.
    async fn assert_no_pre_epoch_oracle_rows(
        &self,
        spec: &UsdResetSpec,
    ) -> Result<(), ChEnrichError> {
        let sql = pre_epoch_oracle_rows_sql(&self.cfg.database, spec.quote_asset_id);
        let rows = self
            .client
            .query(&sql)
            .bind(&self.cfg.oracle_name)
            .fetch_one::<u64>()
            .await?;
        if rows > 0 {
            return Err(ChEnrichError::ResetBlockedByPreEpochOracleRows {
                quote_asset_id: spec.quote_asset_id,
                oracle_name: self.cfg.oracle_name.clone(),
                rows,
            });
        }
        Ok(())
    }

    /// Zero the USD columns on rows the spec names, in `batch_size` chunks, so
    /// the tiers below can recompute them. Returns the number of rows re-opened.
    ///
    /// Runs *before* `candidates_before` is measured, so the pass's own
    /// accounting (`rows_enriched`, `candidates_after`) describes the repair
    /// rather than the pre-repair table.
    async fn reset_step(&self, spec: &UsdResetSpec, watermark: u32) -> Result<u64, ChEnrichError> {
        // Three refusals, in increasing cost order. Every one of them protects
        // the same property: nothing is zeroed unless a tier in THIS pass can
        // put a value back.
        // Zeroth, and free: an empty window (WR-05). The CLI refuses it too,
        // before the driver's month enumeration silently finds nothing.
        spec.validate()?;
        if !self.cfg.one_shot {
            return Err(ChEnrichError::ResetRequiresOneShot {
                quote_asset_id: spec.quote_asset_id,
            });
        }
        self.assert_reset_target_is_priceable(spec).await?;
        self.assert_reset_not_shadowed_by_oracle(spec).await?;
        // Fourth refusal (task 0268), last because it costs one more count and
        // only applies to the 0268 mode. Same property as the other three:
        // nothing is zeroed unless a tier in THIS pass can put a value back.
        if spec.require_external_rate {
            // Before anything counts rows: the external path is USDC-only, so a
            // different quote leg cannot be refilled by it at all.
            self.assert_external_rate_leg_is_usdc(spec).await?;
            self.assert_external_rates_are_loaded(spec).await?;
            // A sub-daily table needs the HOURLY series, or it is priced at the
            // day close once and can never be re-opened.
            self.assert_hourly_rates_are_loaded().await?;
            // Fifth (review WR-09): the external tier's own premise, measured on
            // the table the oracle tier reads. Independent of `not_before`.
            self.assert_no_pre_epoch_oracle_rows(spec).await?;
        }
        // Task 0228's mode, whose refill path is the SCALED PIVOT rather than the
        // external tier. Same property as every refusal above: nothing is zeroed
        // unless a tier in THIS pass can put a value back.
        if spec.require_pivot_usdc_rate {
            // The pivot only runs for a leg with a USDC market to measure against,
            // and canonical USDC is not such a leg — that one is the 0268 mode.
            self.assert_pivot_rate_leg_is_a_pivot_reference(spec)
                .await?;
            // The day-set both modes share is USDC's `external` series. Empty
            // means the campaign would report a clean, entirely empty repair.
            self.assert_external_rates_are_loaded(spec).await?;
            // ⚠️ `assert_hourly_rates_are_loaded` is deliberately NOT called here,
            // and the omission is the reasoning, not an oversight. 0268 needs it
            // because its candidate is the par signature `close_usd = close`: a
            // sub-daily USDC candle priced from the DAY close stops carrying that
            // signature, so loading the hourly file afterwards can never re-open
            // it — one shot, permanently. A pivoted row carries no such signature,
            // so this mode's candidate ("a written value, on a day the series
            // covers") still matches after a repair. Loading the hourly file later
            // and re-running simply recomputes, which is exactly the
            // value-idempotence this mode is built on. The gate guards an
            // irreversibility that does not exist here.
            //
            // ⚠️ `assert_no_pre_epoch_oracle_rows` is likewise not called. It
            // measures the EXTERNAL tier's own premise — that tier recomputes
            // `volume_quote_usd` unconditionally below the epoch, and its safety
            // rests on no poll having priced USDC there. The pivot keeps
            // `volume_quote_usd` write-once, so that premise is not load-bearing
            // for this mode; the oracle-shadow guard above, which is window-scoped
            // and runs for every spec, is what stops the oracle tier re-pricing
            // anything this mode re-opens.
        }

        let pending_before = self.count_reset_pending(spec, watermark).await?;
        if pending_before == 0 {
            return Ok(0);
        }
        info!(
            quote_asset_id = spec.quote_asset_id,
            not_before = spec.not_before,
            pending = pending_before,
            table = %self.cfg.table,
            "USD reset: re-opening already-written rows for re-enrichment"
        );

        let sql = reset_sql(
            &self.cfg.database,
            &self.cfg.table,
            spec,
            &self.window_pred("p.timestamp"),
        );

        let mut pending = pending_before;
        for _ in 0..self.effective_max_batches() {
            if pending == 0 {
                break;
            }
            self.client
                .query(&sql)
                .bind(watermark)
                .bind(self.cfg.batch_size)
                .execute()
                .await?;
            let after = self.count_reset_pending(spec, watermark).await?;
            if after >= pending {
                // No row left the pending set. Every remaining match is one the
                // statement cannot act on; looping again would only re-scan.
                warn!(
                    remaining = after,
                    "USD reset made no progress — stopping (rows may be unreachable)"
                );
                pending = after;
                break;
            }
            pending = after;
        }

        let reopened = pending_before.saturating_sub(pending);
        info!(
            reopened,
            remaining = pending,
            table = %self.cfg.table,
            "USD reset done"
        );
        Ok(reopened)
    }

    /// Enrich up to `batch_size` candidates in one server-side statement.
    ///
    /// The `ASOF LEFT JOIN` forward-fills the newest oracle price at or
    /// before each candidate's `timestamp`; the post-join `WHERE` clause
    /// enforces the staleness window floor and drops oracle misses (which
    /// stay at `volume_quote_usd = 0` for a later pass). `version + 1`
    /// makes the corrected row win the `ReplacingMergeTree` merge, and the
    /// inner `CAST(… AS Decimal(38, 14))` keeps the `Decimal(38,14) ×
    /// Decimal(38,14)` product inside the column's precision.
    ///
    /// `volume_quote_usd` is write-once (`if(volume_quote_usd > 0, …)`), matching
    /// the peg/pivot statements: the widened candidate filter
    /// (`volume_quote_usd = 0 OR close_usd = 0`) re-admits rows enriched before
    /// `close_usd` existed (`volume_quote_usd > 0`, `close_usd = 0`) so their
    /// `close_usd` gets backfilled, but their already-set (depeg-aware)
    /// `volume_quote_usd` must not be silently rewritten from a different ASOF
    /// match. `close_usd` is unconditional — it is the column this pass owns.
    async fn enrich_batch(&self, watermark: u32) -> Result<(), ChEnrichError> {
        let sql = format!(
            "INSERT INTO {db}.{tbl} \
                 (timestamp, asset_id, quote_asset_id, source, \
                  open, high, low, close, \
                  volume_base, volume_quote, volume_quote_usd, close_usd, vwap, \
                  trade_count, version) \
             SELECT \
                 p.timestamp, p.asset_id, p.quote_asset_id, p.source, \
                 p.open, p.high, p.low, p.close, \
                 p.volume_base, p.volume_quote, \
                 if(p.volume_quote_usd > 0, p.volume_quote_usd, CAST(o.price_usd * p.volume_quote AS Decimal(38, 14))) AS volume_quote_usd, \
                 CAST(o.price_usd * p.close AS Decimal(38, 14)) AS close_usd, \
                 p.vwap, p.trade_count, \
                 p.version + 1 AS version \
             FROM {db}.{tbl} AS p FINAL \
             ASOF LEFT JOIN {db}.oracle_prices AS o \
                     ON o.asset_id = p.quote_asset_id \
                    AND o.oracle_name = ? \
                    AND o.timestamp <= p.timestamp \
             WHERE (p.volume_quote_usd = 0 OR p.close_usd = 0) \
               AND p.volume_quote > 0 \
               AND o.price_usd IS NOT NULL \
               AND (p.timestamp - o.timestamp) <= ? \
               AND p.timestamp <= toDateTime(?){win} \
             ORDER BY p.timestamp \
             LIMIT ?",
            db = self.cfg.database,
            tbl = self.cfg.table,
            win = self.window_pred("p.timestamp"),
        );
        self.client
            .query(&sql)
            .bind(&self.cfg.oracle_name)
            .bind(self.cfg.window_s)
            .bind(watermark)
            .bind(self.cfg.batch_size)
            .execute()
            .await?;
        Ok(())
    }

    /// Resolve the internal `asset_id`s of XLM / USDC / USDT from
    /// `prices.assets`, by their canonical (code, issuer) identity. `FINAL`
    /// collapses the `ReplacingMergeTree`. Any that the dataset never saw are
    /// left `None` and that branch of the peg-pivot tier is skipped.
    async fn resolve_reference_ids(&self) -> Result<ReferenceIds, ChEnrichError> {
        let sql = format!(
            "SELECT asset_id, asset_code, issuer_address \
             FROM {db}.assets FINAL \
             WHERE (asset_code = 'XLM'  AND issuer_address = '' AND contract_address = '') \
                OR (asset_code = 'USDC' AND issuer_address = '{usdc}') \
                OR (asset_code = 'USDT' AND issuer_address = '{usdt}')",
            db = self.cfg.database,
            usdc = USDC_ISSUER,
            usdt = USDT_ISSUER,
        );
        let rows = self.client.query(&sql).fetch_all::<RefAssetRow>().await?;

        let mut refs = ReferenceIds::default();
        for r in rows {
            if r.asset_code == "XLM" && r.issuer_address.is_empty() {
                refs.xlm = Some(r.asset_id);
            } else if r.asset_code == "USDC" && r.issuer_address == USDC_ISSUER {
                refs.usdc = Some(r.asset_id);
            } else if r.asset_code == "USDT" && r.issuer_address == USDT_ISSUER {
                refs.usdt = Some(r.asset_id);
            }
        }

        // ⚠️ Task 0215: a reference that does not resolve narrows this pass
        // SILENTLY. `stable_ids()`/`pivot_ids()` flatten the `None` away, so the
        // step issues one statement where it should issue two and still reports
        // success — the exact shape of the defect that took 26 days to find,
        // and which was readable only by pulling emitted SQL out of production's
        // `system.query_log`. Partial sets stay legal (bootstrap orders assets
        // arbitrarily), so this warns rather than fails; what it must never do
        // is happen quietly.
        let missing: Vec<&str> = [
            ("XLM", refs.xlm.is_none()),
            ("USDC", refs.usdc.is_none()),
            ("USDT", refs.usdt.is_none()),
        ]
        .into_iter()
        .filter_map(|(code, absent)| absent.then_some(code))
        .collect();
        if !missing.is_empty() {
            warn!(
                missing = %missing.join(","),
                stable_ids = ?refs.stable_ids(),
                pivot_ids = ?refs.pivot_ids(),
                "reference asset did not resolve — peg/pivot set is NARROWED for this pass"
            );
        }

        Ok(refs)
    }

    /// One peg-pivot step: the peg statement (USDC quotes → ×$1) followed by one
    /// pivot statement per reference asset ([`ReferenceIds::pivot_ids`] — XLM and
    /// USDT), each valuing its quote leg at that asset's measured close against
    /// USDC. All target only rows the oracle tier left at `close_usd = 0`, so the
    /// oracle value always wins where it exists. The peg runs whenever USDC is in
    /// the registry; each pivot runs only when both its reference asset and that
    /// asset's USDC market are known — the reference is computed inline, with no
    /// pre-materialized table (see [`pivot_sql`]).
    ///
    /// ⚠️ USDT is a *pivot* reference, not a peg member (task 0172) — it depegged
    /// in June 2022 and trades at ~$0.13.
    async fn enrich_peg_pivot_step(
        &self,
        refs: &ReferenceIds,
        watermark: u32,
    ) -> Result<(), ChEnrichError> {
        // ⚠️ TWO window fragments, and they are not interchangeable (task 0228).
        // The peg statement scans `{tbl} AS p` directly, so its partition bound
        // names `p.timestamp`. The pivot's candidate scan moved into a subquery
        // with no alias in scope, so its bound names the BARE column — the
        // `external_sql` form. Handing one string to both would be a syntax error
        // on whichever statement it did not fit.
        let peg_window = self.window_pred("p.timestamp");
        let pivot_window = self.window_pred("timestamp");
        for stmt in plan_peg_pivot_step(
            &self.cfg.database,
            &self.cfg.table,
            refs,
            &peg_window,
            &pivot_window,
        ) {
            match stmt {
                StepStatement::Peg { sql } => {
                    self.client
                        .query(&sql)
                        .bind(watermark)
                        .bind(self.cfg.batch_size)
                        .execute()
                        .await?;
                }
                StepStatement::Pivot { sql, .. } => {
                    // Candidate watermark, reference-subquery watermark, pivot
                    // staleness, batch size — the candidate subquery is rendered
                    // first, so its watermark binds first (see [`pivot_sql`]).
                    self.client
                        .query(&sql)
                        .bind(watermark)
                        .bind(watermark)
                        .bind(self.cfg.pivot_window_s)
                        .bind(self.cfg.batch_size)
                        .execute()
                        .await?;
                }
            }
        }
        Ok(())
    }

    /// Tier 2 — the peg-pivot deep-history backbone, over the fixed `watermark`
    /// snapshot. Each per-batch pivot computes its volume-weighted reference/USDC
    /// rate **inline** ([`pivot_sql`]) rather than from a pre-materialized table,
    /// so it needs no `CREATE TABLE` grant on the shared tenant (task 0083).
    ///
    /// TRADE-OFF (reverses review #10's materialize-once): each ref is now
    /// re-aggregated per batch, so total ref work is O(slice × batches) instead of
    /// O(slice). Each slice is a single-pair sort-key prefix, so it's cheap **while
    /// the reference markets' history is small** — but it grows with backfill depth
    /// × the batch count (up to `max_batches`, unbounded in `one_shot`) and could
    /// risk the 300s Lambda timeout post-backfill. Restore materialize-once
    /// (in-memory ref or a session-scoped `CREATE TEMPORARY TABLE`) before the 0053
    /// backfill runs — tracked in **0085**.
    ///
    /// ⚠️ Task 0172 added a second pivot (USDT), so this tier now issues **three**
    /// statements per batch rather than two — ~50% more scan work in the pass task
    /// **0111** is open on for full-table scans. `pivot_sql` pins `quote_asset_id`
    /// to the reference id so each pivot prunes on the sort key's 2nd column.
    ///
    /// Tier 2 (task 0268) — price USDC-quoted candles from the MEASURED rate.
    ///
    /// Loops [`external_sql`] to a fixed point, exactly as
    /// [`ChEnrichmentPass::run_peg_pivot_tier`] does: run one statement, re-count,
    /// stop as soon as a batch flips nothing out of the zero-set. On a table with
    /// no `external` rows in `usd_rate` — every scheduled path today, until task
    /// 0267's series is loaded on prod — the first batch makes no progress and the
    /// loop breaks immediately, so the recurring pass is unchanged.
    ///
    /// The no-progress case is `info!` rather than `warn!`: "these candles have no
    /// imported rate for their bucket" is the EXPECTED steady state, not a defect.
    /// The peg tier below still prices them at $1, which is what the 0268 reset is
    /// then careful never to zero.
    ///
    /// Returns the updated `(remaining, batches)`.
    async fn run_external_tier(
        &self,
        usdc_id: u32,
        watermark: u32,
        mut remaining: u64,
        mut batches: u32,
    ) -> Result<(u64, u32), ChEnrichError> {
        let sql = external_sql(
            &self.cfg.database,
            &self.cfg.table,
            usdc_id,
            &self.window_pred("timestamp"),
        );
        let stale = external_window_s(&self.cfg.table);
        for _ in 0..self.effective_max_batches() {
            if remaining == 0 {
                break;
            }
            self.client
                .query(&sql)
                .bind(watermark)
                .bind(stale)
                .bind(self.cfg.batch_size)
                .execute()
                .await?;
            let after = self.count_candidates(watermark).await?;
            batches += 1;
            if after >= remaining {
                info!(
                    remaining = after,
                    table = %self.cfg.table,
                    "external tier drained — remaining candles have no imported USD rate for their bucket"
                );
                remaining = after;
                break;
            }
            remaining = after;
        }
        Ok((remaining, batches))
    }

    /// Returns the updated `(remaining, batches)`.
    async fn run_peg_pivot_tier(
        &self,
        refs: &ReferenceIds,
        watermark: u32,
        mut remaining: u64,
        mut batches: u32,
    ) -> Result<(u64, u32), ChEnrichError> {
        for _ in 0..self.effective_max_batches() {
            if remaining == 0 {
                break;
            }
            self.enrich_peg_pivot_step(refs, watermark).await?;
            let after = self.count_candidates(watermark).await?;
            batches += 1;
            if after >= remaining {
                // The leftovers have no usable USD reference. Two causes now, and
                // the message names both (task 0228): the quote is neither
                // USDC/USDT/XLM (nor oracle-priced), OR it is a pivot leg whose
                // bucket has no measured USDC/USD rate in window, which since 0228
                // leaves the row unpriced rather than storing the unscaled vwap.
                // Either way they stay `no_reference`, never a wrong value.
                //
                // Still `warn!` where `run_external_tier`'s twin is `info!`, and
                // deliberately: the external tier has the peg tier below it to
                // catch what it drops, so its no-progress break is a handover. This
                // is the LAST tier, so its leftovers are published as
                // `close_usd = 0` — a state ~130 unguarded `argMax(close_usd, …)`
                // sites read as a real price. A backlog that stops draining here is
                // worth a monitoring signal even when the cause is benign.
                warn!(
                    remaining = after,
                    "peg-pivot tier made no progress — remaining candles have no USD reference \
                     (exotic quotes, or a pivot leg with no measured USDC/USD rate in window)"
                );
                remaining = after;
                break;
            }
            remaining = after;
        }
        Ok((remaining, batches))
    }

    /// Run the bounded enrichment pass in two ordered tiers (task 0061 §12.1):
    /// the recent-window oracle tier first, then the peg-pivot deep-history tier
    /// over whatever it left at `close_usd = 0`. Each tier loops up to
    /// `max_batches × batch_size` rows and stops early when a batch makes no
    /// progress — i.e. its remaining candidates have no reference of that kind.
    ///
    /// The peg-pivot tier runs only if the oracle tier *drained* (reached a
    /// fixed point). If the oracle tier instead exhausted its batch budget while
    /// still making progress, its leftovers may still hold an unapplied in-window
    /// oracle price, so they are deferred to the next invocation rather than
    /// pegged — pegging an oracle-eligible candle would bake a wrong flat $1.
    ///
    /// Snapshots the candidate population at the newest existing candle: every
    /// count and enrich statement is bounded to `timestamp <= watermark`, so
    /// candles the live Ledger Processor inserts concurrently (newer timestamps)
    /// can't inflate the count and falsely trip the no-progress break; they roll
    /// over to the next scheduled run. See [`Self::run_through`] to pin the
    /// boundary explicitly.
    pub async fn run(&self) -> Result<ChPassStats, ChEnrichError> {
        self.run_through(self.watermark().await?).await
    }

    /// [`Self::run`] over candidates with `timestamp <= watermark`, with the
    /// snapshot boundary supplied by the caller instead of read as the current
    /// max. `run()` is `run_through(self.watermark().await?)`; tests use this to
    /// pin the boundary and assert that candles newer than the snapshot are
    /// deferred, not enriched, in the current pass.
    pub async fn run_through(&self, watermark: u32) -> Result<ChPassStats, ChEnrichError> {
        let start = std::time::Instant::now();

        // Task 0182 — re-open already-written USD values so the tiers below can
        // recompute them. Must precede `candidates_before`: the reset is what
        // *creates* those candidates, so measuring first would report the
        // pre-repair table and score the whole repair as `rows_enriched = 0`.
        // `None` on every scheduled path, so this is a no-op in steady state.
        let rows_reset = match self.cfg.usd_reset.as_ref() {
            Some(spec) => self.reset_step(spec, watermark).await?,
            None => 0,
        };

        let candidates_before = self.count_candidates(watermark).await?;
        info!(
            candidates = candidates_before,
            watermark,
            table = %self.cfg.table,
            "enrichment pass start"
        );

        let mut batches = 0u32;
        let mut remaining = candidates_before;

        // Whether the oracle tier reached a fixed point — drained every row it
        // can enrich — versus exhausting its batch budget while still making
        // progress. Only a drained tier leaves *true* oracle misses behind; if
        // it instead ran out of `max_batches`, the leftovers may still carry an
        // in-window oracle price this run simply did not reach. Handing those to
        // the peg-pivot tier would bake a flat $1 peg over a depeg-aware oracle
        // value (and, once `close_usd > 0`, they never re-enter the oracle tier
        // on a later pass). So Tier 2 is gated on this flag; un-drained leftovers
        // roll over to the next invocation's oracle tier instead.
        let mut oracle_drained = remaining == 0;

        // Tier 1 — recent-window oracle (depeg-aware; wins where it applies).
        for _ in 0..self.effective_max_batches() {
            if remaining == 0 {
                oracle_drained = true;
                break;
            }
            self.enrich_batch(watermark).await?;
            let after = self.count_candidates(watermark).await?;
            batches += 1;

            if after >= remaining {
                // No row flipped out of the zero-set: the leftover candidates
                // have no in-window oracle price. They are true oracle misses —
                // hand them to the peg-pivot tier (deep-history / exotic quotes).
                info!(
                    remaining = after,
                    "oracle tier drained — handing remaining candles to peg-pivot tier"
                );
                remaining = after;
                oracle_drained = true;
                break;
            }
            remaining = after;
        }

        // Candidates the oracle tier could not cover — the EnrichmentOracleMiss
        // metric. Only meaningful when the oracle tier *drained* (reached a fixed
        // point): then `remaining` is the set with no in-window oracle price. If
        // the tier instead exhausted its batch budget while still making progress
        // (`oracle_drained == false`), the leftovers were simply not reached this
        // pass — they are NOT misses, so report 0 rather than inflating the metric
        // by the whole un-processed remainder (which would over-count by orders of
        // magnitude on any large-backlog catch-up).
        let oracle_misses = if oracle_drained { remaining } else { 0 };

        // Tier 2 — peg-pivot deep-history backbone (USDC≡$1; XLM and USDT each
        // via their own measured market against USDC).
        // Gated on `oracle_drained`: an oracle tier that exhausted its batch
        // budget while still making progress may have left rows with an unapplied
        // in-window oracle price, which must not be pegged to $1 (they roll over
        // to the next run's oracle tier instead).
        if remaining > 0 && !oracle_drained {
            info!(
                remaining,
                "oracle tier hit its batch budget while still making progress — \
                 deferring peg-pivot tier so unreached oracle candles are not pegged"
            );
        } else if remaining > 0 {
            // Resolved ONCE and shared by both tiers below — two calls would be
            // two round-trips for the same answer.
            let refs = self.resolve_reference_ids().await?;

            // Tier 2 — external (task 0268): USDC-quoted candles priced from the
            // MEASURED USDC/USD rate task 0267 imports into `usd_rate`.
            //
            // Ordered BEFORE the peg tier and gated on the SAME `oracle_drained`
            // flag, for the same two reasons: an un-drained oracle tier may still
            // hold an unapplied in-window oracle price, and once `close_usd > 0` a
            // row never re-enters the oracle tier on a later pass. It runs after
            // the oracle tier because a polled Reflector reading is the better
            // evidence where both exist, and before the peg tier because a
            // measured rate is better evidence than a $1 assumption anywhere.
            //
            // `remaining`/`batches` are handed down, so the peg tier below only
            // ever sees what NEITHER of the two tiers above could price.
            if let Some(usdc_id) = refs.usdc {
                let (r, b) = self
                    .run_external_tier(usdc_id, watermark, remaining, batches)
                    .await?;
                remaining = r;
                batches = b;
            }

            if remaining > 0 && refs.has_any() {
                let (r, b) = self
                    .run_peg_pivot_tier(&refs, watermark, remaining, batches)
                    .await?;
                remaining = r;
                batches = b;
            } else if !refs.has_any() {
                warn!(
                    "no USDC/USDT/XLM reference assets in prices.assets — peg-pivot tier skipped"
                );
            }
        }

        let rows_enriched = candidates_before.saturating_sub(remaining);

        // A pass that enriches *nothing* despite a non-empty backlog is the
        // fingerprint of the failure 0061 fixes: the oracle↔asset-id join matches
        // nothing (mis-reconciliation) or no USDC/USDT/XLM reference exists in
        // prices.assets — both leave every candidate at zero. In a healthy system
        // one of the two tiers always makes a dent, so this is warn-worthy (a
        // monitoring signal), not the routine info the per-tier drains emit.
        if candidates_before > 0 && rows_enriched == 0 {
            warn!(
                candidates = candidates_before,
                "enrichment pass enriched 0 rows despite a non-empty backlog — \
                 check oracle↔asset-id reconciliation and that USDC/USDT/XLM \
                 reference assets exist in prices.assets"
            );
        }

        let remaining_counts = self.count_remaining_at_volume_zero(watermark).await?;

        let stats = ChPassStats {
            batches,
            candidates_before,
            candidates_after: remaining,
            rows_enriched,
            rows_reset,
            oracle_misses,
            rows_remaining_at_volume_zero: remaining_counts.total,
            rows_remaining_recent: remaining_counts.recent,
            duration_ms: start.elapsed().as_millis() as u64,
        };
        info!(
            batches = stats.batches,
            enriched = stats.rows_enriched,
            remaining = stats.candidates_after,
            "enrichment pass complete"
        );
        Ok(stats)
    }
}

/// The 15-column INSERT target list, shared by every enrichment statement so the
/// SELECT projections stay positionally aligned with it.
const INSERT_COLUMNS: &str = "timestamp, asset_id, quote_asset_id, source, \
     open, high, low, close, \
     volume_base, volume_quote, volume_quote_usd, close_usd, vwap, \
     trade_count, version";

/// One statement of a peg-pivot step, with the shape that decides how it is
/// bound. The two variants take **different bind sequences**, which is why this
/// is an enum and not a bare `Vec<String>`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StepStatement {
    /// The peg pass: USDC-quoted candles valued at ×$1.
    Peg { sql: String },
    /// One pivot pass, valuing `ref_id`-quoted candles at that asset's own
    /// measured USDC close. `ref_id` is carried so a test can assert *which*
    /// references were planned, not merely how many.
    Pivot { sql: String, ref_id: u32 },
}

/// Decide which statements one peg-pivot step issues, without sending any.
///
/// Split out of [`ChEnrichmentPass::enrich_peg_pivot_step`] for task 0215: the
/// defect that task chased was a pivot set that had silently narrowed from two
/// statements to one, and the only way anyone could tell was by reading emitted
/// SQL back out of `system.query_log` on production. Planning is pure, so
/// `plan_issues_one_peg_and_two_pivots` can assert the shape in CI instead.
///
/// ⚠️ The integration tests that cover this end-to-end
/// (`usdt_quoted_candles_pivot_on_the_measured_rate_not_a_dollar_peg` and
/// `enrich_fills_close_usd_across_oracle_peg_and_pivot_tiers`) are `#[ignore]`
/// and need a live ClickHouse, so **they do not run in CI** — see task 0275.
/// Until they do, this unit test is the only automatic guard on the pivot set.
///
/// ⚠️ The two window fragments are NOT interchangeable — see
/// [`ChEnrichmentPass::enrich_peg_pivot_step`]. `peg_window` is `p.`-qualified;
/// `pivot_window` names the bare column inside the pivot's candidate subquery.
fn plan_peg_pivot_step(
    db: &str,
    tbl: &str,
    refs: &ReferenceIds,
    peg_window: &str,
    pivot_window: &str,
) -> Vec<StepStatement> {
    let mut plan = Vec::new();
    if let Some(sql) = peg_sql(db, tbl, &refs.stable_ids(), peg_window) {
        plan.push(StepStatement::Peg { sql });
    }
    // One pivot pass per measured reference asset (XLM, then USDT — task 0172).
    // Order matters only for cost, not correctness: each pass fills rows the
    // previous ones left at `close_usd = 0`, and the two reference assets match
    // disjoint sets of candles (`r.ref_asset_id = p.quote_asset_id`).
    if let Some(usdc_id) = refs.usdc {
        for ref_id in refs.pivot_ids() {
            plan.push(StepStatement::Pivot {
                sql: pivot_sql(db, tbl, ref_id, usdc_id, pivot_window),
                ref_id,
            });
        }
    }
    plan
}

/// Peg statement: USDC/USDT-quoted candles get `close_usd = close × $1`. Returns
/// `None` when neither stablecoin is in the registry (nothing to peg). Bound
/// parameters, in order: the snapshot watermark (`p.timestamp <= toDateTime(?)`,
/// shared with the rest of the pass — see [`ChEnrichmentPass::watermark`]) and the
/// `LIMIT` (batch size). `volume_quote_usd` is only filled when still zero, so an
/// oracle-set (depeg-aware) value survives.
fn peg_sql(db: &str, tbl: &str, stable_ids: &[u32], window: &str) -> Option<String> {
    if stable_ids.is_empty() {
        return None;
    }
    let in_list = stable_ids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "INSERT INTO {db}.{tbl} ({INSERT_COLUMNS}) \
         SELECT \
             p.timestamp, p.asset_id, p.quote_asset_id, p.source, \
             p.open, p.high, p.low, p.close, \
             p.volume_base, p.volume_quote, \
             if(p.volume_quote_usd > 0, p.volume_quote_usd, CAST(p.volume_quote AS Decimal(38, 14))) AS volume_quote_usd, \
             CAST(p.close AS Decimal(38, 14)) AS close_usd, \
             p.vwap, p.trade_count, \
             p.version + 1 AS version \
         FROM {db}.{tbl} AS p FINAL \
         WHERE p.close_usd = 0 \
           AND p.volume_quote > 0 \
           AND p.quote_asset_id IN ({in_list}) \
           AND p.timestamp <= toDateTime(?){window} \
         ORDER BY p.timestamp \
         LIMIT ?"
    ))
}

/// The seven candle tables, in ascending grain. The ONE list every per-grain
/// match below is checked against (review IN-09): a grain added to
/// [`bucket_width_s`] and forgotten in [`bucket_end_expr`] would otherwise
/// pass every test that hardcodes its own list.
pub const GRAINS: [&str; 7] = [
    "price_ohlcv_1m",
    "price_ohlcv_15m",
    "price_ohlcv_1h",
    "price_ohlcv_4h",
    "price_ohlcv_1d",
    "price_ohlcv_1w",
    "price_ohlcv_1M",
];

/// A candle table's bucket width in seconds. `0` for a name that is not one of
/// the seven grains — callers floor it, so an unknown table can only ever widen
/// a bound, never narrow one.
fn bucket_width_s(table: &str) -> u32 {
    match table {
        "price_ohlcv_1m" => 60,
        "price_ohlcv_15m" => 900,
        "price_ohlcv_1h" => 3_600,
        "price_ohlcv_4h" => 14_400,
        "price_ohlcv_1d" => 86_400,
        "price_ohlcv_1w" => 604_800,
        "price_ohlcv_1M" => 2_678_400,
        _ => 0,
    }
}

/// The SQL expression for a bucket's END, given the table and the column holding
/// its START (task 0268, decision G).
///
/// `timestamp` is the bucket's START but `close` is the period's LAST close, so
/// a rate resolved at `timestamp` prices a weekly or monthly candle with the
/// rate from the day the period OPENED — up to a month stale, and systematically
/// so. The bucket end is the settled rule on both surfaces already
/// (`views.sql`'s `price_usd_series`, `queries_ch::ohlcv_peg_series`); this
/// reuses it rather than coining a third convention.
///
/// ⚠️ **Calendar functions for the calendar grains.** A `_1M` bucket is not 31
/// fixed days: `timestamp + 2_678_400` overshoots every 30-day month and every
/// February by days, so the ASOF resolves against the wrong day's rate for
/// eleven months of twelve. It produces a plausible number and fails nowhere.
///
/// ⚠️ **Each fixed grain uses its OWN width — never a floor.** `bend` is the
/// ASOF's upper bound (`r.rts < p.bend`), not a staleness allowance (that is
/// [`external_window_s`]), so widening it changes WHICH rate is selected. The
/// first version of this fn floored every sub-daily grain at one hour; a `_1m`
/// candle at 23:30 UTC then got `bend = 00:30` the next day and, with a daily
/// series stamped at day start, resolved to the NEXT day's rate — 59 of 1440
/// one-minute buckets and 3 of 96 fifteen-minute buckets per UTC day, priced
/// from the wrong day, on the default table of the scheduled Lambda. Only an
/// UNKNOWN table (width 0) falls back to an hour, and only because there is no
/// right answer for it; `bucket_end_and_width_agree_for_every_known_grain`
/// pins the rest.
///
/// ⚠️ **`'UTC'` is spelled out on the calendar functions.** `timestamp` is a
/// bare `DateTime`, so `addDays`/`addWeeks`/`addMonths` would otherwise use the
/// SERVER timezone, which nothing pins. Across a DST fall-back `addDays(ts, 1)`
/// in a local zone is 25 hours, `bend - rts` then exceeds the one-day staleness
/// bound, and an anchor that exists is dropped — the reset has already zeroed
/// that row, so it falls to the peg tier and silently regresses to $1.00 on DST
/// days. Under `'UTC'` there is no DST and a day is 86 400 s.
fn bucket_end_expr(table: &str, col: &str) -> String {
    match table {
        "price_ohlcv_1d" => format!("addDays({col}, 1, 'UTC')"),
        "price_ohlcv_1w" => format!("addWeeks({col}, 1, 'UTC')"),
        "price_ohlcv_1M" => format!("addMonths({col}, 1, 'UTC')"),
        other => match bucket_width_s(other) {
            0 => format!("{col} + 3600"),
            w => format!("{col} + {w}"),
        },
    }
}

/// How stale an `external` rate may be, relative to the bucket's end, before the
/// external tier refuses to use it. **Derived from the table, deliberately not a
/// [`ChEnrichConfig`] knob.**
///
/// The floor of one day is task 0267's series cadence: it is daily, so a bucket
/// whose anchor is the previous day's observation is the normal case, not a
/// stale one. Grains wider than a day take their own width instead, because
/// there the anchor is by construction up to one bucket old.
///
/// ## Why derived and not configured
///
/// `pivot_window_s` IS configurable, and that is exactly why `coarse-repair`
/// needs an explicit refusal for a value shorter than the bucket width: with a
/// reset in play, too narrow a window discards a stored value and then fails to
/// recompute it. A derived bound cannot be set wrong, so there is no refusal to
/// write and no way for an operator to reach the destructive combination. It
/// also avoids threading a knob nobody should turn through six
/// `ChEnrichConfig` literal sites.
pub fn external_window_s(table: &str) -> u32 {
    bucket_width_s(table).max(86_400)
}

/// External statement (task 0268): a USDC-quoted candle the oracle tier left at
/// `close_usd = 0` is priced from the MEASURED USDC/USD rate task 0267 imports
/// into `prices.usd_rate`, instead of falling through to the peg tier's `× $1`.
///
/// USDC closed at **0.9681** on 2023-03-11. The peg tier's assumption is wrong
/// by up to 3% daily and 12% intraday on ~654,291 stored candles, and this is
/// the statement that stops storing that error.
///
/// ## Bind order (positional)
///
/// 1. the snapshot watermark — inside the CANDIDATE subquery, not the outer
///    `WHERE`, because the candidate side is where it bounds the scan;
/// 2. the staleness bound in seconds ([`external_window_s`]);
/// 3. the `LIMIT` (batch size).
///
/// ## Why the candidate side is bounded and the reference side is not
///
/// `window` carries the 0114/0111 partition bound and is spliced into the
/// candidate subquery only. The `usd_rate` reference stays unbounded so the
/// month's FIRST buckets can still ASOF back to an anchor in an earlier
/// partition — bounding it would leave the first day or two of every month
/// unpriced. This is `pivot_sql`'s proven shape.
///
/// ## Why `FINAL` plus an explicit `method`, and never `argMax`
///
/// `method` is part of `usd_rate`'s sorting key, deliberately, so an `oracle`
/// row and an `external` row at the same (identity, timestamp) COEXIST rather
/// than one replacing the other. `argMax(usd_rate, timestamp)` across methods
/// would therefore let part read order decide which one prices the candle. The
/// filter picks the series by name.
///
/// ## Why the bucket's end
///
/// See [`bucket_end_expr`]. The `bend` is materialized in the candidate subquery
/// rather than written into the `ASOF ON` clause because ASOF wants a column for
/// its inequality — the same reason `ohlcv_peg_series` projects `bend`.
///
/// ## Why `r.usd > 0` and never `IS NULL`
///
/// `join_use_nulls = 0` on prod: an unmatched ASOF yields the column DEFAULT,
/// which for `Decimal(38, 14)` is `0`, not NULL. A null test would never fire
/// and every unmatched bucket would be written `close_usd = 0 * close = 0` —
/// re-zeroing rows another tier had already priced.
///
/// ## Why the reference needs no `GROUP BY rts` (unlike `ohlcv_peg_series`)
///
/// The read side wraps its `usd_rate` scan in `argMax(rate, rts) … GROUP BY
/// rts` because it reads `method = 'oracle'` rows through a projection that
/// may later be widened. Here `method` is pinned to one value and `method` is
/// the LAST column of `usd_rate`'s sorting key, so `FINAL` yields at most one
/// row per `rts` and the ASOF is unambiguous. ⚠️ That holds only while the
/// `method` filter stays a single equality: relax it to an `IN` and two rows
/// can share an `rts`, at which point this needs the read side's `GROUP BY`.
///
/// ## Why the candidate side is ALSO bounded above by `USDC_ORACLE_EPOCH_S`
///
/// The API labels a scaled USDC-quoted candle by its timestamp alone: below the
/// epoch `external`, at or above it `oracle` (`queries_ch::usd_method_expr`,
/// decision E). That label is only true if this tier never writes above the
/// epoch — otherwise a post-epoch candle the oracle tier missed (a Reflector
/// outage, a poll gap wider than `window_s`) would be priced from the import
/// and reported as a poll that never happened, the mis-attribution task 0247
/// forbids. Bounding the tier to the SAME constant makes the two populations
/// identical by construction; a post-epoch oracle miss falls to the peg tier
/// and is reported as `assumed-par`, which is the truth.
///
/// ## Why BOTH USD columns are recomputed from the one reference
///
/// The other tiers keep `volume_quote_usd` write-once. This one does not,
/// because its candidate set includes rows enriched before `close_usd` existed
/// (`volume_quote_usd > 0`, `close_usd = 0`), and on a USDC leg below the epoch
/// that `volume_quote_usd` can only have been `volume_quote × $1.00` — no
/// oracle had priced USDC yet. Keeping it beside a `close_usd` at 0.9681 is the
/// row `reset_sql`'s doc block rejects: two USD figures for one candle derived
/// from different rates, incoherent no matter which a consumer reads. And a
/// write-once guard here would leave those rows for the peg tier, giving them
/// the par signature AFTER the reset step has already run — so the operator's
/// one-shot campaign would end with rows AC 1 says must not exist.
///
/// "Oracle values win" holds for `close_usd` by the candidate filter alone:
/// the oracle tier writes `close_usd` unconditionally, so an oracle-priced row
/// is never `close_usd = 0` and never a candidate here. For `volume_quote_usd`
/// the argument is the epoch bound above — with one caveat that must be stated
/// rather than assumed (review WR-09). `USDC_ORACLE_EPOCH_S` is defined against
/// `prices.usd_rate`'s first `oracle` row for canonical USDC, but the oracle
/// tier reads **`prices.oracle_prices`**, and `usd_rate`'s oracle rows are
/// copied out of `oracle_prices` behind a `since` watermark
/// (`prices-ingest-core::writer`), so `oracle_prices` MAY hold a USDC reading
/// strictly earlier than the epoch. The residual exposure is therefore exactly
/// this: a row enriched before `close_usd` existed, whose `volume_quote_usd`
/// the oracle tier set from such a reading, is a candidate (`close_usd = 0`,
/// pre-epoch) and has its `volume_quote_usd` recomputed from the bucket-end
/// rate. No `close_usd` is ever wrong; the two USD columns of that row then
/// agree with each other, which is the property this statement exists for.
/// The 0268 reset run refuses outright if `oracle_prices` holds ANY canonical
/// USDC reading below the epoch (`assert_no_pre_epoch_oracle_rows`), so on the
/// operator's campaign the exposure is zero by measurement, not by prose.
/// The SCHEDULED pass carries no such guard (review round 3, IN-17): its
/// exposure needs the very same pre-epoch `oracle_prices` reading, which is
/// the count Appendix B precondition 3 blocks the campaign on, so an operator
/// who ran the precondition has also bounded the Lambda's pass; if that count
/// ever becomes non-zero, the guard belongs in `run()` as well.
/// `external_tier_never_overwrites_a_candle_the_oracle_tier_priced` proves the
/// `close_usd` outcome end to end.
fn external_sql(db: &str, tbl: &str, usdc_id: u32, window: &str) -> String {
    let bend = bucket_end_expr(tbl, "timestamp");
    let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
    format!(
        "INSERT INTO {db}.{tbl} ({INSERT_COLUMNS}) \
         SELECT \
             p.timestamp, p.asset_id, p.quote_asset_id, p.source, \
             p.open, p.high, p.low, p.close, \
             p.volume_base, p.volume_quote, \
             CAST(r.usd * p.volume_quote AS Decimal(38, 14)) AS volume_quote_usd, \
             CAST(r.usd * p.close AS Decimal(38, 14)) AS close_usd, \
             p.vwap, p.trade_count, \
             p.version + 1 AS version \
         FROM ( \
             SELECT \
                 timestamp, asset_id, quote_asset_id, source, \
                 open, high, low, close, \
                 volume_base, volume_quote, volume_quote_usd, close_usd, vwap, \
                 trade_count, version, \
                 1 AS k, \
                 {bend} AS bend \
             FROM {db}.{tbl} FINAL \
             WHERE close_usd = 0 \
               AND volume_quote > 0 \
               AND quote_asset_id = {usdc_id} \
               AND timestamp < toDateTime({epoch}) \
               AND timestamp <= toDateTime(?){window} \
         ) AS p \
         ASOF LEFT JOIN ( \
             SELECT 1 AS k, timestamp AS rts, usd_rate AS usd \
             FROM {db}.usd_rate FINAL \
             WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
               AND issuer_address = '{USDC_ISSUER}' AND contract_address = '' \
               AND method = 'external' AND usd_rate > 0 \
         ) AS r \
             ON r.k = p.k AND r.rts < p.bend \
         WHERE r.usd > 0 \
           AND (toUInt32(p.bend) - toUInt32(r.rts)) <= ? \
         ORDER BY p.timestamp \
         LIMIT ?"
    )
}

/// The count the 0268 reset mode refuses on: canonical USDC readings in
/// `oracle_prices` — the table the oracle tier READS, not `usd_rate` — stamped
/// below `USDC_ORACLE_EPOCH_S`. Pure, so the string is unit-testable; bound
/// parameter: the oracle name.
///
/// Deliberately NOT bounded by the spec's `[not_before, not_after)`, unlike
/// [`ChEnrichmentPass::assert_reset_not_shadowed_by_oracle`]: the external
/// tier's candidate set is bounded by the epoch and the month window only,
/// never by `not_before`, so a pre-epoch reading anywhere below the epoch is
/// inside its reach.
fn pre_epoch_oracle_rows_sql(db: &str, usdc_id: u32) -> String {
    format!(
        "SELECT count() FROM {db}.oracle_prices \
         WHERE asset_id = {usdc_id} AND oracle_name = ? \
           AND timestamp < toDateTime({})",
        prices_clickhouse::USDC_ORACLE_EPOCH_S
    )
}

/// Re-open already-written USD columns for the quote leg a [`UsdResetSpec`]
/// names, by re-inserting the row with both USD columns at 0 and `version + 1`
/// (task 0182).
///
/// ## Why an insert and not `ALTER TABLE … UPDATE`
///
/// A mutation rewrites whole parts and is neither transactional nor cheaply
/// revertible. The `version + 1` re-insert is the same additive move every other
/// statement here makes, so it composes with the `FREEZE` snapshot the repair
/// driver takes: the pre-reset row is still on disk under its old version, and
/// `ATTACH PARTITION` restores it.
///
/// ## Why both USD columns
///
/// `volume_quote_usd` is preserved write-once by the tiers
/// (`if(volume_quote_usd > 0, …)`), so zeroing `close_usd` alone would leave the
/// row carrying two USD figures derived from *different* rates — a `close_usd`
/// at the corrected market rate beside a `volume_quote_usd` still at the old peg,
/// disagreeing by ~7.4× on the USDT rows this was written for. Both columns
/// describe the same candle at the same instant; a repair that fixes one and
/// pins the other produces a row that is internally incoherent no matter which
/// figure a consumer reads. Zeroing both lets the pivot recompute both from one
/// reference.
///
/// Bound parameters, in SQL order: the snapshot watermark, then the `LIMIT`.
fn reset_sql(db: &str, tbl: &str, spec: &UsdResetSpec, window: &str) -> String {
    // Task 0268's two extensions, appended so a spec that asks for neither leaves
    // the 0182 statement byte-identical.
    //
    // ⚠️ The par signature MUST be `p.`-qualified here, and only here. This
    // statement's own projection declares `CAST(0 AS Decimal(38, 14)) AS
    // close_usd`, and ClickHouse resolves an identifier against SELECT aliases
    // BEFORE table columns (`prefer_column_name_to_alias` defaults to 0 — that
    // setting exists for exactly this collision). A bare `close_usd = close`
    // therefore evaluates as `0 = close`, matches nothing, and the campaign
    // exits 0 having reset no rows: `count_reset_pending` reports the full
    // population, the reset writes nothing, and the run logs only "made no
    // progress". Verified on ClickHouse 26.3.10.60:
    //     SELECT CAST(0 AS Decimal(38,14)) AS close_usd, p.close AS close
    //     FROM (SELECT toDecimal128(5,14) AS close_usd,
    //                  toDecimal128(5,14) AS close) AS p
    //     WHERE close_usd = close        -- EMPTY: evaluated 0 = 5
    //     WHERE p.close_usd = p.close    -- 1 row
    // `reset_pending_pred` and the month enumeration keep the BARE form: they
    // render into alias-free statements with no `p` in scope, where a `p.`
    // prefix would be a syntax error. The day-set fragment is shared verbatim by
    // all three, which is what the invariant test can still prove.
    let mut bounds = String::new();
    if let Some(na) = spec.not_after {
        bounds.push_str(&format!(" AND p.timestamp < toDateTime({na})"));
    }
    if spec.require_external_rate {
        bounds.push_str(&format!(
            " AND p.close_usd = p.close AND {}",
            external_rate_day_pred(db)
        ));
    }
    // Task 0228. The rate fragment names `timestamp`, not a projected alias, so
    // it stays BARE here exactly as it is at the other two sites — which is what
    // lets the three-sites test prove they are one definition. Only terms naming
    // a column this statement also projects (`close_usd`, `volume_quote_usd`) need
    // the `p.` qualifier, and this mode appends none.
    if spec.require_pivot_usdc_rate {
        bounds.push_str(&format!(" AND {}", external_rate_day_pred(db)));
    }
    format!(
        "INSERT INTO {db}.{tbl} ({INSERT_COLUMNS}) \
         SELECT \
             p.timestamp, p.asset_id, p.quote_asset_id, p.source, \
             p.open, p.high, p.low, p.close, \
             p.volume_base, p.volume_quote, \
             CAST(0 AS Decimal(38, 14)) AS volume_quote_usd, \
             CAST(0 AS Decimal(38, 14)) AS close_usd, \
             p.vwap, p.trade_count, \
             p.version + 1 AS version \
         FROM {db}.{tbl} AS p FINAL \
         WHERE p.quote_asset_id = {q} \
           AND p.timestamp >= toDateTime({nb}){bounds} \
           AND (p.close_usd > 0 OR p.volume_quote_usd > 0) \
           AND p.volume_quote > 0 \
           AND p.timestamp <= toDateTime(?){window} \
         ORDER BY p.timestamp \
         LIMIT ?",
        q = spec.quote_asset_id,
        nb = spec.not_before,
        bounds = bounds,
    )
}

/// Pivot statement: candles quoted in `ref_id` get
/// `close_usd = close × ref_usd × usdc_usd`, where `ref_usd` is the reference
/// asset's volume-weighted close against USDC (forward-filled by an
/// `ASOF LEFT JOIN`) and `usdc_usd` is the MEASURED USDC/USD rate at the
/// candidate's bucket END. `ref_id` is XLM, or — since task 0172 — the depegged
/// USDT, which is priced by measurement rather than assumed to be $1 (see
/// [`ReferenceIds::pivot_ids`]).
///
/// ## Why the USDC/USD factor is here at all (task 0228, decision A)
///
/// `ref_usd` is a price in **USDC**, not in dollars. Until task 0228 this
/// statement stopped there, so every XLM- and USDT-quoted candle carried the
/// `USDC = $1` assumption that task 0268 had just removed from the USDC leg
/// itself — the last population of stored USD values resting on the peg. USDC
/// closed at **0.9681** on 2023-03-11, so the stored value was ~3.2% high across
/// the whole pre-epoch pivot population. The factor enters HERE, inside the
/// write path, rather than on the read side: stored and served must not disagree
/// again (that was the 0267 state 0268 ended).
///
/// ⚠️ **Vocabulary (task 0228, decision E).** A pivot leg's stored USD value is
/// now `close × the reference's own USDC close × the measured USDC/USD rate`.
/// That composition coins **no new `method` word**: `/ohlcv` still labels a pivot
/// leg `traded` (`queries_ch::usd_method_expr`), because the label names how the
/// price was reached — through the reference asset's own market — not which
/// factors the arithmetic carried.
///
/// ## The reference stays INLINE
///
/// Computed inline as a subquery (no DDL, so the writer needs no `CREATE TABLE`
/// grant on the shared tenant; task 0083). Its
/// `WHERE asset_id = ref AND quote_asset_id = usdc` is a sort-key prefix, so each
/// batch re-aggregates only that single pair's slice, not the whole table.
/// `ref_asset_id` is the constant reference id so the ASOF join keeps its
/// required equality predicate (`r.ref_asset_id = p.quote_asset_id`) and matches
/// only candles quoted in that reference asset — which is what makes the XLM and
/// USDT passes disjoint and safe to run in sequence.
///
/// ## Bind order (positional)
///
/// 1. the snapshot watermark — inside the CANDIDATE subquery, which is rendered
///    FIRST (`external_sql`'s layout);
/// 2. the snapshot watermark again — inside the inline reference subquery;
/// 3. the pivot staleness window in seconds (`pivot_window_s`), bounding how far
///    the reference may be forward-filled;
/// 4. the `LIMIT` (batch size).
///
/// ⚠️ The first two moved relative to the pre-0228 statement. The candidate scan
/// used to be the outer `FROM`; it is now a subquery, because ASOF needs a
/// materialized COLUMN for its inequality and the bucket end has to be projected
/// somewhere. Its watermark therefore sits textually BEFORE the reference
/// subquery's. Positional binds do not fail loudly when reordered — they bind a
/// batch size as a timestamp at RUN time, on prod — so
/// `pivot_sql_bind_order_is_watermark_watermark_window_then_limit` pins the text.
///
/// ## Why the bucket's END for the rate
///
/// See [`bucket_end_expr`] and `external_sql`'s own block: `timestamp` is the
/// bucket's START but `close` is the period's LAST close, so a rate resolved at
/// `timestamp` prices a weekly or monthly candle from the day the period opened.
/// The reference vwap keeps resolving at the bucket START (`r.timestamp <=
/// p.timestamp`) — that leg is unchanged from the pre-0228 statement and its
/// staleness is the configurable `pivot_window_s`, not the derived bound.
///
/// ## Why `FINAL` plus an explicit `method` per leg, and never `argMax`
///
/// `method` is part of `usd_rate`'s sorting key, deliberately, so an `oracle` row
/// and an `external` row at the same (identity, timestamp) COEXIST rather than
/// one replacing the other. `argMax(usd_rate, timestamp)` across methods would
/// let part read order decide which one prices the candle. Each leg picks its
/// series by name, and the preference (`oracle`, else `external`) is expressed by
/// the `multiIf` rather than by recency.
///
/// ## Why the two rate joins are NESTED, not chained at one level
///
/// `queries_ch::peg_series_sql` states the reason and is the only in-repo
/// precedent for two method-specific ASOF joins: a nested subquery needs nothing
/// from the multi-JOIN rewrite and reads the same under both analyzers. The two
/// right sides carry DISTINCT column names (`orts`/`orate` vs `erts`/`erate`) for
/// the same reason.
///
/// ## Why the rate legs are tested POSITIVELY and never for nullity
///
/// `join_use_nulls = 0` on prod: an unmatched ASOF yields the column DEFAULT,
/// which for `Decimal(38, 14)` is `0` and not NULL. A null test would never fire.
/// The one `IS NOT NULL` in this statement is on the REFERENCE leg, where it is
/// legal precisely because that subquery's `nullIf(sum(...), 0)` makes the column
/// Nullable.
///
/// An unmatched staleness test fails closed the same way: an unmatched `DateTime`
/// defaults to 1970, so `bend - rts` is enormous and exceeds the bound.
///
/// ## Why a bucket with NEITHER rate is left unpriced
///
/// The `multiIf`'s else-branch is 0 and the final `WHERE … > 0` drops the row, so
/// it stays at `close_usd = 0` for this pass rather than being written as
/// `0 × close`. That is the module doc's "no reference → never a wrong non-NULL
/// value" rule (see the top of this file), and it is what makes the 0228 campaign
/// safe: the reset only re-opens rows a rate can refill.
fn pivot_sql(db: &str, tbl: &str, ref_id: u32, usdc_id: u32, window: &str) -> String {
    let bend = bucket_end_expr(tbl, "timestamp");
    // DERIVED from the table, never configured — see `external_window_s`. Inlined
    // rather than bound: each `?` is a separate positional parameter, so
    // referencing it from both rate legs would cost two more binds for a value no
    // operator may set.
    let stale = external_window_s(tbl);
    // "This leg supplied a usable rate for this bucket": matched (the rate is
    // positive) AND its anchor is within the derived bound of the bucket end.
    // The ONLY definition of validity; every expression below reads these.
    let o_ok = format!("(po.orate > 0 AND (toUInt32(po.bend) - toUInt32(po.orts)) <= {stale})");
    let e_ok = format!("(re.erate > 0 AND (toUInt32(po.bend) - toUInt32(re.erts)) <= {stale})");
    // A valid ORACLE reading wins the bucket outright — the same preference
    // `views.sql`'s rank-first tuple and `peg_series_sql`'s `multiIf` apply, so
    // the write path and the two read surfaces agree on any bucket holding both.
    // The else-branch is 0, which the outer `WHERE` then drops.
    let rate = format!("multiIf({o_ok}, po.orate, {e_ok}, re.erate, toDecimal128(0, 14))");
    format!(
        "INSERT INTO {db}.{tbl} ({INSERT_COLUMNS}) \
         SELECT \
             po.timestamp, po.asset_id, po.quote_asset_id, po.source, \
             po.open, po.high, po.low, po.close, \
             po.volume_base, po.volume_quote, \
             if(po.volume_quote_usd > 0, po.volume_quote_usd, CAST(po.refusd * toFloat64(po.volume_quote) * toFloat64({rate}) AS Decimal(38, 14))) AS volume_quote_usd, \
             CAST(po.refusd * toFloat64(po.close) * toFloat64({rate}) AS Decimal(38, 14)) AS close_usd, \
             po.vwap, po.trade_count, \
             po.version + 1 AS version \
         FROM ( \
             SELECT \
                 pr.timestamp AS timestamp, pr.asset_id AS asset_id, \
                 pr.quote_asset_id AS quote_asset_id, pr.source AS source, \
                 pr.open AS open, pr.high AS high, pr.low AS low, pr.close AS close, \
                 pr.volume_base AS volume_base, pr.volume_quote AS volume_quote, \
                 pr.volume_quote_usd AS volume_quote_usd, pr.vwap AS vwap, \
                 pr.trade_count AS trade_count, pr.version AS version, \
                 pr.k AS k, pr.bend AS bend, pr.refusd AS refusd, \
                 ro.orts AS orts, ro.orate AS orate \
             FROM ( \
                 SELECT \
                     p.timestamp AS timestamp, p.asset_id AS asset_id, \
                     p.quote_asset_id AS quote_asset_id, p.source AS source, \
                     p.open AS open, p.high AS high, p.low AS low, p.close AS close, \
                     p.volume_base AS volume_base, p.volume_quote AS volume_quote, \
                     p.volume_quote_usd AS volume_quote_usd, p.vwap AS vwap, \
                     p.trade_count AS trade_count, p.version AS version, \
                     p.k AS k, p.bend AS bend, \
                     r.usd AS refusd \
                 FROM ( \
                     SELECT \
                         timestamp, asset_id, quote_asset_id, source, \
                         open, high, low, close, \
                         volume_base, volume_quote, volume_quote_usd, close_usd, vwap, \
                         trade_count, version, \
                         1 AS k, \
                         {bend} AS bend \
                     FROM {db}.{tbl} FINAL \
                     WHERE quote_asset_id = {ref_id} \
                       AND close_usd = 0 \
                       AND volume_quote > 0 \
                       AND timestamp <= toDateTime(?){window} \
                 ) AS p \
                 ASOF LEFT JOIN ( \
                     SELECT \
                         CAST({ref_id} AS UInt32) AS ref_asset_id, \
                         timestamp, \
                         sum(toFloat64(close) * toFloat64(volume_base)) / nullIf(sum(toFloat64(volume_base)), 0) AS usd \
                     FROM {db}.{tbl} FINAL \
                     WHERE asset_id = {ref_id} AND quote_asset_id = {usdc_id} \
                       AND timestamp <= toDateTime(?) \
                     GROUP BY timestamp \
                     ORDER BY timestamp \
                 ) AS r \
                     ON r.ref_asset_id = p.quote_asset_id AND r.timestamp <= p.timestamp \
                 WHERE r.usd IS NOT NULL \
                   AND (p.timestamp - r.timestamp) <= ? \
             ) AS pr \
             ASOF LEFT JOIN ( \
                 SELECT 1 AS ok, timestamp AS orts, usd_rate AS orate \
                 FROM {db}.usd_rate FINAL \
                 WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
                   AND issuer_address = '{USDC_ISSUER}' AND contract_address = '' \
                   AND method = 'oracle' AND usd_rate > 0 \
             ) AS ro \
                 ON pr.k = ro.ok AND ro.orts < pr.bend \
         ) AS po \
         ASOF LEFT JOIN ( \
             SELECT 1 AS ek, timestamp AS erts, usd_rate AS erate \
             FROM {db}.usd_rate FINAL \
             WHERE asset_kind = 'credit' AND asset_code = 'USDC' \
               AND issuer_address = '{USDC_ISSUER}' AND contract_address = '' \
               AND method = 'external' AND usd_rate > 0 \
         ) AS re \
             ON po.k = re.ek AND re.erts < po.bend \
         WHERE {rate} > 0 \
         ORDER BY po.timestamp \
         LIMIT ?"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peg_sql_is_none_without_stablecoins() {
        assert!(peg_sql("prices", "price_ohlcv_1m", &[], "").is_none());
    }

    // -- task 0182: the USD reset ------------------------------------------

    // ---- task 0268: the external-scoped reset -----------------------------

    /// A 0268-shaped spec: USDC's quote leg, bounded above by the oracle epoch,
    /// and only where the imported series can refill what it zeroes.
    fn usdc_external_reset() -> UsdResetSpec {
        UsdResetSpec {
            quote_asset_id: 2,
            not_before: 0,
            not_after: Some(prices_clickhouse::USDC_ORACLE_EPOCH_S),
            require_external_rate: true,
            require_pivot_usdc_rate: false,
        }
    }

    /// A 0228-shaped spec: an XLM quote leg (a pivot reference), bounded above by
    /// the oracle epoch, and only where the imported USDC series can refill what
    /// it zeroes. The quote leg is a PIVOT id, which is what separates it from
    /// [`usdc_external_reset`].
    fn xlm_pivot_reset() -> UsdResetSpec {
        UsdResetSpec {
            quote_asset_id: 4,
            not_before: 1_611_532_800,
            not_after: Some(prices_clickhouse::USDC_ORACLE_EPOCH_S),
            require_external_rate: false,
            require_pivot_usdc_rate: true,
        }
    }

    /// 🔑 D-06's LOCKSTEP REQUIREMENT. The loop's termination test
    /// (`reset_pending_pred`), the statement that zeroes (`reset_sql`) and the
    /// month enumeration (`repair_target_pred`, spliced into `repair.rs`) must
    /// carry the SAME predicate.
    ///
    /// Two halves of this disagreeing is what hid task 0182 for a month: the
    /// driver enumerated `CANDIDATE_PRED`, every row 0182 targeted had
    /// `close_usd > 0`, so `--dry-run` reported "no months with enrichable
    /// zeros" — a green all-clear over 44,657 wrong values. One definition,
    /// asserted present in all three renderings.
    #[test]
    fn the_external_predicate_is_one_definition_used_by_all_three_sites() {
        let spec = usdc_external_reset();
        let frag = external_rate_day_pred("prices");
        assert!(
            reset_pending_pred("prices", &spec).contains(&frag),
            "pending"
        );
        assert!(
            reset_sql("prices", "price_ohlcv_1d", &spec, "").contains(&frag),
            "reset_sql"
        );
        assert!(
            repair_target_pred("prices", Some(&spec)).contains(&frag),
            "repair_target_pred"
        );
    }

    // ---- task 0228: the pivot-leg reset mode -------------------------------

    /// 🔑 THE SAME LOCKSTEP REQUIREMENT, for the pivot-leg mode. One definition
    /// of "an imported rate covers this bucket's day", rendered at the pending
    /// count, the reset statement and the month enumeration. The sibling of
    /// `the_external_predicate_is_one_definition_used_by_all_three_sites`.
    #[test]
    fn the_pivot_rate_predicate_is_one_definition_used_by_all_three_sites() {
        let spec = xlm_pivot_reset();
        let frag = external_rate_day_pred("prices");
        assert!(
            reset_pending_pred("prices", &spec).contains(&frag),
            "pending"
        );
        assert!(
            reset_sql("prices", "price_ohlcv_1d", &spec, "").contains(&frag),
            "reset_sql"
        );
        assert!(
            repair_target_pred("prices", Some(&spec)).contains(&frag),
            "repair_target_pred"
        );
    }

    /// 🔑 The 0228 mode appends the rate fragment and NOTHING ELSE. The par
    /// signature is 0268's, and a pivoted row never carries it — including it here
    /// would select zero rows and report a clean, entirely empty campaign.
    #[test]
    fn the_pivot_reset_carries_no_par_signature() {
        let pred = reset_pending_pred("prices", &xlm_pivot_reset());
        assert!(!pred.contains("close_usd = close"), "{pred}");
        assert!(pred.contains("quote_asset_id = 4"), "{pred}");
        // The termination term survives: a zeroed row stops matching.
        assert!(
            pred.contains("(close_usd > 0 OR volume_quote_usd > 0)"),
            "{pred}"
        );
        // And it mirrors the pivot's own filter, so it cannot zero a row the
        // pivot is structurally unable to refill.
        assert!(pred.contains("volume_quote > 0"), "{pred}");

        let sql = reset_sql("prices", "price_ohlcv_1d", &xlm_pivot_reset(), "");
        assert!(!sql.contains("p.close_usd = p.close"), "{sql}");
        assert!(!sql.contains(" AND close_usd = close AND"), "{sql}");
    }

    /// The alias split 0268 learned the hard way, holding for the new mode:
    /// terms naming a column `reset_sql` also PROJECTS stay `p.`-qualified, while
    /// the shared rate fragment — which names `timestamp`, not a projected alias —
    /// stays BARE at all three sites. That is what lets the three-sites test above
    /// compare the fragment for equality rather than for resemblance.
    #[test]
    fn the_pivot_reset_keeps_its_projected_columns_qualified_and_the_rate_bare() {
        let sql = reset_sql("prices", "price_ohlcv_1h", &xlm_pivot_reset(), "");
        assert!(
            sql.contains("CAST(0 AS Decimal(38, 14)) AS close_usd"),
            "the colliding alias is still declared: {sql}"
        );
        assert!(
            sql.contains("(p.close_usd > 0 OR p.volume_quote_usd > 0)"),
            "{sql}"
        );
        assert!(sql.contains("p.volume_quote > 0"), "{sql}");
        assert!(sql.contains("p.quote_asset_id = 4"), "{sql}");
        // The shared fragment is verbatim, unqualified, in the same statement.
        assert!(
            sql.contains(&external_rate_day_pred("prices")),
            "the rate fragment must not be rewritten here: {sql}"
        );
    }

    /// The two modes are mutually exclusive, refused PURELY so the CLI can say so
    /// before it opens a connection and the library says so even when driven by
    /// something else. Both predicates are appended, so a combined spec ANDs a par
    /// signature onto a population that never carries one: zero candidates, and a
    /// run that reports a clean repair having discarded and recomputed nothing.
    #[test]
    fn the_two_reset_modes_are_refused_together() {
        let both = UsdResetSpec {
            require_external_rate: true,
            ..xlm_pivot_reset()
        };
        assert!(matches!(
            both.validate(),
            Err(ChEnrichError::ResetModesAreMutuallyExclusive { quote_asset_id }) if quote_asset_id == 4
        ));
        let msg = both.validate().unwrap_err().to_string();
        assert!(msg.contains("--reset-require-external-rate"), "{msg}");
        assert!(msg.contains("--reset-require-pivot-usdc-rate"), "{msg}");
        // Each mode alone is fine, and so is neither.
        xlm_pivot_reset().validate().unwrap();
        usdc_external_reset().validate().unwrap();
        usdt_reset().validate().unwrap();
    }

    /// WR-05's refusal covers the new mode too, and it is PURE — so the CLI
    /// refuses an empty `[not_before, not_after)` before it opens a connection,
    /// and before the driver's month enumeration silently finds nothing.
    ///
    /// Kept a unit test rather than a seeded one deliberately: a refusal that
    /// needs no database is one CI actually runs.
    #[test]
    fn an_empty_window_is_refused_for_the_pivot_mode_too() {
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        let empty = UsdResetSpec {
            not_before: epoch,
            ..xlm_pivot_reset()
        };
        assert!(matches!(
            empty.validate(),
            Err(ChEnrichError::ResetWindowEmpty { quote_asset_id, .. }) if quote_asset_id == 4
        ));
        let inverted = UsdResetSpec {
            not_before: epoch + 1,
            ..xlm_pivot_reset()
        };
        assert!(inverted.validate().is_err());
    }

    /// The bounds and the versioned insert survive the new mode, so a FREEZE is
    /// still a rollback point and the scan still prunes to one partition.
    #[test]
    fn the_pivot_reset_keeps_the_partition_window_and_the_versioned_insert() {
        let win = " AND p.timestamp >= toDateTime(100) AND p.timestamp < toDateTime(200)";
        let sql = reset_sql("prices", "price_ohlcv_1d", &xlm_pivot_reset(), win);
        assert!(sql.contains(win), "{sql}");
        assert!(sql.contains("p.version + 1 AS version"), "{sql}");
        assert!(!sql.contains("ALTER TABLE"), "{sql}");
        assert_eq!(
            sql.matches('?').count(),
            2,
            "binds stay watermark, limit: {sql}"
        );
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        assert!(
            sql.contains(&format!("p.timestamp < toDateTime({epoch})")),
            "{sql}"
        );
    }

    /// The predicate must be an UNCORRELATED day-set. `months_with_zeros` splices
    /// `repair_target_pred` into a bare `WHERE` of its own grouped scan, with no
    /// outer alias in scope, so a correlated `EXISTS` or a JOIN is not merely
    /// stylistically different — it is illegal there.
    #[test]
    fn external_rate_day_pred_is_an_uncorrelated_day_set() {
        let frag = external_rate_day_pred("prices");
        assert!(
            frag.starts_with("toDate(timestamp, 'UTC') IN (SELECT toDate(timestamp, 'UTC')"),
            "{frag}"
        );
        assert!(frag.contains("prices.usd_rate FINAL"), "{frag}");
        assert!(frag.contains("method = 'external'"), "{frag}");
        assert!(frag.contains("asset_kind = 'credit'"), "{frag}");
        assert!(frag.contains("asset_code = 'USDC'"), "{frag}");
        assert!(
            frag.contains(&format!("issuer_address = '{USDC_ISSUER}'")),
            "{frag}"
        );
        assert!(frag.contains("contract_address = ''"), "{frag}");
        assert!(
            !frag.contains("p."),
            "a correlated reference is illegal in months_with_zeros: {frag}"
        );
        assert!(!frag.contains("EXISTS"), "{frag}");
    }

    /// ⚠️ The par signature must be `p.`-qualified in [`reset_sql`] and BARE in
    /// [`reset_pending_pred`], because only `reset_sql` declares a `close_usd`
    /// alias in its own projection — and ClickHouse resolves aliases before
    /// columns, so the bare form there silently becomes `0 = close` and the
    /// campaign resets nothing while reporting a full population. This is the
    /// regression that shipped through three review rounds and was caught only
    /// by a live ClickHouse; the string test that guarded this file compared the
    /// two fragments for EQUALITY, which is precisely what must not hold.
    #[test]
    fn the_reset_statement_qualifies_the_par_signature_against_its_own_alias() {
        let spec = usdc_external_reset();
        let sql = reset_sql("prices", "price_ohlcv_1h", &spec, "");
        assert!(
            sql.contains("CAST(0 AS Decimal(38, 14)) AS close_usd"),
            "the projection still declares the colliding alias: {sql}"
        );
        assert!(
            sql.contains("AND p.close_usd = p.close AND"),
            "the reset must compare COLUMNS, not its own zero alias: {sql}"
        );
        assert!(
            !sql.contains(" AND close_usd = close AND"),
            "an unqualified par signature here resolves to the alias: {sql}"
        );
        // The alias-free sites keep the bare form: `p` is not in scope there.
        let pred = reset_pending_pred("prices", &spec);
        assert!(
            pred.contains(" AND close_usd = close AND"),
            "the pending predicate has no table alias to qualify: {pred}"
        );
        assert!(!pred.contains("p.close_usd"), "{pred}");
    }

    /// D-04: the reset's candidate set carries the peg tier's EXACT signature, so
    /// a candle the oracle or external tier already priced is never re-opened.
    /// And it still terminates: a zeroed row stops matching
    /// `(close_usd > 0 OR volume_quote_usd > 0)`.
    #[test]
    fn the_external_reset_narrows_to_the_par_signature_and_still_terminates() {
        let pred = reset_pending_pred("prices", &usdc_external_reset());
        assert!(pred.contains("close_usd = close"), "{pred}");
        assert!(
            pred.contains("(close_usd > 0 OR volume_quote_usd > 0)"),
            "the termination term must survive: {pred}"
        );
    }

    /// `not_after` bounds the reset above; `None` adds nothing at all.
    #[test]
    fn not_after_bounds_the_reset_above_and_none_adds_nothing() {
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        let bounded = reset_pending_pred("prices", &usdc_external_reset());
        assert!(
            bounded.contains(&format!("timestamp < toDateTime({epoch})")),
            "{bounded}"
        );
        let sql = reset_sql("prices", "price_ohlcv_1d", &usdc_external_reset(), "");
        assert!(
            sql.contains(&format!("timestamp < toDateTime({epoch})")),
            "{sql}"
        );

        let unbounded = reset_pending_pred("prices", &usdt_reset());
        assert!(
            !unbounded.contains("timestamp < toDateTime("),
            "{unbounded}"
        );
    }

    /// 🔑 THE 0182 PATH PROVABLY DOES NOT CHANGE. A spec that asks for NEITHER
    /// rate-gated mode and no upper bound renders byte-identically to the
    /// pre-0268 statement in all three sites. Anything less than byte equality
    /// here means a later task quietly altered the behaviour of a repair mode that
    /// has already run against production.
    ///
    /// Task 0228 appends its mode the same way 0268 did, so this test now pins
    /// the pre-0228 strings as well — the `usdt_reset()` fixture sets both flags
    /// false and this equality is what proves nothing leaked into that path.
    #[test]
    fn a_0182_shaped_spec_renders_byte_identically_to_the_pre_0268_statement() {
        let spec = usdt_reset();
        assert_eq!(
            reset_pending_pred("prices", &spec),
            "quote_asset_id = 111 AND timestamp >= toDateTime(1612656000) \
             AND (close_usd > 0 OR volume_quote_usd > 0) AND volume_quote > 0"
        );
        assert_eq!(
            repair_target_pred("prices", Some(&spec)),
            format!(
                "({CANDIDATE_PRED}) OR (quote_asset_id = 111 \
                 AND timestamp >= toDateTime(1612656000) \
                 AND (close_usd > 0 OR volume_quote_usd > 0) AND volume_quote > 0)"
            )
        );
        let sql = reset_sql("prices", "price_ohlcv_1h", &spec, "");
        assert!(
            !sql.contains("usd_rate"),
            "no reference join on the 0182 path: {sql}"
        );
        assert!(!sql.contains("close_usd = close"), "{sql}");
        assert!(!sql.contains("toDate("), "{sql}");
        assert!(!sql.contains("timestamp < toDateTime("), "{sql}");
    }

    /// The 0114 partition bound still reaches the outer `WHERE`, and the reset is
    /// still a versioned insert rather than a mutation — so a FREEZE stays a
    /// rollback point. Mirrors `reset_sql_threads_the_partition_window`.
    #[test]
    fn the_external_reset_keeps_the_partition_window_and_the_versioned_insert() {
        let win = " AND p.timestamp >= toDateTime(100) AND p.timestamp < toDateTime(200)";
        let sql = reset_sql("prices", "price_ohlcv_1d", &usdc_external_reset(), win);
        assert!(sql.contains(win), "{sql}");
        assert!(sql.contains("p.version + 1 AS version"), "{sql}");
        assert!(!sql.contains("ALTER TABLE"), "{sql}");
        assert_eq!(
            sql.matches('?').count(),
            2,
            "binds stay watermark, limit: {sql}"
        );
    }

    fn usdt_reset() -> UsdResetSpec {
        // 2021-02-07, the start of USDT's USDC market.
        UsdResetSpec {
            quote_asset_id: 111,
            not_before: 1_612_656_000,
            // Task 0182's shape, stated rather than defaulted: unbounded above,
            // and no reference join of either kind. This fixture is what pins
            // that path, and every later mode is APPENDED so it stays byte-exact.
            not_after: None,
            require_external_rate: false,
            require_pivot_usdc_rate: false,
        }
    }

    /// The whole point of the reset: it must select rows the steady-state pass
    /// filters *out*. A reset that only matched `close_usd = 0` would be a no-op
    /// over the exact population task 0182 exists to correct.
    #[test]
    fn reset_sql_targets_written_values_not_zeros() {
        let sql = reset_sql("prices", "price_ohlcv_1d", &usdt_reset(), "");
        assert!(sql.contains("(p.close_usd > 0 OR p.volume_quote_usd > 0)"));
        assert!(!sql.contains("p.close_usd = 0"));
    }

    /// Both USD columns go to zero. Zeroing `close_usd` alone would leave
    /// `volume_quote_usd` pinned by the tiers' write-once guard, so the repaired
    /// row would carry two figures derived from different rates.
    #[test]
    fn reset_sql_zeroes_both_usd_columns() {
        let sql = reset_sql("prices", "price_ohlcv_1d", &usdt_reset(), "");
        assert!(sql.contains("CAST(0 AS Decimal(38, 14)) AS volume_quote_usd"));
        assert!(sql.contains("CAST(0 AS Decimal(38, 14)) AS close_usd"));
    }

    /// The epoch bound is load-bearing, not cosmetic: below it the pivot has no
    /// reference, so a reset row can never be refilled and stays at 0 forever.
    #[test]
    fn reset_sql_honours_the_epoch_and_the_quote_leg() {
        let sql = reset_sql("prices", "price_ohlcv_1d", &usdt_reset(), "");
        assert!(sql.contains("p.quote_asset_id = 111"));
        assert!(sql.contains("p.timestamp >= toDateTime(1612656000)"));
    }

    /// Mirrors the pivot's own filter, so the reset cannot zero a row the pivot
    /// is structurally unable to refill.
    #[test]
    fn reset_sql_will_not_reopen_a_row_the_pivot_cannot_refill() {
        let sql = reset_sql("prices", "price_ohlcv_1d", &usdt_reset(), "");
        assert!(sql.contains("p.volume_quote > 0"));
        // Task 0228 moved the pivot's candidate filters into its candidate
        // subquery, where they are alias-free. Same filter, same meaning.
        assert!(pivot_sql("prices", "price_ohlcv_1d", 111, 3, "").contains("AND volume_quote > 0"));
    }

    /// Additive, like every other statement here — so the FREEZE snapshot the
    /// repair driver takes is a real rollback point.
    #[test]
    fn reset_sql_is_a_versioned_insert_not_a_mutation() {
        let sql = reset_sql("prices", "price_ohlcv_1d", &usdt_reset(), "");
        assert!(sql.starts_with("INSERT INTO prices.price_ohlcv_1d"));
        assert!(sql.contains("p.version + 1 AS version"));
        assert!(!sql.contains("ALTER"));
    }

    #[test]
    fn reset_sql_threads_the_partition_window() {
        let sql = reset_sql(
            "prices",
            "price_ohlcv_1d",
            &usdt_reset(),
            " AND p.timestamp >= toDateTime(100) AND p.timestamp < toDateTime(200)",
        );
        assert!(sql.contains("AND p.timestamp >= toDateTime(100)"));
        assert!(sql.contains("AND p.timestamp < toDateTime(200)"));
    }

    /// The reset loop terminates because a reset row stops matching. Without the
    /// `(close_usd > 0 OR volume_quote_usd > 0)` term the count would never fall
    /// and the pass would spin to its batch ceiling on every run.
    #[test]
    fn reset_pending_pred_stops_matching_once_a_row_is_zeroed() {
        let pred = reset_pending_pred("prices", &usdt_reset());
        assert!(pred.contains("(close_usd > 0 OR volume_quote_usd > 0)"));
        assert!(pred.contains("quote_asset_id = 111"));
        assert!(pred.contains("volume_quote > 0"));
    }

    /// Without a spec the repair driver enumerates exactly what it always did —
    /// so the 0114 historical repair and the hourly sweep are unchanged.
    #[test]
    fn repair_target_pred_is_unchanged_without_a_reset() {
        assert_eq!(repair_target_pred("prices", None), CANDIDATE_PRED);
    }

    /// The regression that made 0182 invisible: the driver enumerated
    /// `close_usd = 0`, every affected row had `close_usd > 0`, so a dry run
    /// reported "no months with enrichable zeros" over 44,657 wrong values.
    /// With a spec the enumeration must admit the written-value rows too.
    #[test]
    fn repair_target_pred_sees_months_that_hold_only_written_values() {
        let pred = repair_target_pred("prices", Some(&usdt_reset()));
        assert!(pred.contains(CANDIDATE_PRED));
        assert!(pred.contains("(close_usd > 0 OR volume_quote_usd > 0)"));
        // Still an OR, not a replacement — a reset run must not stop finding
        // ordinary zeros in the same months.
        assert!(pred.contains(") OR ("));
    }

    #[test]
    fn peg_sql_fills_close_usd_for_stable_quotes() {
        let sql = peg_sql("prices", "price_ohlcv_1m", &[3, 7], "").unwrap();
        // close_usd column present and positionally after volume_quote_usd.
        assert!(sql.contains("volume_quote_usd, close_usd, vwap"));
        assert!(sql.contains("CAST(p.close AS Decimal(38, 14)) AS close_usd"));
        // Only touches oracle-missed rows, only the named stablecoin quotes.
        assert!(sql.contains("p.close_usd = 0"));
        assert!(sql.contains("p.quote_asset_id IN (3, 7)"));
        // Never clobbers an oracle-set volume_quote_usd.
        assert!(sql.contains("if(p.volume_quote_usd > 0, p.volume_quote_usd,"));
        // Snapshot watermark bound (binds before LIMIT): watermark, then batch.
        assert!(sql.contains("p.timestamp <= toDateTime(?)"));
        assert!(
            sql.find("p.timestamp <= toDateTime(?)").unwrap() < sql.find("LIMIT ?").unwrap(),
            "watermark bind must precede the LIMIT bind"
        );
    }

    #[test]
    fn pivot_sql_computes_the_xlm_usdc_reference_inline() {
        let sql = pivot_sql("prices", "price_ohlcv_1m", 5, 3, "");
        // Reference is an inline ASOF-join subquery — NOT a pre-materialized table
        // (task 0083: no CREATE TABLE grant needed on the shared tenant).
        assert!(sql.contains("ASOF LEFT JOIN ("));
        assert!(!sql.contains("CREATE TABLE"));
        // Subquery aggregates the XLM/USDC market (asset 5 quoted in asset 3).
        assert!(sql.contains("asset_id = 5 AND quote_asset_id = 3"));
        assert!(sql.contains("CAST(5 AS UInt32) AS ref_asset_id"));
        assert!(sql.contains("GROUP BY timestamp"));
        // ASOF equality predicate + forward-fill inequality. The reference leg
        // still resolves at the bucket START; only the RATE legs moved to the end.
        assert!(sql.contains("r.ref_asset_id = p.quote_asset_id AND r.timestamp <= p.timestamp"));
        // Redundant-but-load-bearing: the ASOF `ON` already constrains
        // p.quote_asset_id to ref_id, but only as a join condition, so the outer
        // scan had no literal on the sort key's 2nd column
        // (asset_id, quote_asset_id, source, timestamp) and read the whole table
        // FINAL. Task 0172 added a second pivot pass, making this a 3-statement
        // tier in the pass task 0111 is open on. Keep the literal — task 0228
        // moved it into the candidate subquery, where it prunes the same way.
        assert!(sql.contains("WHERE quote_asset_id = 5"));
    }

    // ---- task 0228: the pivot scales by the measured USDC/USD rate ----------

    /// 🔑 THE 0228 DEFECT, as a string. The stored value is the TRIPLE product —
    /// the candidate close, the reference's own USDC close, and the measured
    /// USDC/USD rate. The pre-0228 statement stopped at the pair, which is a price
    /// in USDC wearing a dollar column name.
    ///
    /// Every Float/Decimal boundary is crossed by an explicit `toFloat64`, as
    /// every other arithmetic site in this repo does: `refusd` is a Float64 (the
    /// reference subquery's `sum(...) / nullIf(...)`) while `usd_rate` is
    /// `Decimal(38, 14)`.
    #[test]
    fn pivot_sql_writes_close_usd_as_the_triple_product() {
        let sql = pivot_sql("prices", "price_ohlcv_1m", 5, 3, "");
        assert!(
            sql.contains("CAST(po.refusd * toFloat64(po.close) * toFloat64(multiIf("),
            "close_usd must be close × reference vwap × the measured rate: {sql}"
        );
        assert!(sql.contains(") AS Decimal(38, 14)) AS close_usd"), "{sql}");
        // The pre-0228 pair product must be gone entirely — a leftover would mean
        // one of the two USD columns still assumes USDC = $1.
        assert!(
            !sql.contains("CAST(r.usd * toFloat64(p.close) AS Decimal(38, 14))"),
            "the unscaled pair product is the defect: {sql}"
        );
    }

    /// `volume_quote_usd` stays WRITE-ONCE (BRIEF §5) — an oracle-set, depeg-aware
    /// figure is never clobbered — but its else-branch is scaled by the SAME rate,
    /// or the row carries two USD figures derived from different rates: the
    /// incoherent row `reset_sql`'s doc block rejects.
    #[test]
    fn pivot_sql_keeps_volume_quote_usd_write_once_and_scales_its_else_branch() {
        let sql = pivot_sql("prices", "price_ohlcv_1m", 5, 3, "");
        assert!(
            sql.contains("if(po.volume_quote_usd > 0, po.volume_quote_usd,"),
            "{sql}"
        );
        assert!(
            sql.contains("CAST(po.refusd * toFloat64(po.volume_quote) * toFloat64(multiIf("),
            "the else-branch takes the same rate as close_usd: {sql}"
        );
        // Both USD columns carry the rate the same number of times.
        assert_eq!(
            sql.matches("toFloat64(multiIf(").count(),
            2,
            "exactly the two USD columns are scaled: {sql}"
        );
    }

    /// ⚠️ `method` is part of `usd_rate`'s SORTING KEY, so an `oracle` row and an
    /// `external` row at the same (identity, timestamp) coexist. `argMax` across
    /// methods would let part read order decide which prices the candle. Two legs,
    /// each a single `method` equality — the same rule `external_sql` is pinned to.
    #[test]
    fn pivot_sql_reads_usd_rate_final_with_two_explicit_methods_never_argmax() {
        let sql = pivot_sql("prices", "price_ohlcv_1d", 5, 3, "");
        assert_eq!(
            sql.matches("usd_rate FINAL").count(),
            2,
            "one rate leg per method, each reading FINAL: {sql}"
        );
        assert_eq!(sql.matches("method = 'oracle'").count(), 1, "{sql}");
        assert_eq!(sql.matches("method = 'external'").count(), 1, "{sql}");
        assert!(
            !sql.contains("method IN ("),
            "an IN would let two rows share an anchor instant: {sql}"
        );
        assert!(
            !sql.contains("argMax"),
            "argMax across methods lets part read order pick the winner: {sql}"
        );
    }

    /// All four conjuncts of the identity tuple, on BOTH rate legs. A missing
    /// `contract_address = ''` silently matches a Soroban USDC row — a different
    /// asset with the same code and issuer — and prices the whole deep history
    /// off it.
    #[test]
    fn pivot_sql_pins_the_full_usdc_identity_tuple_on_both_rate_legs() {
        let sql = pivot_sql("prices", "price_ohlcv_1d", 5, 3, "");
        for conjunct in [
            "asset_kind = 'credit'",
            "asset_code = 'USDC'",
            &format!("issuer_address = '{USDC_ISSUER}'"),
            "contract_address = ''",
        ] {
            assert_eq!(
                sql.matches(conjunct).count(),
                2,
                "{conjunct} must appear on both rate legs: {sql}"
            );
        }
    }

    /// ⚠️ `join_use_nulls = 0` on prod: an unmatched ASOF yields the column
    /// DEFAULT, which for `Decimal(38, 14)` is 0 and not NULL. A null test on a
    /// rate would never fire. The legs are therefore tested `> 0`.
    ///
    /// The ONE `IS NOT NULL` left is on the REFERENCE leg, and it is legal only
    /// there: that subquery's `nullIf(sum(...), 0)` makes its column Nullable.
    #[test]
    fn pivot_sql_tests_both_rate_legs_positively_never_for_nullity() {
        let sql = pivot_sql("prices", "price_ohlcv_1d", 5, 3, "");
        assert!(sql.contains("po.orate > 0"), "{sql}");
        assert!(sql.contains("re.erate > 0"), "{sql}");
        // Each leg's own subquery also filters the series positively.
        assert_eq!(sql.matches("usd_rate > 0").count(), 2, "{sql}");
        assert!(!sql.contains("orate IS"), "{sql}");
        assert!(!sql.contains("erate IS"), "{sql}");
        assert_eq!(
            sql.matches("IS NOT NULL").count(),
            1,
            "only the Nullable reference leg may be null-tested: {sql}"
        );
        assert!(sql.contains("r.usd IS NOT NULL"), "{sql}");
        assert!(!sql.contains("IS NULL"), "{sql}");
    }

    /// 🔑 The rate legs resolve at the bucket's END, on EVERY grain. `timestamp`
    /// is the bucket's start but `close` is the period's last close, so a weekly
    /// candle resolved at its start takes the rate from the day the week opened.
    ///
    /// Iterates [`GRAINS`] rather than a list of its own, so a grain added to
    /// `bucket_end_expr` and forgotten here cannot pass.
    #[test]
    fn pivot_sql_resolves_both_rate_legs_at_the_bucket_end_on_every_grain() {
        for t in GRAINS {
            let sql = pivot_sql("prices", t, 5, 3, "");
            let projected = format!("{} AS bend", bucket_end_expr(t, "timestamp"));
            assert!(
                sql.contains(&projected),
                "{t}: the candidate subquery must project its own bucket end: {sql}"
            );
            // Strict inequality on both legs, against the materialized column.
            assert!(sql.contains("ON pr.k = ro.ok AND ro.orts < pr.bend"), "{t}");
            assert!(sql.contains("ON po.k = re.ek AND re.erts < po.bend"), "{t}");
        }
    }

    /// The rate staleness is the INLINED `external_window_s` literal on both legs
    /// — derived from the table, so no operator can set it short enough to make a
    /// reset destructive (`external_window_s`'s own argument). The REFERENCE leg
    /// keeps its configurable `pivot_window_s` bind, which is why
    /// `coarse-repair` still needs the explicit minimum-width refusal for it.
    #[test]
    fn pivot_sql_inlines_the_derived_rate_staleness_and_binds_only_the_reference() {
        for t in GRAINS {
            let sql = pivot_sql("prices", t, 5, 3, "");
            let stale = external_window_s(t);
            // The rate expression is rendered three times — the two USD columns
            // and the final filter — so each leg's bound appears three times.
            for leg in ["po.orts", "re.erts"] {
                assert_eq!(
                    sql.matches(&format!("toUInt32({leg})) <= {stale}")).count(),
                    3,
                    "{t}: {leg} must inline the derived {stale} s bound: {sql}"
                );
            }
            assert!(
                sql.contains("(p.timestamp - r.timestamp) <= ?"),
                "{t}: the reference leg keeps its bound pivot_window_s: {sql}"
            );
        }
    }

    /// The two rate joins are NESTED, not chained at one level, and their right
    /// sides carry DISTINCT column names — `peg_series_sql`'s rule
    /// (`queries_ch.rs`: a nested subquery needs nothing from the multi-JOIN
    /// rewrite and reads the same under both analyzers).
    #[test]
    fn pivot_sql_nests_the_two_rate_legs_with_distinct_column_names() {
        let sql = pivot_sql("prices", "price_ohlcv_1d", 5, 3, "");
        // The oracle leg is joined INSIDE the subquery the external leg reads.
        // ⚠️ The trailing token matters: `") AS re"` alone also matches the
        // reference subquery's `") AS ref_asset_id"`.
        let ro = sql.find(") AS ro ON").unwrap();
        let po = sql.find(") AS po ASOF").unwrap();
        let re = sql.find(") AS re ON").unwrap();
        assert!(
            ro < po && po < re,
            "the oracle leg must close before the level it feeds: {sql}"
        );
        // Distinct column names on the two right sides: neither leg's subquery
        // mentions the other's. A shared name is what the nesting exists to avoid.
        let oracle_leg = &sql[sql.find("SELECT 1 AS ok").unwrap()..ro];
        let external_leg = &sql[sql.find("SELECT 1 AS ek").unwrap()..re];
        assert!(oracle_leg.contains("AS orts") && oracle_leg.contains("AS orate"));
        assert!(!oracle_leg.contains("erts") && !oracle_leg.contains("erate"));
        assert!(external_leg.contains("AS erts") && external_leg.contains("AS erate"));
        assert!(!external_leg.contains("orts") && !external_leg.contains("orate"));
    }

    /// 🔑 A bucket with NEITHER rate is LEFT UNPRICED, never written as
    /// `0 × close`. The `multiIf` else-branch is zero and the final filter drops
    /// it, so the row stays a candidate for a later pass instead of becoming an
    /// ambiguous stored zero — the module doc's "no reference, never a wrong
    /// non-NULL value" rule, and the premise the 0228 campaign's reset rests on.
    #[test]
    fn pivot_sql_leaves_a_bucket_with_no_usable_rate_unpriced() {
        let sql = pivot_sql("prices", "price_ohlcv_1d", 5, 3, "");
        assert!(
            sql.contains("toDecimal128(0, 14)"),
            "the else-branch must be zero, not a $1 fallback: {sql}"
        );
        assert!(
            !sql.contains("toDecimal128(1, 14)"),
            "a $1 else-branch would re-introduce the peg this task removes: {sql}"
        );
        let tail = &sql[sql.rfind("WHERE ").unwrap()..];
        assert!(
            tail.contains("multiIf(") && tail.contains(") > 0"),
            "the selected rate must be filtered positively: {tail}"
        );
    }

    /// A versioned INSERT, never a mutation, so a FREEZE stays a rollback point —
    /// and the projection stays positionally aligned with [`INSERT_COLUMNS`].
    #[test]
    fn pivot_sql_is_a_versioned_insert_aligned_with_the_insert_columns() {
        let sql = pivot_sql("prices", "price_ohlcv_1d", 5, 3, "");
        assert!(
            sql.starts_with("INSERT INTO prices.price_ohlcv_1d"),
            "{sql}"
        );
        assert!(sql.contains(INSERT_COLUMNS), "{sql}");
        assert!(sql.contains("po.version + 1 AS version"), "{sql}");
        assert!(!sql.contains("ALTER TABLE"), "{sql}");
        // The two USD columns keep their position between volume_quote and vwap.
        let vqu = sql.find("AS volume_quote_usd").unwrap();
        let cu = sql.find("AS close_usd").unwrap();
        let vwap = sql.find("po.vwap").unwrap();
        assert!(vqu < cu && cu < vwap, "projection order: {sql}");
    }

    /// Bind order, as documented on the fn: candidate watermark (inside the
    /// candidate subquery, rendered FIRST), reference-subquery watermark, pivot
    /// staleness, batch size. Positional binds — a reordering binds the batch size
    /// as a timestamp and fails at RUN time, on prod.
    #[test]
    fn pivot_sql_bind_order_is_watermark_watermark_window_then_limit() {
        let sql = pivot_sql("prices", "price_ohlcv_1m", 5, 3, "");
        // Both watermarks render the same fragment; the candidate subquery is
        // FIRST in the text, the inline reference's is last.
        let cand_wm = sql.find("AND timestamp <= toDateTime(?)").unwrap();
        let ref_wm = sql.rfind("AND timestamp <= toDateTime(?)").unwrap();
        let win = sql.find("(p.timestamp - r.timestamp) <= ?").unwrap();
        let lim = sql.find("LIMIT ?").unwrap();
        assert!(
            cand_wm < ref_wm && ref_wm < win && win < lim,
            "bind order: candidate watermark, reference watermark, window, limit: {sql}"
        );
        assert_eq!(
            sql.matches("toDateTime(?)").count(),
            2,
            "exactly two watermark binds: {sql}"
        );
        assert_eq!(sql.matches('?').count(), 4, "{sql}");
    }

    #[test]
    fn peg_sql_threads_the_partition_window_into_the_outer_where() {
        // Unbounded (window = "") is byte-identical to the pre-0114 statement:
        // no extra timestamp predicate.
        let unbounded = peg_sql("prices", "price_ohlcv_1h", &[3], "").unwrap();
        assert!(!unbounded.contains(">= toDateTime("));

        // A window fragment is inlined verbatim after the outer watermark bound,
        // so ClickHouse can prune to the month's partition.
        let win = " AND p.timestamp >= toDateTime(100) AND p.timestamp < toDateTime(200)";
        let bounded = peg_sql("prices", "price_ohlcv_1h", &[3], win).unwrap();
        assert!(bounded.contains(win));
        assert!(
            bounded.find("p.timestamp <= toDateTime(?)").unwrap()
                < bounded.find("p.timestamp >= toDateTime(100)").unwrap(),
            "window predicate follows the watermark bound"
        );
        // The window must NOT precede LIMIT's bind position in a way that reorders
        // params — it is inlined (no `?`), so the single `?`s stay watermark, limit.
        assert_eq!(
            bounded.matches('?').count(),
            2,
            "window adds no bind params"
        );
    }

    /// The 0111 partition bound goes on the CANDIDATE side only — and since task
    /// 0228 that side is a subquery with no alias in scope, so the fragment names
    /// the BARE column (`external_sql`'s form) where the peg statement's stays
    /// `p.`-qualified.
    ///
    /// The occurrence count now proves three things, not one: neither the inline
    /// reference subquery nor EITHER rate leg carries the partition bound. All
    /// three must stay unbounded so a month's first buckets can still ASOF back to
    /// an anchor in an earlier partition.
    #[test]
    fn pivot_sql_bounds_only_the_candidate_side_not_the_reference() {
        let win = " AND timestamp >= toDateTime(100) AND timestamp < toDateTime(200)";
        let sql = pivot_sql("prices", "price_ohlcv_1h", 5, 3, win);
        // The candidate subquery is bounded …
        assert!(sql.contains(win), "{sql}");
        // … and nothing else is.
        assert_eq!(
            sql.matches("toDateTime(100)").count(),
            1,
            "the partition lower bound appears once — on the candidate side only: {sql}"
        );
        let unbounded = pivot_sql("prices", "price_ohlcv_1h", 5, 3, "");
        assert!(!unbounded.contains(">= toDateTime("), "{unbounded}");
        // Inlining adds no bind params: still watermark, watermark, window, limit.
        assert_eq!(sql.matches('?').count(), 4, "window adds no bind params");
    }

    /// Task 0215 regression: a peg-pivot step must issue **one peg and TWO
    /// pivots** — XLM and USDT — so a pivot set that silently narrows to one
    /// fails here rather than on production's quote legs.
    ///
    /// The defect this guards was diagnosed only by reading emitted SQL out of
    /// `system.query_log` after 26 days, because nothing in the suite asserted
    /// how many statements a step sends. The end-to-end integration tests do
    /// cover it, but they are `#[ignore]` and need a live ClickHouse, so they
    /// never run in CI (task 0275) — this is the guard that actually runs.
    #[test]
    fn plan_issues_one_peg_and_two_pivots() {
        let db = "prices";
        let tbl = "price_ohlcv_1m";
        let (peg_window, pivot_window) = ("", "");
        let refs = ReferenceIds {
            xlm: Some(5),
            usdc: Some(3),
            usdt: Some(7),
        };

        let plan = plan_peg_pivot_step(db, tbl, &refs, peg_window, pivot_window);

        let pivot_refs: Vec<u32> = plan
            .iter()
            .filter_map(|s| match s {
                StepStatement::Pivot { ref_id, .. } => Some(*ref_id),
                StepStatement::Peg { .. } => None,
            })
            .collect();
        let pegs = plan
            .iter()
            .filter(|s| matches!(s, StepStatement::Peg { .. }))
            .count();

        assert_eq!(pegs, 1, "exactly one peg statement per step");
        assert_eq!(
            pivot_refs,
            vec![5, 7],
            "both references must pivot, XLM then USDT — a narrowed set is the 0215 defect"
        );
        assert_eq!(plan.len(), 3, "one peg + two pivots");

        // The reference id is baked into each pivot's SQL as a literal, which is
        // what made the defect readable in `system.query_log` at all. Assert on
        // the emitted text so a plan that reports the right ids while building
        // the wrong statement cannot pass.
        for (stmt, expected) in plan
            .iter()
            .filter(|s| matches!(s, StepStatement::Pivot { .. }))
            .zip([5u32, 7])
        {
            let StepStatement::Pivot { sql, .. } = stmt else {
                unreachable!("filtered to pivots")
            };
            assert!(
                sql.contains(&format!("CAST({expected} AS UInt32) AS ref_asset_id")),
                "pivot SQL must carry ref {expected} as a literal"
            );
        }

        // No USDC market means nothing to peg (task 0172 made USDC the ONLY peg
        // member, so `stable_ids()` is empty and `peg_sql` returns `None`) AND
        // nothing to measure a pivot against. The step plans nothing at all —
        // it must never degrade to a silent single pivot.
        let no_usdc = ReferenceIds {
            xlm: Some(5),
            usdc: None,
            usdt: Some(7),
        };
        assert!(
            plan_peg_pivot_step(db, tbl, &no_usdc, peg_window, pivot_window).is_empty(),
            "no USDC market → no peg and no pivot"
        );
    }

    /// 🔑 Task 0228's alias split, as a test. The peg statement scans the table
    /// directly, so its partition bound must be `p.`-qualified; the pivot's
    /// candidate scan is a subquery with no alias in scope, so its bound must name
    /// the bare column. Handing ONE window string to both — which is what
    /// `enrich_peg_pivot_step` did before 0228 — puts a `p.` prefix where no `p`
    /// exists, or drops one where the alias is required.
    #[test]
    fn the_peg_and_pivot_windows_are_qualified_differently() {
        let refs = ReferenceIds {
            xlm: Some(5),
            usdc: Some(3),
            usdt: None,
        };
        let peg_window = " AND p.timestamp >= toDateTime(100) AND p.timestamp < toDateTime(200)";
        let pivot_window = " AND timestamp >= toDateTime(100) AND timestamp < toDateTime(200)";
        for stmt in plan_peg_pivot_step("prices", "price_ohlcv_1h", &refs, peg_window, pivot_window)
        {
            match stmt {
                StepStatement::Peg { sql } => {
                    assert!(sql.contains(peg_window), "peg keeps the alias: {sql}");
                }
                StepStatement::Pivot { sql, .. } => {
                    assert!(sql.contains(pivot_window), "pivot goes bare: {sql}");
                    assert!(
                        !sql.contains("AND p.timestamp >= toDateTime(100)"),
                        "a p.-qualified bound has no `p` in the candidate subquery: {sql}"
                    );
                }
            }
        }
    }

    // ---- task 0268: the external tier -------------------------------------

    /// D-07: the rate resolves at the bucket's END, and the calendar grains need
    /// calendar functions. A `_1M` bucket is not 31 fixed days — a fixed-seconds
    /// end lands in the wrong month for eleven months of twelve, and every one of
    /// those buckets then resolves against the wrong day's rate. Nothing errors.
    ///
    /// Every grain is listed, including the two the first version of this test
    /// left out — `_1m` and `_15m` were floored to `+ 3600` and nothing noticed
    /// (review CR-01). The calendar functions carry `'UTC'` (review WR-07).
    #[test]
    fn bucket_end_expr_uses_calendar_functions_for_calendar_grains() {
        assert_eq!(
            bucket_end_expr("price_ohlcv_1m", "timestamp"),
            "timestamp + 60"
        );
        assert_eq!(
            bucket_end_expr("price_ohlcv_15m", "timestamp"),
            "timestamp + 900"
        );
        assert_eq!(
            bucket_end_expr("price_ohlcv_1h", "timestamp"),
            "timestamp + 3600"
        );
        assert_eq!(
            bucket_end_expr("price_ohlcv_4h", "timestamp"),
            "timestamp + 14400"
        );
        assert_eq!(
            bucket_end_expr("price_ohlcv_1d", "timestamp"),
            "addDays(timestamp, 1, 'UTC')"
        );
        assert_eq!(
            bucket_end_expr("price_ohlcv_1w", "timestamp"),
            "addWeeks(timestamp, 1, 'UTC')"
        );
        assert_eq!(
            bucket_end_expr("price_ohlcv_1M", "timestamp"),
            "addMonths(timestamp, 1, 'UTC')"
        );
        // Only a table this module does not know falls back to an hour.
        assert_eq!(
            bucket_end_expr("not_a_table", "timestamp"),
            "timestamp + 3600"
        );
    }

    /// 🔑 CR-01's regression guard, and the guard for the next grain added.
    /// `bend` is the ASOF's UPPER BOUND, not a staleness allowance, so for every
    /// known fixed grain the end must be exactly `start + bucket_width_s` — a
    /// floor there changes WHICH rate is selected, and at a UTC day boundary it
    /// picks the next day's. The calendar grains are pinned to their calendar
    /// function with the zone spelled out. Nothing else is acceptable.
    ///
    /// Iterates [`GRAINS`], not a list of its own (review IN-09): every grain
    /// is either a fixed-width one whose end is `start + width`, or a calendar
    /// one pinned to its function — and there is no third kind, so a grain
    /// missing from either arm fails here.
    #[test]
    fn bucket_end_and_width_agree_for_every_known_grain() {
        let calendar = [
            ("price_ohlcv_1d", "addDays"),
            ("price_ohlcv_1w", "addWeeks"),
            ("price_ohlcv_1M", "addMonths"),
        ];
        for t in GRAINS {
            let w = bucket_width_s(t);
            assert!(w > 0, "{t}: a known grain has a width");
            match calendar.iter().find(|(c, _)| *c == t) {
                Some((_, f)) => {
                    assert!(w >= 86_400, "{t}: a calendar grain");
                    assert_eq!(bucket_end_expr(t, "ts"), format!("{f}(ts, 1, 'UTC')"));
                }
                None => assert_eq!(
                    bucket_end_expr(t, "ts"),
                    format!("ts + {w}"),
                    "{t}: the bucket end must be exactly one bucket width after the start"
                ),
            }
        }
        // And the fallback is reserved for tables this module does not know.
        assert_eq!(bucket_width_s("not_a_table"), 0);
        assert_eq!(bucket_end_expr("not_a_table", "ts"), "ts + 3600");
    }

    /// 🔑 Review IN-08: the reset↔refill pairing holds with ZERO slack. The
    /// day-set predicate demands only that a rate exist on the bucket's FIRST
    /// UTC day, so the worst-case anchor a reset can rely on is stamped at that
    /// day's start; the refill then measures `bend − rts`, which for a bucket
    /// narrower than a day (its last bucket of the day) is one day, and for a
    /// wider one is the bucket's own width. `external_window_s` must admit
    /// exactly that on every grain, or "reset zeroed it" turns into "no tier
    /// can refill it" — the 0182 class. Today the inequality is an equality on
    /// every grain; any future narrowing of the bound fails here.
    #[test]
    fn the_staleness_bound_admits_the_worst_case_anchor_on_every_grain() {
        for t in GRAINS {
            let width = bucket_width_s(t);
            let worst_case_staleness = width.max(86_400);
            assert!(
                worst_case_staleness <= external_window_s(t),
                "{t}: an anchor on the bucket's first UTC day is {worst_case_staleness} s \
                 before the bucket end, but the tier only accepts {} s",
                external_window_s(t)
            );
        }
        // The `_1M` width is the LONGEST month, so a 31-day bucket's day-one
        // anchor is exactly at the bound, not inside it.
        assert_eq!(bucket_width_s("price_ohlcv_1M"), 31 * 86_400);
        assert_eq!(external_window_s("price_ohlcv_1M"), 31 * 86_400);
    }

    /// WR-07: no timezone-dependent function in `external_sql` or in the shared
    /// day-set predicate is left to the server's zone. `views.sql` already
    /// refuses that assumption; so do these two builders.
    ///
    /// Scope, stated so the claim stays true (review WR-10): this inspects the
    /// two 0268 statement builders and nothing else. `repair::months_with_zeros`
    /// still derives its per-month `[start, end)` windows with the server-zone
    /// `toStartOfMonth` / `addMonths` — the 0114 driver's shape, matched to the
    /// tables' `toYYYYMM` partition key, which is server-zone too. The reset and
    /// the refill share one such window inside a month pass, so they stay
    /// paired; the calendar-grain executions are covered by the `_1d` and
    /// `_1M` fixtures in `ch_enrich_it.rs`. Task file, Issues 10.
    #[test]
    fn every_timezone_sensitive_expression_pins_utc() {
        for t in ["price_ohlcv_1d", "price_ohlcv_1w", "price_ohlcv_1M"] {
            let sql = external_sql("prices", t, 3, "");
            for f in ["addDays(", "addWeeks(", "addMonths(", "toDate("] {
                for (i, _) in sql.match_indices(f) {
                    let tail = &sql[i..];
                    let close = tail.find(')').unwrap();
                    assert!(
                        tail[..close].ends_with("'UTC'"),
                        "{t}: {f} without an explicit 'UTC': {}",
                        &tail[..close + 1]
                    );
                }
            }
        }
        let pred = external_rate_day_pred("prices");
        assert_eq!(
            pred.matches("toDate(timestamp, 'UTC')").count(),
            2,
            "both sides of the day-set predicate name the zone: {pred}"
        );
        assert!(!pred.contains("toDate(timestamp)"), "{pred}");
    }

    /// The staleness bound is DERIVED from the table, never configured: one day
    /// (task 0267's series is daily) or the grain's own width, whichever is
    /// wider. A bound shorter than the bucket width drops the reference for every
    /// bucket whose anchor is the previous one — the failure `--pivot-window-s`
    /// needs an explicit refusal for precisely because it IS configurable.
    #[test]
    fn external_window_s_floors_at_one_day_and_widens_to_the_grain() {
        assert_eq!(external_window_s("price_ohlcv_1m"), 86_400);
        assert_eq!(external_window_s("price_ohlcv_1d"), 86_400);
        assert_eq!(external_window_s("price_ohlcv_1w"), 604_800);
        assert_eq!(external_window_s("price_ohlcv_1M"), 2_678_400);
        for t in [
            "price_ohlcv_1m",
            "price_ohlcv_15m",
            "price_ohlcv_1h",
            "price_ohlcv_4h",
            "price_ohlcv_1d",
            "price_ohlcv_1w",
            "price_ohlcv_1M",
        ] {
            assert!(
                external_window_s(t) >= bucket_width_s(t),
                "{t}: the staleness bound can never be shorter than the bucket width"
            );
        }
    }

    /// ⚠️ `method` is part of `usd_rate`'s SORTING KEY, so an `oracle` row at the
    /// same (identity, timestamp) coexists with the `external` one. `argMax`
    /// across methods would let part read order decide which wins. `FINAL` plus an
    /// explicit `method = 'external'` is the only correct read.
    #[test]
    fn external_sql_reads_usd_rate_final_with_an_explicit_method_never_argmax() {
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        assert!(sql.contains("usd_rate FINAL"), "{sql}");
        assert!(sql.contains("method = 'external'"), "{sql}");
        assert!(
            !sql.contains("argMax"),
            "argMax across methods lets part read order pick the winner: {sql}"
        );
    }

    /// All four conjuncts of the identity tuple. A missing `contract_address = ''`
    /// silently matches a Soroban USDC row — a different asset with the same code
    /// and issuer — and prices the whole deep history off it.
    #[test]
    fn external_sql_pins_the_full_usdc_identity_tuple() {
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        assert!(sql.contains("asset_kind = 'credit'"), "{sql}");
        assert!(sql.contains("asset_code = 'USDC'"), "{sql}");
        assert!(
            sql.contains(&format!("issuer_address = '{USDC_ISSUER}'")),
            "{sql}"
        );
        assert!(sql.contains("contract_address = ''"), "{sql}");
    }

    /// The peg tier's narrow candidate filter, NOT the oracle tier's `OR` form:
    /// the external tier fills only what the oracle tier left at zero, so an
    /// oracle-priced candle is never a candidate in the first place.
    #[test]
    fn external_sql_targets_only_what_the_oracle_tier_left() {
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        assert!(sql.contains("close_usd = 0"), "{sql}");
        assert!(
            !sql.contains("volume_quote_usd = 0 OR"),
            "the oracle tier's wider OR form would re-price oracle rows: {sql}"
        );
        assert!(sql.contains("volume_quote > 0"), "{sql}");
        assert!(sql.contains("quote_asset_id = 3"), "{sql}");
    }

    /// 🔑 THE "oracle wins" GUARD, by its 0268-review mechanism (WR-04): the
    /// candidate side is bounded above by the SAME constant the API's label arm
    /// keys on, so the tier can never write a row the wire would call `oracle`,
    /// and never a row an oracle could have priced. The bound sits on the
    /// candidate side with the watermark, not on the reference.
    #[test]
    fn external_sql_is_bounded_above_by_the_usdc_oracle_epoch() {
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        let bound = format!("timestamp < toDateTime({epoch})");
        assert_eq!(sql.matches(&bound).count(), 1, "{sql}");
        let inner = sql.find("FROM prices.price_ohlcv_1d FINAL").unwrap();
        let asof = sql.find("ASOF LEFT JOIN").unwrap();
        let at = sql.find(&bound).unwrap();
        assert!(
            inner < at && at < asof,
            "the epoch bound belongs to the candidate subquery: {sql}"
        );
        // The bound adds no bind param — it is the shared constant, inlined, so
        // the reset window, the label arm and this tier read ONE value.
        assert_eq!(sql.matches('?').count(), 3, "{sql}");
    }

    /// 🔑 Review WR-09: the 0268 reset mode refuses on `oracle_prices` — the
    /// table the oracle tier READS — below the epoch, for the quote asset, for
    /// the configured oracle, and with no `not_before` in sight: the external
    /// tier's reach is bounded by the epoch and the month window, never by the
    /// spec's lower bound, so the guard must not be either.
    #[test]
    fn the_pre_epoch_oracle_guard_reads_oracle_prices_below_the_epoch_unbounded_below() {
        let sql = pre_epoch_oracle_rows_sql("prices", 3);
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        assert!(
            sql.starts_with("SELECT count() FROM prices.oracle_prices"),
            "{sql}"
        );
        assert!(!sql.contains("usd_rate"), "the wrong table: {sql}");
        assert!(sql.contains("asset_id = 3"), "{sql}");
        assert!(
            sql.contains("oracle_name = ?"),
            "the configured oracle, bound: {sql}"
        );
        assert!(
            sql.contains(&format!("timestamp < toDateTime({epoch})")),
            "the epoch, inlined from the one constant: {sql}"
        );
        assert!(
            !sql.contains(">="),
            "no lower bound — the tier has none: {sql}"
        );
        assert_eq!(sql.matches('?').count(), 1, "{sql}");
    }

    /// WR-06: BOTH USD columns come from the one reference. A write-once
    /// `volume_quote_usd` here would leave a row enriched before `close_usd`
    /// existed (`volume_quote_usd = volume_quote × $1`, `close_usd = 0`) carrying
    /// two USD figures at different rates — the incoherent row `reset_sql`'s doc
    /// block rejects — and would hand it to the peg tier after the reset step has
    /// already run, so the operator's one-shot campaign ends with par rows.
    #[test]
    fn external_sql_recomputes_both_usd_columns_from_the_one_reference() {
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        assert!(
            sql.contains("CAST(r.usd * p.volume_quote AS Decimal(38, 14)) AS volume_quote_usd"),
            "{sql}"
        );
        assert!(
            sql.contains("CAST(r.usd * p.close AS Decimal(38, 14)) AS close_usd"),
            "{sql}"
        );
        assert!(
            !sql.contains("if(p.volume_quote_usd > 0"),
            "a write-once guard here is the half-priced-row bug: {sql}"
        );
        // The safety of the overwrite rests on the epoch bound; the two must
        // travel together, so this test asserts both.
        assert!(
            sql.contains(&format!(
                "timestamp < toDateTime({})",
                prices_clickhouse::USDC_ORACLE_EPOCH_S
            )),
            "recomputing volume_quote_usd is only safe below the oracle epoch: {sql}"
        );
    }

    /// The 0111 partition bound goes on the CANDIDATE side only. The `usd_rate`
    /// reference stays unbounded so a month's first buckets keep an anchor —
    /// `pivot_sql_bounds_only_the_candidate_side_not_the_reference`'s rule, and
    /// the same occurrence count is what proves it.
    #[test]
    fn external_sql_bounds_only_the_candidate_side_not_the_reference() {
        let win = " AND timestamp >= toDateTime(100) AND timestamp < toDateTime(200)";
        let sql = external_sql("prices", "price_ohlcv_1d", 3, win);
        assert!(sql.contains(win), "{sql}");
        assert_eq!(
            sql.matches("toDateTime(100)").count(),
            1,
            "the partition lower bound appears once — on the candidate side only: {sql}"
        );
        let unbounded = external_sql("prices", "price_ohlcv_1d", 3, "");
        assert!(!unbounded.contains(">= toDateTime("), "{unbounded}");
        assert_eq!(
            sql.matches('?').count(),
            3,
            "the window adds no bind params"
        );
    }

    /// ⚠️ `join_use_nulls = 0` on prod: an unmatched ASOF yields the column
    /// DEFAULT, which for `Decimal(38, 14)` is 0 and not NULL. `IS NULL` would
    /// therefore never fire, and every unmatched bucket would be written with
    /// `close_usd = 0 * close = 0` — re-zeroing rows the peg tier had priced.
    #[test]
    fn external_sql_tests_the_rate_positively_never_is_null() {
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        assert!(sql.contains("r.usd > 0"), "{sql}");
        assert!(
            !sql.contains("IS NULL") && !sql.contains("IS NOT NULL"),
            "under join_use_nulls = 0 a null test silently never fires: {sql}"
        );
    }

    /// A versioned INSERT, never a mutation, so a FREEZE stays a rollback point.
    #[test]
    fn external_sql_is_a_versioned_insert_not_a_mutation() {
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        assert!(
            sql.starts_with("INSERT INTO prices.price_ohlcv_1d"),
            "{sql}"
        );
        assert!(sql.contains("p.version + 1 AS version"), "{sql}");
        assert!(!sql.contains("ALTER TABLE"), "{sql}");
    }

    /// Bind order, as documented on the fn: candidate watermark (inside the inner
    /// subquery), staleness seconds, batch size. Positional binds — a reordering
    /// binds the batch size as a timestamp and fails at RUN time, on prod.
    #[test]
    fn external_sql_bind_order_is_watermark_then_staleness_then_limit() {
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        let wm = sql.find("timestamp <= toDateTime(?)").unwrap();
        let stale = sql.find(") <= ?").unwrap();
        let lim = sql.find("LIMIT ?").unwrap();
        assert!(
            wm < stale && stale < lim,
            "bind order: watermark, staleness, limit: {sql}"
        );
    }

    /// D-07 end to end: the ASOF resolves against the bucket's END, and for a
    /// daily grain that end is a calendar day later — not `timestamp` itself.
    #[test]
    fn external_sql_resolves_the_rate_at_the_bucket_end() {
        let sql = external_sql("prices", "price_ohlcv_1d", 3, "");
        assert!(
            sql.contains("addDays(timestamp, 1, 'UTC') AS bend"),
            "{sql}"
        );
        assert!(sql.contains("ON r.k = p.k AND r.rts < p.bend"), "{sql}");
        let monthly = external_sql("prices", "price_ohlcv_1M", 3, "");
        assert!(
            monthly.contains("addMonths(timestamp, 1, 'UTC') AS bend"),
            "{monthly}"
        );
        // CR-01: the scheduled Lambda's own table. `+ 3600` here is the bug.
        let minute = external_sql("prices", "price_ohlcv_1m", 3, "");
        assert!(minute.contains("timestamp + 60 AS bend"), "{minute}");
    }

    // ---- task 0268 review: WR-05, the empty window --------------------------

    /// An inverted or empty `[not_before, not_after)` is refused, not run.
    #[test]
    fn a_reset_window_that_can_match_nothing_is_refused() {
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S;
        let inverted = UsdResetSpec {
            not_before: epoch + 1,
            ..usdc_external_reset()
        };
        assert!(matches!(
            inverted.validate(),
            Err(ChEnrichError::ResetWindowEmpty { not_before, not_after, .. })
                if not_before == epoch + 1 && not_after == epoch
        ));
        // Equal bounds are an empty half-open window too.
        let empty = UsdResetSpec {
            not_before: epoch,
            ..usdc_external_reset()
        };
        assert!(matches!(
            empty.validate(),
            Err(ChEnrichError::ResetWindowEmpty { .. })
        ));
        // The realistic typo — a year, or the epoch pasted into the wrong flag
        // — is any value above the bound; clap already rejects a millisecond
        // timestamp as out of range for u32.
        let above = UsdResetSpec {
            not_before: u32::MAX,
            ..usdc_external_reset()
        };
        assert!(above.validate().is_err());
        // The message tells the operator where the default came from.
        let msg = inverted.validate().unwrap_err().to_string();
        assert!(msg.contains("USDC_ORACLE_EPOCH_S"), "{msg}");
        assert!(msg.contains(&epoch.to_string()), "{msg}");
    }

    /// A well-formed bounded window and every unbounded spec (the 0182 shape)
    /// pass, so the pre-0268 path is untouched by the new refusal.
    #[test]
    fn well_formed_and_unbounded_reset_windows_are_accepted() {
        usdc_external_reset().validate().unwrap();
        usdt_reset().validate().unwrap();
        let wide = UsdResetSpec {
            not_before: u32::MAX,
            not_after: None,
            ..usdt_reset()
        };
        wide.validate().unwrap();
    }

    // ---- task 0268 review: IN-06, the runbook's epoch ------------------------

    /// The runbook (`docs/runbooks/repair-coarse-usd-values.md`, Appendix B)
    /// hand-types the oracle epoch ONCE, as a client `param`, and every query
    /// reads `{epoch:UInt32}`. This pins that single literal to the constant,
    /// because a drifted literal there produces a precondition query over the
    /// wrong window that reports 0 — a green all-clear over the one assumption
    /// the label arm rests on.
    #[test]
    fn the_runbook_hand_types_the_oracle_epoch_once_and_it_is_the_constant() {
        const RUNBOOK: &str = include_str!("../../../docs/runbooks/repair-coarse-usd-values.md");
        let epoch = prices_clickhouse::USDC_ORACLE_EPOCH_S.to_string();
        assert_eq!(
            RUNBOOK.matches(&epoch).count(),
            1,
            "the epoch appears exactly once in the runbook, as `SET param_epoch`"
        );
        assert!(
            RUNBOOK.contains(&format!("SET param_epoch = {epoch}")),
            "the one occurrence is the client parameter"
        );
        // Every place that USED to carry the literal now reads the parameter.
        assert!(
            RUNBOOK.matches("toDateTime({epoch:UInt32})").count() >= 2,
            "the precondition and baseline queries read the parameter"
        );
        // And no other 2026-era ten-digit epoch sneaks in beside it.
        let stray = RUNBOOK
            .split(|c: char| !c.is_ascii_digit())
            .filter(|w| w.len() == 10 && w.starts_with("177") && *w != epoch)
            .count();
        assert_eq!(stray, 0, "a second hand-typed 2026 epoch in the runbook");
    }

    #[test]
    fn reference_ids_helpers() {
        let full = ReferenceIds {
            xlm: Some(5),
            usdc: Some(3),
            usdt: Some(7),
        };
        // Task 0172: USDC is the ONLY $1 peg. USDT is measured, not assumed.
        assert_eq!(full.stable_ids(), vec![3]);
        assert_eq!(full.pivot_ids(), vec![5, 7]);
        assert!(full.can_pivot());
        assert!(full.has_any());

        // No USDC market → nothing to measure against, so no pivot at all, and
        // USDT is NOT silently promoted back into the peg set as a fallback.
        let no_usdc = ReferenceIds {
            xlm: Some(5),
            usdc: None,
            usdt: Some(7),
        };
        assert!(!no_usdc.can_pivot());
        assert!(no_usdc.stable_ids().is_empty());
        assert!(!no_usdc.has_any());

        // USDT alone, with a USDC market, still pivots — it does not need XLM.
        let usdt_only = ReferenceIds {
            xlm: None,
            usdc: Some(3),
            usdt: Some(7),
        };
        assert_eq!(usdt_only.pivot_ids(), vec![7]);
        assert!(usdt_only.can_pivot());

        assert!(!ReferenceIds::default().has_any());
    }

    /// Task 0172 regression: the peg statement must never target USDT's
    /// `asset_id`. If this fails, USDT-quoted candles are being valued at $1
    /// again and every one of them is ~7.4x overstated.
    #[test]
    fn peg_sql_never_pegs_usdt() {
        let refs = ReferenceIds {
            xlm: Some(5),
            usdc: Some(3),
            usdt: Some(7),
        };
        let sql = peg_sql("prices", "price_ohlcv_1m", &refs.stable_ids(), "").unwrap();
        assert!(
            sql.contains("quote_asset_id IN (3)"),
            "peg set must be USDC alone, got: {sql}"
        );
    }

    /// Task 0172: USDT-quoted candles must be priced by the pivot, against the
    /// measured USDT/USDC market — not left at `close_usd = 0`, which this
    /// schema cannot distinguish from "genuinely zero" or "not yet enriched".
    #[test]
    fn pivot_sql_prices_usdt_quoted_candles_from_its_usdc_market() {
        let sql = pivot_sql("prices", "price_ohlcv_1m", 7, 3, "");
        assert!(sql.contains("CAST(7 AS UInt32) AS ref_asset_id"));
        assert!(sql.contains("WHERE asset_id = 7 AND quote_asset_id = 3"));
        assert!(sql.contains("r.ref_asset_id = p.quote_asset_id"));
        // Task 0228: the USDT leg is scaled by the measured USDC/USD rate too —
        // ONE statement fixes both pivot references, as decision A requires.
        assert!(
            sql.contains("CAST(po.refusd * toFloat64(po.close) * toFloat64(multiIf("),
            "{sql}"
        );
        assert_eq!(sql.matches("usd_rate FINAL").count(), 2, "{sql}");
    }
}
