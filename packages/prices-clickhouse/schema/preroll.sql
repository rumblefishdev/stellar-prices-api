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

INSERT INTO prices.price_ohlcv_15m
SELECT
    toStartOfInterval(timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  timestamp)                  AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, timestamp)                  AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    volume_quote_usd / nullIf(volume_base, 0)             AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1m FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1h
SELECT
    toStartOfInterval(timestamp, INTERVAL 1 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  timestamp)                  AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, timestamp)                  AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    volume_quote_usd / nullIf(volume_base, 0)             AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_15m FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_4h
SELECT
    toStartOfInterval(timestamp, INTERVAL 4 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  timestamp)                  AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, timestamp)                  AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    volume_quote_usd / nullIf(volume_base, 0)             AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1h FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1d
SELECT
    toStartOfInterval(timestamp, INTERVAL 1 DAY) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  timestamp)                  AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, timestamp)                  AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    volume_quote_usd / nullIf(volume_base, 0)             AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_4h FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1w
SELECT
    toStartOfInterval(timestamp, INTERVAL 1 WEEK) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  timestamp)                  AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, timestamp)                  AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    volume_quote_usd / nullIf(volume_base, 0)             AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1d FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1M
SELECT
    toStartOfInterval(timestamp, INTERVAL 1 MONTH) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  timestamp)                  AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, timestamp)                  AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    volume_quote_usd / nullIf(volume_base, 0)             AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1w FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;
