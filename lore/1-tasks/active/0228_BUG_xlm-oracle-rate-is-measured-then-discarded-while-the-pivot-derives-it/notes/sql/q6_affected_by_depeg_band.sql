-- Phase 0 / Q6: pre-epoch XLM-quoted candles bucketed by how far the daily
-- USDC/USD external rate sat from par on their day. Bands in bps of |rate-1|.
WITH ext AS (
    SELECT toDate(timestamp) AS d, argMax(toFloat64(usd_rate), timestamp) AS r
    FROM prices.usd_rate FINAL
    WHERE asset_code = 'USDC' AND method = 'external'
    GROUP BY d
)
SELECT tbl,
       multiIf(band_bps < 10, 'a_<10', band_bps < 50, 'b_10-50', band_bps < 100, 'c_50-100', band_bps < 300, 'd_100-300', 'e_>=300') AS band,
       count() AS candles, uniq(d) AS days
FROM (
    SELECT tbl, toDate(p.timestamp) AS d, abs(ext.r - 1) * 10000 AS band_bps
    FROM (
        SELECT '1d' AS tbl, timestamp FROM prices.price_ohlcv_1d WHERE quote_asset_id = 4 AND close_usd > 0 AND timestamp < toDateTime(1773237600)
        UNION ALL
        SELECT '1h', timestamp FROM prices.price_ohlcv_1h WHERE quote_asset_id = 4 AND close_usd > 0 AND timestamp < toDateTime(1773237600)
    ) AS p
    INNER JOIN ext ON ext.d = toDate(p.timestamp)
)
GROUP BY tbl, band
ORDER BY tbl, band
FORMAT TSVWithNames
