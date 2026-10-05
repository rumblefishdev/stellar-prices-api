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
//! 3. `SacContractIdentities` = contract rows of `assets FINAL` whose address
//!    is a SAC: some row's `sac_address`, or a contract BE flags `is_sac`
//!    (task 0242). A SAC is a facet of its classic asset, never an identity of
//!    its own. Baseline 36 until the 0242 heal, then 0.
//!
//! The reads are independent: a failure in one must not suppress another's
//! datum. A read that saw nothing is refused, never published as a
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

/// CloudWatch metric: contract identities in `assets` that are SACs.
pub const SAC_CONTRACT_IDENTITIES_METRIC: &str = "SacContractIdentities";

/// BE's database, holding `soroban_contracts` (prices_writer has SELECT on it).
pub const BE_DATABASE: &str = "default";

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

/// One reading of the contract rows of `assets FINAL` against the SAC sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct SacContractCounts {
    /// Contract rows whose address is a SAC.
    pub sac_rows: u64,
    /// Contract rows examined. Zero means the scan measured nothing.
    pub scanned: u64,
    /// Contracts BE flags `is_sac`. Zero means the second test measured nothing.
    pub be_sacs: u64,
}

/// The SAC-identity read. A contract row is a SAC when its address is the
/// `sac_address` of a classic row, or when BE flags it `is_sac` (a SAC whose
/// classic we never held). `uniqExact`, because `soroban_contracts` is a
/// ReplacingMergeTree read without `FINAL`; `ifNull`, because a scalar
/// subquery is `Nullable` and RowBinary would misread it as a `u64`.
pub fn sac_contract_identities_query(be_db: &str) -> String {
    format!(
        "SELECT \
             countIf(contract_address IN (SELECT sac_address FROM assets WHERE sac_address != '') \
                  OR contract_address IN (SELECT contract_id FROM {be_db}.soroban_contracts WHERE is_sac)) \
                 AS sac_rows, \
             count() AS scanned, \
             ifNull((SELECT uniqExact(contract_id) FROM {be_db}.soroban_contracts WHERE is_sac), 0) \
                 AS be_sacs \
         FROM assets FINAL \
         WHERE contract_address != ''"
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
    /// `assets FINAL` held no contract row.
    NoContractRows,
    /// BE's `soroban_contracts` held no `is_sac` contract.
    EmptyBeSacSet,
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
            Self::NoContractRows => write!(
                f,
                "sac-contract-identities unreadable: assets FINAL holds no contract row, so a \
                 count of 0 would measure nothing; the registry is empty or unreadable to this user"
            ),
            Self::EmptyBeSacSet => write!(
                f,
                "sac-contract-identities unreadable: {BE_DATABASE}.soroban_contracts holds no \
                 is_sac contract, so SACs whose classic we never held go uncounted; BE's table \
                 is empty or unreadable to this user"
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

/// Shape a SAC-identity reading into its datum, or refuse it.
pub fn sac_contract_identities_metric(
    counts: &SacContractCounts,
) -> Result<SanityMetric, UniquenessRefusal> {
    if counts.scanned == 0 {
        return Err(UniquenessRefusal::NoContractRows);
    }
    if counts.be_sacs == 0 {
        return Err(UniquenessRefusal::EmptyBeSacSet);
    }
    Ok(SanityMetric {
        name: SAC_CONTRACT_IDENTITIES_METRIC,
        value: counts.sac_rows as f64,
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
        let contracts = UniquenessRefusal::NoContractRows.to_string();
        assert!(contracts.contains("unreadable"), "{contracts}");
        assert!(contracts.contains("no contract row"), "{contracts}");
        let be = UniquenessRefusal::EmptyBeSacSet.to_string();
        assert!(be.contains("unreadable"), "{be}");
        assert!(be.contains("default.soroban_contracts"), "{be}");
    }

    #[test]
    fn the_sac_query_counts_contract_rows_against_both_sac_sets() {
        let sql = sac_contract_identities_query(BE_DATABASE);
        for part in [
            "FROM assets FINAL",
            "contract_address != ''",
            "contract_address IN (SELECT sac_address FROM assets WHERE sac_address != '')",
            "contract_address IN (SELECT contract_id FROM default.soroban_contracts WHERE is_sac)",
            "ifNull((SELECT uniqExact(contract_id) FROM default.soroban_contracts WHERE is_sac), 0)",
            "AS be_sacs",
            "AS sac_rows",
            "count() AS scanned",
        ] {
            assert!(sql.contains(part), "{part}: {sql}");
        }
        assert!(!sql.contains("SETTINGS"), "{sql}");
    }

    #[test]
    fn sac_contract_identities_are_published_or_refused() {
        let cases = [
            ((0, 5, 10), Ok(0.0), "no SAC among the contracts"),
            ((36, 59, 4_042), Ok(36.0), "production on 2026-10-05"),
            (
                (0, 0, 10),
                Err(UniquenessRefusal::NoContractRows),
                "no contract row",
            ),
            (
                (0, 5, 0),
                Err(UniquenessRefusal::EmptyBeSacSet),
                "empty BE SAC set",
            ),
        ];
        for ((sac_rows, scanned, be_sacs), want, why) in cases {
            let got = sac_contract_identities_metric(&SacContractCounts {
                sac_rows,
                scanned,
                be_sacs,
            });
            assert_eq!(
                got.map(|m| (m.name, m.value)),
                want.map(|v| (SAC_CONTRACT_IDENTITIES_METRIC, v)),
                "{why}"
            );
        }
    }
}
