-- Phase 0 / Q1b: same as Q1 for the 1m table (779 M rows, run alone).
SELECT
    quote_asset_id,
    countIf(close_usd > 0 AND timestamp < toDateTime(1773237600)) AS priced_pre_epoch,
    countIf(close_usd > 0 AND timestamp >= toDateTime(1773237600)) AS priced_post_epoch,
    countIf(close_usd = 0) AS unpriced,
    count() AS total
FROM prices.price_ohlcv_1m
WHERE quote_asset_id IN (4, 111)
GROUP BY quote_asset_id
ORDER BY quote_asset_id
FORMAT TSVWithNames
