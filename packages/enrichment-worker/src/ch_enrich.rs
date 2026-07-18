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
//!    - **peg:** a USDC- or USDT-quoted candle gets `close_usd = close × $1`
//!      (USDC≡USDT≡$1), exact and oracle-free, back to SDEX genesis;
//!    - **pivot:** an XLM-quoted candle gets `close_usd = close × xlm_usd`, where
//!      `xlm_usd` is the volume-weighted XLM/USDC candle close (× $1) at or before
//!      the bucket, forward-filled by an `ASOF LEFT JOIN`.
//!
//! Candles whose quote is none of USDC/USDT/XLM (and had no oracle) keep
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

/// Canonical Stellar issuers for the pegged USD stablecoins, used by the
/// peg-pivot tier to recognise USDC/USDT quote assets in `prices.assets`.
/// Re-exported from `prices-clickhouse` (the single source of truth, also used by
/// `sdex-backfill`) so the `asset_id` the backfill interns under these issuers
/// matches the `quote_asset_id` enriched here — no hand-synced literal to drift.
use prices_clickhouse::{USDC_ISSUER, USDT_ISSUER};

#[derive(Debug, thiserror::Error)]
pub enum ChEnrichError {
    #[error("clickhouse: {0}")]
    Clickhouse(#[from] clickhouse::error::Error),
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
    /// Quote `asset_id`s that peg to exactly $1 (USDC, USDT).
    fn stable_ids(&self) -> Vec<u32> {
        [self.usdc, self.usdt].into_iter().flatten().collect()
    }

    /// The XLM→USD pivot needs both the XLM asset and the XLM/USDC market.
    fn can_pivot(&self) -> bool {
        self.xlm.is_some() && self.usdc.is_some()
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

    /// The newest candle `timestamp` (unix seconds) at pass start, used as a
    /// snapshot watermark. The pass only counts and enriches candidates at or
    /// before it, so candles the live Ledger Processor (task 0038) inserts *during*
    /// the pass — which carry newer ledger-close timestamps — are excluded from
    /// this pass's population. Without the bound, a concurrent insert can inflate
    /// the candidate count and falsely trip the `after >= remaining` no-progress
    /// break, stopping the pass with enrichable rows left. The newer candles are
    /// picked up by the next scheduled run. Returns 0 on an empty table.
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
             WHERE (volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0 \
               AND timestamp <= toDateTime(?)",
            db = self.cfg.database,
            tbl = self.cfg.table,
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
               AND timestamp <= toDateTime(?)",
            db = self.cfg.database,
            tbl = self.cfg.table,
        );
        Ok(self
            .client
            .query(&sql)
            .bind(self.cfg.recent_window_s)
            .bind(watermark)
            .fetch_one::<RemainingCounts>()
            .await?)
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
               AND p.timestamp <= toDateTime(?) \
             ORDER BY p.timestamp \
             LIMIT ?",
            db = self.cfg.database,
            tbl = self.cfg.table,
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

    /// One peg-pivot step: the peg statement (USDC/USDT quotes → ×$1) followed by
    /// the pivot statement (XLM quotes → ×XLM/USDC close). Both target only rows
    /// the oracle tier left at `close_usd = 0`, so the oracle value always wins
    /// where it exists. The peg runs whenever a stablecoin is in the registry; the
    /// pivot runs only when both the XLM asset and the XLM/USDC market are known
    /// ([`ReferenceIds::can_pivot`]) — it computes the XLM/USDC reference inline
    /// (no pre-materialized table; see [`pivot_sql`]).
    async fn enrich_peg_pivot_step(
        &self,
        refs: &ReferenceIds,
        watermark: u32,
    ) -> Result<(), ChEnrichError> {
        if let Some(sql) = peg_sql(&self.cfg.database, &self.cfg.table, &refs.stable_ids()) {
            self.client
                .query(&sql)
                .bind(watermark)
                .bind(self.cfg.batch_size)
                .execute()
                .await?;
        }
        if let (Some(xlm_id), Some(usdc_id)) = (refs.xlm, refs.usdc) {
            let sql = pivot_sql(&self.cfg.database, &self.cfg.table, xlm_id, usdc_id);
            self.client
                .query(&sql)
                .bind(watermark)
                .bind(self.cfg.pivot_window_s)
                .bind(watermark)
                .bind(self.cfg.batch_size)
                .execute()
                .await?;
        }
        Ok(())
    }

    /// Tier 2 — the peg-pivot deep-history backbone, over the fixed `watermark`
    /// snapshot. Each per-batch pivot computes the volume-weighted XLM/USDC
    /// reference **inline** ([`pivot_sql`]) rather than from a pre-materialized
    /// table, so it needs no `CREATE TABLE` grant on the shared tenant (task 0083).
    ///
    /// TRADE-OFF (reverses review #10's materialize-once): the ref is now
    /// re-aggregated per batch, so total ref work is O(slice × batches) instead of
    /// O(slice). The slice is a single-pair sort-key prefix, so it's cheap **while
    /// XLM/USDC history is small** — but it grows with backfill depth × the batch
    /// count (up to `max_batches`, unbounded in `one_shot`) and could risk the
    /// 300s Lambda timeout post-backfill. Restore materialize-once (in-memory ref
    /// or a session-scoped `CREATE TEMPORARY TABLE`) before the 0053 backfill runs
    /// — tracked in **0085**. Returns the updated `(remaining, batches)`.
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

        // Tier 2 — peg-pivot deep-history backbone (USDC/USDT≡$1; XLM via XLM/USDC).
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
fn peg_sql(db: &str, tbl: &str, stable_ids: &[u32]) -> Option<String> {
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
           AND p.timestamp <= toDateTime(?) \
         ORDER BY p.timestamp \
         LIMIT ?"
    ))
}

/// Pivot statement: XLM-quoted candles get `close_usd = close × xlm_usd`, where
/// `xlm_usd` is forward-filled by an `ASOF LEFT JOIN` against the volume-weighted
/// XLM/USDC reference series — computed **inline as a subquery** (no DDL, so the
/// writer needs no `CREATE TABLE` grant on the shared tenant; task 0083). The
/// subquery's `WHERE asset_id = xlm AND quote_asset_id = usdc` is a sort-key prefix
/// on `price_ohlcv_1m`, so each batch re-aggregates only that single pair's slice,
/// not the whole table. `ref_asset_id` is the constant XLM id so the ASOF join
/// keeps its required equality predicate (`r.ref_asset_id = p.quote_asset_id`) and
/// matches only XLM-quoted candles. Bound parameters, in SQL order: the snapshot
/// watermark (ref subquery), the pivot staleness window (seconds), the snapshot
/// watermark again (outer, shared with the rest of the pass — see
/// [`ChEnrichmentPass::watermark`]), and the `LIMIT`.
fn pivot_sql(db: &str, tbl: &str, xlm_id: u32, usdc_id: u32) -> String {
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
                 CAST({xlm_id} AS UInt32) AS ref_asset_id, \
                 timestamp, \
                 sum(toFloat64(close) * toFloat64(volume_base)) / nullIf(sum(toFloat64(volume_base)), 0) AS usd \
             FROM {db}.{tbl} FINAL \
             WHERE asset_id = {xlm_id} AND quote_asset_id = {usdc_id} \
               AND timestamp <= toDateTime(?) \
             GROUP BY timestamp \
             ORDER BY timestamp \
         ) AS r \
             ON r.ref_asset_id = p.quote_asset_id AND r.timestamp <= p.timestamp \
         WHERE p.close_usd = 0 \
           AND p.volume_quote > 0 \
           AND r.usd IS NOT NULL \
           AND (p.timestamp - r.timestamp) <= ? \
           AND p.timestamp <= toDateTime(?) \
         ORDER BY p.timestamp \
         LIMIT ?"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peg_sql_is_none_without_stablecoins() {
        assert!(peg_sql("prices", "price_ohlcv_1m", &[]).is_none());
    }

    #[test]
    fn peg_sql_fills_close_usd_for_stable_quotes() {
        let sql = peg_sql("prices", "price_ohlcv_1m", &[3, 7]).unwrap();
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
        let sql = pivot_sql("prices", "price_ohlcv_1m", 5, 3);
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
    fn reference_ids_helpers() {
        let full = ReferenceIds {
            xlm: Some(5),
            usdc: Some(3),
            usdt: Some(7),
        };
        assert_eq!(full.stable_ids(), vec![3, 7]);
        assert!(full.can_pivot());
        assert!(full.has_any());

        // XLM present but no USDC market → cannot pivot, but can still peg USDT.
        let no_usdc = ReferenceIds {
            xlm: Some(5),
            usdc: None,
            usdt: Some(7),
        };
        assert!(!no_usdc.can_pivot());
        assert_eq!(no_usdc.stable_ids(), vec![7]);
        assert!(no_usdc.has_any());

        assert!(!ReferenceIds::default().has_any());
    }
}
