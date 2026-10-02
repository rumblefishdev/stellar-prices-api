-- Phase 0 / Q8: pre-epoch XLM-quoted priced candles (1d) by year, and how many
-- sit before the first external USDC rate (2021-01-25) — those have no USDC
-- rate at all and a reset must never touch them.
SELECT toYear(timestamp) AS y, count() AS candles,
       countIf(timestamp < toDateTime('2021-01-25 00:00:00')) AS before_external_series
FROM prices.price_ohlcv_1d
WHERE quote_asset_id = 4 AND close_usd > 0 AND timestamp < toDateTime(1773237600)
GROUP BY y ORDER BY y
FORMAT TSVWithNames
