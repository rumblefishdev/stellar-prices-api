-- Phase 0 / Q5: on one post-epoch day, which rate did XLM-quoted 1d candles
-- actually get: Reflector's XLM/USD (oracle tier) or the XLM/USDC pivot?
SELECT 'stored_rate_on_xlm_quotes' AS what, toString(count()) AS n,
       toString(round(quantile(0.5)(toFloat64(close_usd) / toFloat64(close)), 6)) AS p50,
       toString(round(min(toFloat64(close_usd) / toFloat64(close)), 6)) AS mn,
       toString(round(max(toFloat64(close_usd) / toFloat64(close)), 6)) AS mx
FROM prices.price_ohlcv_1d FINAL
WHERE quote_asset_id = 4 AND close_usd > 0 AND close > 0
  AND timestamp = toDateTime('2026-08-01 00:00:00')
UNION ALL
SELECT 'reflector_xlm_usd_2026-08-01', toString(count()),
       toString(round(quantile(0.5)(toFloat64(price_usd)), 6)),
       toString(round(min(toFloat64(price_usd)), 6)), toString(round(max(toFloat64(price_usd)), 6))
FROM prices.oracle_prices
WHERE asset_id = 4 AND oracle_name = 'reflector'
  AND timestamp >= toDateTime('2026-08-01 00:00:00') AND timestamp < toDateTime('2026-08-02 00:00:00')
UNION ALL
SELECT 'xlm_usdc_1d_close_2026-08-01', toString(count()),
       toString(round(min(toFloat64(close)), 6)),
       toString(round(min(toFloat64(close_usd)), 6)), ''
FROM prices.price_ohlcv_1d FINAL
WHERE asset_id = 4 AND quote_asset_id = 3 AND timestamp = toDateTime('2026-08-01 00:00:00')
FORMAT TSVWithNames
