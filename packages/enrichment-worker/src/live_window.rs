//! Partition bound for the **scheduled** `price_ohlcv_1m` enrichment pass
//! (task 0111, option 1).
//!
//! `price_ohlcv_1m` is `PARTITION BY toYYYYMM(timestamp)`, so a
//! `timestamp >= start AND timestamp < end` predicate lets ClickHouse prune to
//! whole monthly partitions. The scheduled pass serves two populations with
//! opposite needs welded into one unbounded scan:
//!
//! | | rows | unpriced | needs |
//! |---|---|---|---|
//! | live window (`202607+`) | 17.05 M | 2.81 K | **freshness** — keeps up fine |
//! | historical (`< 202403`) | 689.64 M | 646.25 M | **throughput** |
//!
//! The live window was being dragged through a 736 M-row `FINAL` scan purely to
//! serve the historical drain, which is what put `Duration` at the 300 s Lambda
//! timeout. This module computes the live bound; the historical drain gets its
//! own frontier-driven sweep (task 0111 phase 2) rather than riding along.
//!
//! Deliberately a pure function of `(now, partitions)` — no clock, no client —
//! so the pruning behaviour is unit-testable without ClickHouse.

use chrono::{DateTime, Datelike, TimeZone, Utc};

/// Monthly partitions the scheduled pass covers: the current month and the
/// previous one. Two rather than one so a candle that lands just after a month
/// boundary (or arrives late) is still in the scheduled pass's population and
/// does not have to wait for the historical sweep to reach it.
pub const DEFAULT_LIVE_PARTITIONS: u32 = 2;

/// `[start, end)` unix-second bounds covering the `partitions` most recent
/// monthly partitions, inclusive of the month containing `now_unix`.
///
/// `partitions = 1` is the current month alone; `2` (the default) adds the
/// previous month; and so on. `partitions = 0` returns `None` — the escape
/// hatch back to the unbounded whole-table pass, so the bound can be switched
/// off by config alone if it ever proves wrong in production.
///
/// The upper bound is the first instant of the month *after* the current one,
/// not `now`: the pass's own `watermark` snapshot already excludes candles the
/// live Ledger Processor inserts mid-pass, and pinning `end` to a partition
/// boundary keeps the predicate prunable for the whole month rather than
/// re-cutting it every invocation.
pub fn live_partition_window(now_unix: i64, partitions: u32) -> Option<(u32, u32)> {
    if partitions == 0 {
        return None;
    }
    let now = DateTime::from_timestamp(now_unix, 0)?;
    // Absolute month index, so the arithmetic carries across year boundaries
    // without a special case.
    let current = now.year() as i64 * 12 + (now.month() as i64 - 1);
    let start = month_start_unix(current - (partitions as i64 - 1))?;
    let end = month_start_unix(current + 1)?;
    Some((start, end))
}

/// First instant (UTC) of the month at absolute index `idx` (`year * 12 +
/// month0`), as a unix second. `None` outside the representable range — which
/// keeps `live_partition_window` total rather than panicking on a nonsense
/// clock.
fn month_start_unix(idx: i64) -> Option<u32> {
    let year = i32::try_from(idx.div_euclid(12)).ok()?;
    let month = u32::try_from(idx.rem_euclid(12)).ok()? + 1;
    let start = Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0).single()?;
    u32::try_from(start.timestamp()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `2026-08-21 15:18:00Z` — the hour the duration-near-timeout alarm fired.
    const NOW: i64 = 1_787_325_480;

    fn ts(y: i32, m: u32) -> u32 {
        Utc.with_ymd_and_hms(y, m, 1, 0, 0, 0).unwrap().timestamp() as u32
    }

    #[test]
    fn default_covers_the_current_and_previous_month() {
        let (start, end) = live_partition_window(NOW, DEFAULT_LIVE_PARTITIONS).unwrap();
        assert_eq!(start, ts(2026, 7), "start is the first instant of 202607");
        assert_eq!(
            end,
            ts(2026, 9),
            "end is exclusive — the first instant of 202609"
        );
    }

    #[test]
    fn one_partition_is_the_current_month_alone() {
        let (start, end) = live_partition_window(NOW, 1).unwrap();
        assert_eq!(start, ts(2026, 8));
        assert_eq!(end, ts(2026, 9));
    }

    #[test]
    fn the_window_always_contains_now() {
        for partitions in 1..=6 {
            let (start, end) = live_partition_window(NOW, partitions).unwrap();
            assert!(
                (start as i64) <= NOW && NOW < (end as i64),
                "partitions = {partitions} excluded the current instant"
            );
        }
    }

    #[test]
    fn the_span_is_exactly_the_configured_partition_count() {
        // The whole point of the bound: N partitions scanned, not 102. Counting
        // month starts inside [start, end) is the same arithmetic ClickHouse's
        // `toYYYYMM` pruning does.
        for partitions in 1..=13 {
            let (start, end) = live_partition_window(NOW, partitions).unwrap();
            let months = (1..)
                .map(|i| month_start_unix(2026 * 12 + 7 + 1 - i))
                .take_while(|m| m.is_some_and(|m| m >= start))
                .count();
            assert_eq!(months as u32, partitions);
            assert!(start < end);
        }
    }

    #[test]
    fn january_walks_back_into_the_previous_year() {
        let jan = Utc
            .with_ymd_and_hms(2027, 1, 9, 3, 0, 0)
            .unwrap()
            .timestamp();
        let (start, end) = live_partition_window(jan, 2).unwrap();
        assert_eq!(start, ts(2026, 12));
        assert_eq!(end, ts(2027, 2));
    }

    #[test]
    fn december_rolls_the_upper_bound_into_the_next_year() {
        let dec = Utc
            .with_ymd_and_hms(2026, 12, 31, 23, 59, 0)
            .unwrap()
            .timestamp();
        let (start, end) = live_partition_window(dec, 2).unwrap();
        assert_eq!(start, ts(2026, 11));
        assert_eq!(end, ts(2027, 1));
    }

    #[test]
    fn zero_partitions_is_the_unbounded_pass() {
        assert_eq!(live_partition_window(NOW, 0), None);
    }
}
