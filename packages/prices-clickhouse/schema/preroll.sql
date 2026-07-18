-- prices coarse-granularity PRE-ROLL — deterministic, full-range.
--
-- Used by the backfill / sizing measurement (task 0060) to populate the
-- _15m … _1M tables from a fully-written _1m, instead of the bounded-window
-- live MV chain (rollups.sql). Each granularity re-aggregates the previous
-- one's FINAL over the WHOLE range, producing exactly one row per
-- (bucket, asset, quote, source) — the production-correct rollup result.
--
-- Run AFTER the _1m backfill completes (the applier in src/lib.rs::apply_sql
-- splits on `;`). Re-runnable: ReplacingMergeTree(version) collapses the
-- duplicate-PK rows a second run would add. For a clean measurement, TRUNCATE
-- the coarse tables first (the runbook does this).
--
-- Correctness (task 0059): the bucket key is aliased `AS timestamp`, which
-- SHADOWS the source `timestamp` column. argMin/argMax must reference the
-- QUALIFIED source column `t.timestamp` (FROM … AS t); the bare `timestamp`
-- would resolve to the constant bucket-start alias and tie-break open / close /
-- close_usd to an arbitrary row instead of the true first / last by time.
-- (This INSERT … SELECT maps to the target BY POSITION, so the bucket COULD be
-- renamed here — but `rollups.sql`'s MVs match BY NAME and require `AS timestamp`
-- (task 0071), so both files keep the same alias for consistency.)
--
-- version = sum(version), NOT max(version) (task 0095). The coarse tables share
-- one monotonic version scheme with the APPEND rollup MVs (rollups.sql): a
-- fuller aggregation sums more source versions than a partial one, so a complete
-- bucket always outranks a partial re-roll of itself under RMT. Mixing schemes
-- (preroll max vs MV sum) would let a partial MV bucket outrank a complete
-- pre-rolled one, because sum ≫ max for any multi-row bucket.

INSERT INTO prices.price_ohlcv_15m
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0)                AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_1m AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1h
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0)                AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_15m AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_4h
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0)                AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_1h AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1d
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0)                AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_4h AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1w
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0)                AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_1d AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1M
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0)                AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_1w AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;
