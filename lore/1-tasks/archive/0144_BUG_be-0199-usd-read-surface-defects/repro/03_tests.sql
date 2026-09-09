SELECT '===== TEST A: does the rollup MV propagate an un-enriched tail as close_usd = 0? =====' AS t FORMAT TSVRaw;
SELECT 'input _15m sub-buckets of the 13:00 hour' AS t FORMAT TSVRaw;
SELECT timestamp, close, close_usd, volume_base FROM prices.price_ohlcv_15m FINAL ORDER BY timestamp FORMAT PrettyCompactMonoBlock;

SELECT 'output of mv_ohlcv_15m_to_1h SELECT (verbatim from rollups.sql, window predicate widened)' AS t FORMAT TSVRaw;
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    argMax(close_usd, t.timestamp)            AS close_usd_asis,
    argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd_if_guarded,
    sum(version)                              AS version
FROM prices.price_ohlcv_15m AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source
FORMAT PrettyCompactMonoBlock;

SELECT '===== TEST B: can a dust print become the whole bucket price? =====' AS t FORMAT TSVRaw;
SELECT 'input _1h rows — 12:00 fully enriched (control), 13:00 partly enriched' AS t FORMAT TSVRaw;
SELECT timestamp, source, close_usd, volume_base FROM prices.price_ohlcv_1h FINAL ORDER BY timestamp, source FORMAT PrettyCompactMonoBlock;

SELECT 'output of prices.price_usd_series_1h (the shipped view)' AS t FORMAT TSVRaw;
SELECT asset_code, bucket, close_usd FROM prices.price_usd_series_1h ORDER BY bucket FORMAT PrettyCompactMonoBlock;

SELECT 'the same buckets weighted over ALL rows (no close_usd > 0 filter), plus the coverage measure' AS t FORMAT TSVRaw;
SELECT
    p.timestamp AS bucket,
    CAST(sum(toFloat64(p.close_usd) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38,6)) AS unfiltered_wavg,
    count()                                                                     AS rows_total,
    countIf(p.close_usd > 0)                                                    AS rows_priced,
    round(sumIf(toFloat64(p.volume_base), p.close_usd > 0) / sum(toFloat64(p.volume_base)), 6) AS priced_volume_share
FROM prices.price_ohlcv_1h AS p FINAL
GROUP BY bucket ORDER BY bucket
FORMAT PrettyCompactMonoBlock;

SELECT '===== TEST C: does an un-enriched tip zero the headline price_usd? =====' AS t FORMAT TSVRaw;
SELECT 'input _1m candles for native XLM' AS t FORMAT TSVRaw;
SELECT timestamp, quote_asset_id, close, close_usd FROM prices.price_ohlcv_1m FINAL ORDER BY timestamp FORMAT PrettyCompactMonoBlock;

SELECT 'the unfiltered CTE from current.sql vs the same with a >0 guard' AS t FORMAT TSVRaw;
SELECT
    asset_id,
    argMax(close_usd, timestamp)                     AS price_usd_asis,
    argMaxIf(close_usd, timestamp, close_usd > 0)    AS price_usd_if_guarded
FROM prices.price_ohlcv_1m FINAL
GROUP BY asset_id
FORMAT PrettyCompactMonoBlock;

SELECT '===== TEST D: does the assets join fan out on a shared asset_id? =====' AS t FORMAT TSVRaw;
SELECT 'assets FINAL — both identities survive dedup (ORDER BY is natural identity, not asset_id)' AS t FORMAT TSVRaw;
SELECT asset_id, asset_code, issuer_address FROM prices.assets FINAL WHERE asset_id = 7 FORMAT PrettyCompactMonoBlock;

SELECT 'one _1d candle joined to assets on asset_id' AS t FORMAT TSVRaw;
SELECT
    count()                                                              AS joined_rows,
    countDistinct(p.asset_id, p.timestamp, p.source, p.quote_asset_id)   AS distinct_candles
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.asset_id = 7
FORMAT PrettyCompactMonoBlock;

SELECT 'what price_usd_series would publish per identity for that one candle' AS t FORMAT TSVRaw;
SELECT
    a.asset_code AS asset_code, a.issuer_address AS issuer_address,
    p.timestamp  AS bucket,
    CAST(sum(toFloat64(p.close_usd) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38,6)) AS close_usd,
    sum(p.volume_base) AS volume_base
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.close_usd > 0 AND p.asset_id = 7
GROUP BY asset_code, issuer_address, bucket
FORMAT PrettyCompactMonoBlock;
