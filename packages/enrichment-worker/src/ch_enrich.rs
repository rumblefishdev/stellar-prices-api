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
//! `close_usd` (and the still-missing `volume_quote_usd`) are filled in two
//! ordered tiers, run inside [`ChEnrichmentPass::run`]:
//!
//! 1. **Recent-window oracle tier** ([`ChEnrichmentPass::enrich_batch`]) — the
//!    `ASOF LEFT JOIN oracle_prices`. Sets the USD columns wherever a Reflector
//!    row exists for the candle's `quote_asset_id` within the staleness window.
//!    This is the depeg-aware tier and it wins where it applies.
//! 2. **Peg-pivot tier** ([`ChEnrichmentPass::enrich_peg_pivot_step`]) — the
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
        "USD reset refused: prices.oracle_prices still holds {rows} row(s) for \
         quote asset_id {quote_asset_id} under oracle '{oracle_name}'.\n\
         The oracle tier runs before the peg-pivot tier and wins where it applies, \
         so resetting now would re-apply the oracle's rate to every row this reset \
         zeroes — and label it method='oracle'.\n\
         Purge those rows first (task 0196), verify the count is 0, then re-run.\n\
         See lore/1-tasks/active/0182_BUG_close-usd-overstated-7x-on-usdt-quoted-candles.md."
    )]
    ResetBlockedByOracleRows {
        quote_asset_id: u32,
        oracle_name: String,
        rows: u64,
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
pub fn reset_pending_pred(spec: &UsdResetSpec) -> String {
    format!(
        "quote_asset_id = {q} AND timestamp >= toDateTime({nb}) \
         AND (close_usd > 0 OR volume_quote_usd > 0) AND volume_quote > 0",
        q = spec.quote_asset_id,
        nb = spec.not_before,
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
pub fn repair_target_pred(reset: Option<&UsdResetSpec>) -> String {
    match reset {
        None => CANDIDATE_PRED.to_string(),
        Some(spec) => format!("({CANDIDATE_PRED}) OR ({})", reset_pending_pred(spec)),
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
            pred = reset_pending_pred(spec),
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

    /// Refuse the reset while `oracle_prices` still holds rows for the quote leg
    /// being reset — task 0182's first ordering constraint, as a gate rather than
    /// a warning.
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
        let sql = format!(
            "SELECT count() FROM {db}.oracle_prices \
             WHERE asset_id = {q} AND oracle_name = ?",
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
        if !self.cfg.one_shot {
            return Err(ChEnrichError::ResetRequiresOneShot {
                quote_asset_id: spec.quote_asset_id,
            });
        }
        self.assert_reset_target_is_priceable(spec).await?;
        self.assert_reset_not_shadowed_by_oracle(spec).await?;

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
        let window = self.window_pred("p.timestamp");
        if let Some(sql) = peg_sql(
            &self.cfg.database,
            &self.cfg.table,
            &refs.stable_ids(),
            &window,
        ) {
            self.client
                .query(&sql)
                .bind(watermark)
                .bind(self.cfg.batch_size)
                .execute()
                .await?;
        }
        // One pivot pass per measured reference asset (XLM, then USDT — task
        // 0172). Order matters only for cost, not correctness: each pass fills
        // rows the previous ones left at `close_usd = 0`, and the two reference
        // assets match disjoint sets of candles (`r.ref_asset_id = p.quote_asset_id`).
        if let Some(usdc_id) = refs.usdc {
            for ref_id in refs.pivot_ids() {
                let sql = pivot_sql(
                    &self.cfg.database,
                    &self.cfg.table,
                    ref_id,
                    usdc_id,
                    &window,
                );
                self.client
                    .query(&sql)
                    .bind(watermark)
                    .bind(self.cfg.pivot_window_s)
                    .bind(watermark)
                    .bind(self.cfg.batch_size)
                    .execute()
                    .await?;
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
                // The leftovers have no USD reference at all — their quote is
                // neither USDC/USDT/XLM (nor oracle-priced). They stay
                // NULL/`no_reference`, never a wrong value.
                warn!(
                    remaining = after,
                    "peg-pivot tier made no progress — remaining candles have no USD reference (exotic quotes)"
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
            let refs = self.resolve_reference_ids().await?;
            if refs.has_any() {
                let (r, b) = self
                    .run_peg_pivot_tier(&refs, watermark, remaining, batches)
                    .await?;
                remaining = r;
                batches = b;
            } else {
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
           AND p.timestamp >= toDateTime({nb}) \
           AND (p.close_usd > 0 OR p.volume_quote_usd > 0) \
           AND p.volume_quote > 0 \
           AND p.timestamp <= toDateTime(?){window} \
         ORDER BY p.timestamp \
         LIMIT ?",
        q = spec.quote_asset_id,
        nb = spec.not_before,
    )
}

/// Pivot statement: candles quoted in `ref_id` get `close_usd = close × ref_usd`,
/// where `ref_usd` is forward-filled by an `ASOF LEFT JOIN` against the
/// volume-weighted `ref_id`/USDC series. `ref_id` is XLM, or — since task 0172 —
/// the depegged USDT, which is priced by measurement rather than assumed to be $1
/// (see [`ReferenceIds::pivot_ids`]). Computed **inline as a subquery** (no DDL, so the
/// writer needs no `CREATE TABLE` grant on the shared tenant; task 0083). The
/// subquery's `WHERE asset_id = ref AND quote_asset_id = usdc` is a sort-key prefix
/// on `price_ohlcv_1m`, so each batch re-aggregates only that single pair's slice,
/// not the whole table. `ref_asset_id` is the constant reference id so the ASOF join
/// keeps its required equality predicate (`r.ref_asset_id = p.quote_asset_id`) and
/// matches only candles quoted in that reference asset — which is what makes the
/// XLM and USDT passes disjoint and safe to run in sequence. Bound parameters, in SQL order: the snapshot
/// watermark (ref subquery), the pivot staleness window (seconds), the snapshot
/// watermark again (outer, shared with the rest of the pass — see
/// [`ChEnrichmentPass::watermark`]), and the `LIMIT`.
fn pivot_sql(db: &str, tbl: &str, ref_id: u32, usdc_id: u32, window: &str) -> String {
    format!(
        "INSERT INTO {db}.{tbl} ({INSERT_COLUMNS}) \
         SELECT \
             p.timestamp, p.asset_id, p.quote_asset_id, p.source, \
             p.open, p.high, p.low, p.close, \
             p.volume_base, p.volume_quote, \
             if(p.volume_quote_usd > 0, p.volume_quote_usd, CAST(r.usd * toFloat64(p.volume_quote) AS Decimal(38, 14))) AS volume_quote_usd, \
             CAST(r.usd * toFloat64(p.close) AS Decimal(38, 14)) AS close_usd, \
             p.vwap, p.trade_count, \
             p.version + 1 AS version \
         FROM {db}.{tbl} AS p FINAL \
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
         WHERE p.quote_asset_id = {ref_id} \
           AND p.close_usd = 0 \
           AND p.volume_quote > 0 \
           AND r.usd IS NOT NULL \
           AND (p.timestamp - r.timestamp) <= ? \
           AND p.timestamp <= toDateTime(?){window} \
         ORDER BY p.timestamp \
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

    fn usdt_reset() -> UsdResetSpec {
        // 2021-02-07, the start of USDT's USDC market.
        UsdResetSpec {
            quote_asset_id: 111,
            not_before: 1_612_656_000,
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
        assert!(pivot_sql("prices", "price_ohlcv_1d", 111, 3, "").contains("p.volume_quote > 0"));
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
        let pred = reset_pending_pred(&usdt_reset());
        assert!(pred.contains("(close_usd > 0 OR volume_quote_usd > 0)"));
        assert!(pred.contains("quote_asset_id = 111"));
        assert!(pred.contains("volume_quote > 0"));
    }

    /// Without a spec the repair driver enumerates exactly what it always did —
    /// so the 0114 historical repair and the hourly sweep are unchanged.
    #[test]
    fn repair_target_pred_is_unchanged_without_a_reset() {
        assert_eq!(repair_target_pred(None), CANDIDATE_PRED);
    }

    /// The regression that made 0182 invisible: the driver enumerated
    /// `close_usd = 0`, every affected row had `close_usd > 0`, so a dry run
    /// reported "no months with enrichable zeros" over 44,657 wrong values.
    /// With a spec the enumeration must admit the written-value rows too.
    #[test]
    fn repair_target_pred_sees_months_that_hold_only_written_values() {
        let pred = repair_target_pred(Some(&usdt_reset()));
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
        // ASOF equality predicate + forward-fill inequality.
        assert!(sql.contains("r.ref_asset_id = p.quote_asset_id AND r.timestamp <= p.timestamp"));
        // Redundant-but-load-bearing: the ASOF `ON` already constrains
        // p.quote_asset_id to ref_id, but only as a join condition, so the outer
        // scan had no literal on the sort key's 2nd column
        // (asset_id, quote_asset_id, source, timestamp) and read the whole table
        // FINAL. Task 0172 added a second pivot pass, making this a 3-statement
        // tier in the pass task 0111 is open on. Keep the literal.
        assert!(sql.contains("WHERE p.quote_asset_id = 5"));
        assert!(sql.contains("CAST(r.usd * toFloat64(p.close) AS Decimal(38, 14)) AS close_usd"));
        // Bind order: subquery watermark, staleness window, outer watermark, LIMIT.
        let sub_wm = sql.find("toDateTime(?)").unwrap();
        let win = sql.find("(p.timestamp - r.timestamp) <= ?").unwrap();
        let outer_wm = sql.find("p.timestamp <= toDateTime(?)").unwrap();
        let lim = sql.find("LIMIT ?").unwrap();
        assert!(
            sub_wm < win && win < outer_wm && outer_wm < lim,
            "bind order: watermark, window, watermark, limit"
        );
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

    #[test]
    fn pivot_sql_bounds_only_the_candidate_side_not_the_reference() {
        let win = " AND p.timestamp >= toDateTime(100) AND p.timestamp < toDateTime(200)";
        let sql = pivot_sql("prices", "price_ohlcv_1h", 5, 3, win);
        // The candidate outer scan is bounded …
        assert!(sql.contains(win));
        // … but the inline XLM/USDC reference subquery is NOT: it must still
        // forward-fill an anchor from earlier months, so the month's first buckets
        // keep a valid pivot reference. The only window fragment present is the one
        // on `p.timestamp`; the subquery filters on a bare `timestamp`.
        assert_eq!(
            sql.matches("toDateTime(100)").count(),
            1,
            "the partition lower bound appears once — on the candidate side only"
        );
        // Inlining adds no bind params: still watermark, window, watermark, limit.
        assert_eq!(sql.matches('?').count(), 4, "window adds no bind params");
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
        assert!(sql.contains("CAST(r.usd * toFloat64(p.close) AS Decimal(38, 14)) AS close_usd"));
    }
}
