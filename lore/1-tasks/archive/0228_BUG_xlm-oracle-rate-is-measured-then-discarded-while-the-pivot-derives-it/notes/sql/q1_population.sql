-- Phase 0 / Q1: how many XLM- and USDT-quoted candles carry a pivot-priced
-- close_usd, split at the USDC oracle epoch (1773237600 = 2026-03-11 14:00 UTC).
-- Pre-epoch rows are the population a 0268-shaped re-enrichment would touch.
-- No FINAL: sizing only, ReplacingMergeTree duplicates are noise here.
SELECT
    tbl,
    quote_asset_id,
    countIf(close_usd > 0 AND timestamp < toDateTime(1773237600)) AS priced_pre_epoch,
    countIf(close_usd > 0 AND timestamp >= toDateTime(1773237600)) AS priced_post_epoch,
    countIf(close_usd = 0) AS unpriced,
    count() AS total
FROM (
    SELECT '1M' AS tbl, quote_asset_id, timestamp, close_usd FROM prices.price_ohlcv_1M WHERE quote_asset_id IN (4, 111)
    UNION ALL SELECT '1w', quote_asset_id, timestamp, close_usd FROM prices.price_ohlcv_1w WHERE quote_asset_id IN (4, 111)
    UNION ALL SELECT '1d', quote_asset_id, timestamp, close_usd FROM prices.price_ohlcv_1d WHERE quote_asset_id IN (4, 111)
    UNION ALL SELECT '4h', quote_asset_id, timestamp, close_usd FROM prices.price_ohlcv_4h WHERE quote_asset_id IN (4, 111)
    UNION ALL SELECT '1h', quote_asset_id, timestamp, close_usd FROM prices.price_ohlcv_1h WHERE quote_asset_id IN (4, 111)
    UNION ALL SELECT '15m', quote_asset_id, timestamp, close_usd FROM prices.price_ohlcv_15m WHERE quote_asset_id IN (4, 111)
)
GROUP BY tbl, quote_asset_id
ORDER BY quote_asset_id, tbl
FORMAT TSVWithNames
