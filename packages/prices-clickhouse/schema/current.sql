-- prices.current_prices refreshable MV (task 0039) — replaces the per-minute
-- "Current Price Updater" Lambda (ADR 0007 / 0039 Q#1). The scheduler lives
-- inside ClickHouse: REFRESH EVERY 1 MINUTE re-derives one row per asset from
-- price_ohlcv_1m and writes prices.current_prices (ReplacingMergeTree → latest
-- per asset_id; read with FINAL). The MV is the SOLE writer of current_prices.
--
-- NOT applied by prices-clickhouse-init (kept out of init.sql so schema-apply
-- stays version-agnostic). Refreshable MVs require ClickHouse ≥ 23.12.
--
-- DEPLOY ORDER / DEPENDENCY: every USD column here derives from
-- price_ohlcv_1m.close_usd and .volume_quote_usd, which the ingest path writes
-- as DEFAULT 0 — they are filled by the enrichment pass (task 0026). Until that
-- pass is deployed and has run, this MV emits price_usd / volume_24h_usd /
-- vwap_24h / market_cap_usd = 0 for every asset. Apply this MV only once
-- enrichment is live, otherwise current_prices serves all-zero rows.
--
-- v1 columns (the cleanly price-data-derivable + market cap):
--   price_usd       — latest USD close (argMax over the 24h window)
--   volume_24h_usd  — trailing-24h USD volume
--   vwap_24h        — USD volume-weighted close across sources (the §5.5
--                     inter-source median-outlier filter is a follow-up)
--   market_cap_usd  — price_usd × circulating supply from prices.asset_supply
--                     (LEFT JOIN; 0 when supply absent — best-effort, the column
--                     is non-nullable so 0 is the "unavailable" sentinel)
-- price_xlm / change_24h_pct / change_7d_pct / sources keep their column
-- DEFAULTs (0 / '') in v1 — tracked as follow-ups (XLM-quote orientation, the
-- 24h/7d reference-close self-join, and the per-source JSON breakdown).

-- Decimal×Decimal widens scale past Decimal(38,14)'s budget (14+14=28 scale
-- leaves only 10 integer digits → overflow at ~1e10), so the two arithmetic
-- columns can't multiply natively:
--   vwap_24h       — a volume-weighted *price*, so it stays price-magnitude and
--                    can't overflow; computed in Float64 (≈15-16 sig digits,
--                    ample for a price) and cast back with toDecimal128(…,14).
--   market_cap_usd — price × supply can be huge, so Float64 would both lose
--                    low-order digits AND throw on overflow (poisoning the whole
--                    refresh). Computed as an EXACT Decimal256 product instead,
--                    then accurateCastOrNull back to Decimal(38,14): out-of-range
--                    → NULL → 0 (the column's "unavailable" sentinel) rather than
--                    a refresh-killing exception.
-- price_usd (argMax, no arithmetic) and volume_24h_usd (a plain sum) stay native
-- Decimal.
--
-- The TO clause carries an EXPLICIT target column list: a materialised view
-- inserts into its target POSITIONALLY otherwise, and this SELECT projects only
-- the v1 subset (not all 10 current_prices columns, nor in table order). The
-- column list maps each projection to its named column; the unlisted columns
-- (price_xlm, change_24h_pct, change_7d_pct, sources) take their table DEFAULTs.
CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_current_prices
REFRESH EVERY 1 MINUTE
TO prices.current_prices
   (asset_id, price_usd, volume_24h_usd, vwap_24h, market_cap_usd, updated_at) AS
SELECT
    c.asset_id                                              AS asset_id,
    argMax(c.close_usd, c.timestamp)                        AS price_usd,
    sum(c.volume_quote_usd)                                 AS volume_24h_usd,
    toDecimal128(
        ifNull(
            sum(toFloat64(c.close_usd) * toFloat64(c.volume_quote_usd))
                / nullIf(sum(toFloat64(c.volume_quote_usd)), 0),
            0),
        14)                                                 AS vwap_24h,
    ifNull(
        accurateCastOrNull(
            toDecimal256(argMax(c.close_usd, c.timestamp), 14)
                * toDecimal256(ifNull(max(s.token_supply), 0), 14),
            'Decimal128(14)'),
        toDecimal128(0, 14))                                AS market_cap_usd,
    now()                                                   AS updated_at
FROM prices.price_ohlcv_1m AS c FINAL
LEFT JOIN prices.asset_supply AS s FINAL ON s.asset_id = c.asset_id
WHERE c.timestamp >= now() - INTERVAL 24 HOUR
GROUP BY c.asset_id;
