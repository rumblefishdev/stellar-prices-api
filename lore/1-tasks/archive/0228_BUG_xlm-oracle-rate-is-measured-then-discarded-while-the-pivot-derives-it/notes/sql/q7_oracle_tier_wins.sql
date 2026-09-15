-- Phase 0 / Q7: on 2026-08-01, does the stored rate on XLM-quoted 1d candles
-- equal a Reflector XLM reading (oracle tier) or the XLM/USDC vwap (pivot)?
WITH refl AS (
    SELECT groupArray(toFloat64(price_usd)) AS readings
    FROM prices.oracle_prices
    WHERE asset_id = 4 AND oracle_name = 'reflector'
      AND timestamp >= toDateTime('2026-07-31 00:00:00') AND timestamp < toDateTime('2026-08-02 00:00:00')
)
SELECT count() AS candles,
       countIf(arrayExists(x -> abs(x / rate - 1) < 0.00005, (SELECT readings FROM refl))) AS matches_a_reflector_reading,
       countIf(abs(rate / 0.170593 - 1) < 0.00005) AS matches_pivot_vwap
FROM (
    SELECT toFloat64(close_usd) / toFloat64(close) AS rate
    FROM prices.price_ohlcv_1d FINAL
    WHERE quote_asset_id = 4 AND close_usd > 0 AND close > 0
      AND timestamp = toDateTime('2026-08-01 00:00:00')
)
FORMAT TSVWithNames
