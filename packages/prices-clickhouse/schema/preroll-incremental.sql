-- prices coarse PRE-ROLL — INCREMENTAL, NON-TRUNCATING, range-bounded to the
-- pre-Soroban SDEX tail `[genesis, activation)`.
--
-- WHY THIS EXISTS (the pre-roll trap, task 0088 / 0090):
--   `preroll.sql` is a FULL rebuild that expects to run after TRUNCATE-ing the
--   coarse tables. Do NOT use it here. The Soroban-era coarse (`activation →
--   ~2026-07`) is ALREADY durably pre-rolled (0090) and its source `1m` has
--   since been partition-dropped, so a TRUNCATE + full re-roll would rebuild the
--   coarse tables from only the pre-Soroban tail and WIPE the Soroban-era
--   history. This script instead APPENDS the pre-Soroban buckets only, leaving
--   every existing coarse row untouched.
--
-- WHEN TO RUN:
--   After the fishuser-hero `--mode sdex-only --start 1 --end 50457423` tail
--   backfill has finished writing `1m` (floor reaches activation-1). See
--   docs/runbooks/preroll-incremental-presoroban.md for the full procedure,
--   pre-flight checks, and the cleanup re-enable step.
--
-- SAFETY — why appending can never corrupt the existing Soroban coarse:
--   The coarse tables are ReplacingMergeTree(version), version = ledger*1000 +
--   op_index. Pre-Soroban ledgers are all < activation, so every row this
--   script inserts has a version STRICTLY LOWER than any Soroban-era row for the
--   same key. On merge, RMT keeps max(version) => the Soroban row always wins.
--   So at the single activation-boundary bucket (the 1d/1w/1M bucket that spans
--   the activation moment) the existing Soroban-side value is preserved; this
--   script's pre-Soroban partial simply loses. Net: SAFE, but that one boundary
--   month/week reflects only its post-activation slice (an accepted residual —
--   see the runbook for the optional boundary-repair from full `1m`).
--
-- BOUNDARY PARAMETER:
--   `{boundary:DateTime}` = the Soroban activation timestamp (~2024-02-20). Pass
--   it explicitly, e.g. clickhouse-client --param_boundary='2024-02-20 00:00:00'.
--   Confirm the exact value first (runbook pre-flight): it is the min timestamp
--   of any Soroban-only source, e.g.
--     SELECT min(timestamp) FROM prices.price_ohlcv_1m
--     WHERE source IN ('aquarius','phoenix','soroswap');
--
-- MEMORY (0090 finding): ch-prod-01 has a ~5.59 GiB query quota. The heavy step
--   is `15m <- 1m FINAL`; it is CHUNKED BY YEAR below so each statement only
--   scans one year's monthly partitions (PARTITION BY toYYYYMM => partition
--   pruning). If any single statement still hits the quota, split that year into
--   halves, or drop `FINAL` on the dup-free intermediate stages (1h/4h/1d/1w/1M)
--   as 0090 did — those read from single-pass GROUP BY output and carry no dups.
--
-- IDEMPOTENCY: re-runnable. RMT collapses the duplicate-PK rows a second run
--   would add (dedup on next merge / on read with FINAL). Never TRUNCATE.
--
-- Correctness (task 0059): the bucket key is aliased `AS timestamp` and SHADOWS
--   the source `timestamp`; argMin/argMax MUST reference the qualified
--   `t.timestamp` (FROM ... AS t) or open/close/close_usd tie-break to an
--   arbitrary row. Column order matches the target table (INSERT ... SELECT maps
--   by position); kept identical to preroll.sql.

-- =====================================================================
-- STAGE 1 — 15m <- 1m, CHUNKED BY YEAR (heavy; FINAL to dedup re-ingests).
--   One statement per calendar year. The first block folds genesis (2015-09)
--   through 2016 (pre-2017 SDEX is sparse; earliest candle ~2016-03). The final
--   block is the partial activation year, bounded by {boundary}.
-- =====================================================================

-- 2015-16 (Stellar mainnet genesis is 2015-09-30; the lower bound excludes any
-- spurious pre-genesis / epoch-dated rows from a bad decode).
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2015-09-01' AND t.timestamp < '2017-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2017
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2017-01-01' AND t.timestamp < '2018-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2018
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2018-01-01' AND t.timestamp < '2019-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2019
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2019-01-01' AND t.timestamp < '2020-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2020
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2020-01-01' AND t.timestamp < '2021-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2021
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2021-01-01' AND t.timestamp < '2022-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2022
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2022-01-01' AND t.timestamp < '2023-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2023
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2023-01-01' AND t.timestamp < '2024-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2024 up to activation (partial year, bounded by {boundary})
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= '2024-01-01' AND t.timestamp < {boundary:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- =====================================================================
-- STAGE 2 — coarse chain, each level FROM the previous, bounded < {boundary}.
--   These read progressively smaller tables, so a single bounded statement per
--   level is fine (partition pruning drops all >= boundary months). FINAL kept
--   for correctness/idempotency; drop it here first if the quota is hit (0090).
--   The chain is 15m -> 1h -> 4h -> 1d -> 1w -> 1M (identical to preroll.sql), so
--   run STAGE 1 to completion before STAGE 2.
--   If the heavy 1h<-15m step still exceeds the quota after dropping FINAL,
--   chunk IT by year like STAGE 1 (add `AND t.timestamp >= 'YYYY-01-01' AND
--   t.timestamp < 'YYYY+1-01-01'` per statement) — 1h/4h/1d buckets never
--   straddle a calendar year so year-chunking is safe there. Do NOT chunk 1w/1M
--   by year: their buckets straddle year boundaries, so they must stay a single
--   full-range statement (they read the small 1d/1w tables, so memory is fine).
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
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.timestamp < {boundary:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.timestamp < {boundary:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.timestamp < {boundary:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1w
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.timestamp < {boundary:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1M
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1w AS t FINAL
WHERE t.timestamp < {boundary:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;
