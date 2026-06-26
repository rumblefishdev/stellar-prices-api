-- Proof harness for task 0059 — minimal _1m -> _15m rollup chain.
-- Reproduces the draft insert-trigger MV from database-schema-overview.md §3.2
-- alongside a re-aggregate-from-FINAL target, so the two can be compared
-- side by side after identical inserts + an enrichment re-insert.

DROP DATABASE IF EXISTS prices;
CREATE DATABASE prices;

-- Source granularity (same shape as production, volume_quote restored per 0058).
CREATE TABLE prices.price_ohlcv_1m (
    timestamp        DateTime,
    asset_id         UInt32,
    quote_asset_id   UInt32,
    source           LowCardinality(String),
    open             Decimal(38, 14),
    high             Decimal(38, 14),
    low              Decimal(38, 14),
    close            Decimal(38, 14),
    volume_base      Decimal(38, 14) DEFAULT 0,
    volume_quote     Decimal(38, 14) DEFAULT 0,
    volume_quote_usd Decimal(38, 14) DEFAULT 0,
    vwap             Decimal(38, 14),
    trade_count      UInt32 DEFAULT 0,
    version          UInt64
)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, quote_asset_id, source, timestamp);

-- Target A: fed by the DRAFT insert-trigger MV (schema-overview §3.2).
CREATE TABLE prices.price_ohlcv_15m_draft AS prices.price_ohlcv_1m;

-- Target B: populated by the re-aggregate-from-_1m-FINAL approach (Option A).
CREATE TABLE prices.price_ohlcv_15m_fix AS prices.price_ohlcv_1m;

-- FINDING #1 (compile failure): the §3.2 draft, transcribed VERBATIM, does
-- not create. Its `vwap` line repeats `sum(volume_quote_usd)/nullIf(sum(
-- volume_base),0)` while those same sums are aliased `AS volume_quote_usd`
-- / `AS volume_base` above. ClickHouse resolves the inner column refs to the
-- aliases, nesting aggregate-in-aggregate:
--   Code: 184. ILLEGAL_AGGREGATION: Aggregate function sum(volume_quote_usd)
--   AS volume_quote_usd is found inside another aggregate function in query.
-- The draft MV is therefore non-functional as written (CH 24.8.14).
--
-- Below is the alias-collision-FIXED draft (vwap references the aliases
-- instead of re-summing) so we can still demonstrate the *runtime* under-count
-- (finding #2). This is the minimum change to make the draft compile; it is
-- still an insert-trigger MV summing the inserted block.
CREATE MATERIALIZED VIEW prices.mv_ohlcv_1m_to_15m_draft
TO prices.price_ohlcv_15m_draft AS
SELECT
    toStartOfInterval(timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id,
    quote_asset_id,
    source,
    argMin(open,  timestamp)         AS open,
    max(high)                        AS high,
    min(low)                         AS low,
    argMax(close, timestamp)         AS close,
    sum(volume_base)                 AS volume_base,
    sum(volume_quote)                AS volume_quote,
    sum(volume_quote_usd)            AS volume_quote_usd,
    volume_quote_usd / nullIf(volume_base, 0) AS vwap,  -- ref aliases, not re-sum
    sum(trade_count)                 AS trade_count,
    max(version)                     AS version
FROM prices.price_ohlcv_1m
GROUP BY timestamp, asset_id, quote_asset_id, source;
