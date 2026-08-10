//! Transport-agnostic ClickHouse writer for `prices.*`.
//!
//! Holds a `clickhouse::Client` and knows how to write the shared row shapes
//! (`price_ohlcv_1m`, `assets`, `oracle_prices`) and load the asset registry.
//! It does **not** care how the client was built: a plaintext local-dev client
//! ([`OhlcvWriter::plaintext`]) and the task-0052 mTLS client (passed to
//! [`OhlcvWriter::new`]) are both just a `clickhouse::Client`, so the same
//! writer serves the local backfill and the live Lambda's remote mTLS sink.
//!
//! Backfill-only bookkeeping (`backfill_sdex_ledgers` resume set) is **not**
//! here — it lives in `sdex-backfill`'s thin wrapper, since the live Lambda
//! uses its own doorbell cursor instead.

use clickhouse::Client;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use tracing::info;

use crate::bucket::OhlcvCandle;
use crate::canonical::{AssetIdentity, AssetRegistry};
use crate::error::IngestError;
use crate::registry_io::PoolRegistryRow;
use crate::soroban::Registries;

/// Convert a `Decimal` to the `i128` mantissa ClickHouse expects for a
/// `Decimal(38, 14)` column. Saturates rather than panicking: AMM
/// amounts/prices are i128-derived and can exceed the 38-digit budget, and an
/// out-of-range value should clamp, not abort the whole run.
pub fn decimal_to_i128(d: Decimal) -> i128 {
    let d = d.round_dp(14);
    let factor = 10i128.pow(14 - d.scale());
    d.mantissa().saturating_mul(factor)
}

/// A ClickHouse writer over `prices.*`. Cheap to clone (the client is).
pub struct OhlcvWriter {
    client: Client,
}

impl OhlcvWriter {
    /// Wrap an already-built client (e.g. the mTLS client from
    /// `prices_clickhouse::mtls::client_from_lambda_env`).
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    /// Build a plaintext client for local-dev / Docker ClickHouse.
    pub fn plaintext(url: &str) -> Self {
        Self {
            client: Client::default().with_url(url),
        }
    }

    /// Borrow the underlying client (e.g. for backfill-only resume queries).
    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Cheap connectivity probe (`SELECT 1`).
    pub async fn preflight(&self) -> Result<(), IngestError> {
        self.client.query("SELECT 1").execute().await?;
        Ok(())
    }

    /// Load the existing `prices.assets` rows as `(asset_id, identity)` so a
    /// run reuses surrogate ids rather than reassigning them.
    pub async fn load_assets(&self) -> Result<Vec<(u32, AssetIdentity)>, IngestError> {
        let rows = self
            .client
            .query(
                "SELECT asset_id, asset_code, issuer_address, contract_address FROM prices.assets",
            )
            .fetch_all::<ExistingAssetRow>()
            .await?;

        let assets: Vec<(u32, AssetIdentity)> = rows
            .into_iter()
            .map(|r| {
                let identity = if !r.contract_address.is_empty() {
                    AssetIdentity::Contract(r.contract_address)
                } else if r.asset_code == "XLM" && r.issuer_address.is_empty() {
                    AssetIdentity::Native
                } else {
                    AssetIdentity::Credit {
                        code: r.asset_code,
                        issuer: r.issuer_address,
                    }
                };
                (r.asset_id, identity)
            })
            .collect();

        info!(
            existing_assets = assets.len(),
            "loaded asset registry from ClickHouse"
        );
        Ok(assets)
    }

    /// Rehydrate the discovered AMM pool [`Registries`] from `prices.pool_registry`
    /// so a consumer resolves pools created before its processing window instead of
    /// re-deriving from Soroban activation (task 0053 decision #4). Returns empty
    /// registries when the table has not been seeded. Shared by the SDEX backfill
    /// startup and the live ledger-processor cold start (task 0078) — a single
    /// query so the two paths can never drift.
    pub async fn load_pool_registry(&self) -> Result<Registries, IngestError> {
        let rows = self
            .client
            .query(
                "SELECT contract_id, venue, token0, token1, pool_type, wasm_hash \
                 FROM prices.pool_registry FINAL",
            )
            .fetch_all::<PoolRegistryRow>()
            .await?;
        let mut reg = Registries::new();
        reg.load_pool_rows(&rows);
        info!(
            entries = rows.len(),
            "loaded discovered pool registry from ClickHouse"
        );
        Ok(reg)
    }

    /// Persist the discovered AMM pool [`Registries`] to `prices.pool_registry`
    /// (task 0053 decision #4). The durable counterpart of [`load_pool_registry`]
    /// so read and write share one row shape and table name and can never drift.
    /// Idempotent: ReplacingMergeTree on `contract_id`, so a re-run replaces
    /// rather than duplicates. A no-op (no INSERT) when the registry is empty.
    ///
    /// Shared by the SDEX backfill's end-of-run persist and the periodic
    /// asset-discovery worker's pool-registry maintenance (task 0069).
    pub async fn write_pool_registry(&self, reg: &Registries) -> Result<(), IngestError> {
        let rows = reg.to_pool_rows();
        if rows.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert("prices.pool_registry")?;
        for row in &rows {
            insert.write(row).await?;
        }
        insert.end().await?;
        info!(pools = rows.len(), "persisted discovered pool registry");
        Ok(())
    }

    /// Write a batch of candles for one `source` into `prices.price_ohlcv_1m`.
    pub async fn write_candles(
        &self,
        candles: &[OhlcvCandle],
        source: &str,
    ) -> Result<(), IngestError> {
        if candles.is_empty() {
            return Ok(());
        }

        let mut insert = self.client.insert("prices.price_ohlcv_1m")?;

        for candle in candles {
            insert
                .write(&OhlcvRow {
                    timestamp: candle.minute_start,
                    asset_id: candle.asset_id,
                    quote_asset_id: candle.quote_asset_id,
                    source: source.to_string(),
                    open: decimal_to_i128(candle.open),
                    high: decimal_to_i128(candle.high),
                    low: decimal_to_i128(candle.low),
                    close: decimal_to_i128(candle.close),
                    volume_base: decimal_to_i128(candle.volume_base),
                    volume_quote: decimal_to_i128(candle.volume_quote),
                    // DEFAULT 0 — the 0026 enrichment Lambda fills this
                    // (volume_quote_usd = oracle_price * volume_quote).
                    volume_quote_usd: 0,
                    // DEFAULT 0 — the enrichment pass fills this (task 0061,
                    // close_usd = oracle_price * close), same as volume_quote_usd.
                    close_usd: 0,
                    vwap: decimal_to_i128(candle.vwap),
                    trade_count: candle.trade_count,
                    version: candle.version,
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }

    /// Write the **whole** asset registry into `prices.assets` (idempotent via
    /// ReplacingMergeTree on the asset sort key). Used by one-shot paths — the
    /// backfill's end-of-run persist and the discovery/oracle workers — where a
    /// single full write per run is cheap.
    ///
    /// The **live** ledger-processor must NOT use this: re-emitting all ~200k
    /// rows on every reconcile is a 9,413× write amplification billed as AWS→
    /// Hetzner egress (task 0132). It uses [`write_new_assets`] instead.
    ///
    /// [`write_new_assets`]: OhlcvWriter::write_new_assets
    pub async fn write_assets(&self, registry: &AssetRegistry) -> Result<(), IngestError> {
        self.write_asset_rows(registry, registry.assets()).await
    }

    /// Write only the assets interned on/after the `since` watermark — the
    /// caller's high-water mark of assets already durably in `prices.assets` (see
    /// [`AssetRegistry::watermark`]). A run that discovered no new assets writes
    /// **nothing**. This is the live processor's asset write path: it replaces the
    /// full-registry re-emit that caused ~$337/mo of redundant egress (task 0132).
    /// Correctness is unchanged — a new asset's row (including its deterministic
    /// `sac_address`) is complete when interned.
    pub async fn write_new_assets(
        &self,
        registry: &AssetRegistry,
        since: u32,
    ) -> Result<(), IngestError> {
        // O(1) steady-state fast path (the common case — no new assets). Ids are
        // handed out monotonically, so once `since` has caught up to the next id
        // no asset can have `id >= since`; skip without scanning the whole
        // ~200k-entry registry to conclude the set is empty.
        if since >= registry.watermark() {
            return Ok(());
        }
        self.write_asset_rows(registry, registry.assets_since(since))
            .await
    }

    /// Shared row-builder for [`write_assets`] / [`write_new_assets`]. Skips the
    /// INSERT entirely when `rows` is empty (matching every other writer here),
    /// so an idle reconcile makes no network round-trip. `registry` is borrowed
    /// for the deterministic `sac_address` derivation.
    ///
    /// [`write_assets`]: OhlcvWriter::write_assets
    /// [`write_new_assets`]: OhlcvWriter::write_new_assets
    async fn write_asset_rows<'a>(
        &self,
        registry: &AssetRegistry,
        rows: impl Iterator<Item = (&'a AssetIdentity, &'a u32)>,
    ) -> Result<(), IngestError> {
        let mut rows = rows.peekable();
        if rows.peek().is_none() {
            return Ok(());
        }

        let mut insert = self.client.insert("prices.assets")?;
        let mut written = 0u64;
        for (identity, &id) in rows {
            let (asset_code, asset_type, issuer_address, contract_address) = match identity {
                AssetIdentity::Native => {
                    ("XLM".to_string(), "classic", String::new(), String::new())
                }
                AssetIdentity::Credit { code, issuer } => {
                    (code.clone(), "classic", issuer.clone(), String::new())
                }
                AssetIdentity::Contract(addr) => {
                    (String::new(), "soroban", String::new(), addr.clone())
                }
            };
            // The SAC that wraps this classic asset (§12.4) — '' for a pure
            // Soroban token. Lets a read-time consumer resolve a SAC-wrapped leg.
            let sac_address = registry.sac_address_of(identity).unwrap_or_default();

            insert
                .write(&AssetRow {
                    asset_id: id,
                    asset_code,
                    asset_type: asset_type.to_string(),
                    issuer_address,
                    contract_address,
                    sac_address,
                    is_active: 1,
                })
                .await?;
            written += 1;
        }
        insert.end().await?;
        info!(assets = written, "wrote asset rows");
        Ok(())
    }

    /// Write asset enrichment (`home_domain`, …) into `prices.asset_metadata`
    /// (task 0067). This is the **single-writer** enrichment surface: identity
    /// columns live in `prices.assets` (written by the ledger processor — a delta
    /// of newly-interned assets per run via [`write_new_assets`], and in full by
    /// the one-shot backfill/discovery paths via [`write_assets`]), enrichment
    /// lives here (written only by the discovery/enrichment worker). Keeping them
    /// in separate tables stops any `prices.assets` re-emit from clobbering
    /// enrichment back to its default on a shared ReplacingMergeTree row.
    /// Idempotent (RMT on `asset_id`); a no-op when `entries` is empty.
    ///
    /// [`write_assets`]: OhlcvWriter::write_assets
    /// [`write_new_assets`]: OhlcvWriter::write_new_assets
    pub async fn write_asset_metadata(&self, entries: &[AssetMetadata]) -> Result<(), IngestError> {
        if entries.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert("prices.asset_metadata")?;
        for e in entries {
            insert
                .write(&AssetMetadataRow {
                    asset_id: e.asset_id,
                    home_domain: e.home_domain.clone(),
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }

    /// Write aggregated unresolved-pool observations into
    /// `prices.unresolved_pools` (see [`UnresolvedPool`]). Idempotent via
    /// `ReplacingMergeTree(version)` on `(contract_id, source)`.
    pub async fn write_unresolved_pools(
        &self,
        pools: &[UnresolvedPool],
    ) -> Result<(), IngestError> {
        if pools.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert("prices.unresolved_pools")?;
        for p in pools {
            insert
                .write(&UnresolvedPoolRow {
                    contract_id: p.contract_id.clone(),
                    source: p.source.clone(),
                    first_ledger: p.first_ledger,
                    last_ledger: p.last_ledger,
                    swap_count: p.swap_count,
                    sample_topics: p.sample_topics.clone(),
                    still_unresolved: p.still_unresolved,
                    // Latest observation wins the RMT collapse.
                    version: p.last_ledger as u64,
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }

    /// Write decoded oracle price samples into `prices.oracle_prices`.
    pub async fn write_oracle(&self, samples: &[OracleSample]) -> Result<(), IngestError> {
        if samples.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert("prices.oracle_prices")?;
        for s in samples {
            insert
                .write(&OracleRow {
                    timestamp: s.timestamp,
                    asset_id: s.asset_id,
                    oracle_name: s.oracle_name.clone(),
                    price_usd: s.price_usd,
                    raw_data: s.raw_data.clone(),
                })
                .await?;
        }
        insert.end().await?;
        Ok(())
    }

    /// Snapshot peg-asset USD rates from `oracle_prices` into `prices.usd_rate`
    /// (task 0167).
    ///
    /// **Why this copy exists at all.** `oracle_prices` is pruned at
    /// `INTERVAL 13 MONTH`, so the earliest depeg-aware readings age out
    /// permanently. A consumer cannot simply join `oracle_prices` instead: the
    /// published series would *mutate* as rows age out — a bucket reading
    /// `0.9993` silently reverting to a `$1` fallback later — which is why
    /// `views.sql` forbids that join. So the rate is snapshotted into a
    /// forever-retained table, the same way every other USD number is baked
    /// rather than joined.
    ///
    /// **Incremental by watermark, per identity.** Only observations newer than
    /// the newest already stored for that identity are copied, so a re-run is
    /// cheap. It is also idempotent: `usd_rate` is a `ReplacingMergeTree` keyed
    /// on `(natural identity, timestamp)`, and `version` is the write time, so a
    /// repeat writes identical content and a genuine later correction of the
    /// same timestamp wins.
    ///
    /// ⚠️ **The 0139 guard is load-bearing, not defensive decoration.**
    /// `oracle_prices` is keyed on `asset_id`, `usd_rate` is keyed on natural
    /// identity, so this is the one place the two key spaces meet — and task
    /// 0139 is confirmed as genuine `asset_id` collisions between unrelated
    /// assets (measured 2026-08-10: 3,281 ids serving 6,568 identities;
    /// `asset_id 4194` is both `STW` and `ARBRIDGE`). Translating without
    /// checking would file one asset's oracle readings under another asset's
    /// identity — silently, and in a table built to be trusted forever. If the
    /// mapping is not 1:1 in **both** directions we refuse the write and say so.
    pub async fn populate_usd_rate_from_oracle(
        &self,
        pegs: &[AssetIdentity],
    ) -> Result<UsdRateStats, IngestError> {
        let mut stats = UsdRateStats::default();

        for identity in pegs {
            let (kind, code, issuer, contract) = identity_columns(identity);

            // --- 0139 guard, both directions -----------------------------
            // Forward: this identity must resolve to exactly one asset_id.
            let ids: Vec<u32> = self
                .client
                .query(
                    "SELECT asset_id FROM prices.assets FINAL \
                     WHERE asset_code = ? AND issuer_address = ? AND contract_address = ?",
                )
                .bind(code)
                .bind(issuer)
                .bind(contract)
                .fetch_all::<u32>()
                .await?;
            let [asset_id] = ids[..] else {
                return Err(IngestError::Precondition(format!(
                    "peg identity {code} resolves to {} asset_ids (expected exactly 1); \
                     refusing to write usd_rate — see task 0139",
                    ids.len()
                )));
            };
            // Reverse: that asset_id must not be shared with another identity,
            // or its oracle rows are not unambiguously this asset's.
            let sharers: u64 = self
                .client
                .query("SELECT count() FROM prices.assets FINAL WHERE asset_id = ?")
                .bind(asset_id)
                .fetch_one::<u64>()
                .await?;
            if sharers != 1 {
                return Err(IngestError::Precondition(format!(
                    "peg identity {code} maps to asset_id {asset_id}, which serves {sharers} \
                     identities; refusing to write usd_rate — see task 0139"
                )));
            }

            // --- watermark ------------------------------------------------
            let before: u32 = self
                .client
                .query(
                    "SELECT toUInt32(ifNull(max(timestamp), toDateTime(0))) \
                     FROM prices.usd_rate \
                     WHERE asset_kind = ? AND asset_code = ? AND issuer_address = ? \
                       AND contract_address = ? AND method = 'oracle'",
                )
                .bind(kind)
                .bind(code)
                .bind(issuer)
                .bind(contract)
                .fetch_one::<u32>()
                .await?;

            // --- copy forward ---------------------------------------------
            // `price_usd > 0` drops unusable readings rather than storing a
            // zero that a consumer cannot distinguish from a real rate — the
            // close_usd = 0 mistake, which this table exists partly to avoid.
            self.client
                .query(
                    "INSERT INTO prices.usd_rate \
                       (asset_kind, asset_code, issuer_address, contract_address, \
                        timestamp, usd_rate, method, reference_asset, hops, version) \
                     SELECT ?, ?, ?, ?, o.timestamp, o.price_usd, 'oracle', '', 0, \
                            toUInt64(now()) \
                     FROM prices.oracle_prices AS o FINAL \
                     WHERE o.asset_id = ? AND o.price_usd > 0 AND o.timestamp > toDateTime(?)",
                )
                .bind(kind)
                .bind(code)
                .bind(issuer)
                .bind(contract)
                .bind(asset_id)
                .bind(before)
                .execute()
                .await?;

            let after: u32 = self
                .client
                .query(
                    "SELECT toUInt32(ifNull(max(timestamp), toDateTime(0))) \
                     FROM prices.usd_rate \
                     WHERE asset_kind = ? AND asset_code = ? AND issuer_address = ? \
                       AND contract_address = ? AND method = 'oracle'",
                )
                .bind(kind)
                .bind(code)
                .bind(issuer)
                .bind(contract)
                .fetch_one::<u32>()
                .await?;

            stats.identities += 1;
            // `min` over a Default-initialised 0 would pin this at 0 forever,
            // so track the first value explicitly.
            stats.watermark_before = Some(match stats.watermark_before {
                Some(w) => w.min(before),
                None => before,
            });
            stats.watermark_after = stats.watermark_after.max(after);
        }
        Ok(stats)
    }
}

/// Split an [`AssetIdentity`] into the four natural-identity columns
/// `usd_rate` (and the read-surface views) are keyed on. The `''` padding for
/// the inapplicable columns is the same convention `views.sql` documents, so a
/// rate row joins a series row without a translation step.
fn identity_columns(id: &AssetIdentity) -> (&str, &str, &str, &str) {
    match id {
        AssetIdentity::Native => ("native", "XLM", "", ""),
        AssetIdentity::Credit { code, issuer } => ("credit", code, issuer, ""),
        AssetIdentity::Contract(addr) => ("contract", "", "", addr),
    }
}

/// Outcome of a [`OhlcvWriter::populate_usd_rate_from_oracle`] pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct UsdRateStats {
    /// Peg identities processed (each one guarded and copied independently).
    pub identities: usize,
    /// Oldest per-identity watermark before the pass. `None` when no identity
    /// was processed — distinct from `Some(0)`, which means "processed, and
    /// nothing had been stored yet".
    pub watermark_before: Option<u32>,
    /// Newest per-identity watermark after it.
    pub watermark_after: u32,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct OhlcvRow {
    timestamp: u32,
    asset_id: u32,
    quote_asset_id: u32,
    source: String,
    open: i128,
    high: i128,
    low: i128,
    close: i128,
    volume_base: i128,
    volume_quote: i128,
    volume_quote_usd: i128,
    close_usd: i128,
    vwap: i128,
    trade_count: u32,
    version: u64,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct AssetRow {
    asset_id: u32,
    asset_code: String,
    asset_type: String,
    issuer_address: String,
    contract_address: String,
    sac_address: String,
    is_active: u8,
}

/// One asset-enrichment entry for [`OhlcvWriter::write_asset_metadata`] — the
/// single-writer counterpart to identity in `prices.assets` (task 0067).
#[derive(Debug, Clone)]
pub struct AssetMetadata {
    pub asset_id: u32,
    pub home_domain: String,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct AssetMetadataRow {
    asset_id: u32,
    home_domain: String,
}

#[derive(Debug, Deserialize, clickhouse::Row)]
struct ExistingAssetRow {
    asset_id: u32,
    asset_code: String,
    issuer_address: String,
    contract_address: String,
}

/// One aggregated unresolved-pool observation, ready for
/// `prices.unresolved_pools`. A caller aggregates the per-ledger
/// [`crate::soroban::UnresolvedPoolSwap`] records by contract and re-checks each
/// against the final registry to set `still_unresolved`.
#[derive(Debug, Clone)]
pub struct UnresolvedPool {
    pub contract_id: String,
    /// Which pipeline observed it: `"backfill"` or `"live"`.
    pub source: String,
    pub first_ledger: u32,
    pub last_ledger: u32,
    pub swap_count: u64,
    pub sample_topics: String,
    /// `1` = still absent from the registry at run-end (a genuine extractor gap
    /// to investigate); `0` = the pool registered later in the run, so only its
    /// early swaps were dropped (recoverable, informational).
    pub still_unresolved: u8,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct UnresolvedPoolRow {
    contract_id: String,
    source: String,
    first_ledger: u32,
    last_ledger: u32,
    swap_count: u64,
    sample_topics: String,
    still_unresolved: u8,
    version: u64,
}

/// One decoded oracle price sample, ready for `prices.oracle_prices`.
#[derive(Debug, Clone)]
pub struct OracleSample {
    pub timestamp: u32,
    pub asset_id: u32,
    pub oracle_name: String,
    /// price_usd scaled to 14 decimals (matches Decimal(38,14)).
    pub price_usd: i128,
    pub raw_data: String,
}

#[derive(Debug, Serialize, clickhouse::Row)]
struct OracleRow {
    timestamp: u32,
    asset_id: u32,
    oracle_name: String,
    price_usd: i128,
    raw_data: String,
}
