-- Phase 0 / Q2: the 2023-03-11 falsifier on the daily table.
-- (a) What rate did the pivot actually apply to XLM-quoted candles that day
--     (close_usd / close, i.e. the stored ref_usd), and how many rows.
-- (b) What the XLM/USDC candle closed at, what the external USDC/USD rate
--     was, and therefore what the correct ref_usd would have been.
SELECT 'stored_pivot_rate_on_xlm_quotes' AS what,
       toString(count()) AS n,
       toString(min(toFloat64(close_usd) / toFloat64(close))) AS min_rate,
       toString(max(toFloat64(close_usd) / toFloat64(close))) AS max_rate
FROM prices.price_ohlcv_1d FINAL
WHERE quote_asset_id = 4 AND close_usd > 0 AND close > 0
  AND timestamp >= toDateTime('2023-03-11 00:00:00') AND timestamp < toDateTime('2023-03-12 00:00:00')
UNION ALL
SELECT 'xlm_usdc_1d_candle', toString(count()),
       toString(min(toFloat64(close))), toString(min(toFloat64(close_usd)))
FROM prices.price_ohlcv_1d FINAL
WHERE asset_id = 4 AND quote_asset_id = 3
  AND timestamp >= toDateTime('2023-03-11 00:00:00') AND timestamp < toDateTime('2023-03-12 00:00:00')
UNION ALL
SELECT 'usdc_external_rate', toString(count()),
       toString(min(toFloat64(usd_rate))), toString(max(toFloat64(usd_rate)))
FROM prices.usd_rate FINAL
WHERE asset_code = 'USDC' AND method = 'external'
  AND timestamp >= toDateTime('2023-03-11 00:00:00') AND timestamp < toDateTime('2023-03-12 00:00:00')
FORMAT TSVWithNames
