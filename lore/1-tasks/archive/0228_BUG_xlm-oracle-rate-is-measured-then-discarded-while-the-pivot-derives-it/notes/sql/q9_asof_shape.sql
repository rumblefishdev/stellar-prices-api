-- Shape probe (read-only) for the planned pivot_sql: a candidate subquery with
-- a bucket-end column, ASOF-joined to the ref/USDC vwap AND to the USDC/USD
-- rate resolved at the bucket end, with "oracle over external" expressed as
-- two nested method-specific ASOF legs (no argMax across methods).
-- Runs against the daily table on the 2023-03-11 falsifier day.
SELECT
    p.timestamp,
    p.asset_id,
    toString(p.close) AS close,
    toString(p.close_usd) AS stored_close_usd,
    round(r.vwap, 8) AS ref_vwap_usdc,
    toString(u.rate) AS usdc_usd,
    u.rate_method,
    round(r.vwap * toFloat64(u.rate) * toFloat64(p.close), 8) AS scaled_close_usd
FROM (
    SELECT timestamp, asset_id, quote_asset_id, close, close_usd,
           addDays(timestamp, 1, 'UTC') AS bend, 1 AS k
    FROM prices.price_ohlcv_1d FINAL
    WHERE quote_asset_id = 4
      AND timestamp >= toDateTime('2023-03-11 00:00:00') AND timestamp < toDateTime('2023-03-12 00:00:00')
      AND close_usd > 0 AND volume_quote > 0
    ORDER BY asset_id
    LIMIT 5
) AS p
ASOF LEFT JOIN (
    SELECT CAST(4 AS UInt32) AS ref_asset_id, timestamp,
           sum(toFloat64(close) * toFloat64(volume_base)) / nullIf(sum(toFloat64(volume_base)), 0) AS vwap
    FROM prices.price_ohlcv_1d FINAL
    WHERE asset_id = 4 AND quote_asset_id = 3
      AND timestamp >= toDateTime('2023-03-01 00:00:00') AND timestamp < toDateTime('2023-03-13 00:00:00')
    GROUP BY timestamp
) AS r ON r.ref_asset_id = p.quote_asset_id AND r.timestamp <= p.timestamp
ASOF LEFT JOIN (
    -- oracle preferred over external: the outer leg is oracle, the inner
    -- fallback is external, each with an explicit method, no argMax.
    SELECT 1 AS k, ts, rate, rate_method FROM (
        SELECT timestamp AS ts, usd_rate AS rate, 'oracle' AS rate_method
        FROM prices.usd_rate FINAL
        WHERE asset_kind = 'credit' AND asset_code = 'USDC'
          AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN' AND contract_address = ''
          AND method = 'oracle' AND usd_rate > 0
          AND timestamp >= toDateTime('2023-03-01 00:00:00') AND timestamp < toDateTime('2023-03-13 00:00:00')
        UNION ALL
        SELECT timestamp AS ts, usd_rate AS rate, 'external' AS rate_method
        FROM prices.usd_rate FINAL
        WHERE asset_kind = 'credit' AND asset_code = 'USDC'
          AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN' AND contract_address = ''
          AND method = 'external' AND usd_rate > 0
          AND timestamp >= toDateTime('2023-03-01 00:00:00') AND timestamp < toDateTime('2023-03-13 00:00:00')
    )
) AS u ON u.k = p.k AND u.ts <= p.bend
WHERE r.vwap IS NOT NULL AND (p.bend - u.ts) <= 86400
FORMAT TSVWithNames
