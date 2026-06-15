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
//! This ASOF join is the **recent-window oracle tier**: it sets
//! `close_usd` only where a Reflector oracle row exists for the candle's
//! `quote_asset_id` within the staleness window. Deep history (no oracle
//! row) stays at `close_usd = 0` and is the job of the **peg-pivot tier**
//! (USDC≡$1 / USDT≡$1 × the XLM/USDC candle) — not yet implemented here.
//! Until it lands, deep-history candles carry `close_usd = 0` (the view's
//! `no_reference` status), never a wrong non-NULL value.
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
use tracing::{info, warn};

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
    pub batch_size: u64,
    pub max_batches: u32,
}

impl Default for ChEnrichConfig {
    fn default() -> Self {
        Self {
            url: "http://localhost:8123".to_string(),
            database: "prices".to_string(),
            table: "price_ohlcv_1m".to_string(),
            oracle_name: "reflector".to_string(),
            window_s: 300,
            batch_size: 10_000,
            max_batches: 20,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ChPassStats {
    pub batches: u32,
    pub candidates_before: u64,
    pub candidates_after: u64,
    pub rows_enriched: u64,
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

    /// Cold-start health check — fail Lambda Init, not per-event.
    pub async fn preflight(&self) -> Result<(), ChEnrichError> {
        self.client.query("SELECT 1").execute().await?;
        Ok(())
    }

    /// Count the still-unenriched, enrichable-shaped candidates. `FINAL`
    /// collapses pending versions so already-enriched rows (which carry
    /// `volume_quote_usd > 0` at `version + 1`) are excluded even before
    /// the background merge runs.
    async fn count_candidates(&self) -> Result<u64, ChEnrichError> {
        let sql = format!(
            "SELECT count() FROM {db}.{tbl} FINAL \
             WHERE (volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0",
            db = self.cfg.database,
            tbl = self.cfg.table,
        );
        Ok(self.client.query(&sql).fetch_one::<u64>().await?)
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
    async fn enrich_batch(&self) -> Result<(), ChEnrichError> {
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
                 CAST(o.price_usd * p.volume_quote AS Decimal(38, 14)) AS volume_quote_usd, \
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
             ORDER BY p.timestamp \
             LIMIT ?",
            db = self.cfg.database,
            tbl = self.cfg.table,
        );
        self.client
            .query(&sql)
            .bind(&self.cfg.oracle_name)
            .bind(self.cfg.window_s)
            .bind(self.cfg.batch_size)
            .execute()
            .await?;
        Ok(())
    }

    /// Run the bounded enrichment pass: up to `max_batches × batch_size`
    /// rows per invocation. Stops early when a batch makes no progress —
    /// i.e. every remaining candidate is a (currently) permanent oracle
    /// miss — or when no candidates remain.
    pub async fn run(&self) -> Result<ChPassStats, ChEnrichError> {
        let candidates_before = self.count_candidates().await?;
        info!(
            candidates = candidates_before,
            table = %self.cfg.table,
            "enrichment pass start"
        );

        let mut batches = 0u32;
        let mut remaining = candidates_before;
        for _ in 0..self.cfg.max_batches {
            if remaining == 0 {
                break;
            }
            self.enrich_batch().await?;
            let after = self.count_candidates().await?;
            batches += 1;

            if after >= remaining {
                // No row flipped out of the zero-set: the leftover
                // candidates have no in-window oracle price yet. Stop —
                // they roll over to a future pass once oracle data lands.
                warn!(
                    remaining = after,
                    "batch made no progress — remaining candidates are oracle misses"
                );
                remaining = after;
                break;
            }
            remaining = after;
        }

        let stats = ChPassStats {
            batches,
            candidates_before,
            candidates_after: remaining,
            rows_enriched: candidates_before.saturating_sub(remaining),
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
