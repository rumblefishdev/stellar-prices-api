-- prices rollup chain — PRODUCTION refreshable MV design (task 0051/0059).
--
-- NOT applied by prices-clickhouse-init (kept out of init.sql so schema-apply
-- stays version-agnostic). Refreshable MVs require ClickHouse ≥ 23.12 and the
-- allow_experimental_refreshable_materialized_view setting on older builds.
--
-- These MVs serve the LIVE path only: each re-aggregates a bounded recent
-- window from the previous granularity's FINAL (post-dedup, post-enrichment),
-- and replaces its target on refresh. Historical/backfilled partitions fall
-- outside the window and are pre-rolled instead (see preroll.sql).
--
-- Correctness (task 0059):
--   - vwap references the summed aliases, never sum(…)/sum(…) (else
--     Code: 184 ILLEGAL_AGGREGATION from aggregate-in-aggregate).
--   - version = max(version) is correct for a TRUE refreshable MV (atomic
--     target replace). If a CH build forces scheduled INSERT…SELECT into a
--     ReplacingMergeTree instead, project a strictly-increasing version.

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1m_to_15m
REFRESH EVERY 1 MINUTE
TO prices.price_ohlcv_15m AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1m FINAL
WHERE timestamp >= now() - INTERVAL 2 HOUR
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_15m_to_1h
REFRESH EVERY 15 MINUTE
TO prices.price_ohlcv_1h AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_15m FINAL
WHERE timestamp >= now() - INTERVAL 8 HOUR
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1h_to_4h
REFRESH EVERY 1 HOUR
TO prices.price_ohlcv_4h AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1h FINAL
WHERE timestamp >= now() - INTERVAL 1 DAY
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_4h_to_1d
REFRESH EVERY 4 HOUR
TO prices.price_ohlcv_1d AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_4h FINAL
WHERE timestamp >= now() - INTERVAL 7 DAY
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1d_to_1w
REFRESH EVERY 1 DAY
TO prices.price_ohlcv_1w AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1d FINAL
WHERE timestamp >= now() - INTERVAL 60 DAY
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1w_to_1M
REFRESH EVERY 1 DAY
TO prices.price_ohlcv_1M AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1w FINAL
WHERE timestamp >= now() - INTERVAL 400 DAY
GROUP BY timestamp, asset_id, quote_asset_id, source;
