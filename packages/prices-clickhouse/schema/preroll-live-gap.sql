-- prices coarse PRE-ROLL — LIVE-ERA GAP, ALL SOURCES, INCREMENTAL.
--
-- WHY THIS EXISTS
--   The six `mv_ohlcv_*` rollup MVs are DROPPED on production (task 0090 — in
--   replace mode they overwrote pre-rolled coarse history). Nothing else rolls
--   live candles up. Measured 2026-07-17:
--
--     1m   2026-07-17 13:05   <- live, current
--     15m  2026-07-09 05:45   |
--     1h   2026-07-09 05:00   |
--     4h   2026-07-09 04:00   +- FROZEN. Every coarse granularity stopped when
--     1d   2026-07-09 00:00   |  the last pre-roll ran.
--     1w   2026-07-06 00:00   |
--     1M   2026-07-01 00:00   |
--
--   So every live candle since 2026-07-09 exists ONLY in `price_ohlcv_1m` —
--   which the cleanup worker prunes by DROPPING whole monthly partitions older
--   than 7 days (`toUInt32(partition) < toYYYYMM(now() - INTERVAL 7 DAY)`).
--   July's partition survives only while that expression is still `202607`;
--   around **2026-08-08** it becomes `202608`, July is dropped, and every live
--   candle from 07-09 on is **permanently lost**. `15m` is no safety net — it is
--   frozen at 07-09 too, and is itself pruned at 30 days.
--
--   This script rolls the gap forward. It is the STOPGAP; the durable fix is
--   re-enabling the MVs in APPEND mode (task 0095). Until 0095 lands, this must
--   be re-run periodically — **at minimum before the current month's `1m`
--   partition ages out.**
--
-- DIFFERENCES FROM `preroll-amm-reprice.sql` (0097) — read these:
--   * **ALL sources**, no `source` filter: sdex is just as frozen as the AMM
--     venues. That file was scoped to AMM because it was repairing an AMM-only
--     reprice; this one is repairing a rollup outage that hit everything.
--   * **No DELETE stage.** 0097 needed one because a corrected mid-bucket swap
--     ties the stale row on `version` (`max(ledger*1000 + op_index)`) and RMT's
--     tie-break is not contractual. Here we are EXTENDING buckets with LATER
--     ledgers, so every rebuilt row carries a strictly HIGHER max version and
--     wins outright. Nothing to delete.
--   * **Not month-chunked.** The window sits inside one monthly partition, which
--     is the granularity 0097 had to chunk down to anyway. `max_threads` is
--     still capped.
--
-- BUCKET ALIGNMENT — the subtle part, and the one that can destroy data.
--   Each level aligns its lower bound to ITS OWN bucket via
--   `toStartOfInterval({start_ts}, ...)`. Without this, a level whose bucket
--   STRADDLES `start_ts` would be rebuilt from only the post-`start_ts` slice —
--   and because that partial row carries a higher max version than the complete
--   one it replaces, it would WIN and silently delete the earlier part of the
--   bucket. Concretely: rolling `1w` from 2026-07-09 would rebuild the week
--   beginning 2026-07-06 out of 07-09-onward data only, destroying 07-06→07-08.
--   Never hand a raw `start_ts` to the wide levels.
--
-- FINAL IS MANDATORY. The target levels are not TRUNCATEd — they hold rows from
--   the earlier pre-roll — so a non-FINAL read sums duplicates and
--   DOUBLE-COUNTS. Bound memory with `max_threads`/chunking, never by dropping
--   dedup. (0090's "drop FINAL" advice only ever applied to freshly-truncated
--   targets.)
--
-- IDEMPOTENT: re-runnable. Re-rolling a bucket re-inserts an identical row
--   (same key, same value, same version) which RMT collapses. Safe to repeat.
--
-- Correctness (task 0059): the bucket key is aliased `AS timestamp` and SHADOWS
--   the source `timestamp`; argMin/argMax MUST reference the qualified
--   `t.timestamp` (FROM ... AS t) or open/close/close_usd tie-break to an
--   arbitrary row. Column order matches the target table (INSERT ... SELECT maps
--   by position).
--
-- Version (task 0095): projects sum(version), matching the APPEND rollup MVs
--   (rollups.sql) so the coarse tables carry ONE monotonic version scheme — a
--   complete bucket sums more source versions than a partial one and therefore
--   wins under RMT. This also strengthens the note above ("later ledgers =>
--   higher version => fuller row wins"): with sum() a fuller bucket wins on
--   version count, not just on the max ledger it happens to contain.
--
-- =====================================================================
-- PARAMS
--   {start_ts:DateTime}  Where the gap begins. Use a value at or BEFORE the
--                        oldest frozen coarse tip; overlap is harmless (RMT
--                        collapses identical re-rolls). For the 2026-07-17 run:
--                          --param_start_ts='2026-07-09 00:00:00'
--
--   {end_ts:DateTime}    Upper bound, EXCLUSIVE. Live ingestion is writing
--                        concurrently, so keep this behind the live frontier;
--                        the bucket straddling it is rebuilt partial and is
--                        self-healing on the next run (later ledgers => higher
--                        version => the fuller row wins). For the 2026-07-17 run:
--                          --param_end_ts='2026-07-17 13:00:00'
--
--   clickhouse-client --param_start_ts='2026-07-09 00:00:00' \
--                     --param_end_ts='2026-07-17 13:00:00' \
--                     --queries-file preroll-live-gap.sql
--
-- PRE-FLIGHT
--   1. Confirm the freeze and pick start_ts:
--        SELECT g, tip FROM (
--          SELECT '1_1m' AS g, max(timestamp) AS tip FROM prices.price_ohlcv_1m
--          UNION ALL SELECT '2_15m', max(timestamp) FROM prices.price_ohlcv_15m
--          UNION ALL SELECT '3_1h',  max(timestamp) FROM prices.price_ohlcv_1h
--          UNION ALL SELECT '4_4h',  max(timestamp) FROM prices.price_ohlcv_4h
--          UNION ALL SELECT '5_1d',  max(timestamp) FROM prices.price_ohlcv_1d
--          UNION ALL SELECT '6_1w',  max(timestamp) FROM prices.price_ohlcv_1w
--          UNION ALL SELECT '7_1M',  max(timestamp) FROM prices.price_ohlcv_1M
--        ) ORDER BY g;
--   2. Confirm `1m` still covers start_ts (if its partition already aged out,
--      that data is GONE and this script cannot recover it):
--        SELECT min(timestamp) FROM prices.price_ohlcv_1m;
--   3. Cleanup does NOT need disabling for this run: we roll only the current
--      month, whose `1m` partition is not yet eligible for dropping. Disable it
--      anyway if the run will straddle 02:00 UTC near a month boundary.
-- =====================================================================

-- =====================================================================
-- STAGE 1 — 15m <- 1m
-- =====================================================================

INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, sum(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= toStartOfInterval({start_ts:DateTime}, INTERVAL 15 MINUTE)
  AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- =====================================================================
-- STAGE 2 — the coarse chain, each level FROM the previous.
--   Run STAGE 1 to completion first; each level reads what the previous wrote.
--   Note the widening lower bound: every level aligns to its own bucket (see
--   BUCKET ALIGNMENT above).
-- =====================================================================

INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, sum(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.timestamp >= toStartOfInterval({start_ts:DateTime}, INTERVAL 1 HOUR)
  AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, sum(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.timestamp >= toStartOfInterval({start_ts:DateTime}, INTERVAL 4 HOUR)
  AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, sum(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.timestamp >= toStartOfInterval({start_ts:DateTime}, INTERVAL 1 DAY)
  AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 1w: lower bound rolls back to the START OF THE WEEK containing start_ts, so
-- the straddling week is rebuilt COMPLETE. Rebuilding it partial would win on
-- version and delete the earlier days of that week.
INSERT INTO prices.price_ohlcv_1w
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, sum(version) AS version
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.timestamp >= toStartOfInterval({start_ts:DateTime}, INTERVAL 1 WEEK)
  AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 1M: same reasoning, rolled back to the start of the month.
INSERT INTO prices.price_ohlcv_1M
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, sum(version) AS version
FROM prices.price_ohlcv_1w AS t FINAL
WHERE t.timestamp >= toStartOfInterval({start_ts:DateTime}, INTERVAL 1 MONTH)
  AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- =====================================================================
-- VERIFY
-- =====================================================================
--
-- 1. The freeze is lifted — every coarse tip should now sit just below
--    {end_ts} (each at its own bucket boundary), instead of 2026-07-09:
--      SELECT g, tip FROM (
--        SELECT '1_1m' AS g, max(timestamp) AS tip FROM prices.price_ohlcv_1m
--        UNION ALL SELECT '2_15m', max(timestamp) FROM prices.price_ohlcv_15m
--        UNION ALL SELECT '3_1h',  max(timestamp) FROM prices.price_ohlcv_1h
--        UNION ALL SELECT '4_4h',  max(timestamp) FROM prices.price_ohlcv_4h
--        UNION ALL SELECT '5_1d',  max(timestamp) FROM prices.price_ohlcv_1d
--        UNION ALL SELECT '6_1w',  max(timestamp) FROM prices.price_ohlcv_1w
--        UNION ALL SELECT '7_1M',  max(timestamp) FROM prices.price_ohlcv_1M
--      ) ORDER BY g;
--
-- 2. Conservation over the rolled window, ONE GRANULARITY AT A TIME (a
--    multi-way UNION ALL of FINAL scans runs its branches concurrently and
--    blows the 5.59 GiB quota):
--      SELECT source, sum(trade_count) AS trades,
--             round(sum(toFloat64(volume_base)), 2) AS vol
--      FROM prices.price_ohlcv_<G> FINAL
--      WHERE timestamp >= '<start_ts>' AND timestamp < '<end_ts>'
--      GROUP BY source ORDER BY source;
--    Run for 1m, then 15m/1h/4h/1d. Per source the totals must MATCH `1m`.
--    Below `1m` => buckets lost. Above => double-counting (a FINAL was dropped).
--    NOTE: 1w/1M legitimately read HIGHER — their lower bound is rolled back to
--    the week/month start, so they include data from before start_ts.
--
-- 3. The earlier week/month were not truncated by a partial rebuild:
--      SELECT timestamp, sum(trade_count) FROM prices.price_ohlcv_1w FINAL
--      WHERE timestamp >= '2026-06-01' GROUP BY timestamp ORDER BY timestamp;
--    The week containing start_ts must still carry its FULL trade count, not
--    just the post-start_ts slice.
