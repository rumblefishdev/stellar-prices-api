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
--   - The bucket key MUST be aliased `AS timestamp`, and that forced name is —
--     separately — what makes the `t.` qualifier below mandatory. Two distinct
--     ClickHouse name mechanisms collide here (task 0071):
--
--       (1) INSERT ROUTING (by name). A `TO`-table MV routes its result by
--           matching each SELECT-output column NAME to the TARGET table's column
--           of the same name (target = `price_ohlcv_15m`; the source table is not
--           consulted). So the bucket output must be named `timestamp` to land in
--           `price_ohlcv_15m.timestamp`. A differently-named bucket (`ts_bucket`)
--           is rejected: `Code: 8 THERE_IS_NO_COLUMN` (verified on CH 26.3.10.60).
--           NB a plain `INSERT … SELECT` routes by POSITION instead — which is why
--           `preroll.sql` accepts `ts_bucket`; it keeps `timestamp` only for
--           parity with the MVs.
--
--       (2) IN-QUERY RESOLUTION (the shadow). Inside the SELECT, the mandatory
--           `AS timestamp` alias shadows the source column `timestamp`, so a bare
--           `timestamp` in argMin/argMax + the WHERE window resolves to the
--           CONSTANT bucket-start value, not the per-row time — tie-breaking
--           open/close/close_usd to an arbitrary row (task 0059 full-chain test).
--           Reading the QUALIFIED source `t.timestamp` (FROM … AS t) is the fix.
--
--     Because (1) forces the name that causes (2), renaming the bucket is NOT an
--     option for the MVs — `t.`-qualification is the only available remedy.

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1m_to_15m
REFRESH EVERY 1 MINUTE
TO prices.price_ohlcv_15m AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= now() - INTERVAL 2 HOUR
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_15m_to_1h
REFRESH EVERY 15 MINUTE
TO prices.price_ohlcv_1h AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.timestamp >= now() - INTERVAL 8 HOUR
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1h_to_4h
REFRESH EVERY 1 HOUR
TO prices.price_ohlcv_4h AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.timestamp >= now() - INTERVAL 1 DAY
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_4h_to_1d
REFRESH EVERY 4 HOUR
TO prices.price_ohlcv_1d AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.timestamp >= now() - INTERVAL 7 DAY
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1d_to_1w
REFRESH EVERY 1 DAY
TO prices.price_ohlcv_1w AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.timestamp >= now() - INTERVAL 60 DAY
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1w_to_1M
REFRESH EVERY 1 DAY
TO prices.price_ohlcv_1M AS
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
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    max(version)                              AS version
FROM prices.price_ohlcv_1w AS t FINAL
WHERE t.timestamp >= now() - INTERVAL 400 DAY
GROUP BY timestamp, asset_id, quote_asset_id, source;
