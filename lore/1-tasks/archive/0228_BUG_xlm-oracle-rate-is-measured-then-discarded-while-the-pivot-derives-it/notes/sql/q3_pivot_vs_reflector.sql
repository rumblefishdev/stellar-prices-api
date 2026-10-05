-- Phase 0 / Q3 (= lore 0228 AC 1): over the oracle window, compare per hour
--   measured  = Reflector's XLM/USD reading (oracle_prices, asset_id 4)
--   pivot_raw = XLM/USDC volume-weighted close from the 1h table (what
--               pivot_sql computes, in USDC units)
--   pivot_usd = pivot_raw × Reflector's USDC/USD reading (asset_id 3)
-- Distribution of the two ratios in basis points.
WITH
    xlm AS (
        SELECT toStartOfHour(timestamp) AS h, argMax(toFloat64(price_usd), timestamp) AS measured
        FROM prices.oracle_prices
        WHERE asset_id = 4 AND oracle_name = 'reflector' AND price_usd > 0
        GROUP BY h
    ),
    usdc AS (
        SELECT toStartOfHour(timestamp) AS h, argMax(toFloat64(price_usd), timestamp) AS usdc_usd
        FROM prices.oracle_prices
        WHERE asset_id = 3 AND oracle_name = 'reflector' AND price_usd > 0
        GROUP BY h
    ),
    piv AS (
        SELECT timestamp AS h,
               sum(toFloat64(close) * toFloat64(volume_base)) / nullIf(sum(toFloat64(volume_base)), 0) AS pivot_raw
        FROM prices.price_ohlcv_1h FINAL
        WHERE asset_id = 4 AND quote_asset_id = 3 AND timestamp >= toDateTime(1773237600)
        GROUP BY timestamp
    ),
    joined AS (
        SELECT xlm.h AS h, measured, pivot_raw, pivot_raw * usdc_usd AS pivot_usd,
               (pivot_raw / measured - 1) * 10000 AS raw_bps,
               (pivot_usd / measured - 1) * 10000 AS usd_bps
        FROM xlm
        INNER JOIN usdc ON usdc.h = xlm.h
        INNER JOIN piv ON piv.h = xlm.h
        WHERE pivot_raw IS NOT NULL
    )
SELECT
    count() AS hours,
    min(h) AS first_hour, max(h) AS last_hour,
    round(quantile(0.5)(raw_bps), 2) AS raw_p50_bps,
    round(quantile(0.05)(raw_bps), 2) AS raw_p05_bps,
    round(quantile(0.95)(raw_bps), 2) AS raw_p95_bps,
    round(quantile(0.99)(abs(raw_bps)), 2) AS raw_abs_p99_bps,
    round(max(abs(raw_bps)), 2) AS raw_abs_max_bps,
    round(quantile(0.5)(usd_bps), 2) AS usd_p50_bps,
    round(quantile(0.05)(usd_bps), 2) AS usd_p05_bps,
    round(quantile(0.95)(usd_bps), 2) AS usd_p95_bps,
    round(quantile(0.99)(abs(usd_bps)), 2) AS usd_abs_p99_bps,
    round(max(abs(usd_bps)), 2) AS usd_abs_max_bps
FROM joined
FORMAT TSVWithNames
