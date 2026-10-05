//! Asset-id uniqueness, checked on the stored data (task 0139).
//!
//! `asset_id` is the surrogate key of an asset's natural identity
//! `(asset_code, issuer_address, contract_address)`. Under the backend counter
//! it was not unique: on 2026-09-30, 3,315 ids each served two or three
//! unrelated identities, and every table keyed on `asset_id` blended them.
//! Since 0139, ClickHouse derives the id from the identity, so two identities
//! can share an id only through rows written before the migration or through a
//! regression in the schema. This module reads two numbers:
//!
//! 1. `AssetIdCollisions` = `count() − uniqExact(asset_id)` over `assets FINAL`.
//!    `FINAL` keeps one row per identity (the table's sort key), so the
//!    difference is the number of identities that have no id of their own.
//!    Before the 0139 window this publishes the production baseline (3,321 on
//!    2026-09-30: 3,315 colliding ids, six of them carrying three identities)
//!    with no alarm. After the window it must read 0.
//! 2. `AssetIdOrphanCandles` = rows of `price_ohlcv_1m` in the last two hours
//!    whose `asset_id` or `quote_asset_id` has no `assets` row. This is the
//!    post-window detector for a writer out of step with the registry, e.g. an
//!    old binary still writing counter ids after the swap (BRIEF §3.7).
//!
//! The two reads are independent: a failure in one must not suppress the
//! other's datum. A read that saw nothing is refused, never published as a
//! healthy zero (same rule as [`crate::zero_invariants`]): the alarms are
//! `NOT_BREACHING` on missing data, so an empty registry or an empty window
//! must surface as a failed check.
//!
//! Both queries are type-agnostic: they run unchanged on the `UInt32` schema
//! before the window and on the `UInt64` one after it.

use crate::usd_sanity::SanityMetric;

/// CloudWatch metric: identities in `assets` that share their id with another.
pub const ASSET_ID_COLLISIONS_METRIC: &str = "AssetIdCollisions";

/// CloudWatch metric: recent candles naming an id that has no `assets` row.
pub const ASSET_ID_ORPHAN_CANDLES_METRIC: &str = "AssetIdOrphanCandles";

/// The tier every writer of candles writes; the coarse tiers derive from it.
pub const ORPHAN_CANDLE_TABLE: &str = "price_ohlcv_1m";

/// Two hours: long enough to hold several ledger-processor batches, short
/// enough that the scan stays on the live tip and an orphan shows within one
/// probe tick of being written.
pub const ORPHAN_LOOKBACK_SECONDS: i64 = 2 * 3_600;

/// One reading of `assets FINAL`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct AssetIdCounts {
    /// Rows in `assets FINAL`, i.e. distinct identities.
    pub identities: u64,
    /// Distinct `asset_id` values among them.
    pub ids: u64,
}

/// The collision read. `FINAL`, because a full re-emit of the registry leaves
/// every identity on two or three unmerged rows until the merge runs.
pub fn collisions_query() -> String {
    "SELECT count() AS identities, uniqExact(asset_id) AS ids FROM assets FINAL".to_string()
}

/// One reading of [`ORPHAN_CANDLE_TABLE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct OrphanCandleCounts {
    /// Candles in the window with a base or quote id missing from `assets`.
    pub orphans: u64,
    /// Candles examined. Zero means the scan measured nothing.
    pub scanned: u64,
}

/// The orphan read. The id set comes from `assets` without `FINAL`: an id is
/// known if any version of its row carries it, and the set is cheaper to build
/// unmerged. The candles are read `FINAL` so a superseded row is not counted.
pub fn orphan_candles_query() -> String {
    format!(
        "SELECT \
             countIf(asset_id NOT IN (SELECT asset_id FROM assets) \
                  OR quote_asset_id NOT IN (SELECT asset_id FROM assets)) AS orphans, \
             count() AS scanned \
         FROM {table} FINAL \
         WHERE timestamp >= now() - INTERVAL {lookback} SECOND",
        table = ORPHAN_CANDLE_TABLE,
        lookback = ORPHAN_LOOKBACK_SECONDS,
    )
}

/// Why a reading was not published: the read measured nothing, so its zero
/// would be a false healthy datum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UniquenessRefusal {
    /// `assets FINAL` held no row.
    EmptyRegistry,
    /// The candle window held no row.
    EmptyWindow,
}

impl std::fmt::Display for UniquenessRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyRegistry => write!(
                f,
                "asset-id uniqueness unreadable: assets FINAL holds no row, so a collision \
                 count of 0 would measure nothing; the registry is empty or unreadable to \
                 this user"
            ),
            Self::EmptyWindow => write!(
                f,
                "orphan-candle check unreadable: no candle in the last {} s of {} — \
                 ingestion has stopped or the table is empty; refusing to publish a healthy \
                 zero over a scan that measured nothing",
                ORPHAN_LOOKBACK_SECONDS, ORPHAN_CANDLE_TABLE
            ),
        }
    }
}

impl std::error::Error for UniquenessRefusal {}

/// Shape a collision reading into its datum, or refuse it.
pub fn collisions_metric(counts: &AssetIdCounts) -> Result<SanityMetric, UniquenessRefusal> {
    if counts.identities == 0 {
        return Err(UniquenessRefusal::EmptyRegistry);
    }
    Ok(SanityMetric {
        name: ASSET_ID_COLLISIONS_METRIC,
        value: counts.identities.saturating_sub(counts.ids) as f64,
    })
}

/// Shape an orphan reading into its datum, or refuse it.
pub fn orphan_candles_metric(
    counts: &OrphanCandleCounts,
) -> Result<SanityMetric, UniquenessRefusal> {
    if counts.scanned == 0 {
        return Err(UniquenessRefusal::EmptyWindow);
    }
    Ok(SanityMetric {
        name: ASSET_ID_ORPHAN_CANDLES_METRIC,
        value: counts.orphans as f64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_collision_query_counts_identities_against_distinct_ids() {
        let sql = collisions_query();
        assert!(sql.contains("count() AS identities"), "{sql}");
        assert!(sql.contains("uniqExact(asset_id) AS ids"), "{sql}");
        assert!(sql.contains("FROM assets FINAL"), "{sql}");
    }

    #[test]
    fn the_orphan_query_checks_both_legs_on_the_windowed_written_tier() {
        let sql = orphan_candles_query();
        assert!(
            sql.contains("asset_id NOT IN (SELECT asset_id FROM assets)"),
            "{sql}"
        );
        assert!(
            sql.contains("OR quote_asset_id NOT IN (SELECT asset_id FROM assets)"),
            "both legs must be checked: {sql}"
        );
        assert!(sql.contains("FROM price_ohlcv_1m FINAL"), "{sql}");
        assert!(
            sql.contains("timestamp >= now() - INTERVAL 7200 SECOND"),
            "{sql}"
        );
        assert!(sql.contains("count() AS scanned"), "{sql}");
    }

    #[test]
    fn collisions_are_identities_minus_distinct_ids() {
        let cases = [
            ((3, 3), 0.0, "every identity has its own id"),
            ((3, 2), 1.0, "two identities share one id"),
            ((210_519, 207_198), 3_321.0, "production on 2026-09-30"),
        ];
        for ((identities, ids), want, why) in cases {
            let metric = collisions_metric(&AssetIdCounts { identities, ids }).unwrap();
            assert_eq!(metric.name, ASSET_ID_COLLISIONS_METRIC);
            assert_eq!(metric.value, want, "{why}");
        }
    }

    #[test]
    fn orphans_are_published_as_a_count() {
        for (orphans, want) in [(0, 0.0), (1, 1.0)] {
            let metric = orphan_candles_metric(&OrphanCandleCounts {
                orphans,
                scanned: 50,
            })
            .unwrap();
            assert_eq!(metric.name, ASSET_ID_ORPHAN_CANDLES_METRIC);
            assert_eq!(metric.value, want);
        }
    }

    #[test]
    fn an_empty_read_is_refused_not_published_as_healthy() {
        assert_eq!(
            collisions_metric(&AssetIdCounts {
                identities: 0,
                ids: 0
            }),
            Err(UniquenessRefusal::EmptyRegistry)
        );
        assert_eq!(
            orphan_candles_metric(&OrphanCandleCounts {
                orphans: 0,
                scanned: 0
            }),
            Err(UniquenessRefusal::EmptyWindow)
        );
    }

    #[test]
    fn each_refusal_names_what_it_could_not_read() {
        let registry = UniquenessRefusal::EmptyRegistry.to_string();
        assert!(registry.contains("unreadable"), "{registry}");
        assert!(registry.contains("assets FINAL"), "{registry}");
        let window = UniquenessRefusal::EmptyWindow.to_string();
        assert!(window.contains("unreadable"), "{window}");
        assert!(window.contains("price_ohlcv_1m"), "{window}");
        assert!(window.contains("7200"), "{window}");
    }
}
