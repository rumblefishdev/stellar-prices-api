//! Weekly check that each stream's stored `earliest_data_available` claim is
//! not earlier than its first `price_ohlcv_1h` row (task 0272; rationale in
//! the task's Design Decisions). Runs only on `{"check":"reconcile"}`.

use crate::{Metric, SDEX_ARCHIVE_STREAM, SOROBAN_AMM_STREAM};

/// Must match the `metricName` of the `…-backfill-earliest-overclaim-*` alarms.
pub const OVERCLAIM_METRIC_NAME: &str = "EarliestOverclaimSeconds";

/// `first _1h bucket − stored claim` in seconds; positive = overclaim.
#[derive(Debug, Clone, PartialEq, Eq, clickhouse::Row, serde::Deserialize)]
pub struct StreamOverclaim {
    pub task_name: String,
    pub overclaim_seconds: i64,
}

/// - `_1h`: `_1m` has no AMM history (false pass), `_1d` hides sub-day overclaims.
/// - Classified like the writer (`sdex` vs everything else), no venue list, no
///   status filter.
/// - `LEFT JOIN` + `join_use_nulls = 1`: a claim with no rows reads
///   `now() − claim` instead of being dropped or joined to 1970.
/// - `FINAL` + `optimize_move_to_prewhere_if_final = 0`: latest claim version.
pub const RECONCILE_QUERY: &str = "SELECT \
     p.task_name AS task_name, \
     toInt64(if(isNull(r.first_ts), \
                toUnixTimestamp(now()), \
                toUnixTimestamp(assumeNotNull(r.first_ts))) \
             - toUnixTimestamp(assumeNotNull(p.earliest_data_available))) \
       AS overclaim_seconds \
   FROM ( \
     SELECT task_name, earliest_data_available \
     FROM backfill_progress FINAL \
     WHERE task_name IN ('sdex_archive', 'soroban_amm') \
       AND earliest_data_available IS NOT NULL \
   ) AS p \
   LEFT JOIN ( \
     SELECT if(source = 'sdex', 'sdex_archive', 'soroban_amm') AS task_name, \
            min(timestamp) AS first_ts \
     FROM price_ohlcv_1h \
     GROUP BY task_name \
   ) AS r ON p.task_name = r.task_name \
   ORDER BY p.task_name \
   SETTINGS join_use_nulls = 1, optimize_move_to_prewhere_if_final = 0, max_execution_time = 30";

/// Signed, not clamped: values in (−3600, 0] are hour-bucket slack.
pub fn overclaim_metrics(rows: &[StreamOverclaim]) -> Vec<Metric> {
    rows.iter()
        .map(|row| Metric {
            stream: row.task_name.clone(),
            value: row.overclaim_seconds as f64,
        })
        .collect()
}

/// Streams with a NULL claim or no `backfill_progress` row: no datum published.
pub fn missing_streams(rows: &[StreamOverclaim]) -> Vec<&'static str> {
    [SDEX_ARCHIVE_STREAM, SOROBAN_AMM_STREAM]
        .into_iter()
        .filter(|s| !rows.iter().any(|r| r.task_name == *s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(task: &str, v: i64) -> StreamOverclaim {
        StreamOverclaim {
            task_name: task.to_string(),
            overclaim_seconds: v,
        }
    }

    #[test]
    fn overclaim_maps_verbatim_and_keeps_the_sign() {
        let m = overclaim_metrics(&[
            row(SDEX_ARCHIVE_STREAM, -2820),
            row(SOROBAN_AMM_STREAM, 18_000),
        ]);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].stream, SDEX_ARCHIVE_STREAM);
        assert_eq!(m[0].value, -2820.0);
        assert_eq!(m[1].stream, SOROBAN_AMM_STREAM);
        assert_eq!(m[1].value, 18_000.0);
    }

    #[test]
    fn missing_streams_lists_every_known_stream_without_a_row() {
        assert_eq!(
            missing_streams(&[]),
            vec![SDEX_ARCHIVE_STREAM, SOROBAN_AMM_STREAM]
        );
        assert_eq!(
            missing_streams(&[row(SDEX_ARCHIVE_STREAM, 0)]),
            vec![SOROBAN_AMM_STREAM]
        );
        assert_eq!(
            missing_streams(&[row(SOROBAN_AMM_STREAM, 0)]),
            vec![SDEX_ARCHIVE_STREAM]
        );
        assert!(
            missing_streams(&[row(SOROBAN_AMM_STREAM, 0), row(SDEX_ARCHIVE_STREAM, 0)]).is_empty()
        );
    }
}
