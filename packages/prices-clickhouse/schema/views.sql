-- prices read-surface VIEWS (task 0061 Step 5 — the USD-close series + the USD
-- reference companion). Applied after init.sql tables; plain views (no special
-- ClickHouse version needed, unlike the refreshable rollup MVs).
--
-- Applier contract (same as init.sql): the Rust splitter splits naively on `;`
-- and strips `-- …` line comments only. Keep this file free of inline string
-- literals containing `;` and of block comments.
--
-- ## Design references
--   - R-historical-usd-close-design.md §8 (view), §12.2 (natural-identity key),
--     §12.3 (NULL + status discriminator + usd_reference companion)
--   - close_usd is BAKED into the candles at enrichment time (§12.1); these views
--     read that column — they do NOT join the retention-capped oracle_prices.
--
-- ## Public key = natural Stellar identity, never asset_id (§12.2)
-- asset_id is an internal UInt32 surrogate. These views resolve it to the
-- portable identity via prices.assets and expose:
--   asset_kind ∈ ('native','credit','contract'),
--   asset_code, issuer_address, contract_address.
-- native XLM → ('native','XLM','',''); classic → ('credit', code, issuer, '');
-- SAC / Soroban token → ('contract','','', contract_address).
--
-- ## Grain variants (1h + 1d)
-- The series + reference are provided at two grains, both on forever-retained
-- OHLCV tables (1h and 1d carry close_usd via the rollup chain):
--   prices.price_usd_series      / prices.usd_reference       — daily buckets
--   prices.price_usd_series_1h   / prices.usd_reference_1h     — hourly buckets
-- Pair a series with its same-grain reference when classifying status. Hourly
-- serves read-time TVL keyed to a ledger's closed_at without collapsing a whole
-- day to one close; daily is cheaper for long-range charts. Grain selection is
-- the caller's (the explorer joins whichever grain its query needs).
--
-- ## Read-time status discriminator (§12.3) — computed by the reader
-- A view cannot enumerate (asset × bucket) combinations that never traded, so
-- `no_asset_price` is a read-time condition, not a stored column. For a lookup
-- of (identity I, bucket T), LEFT JOIN price_usd_series against usd_reference
-- (at the matching grain):
--
--   status = ok             -- row present in price_usd_series for (I, T)
--          | no_asset_price  -- (I, T) absent, BUT usd_reference has bucket T
--                            --   (the USD reference IS up; partial TVL is valid)
--          | no_reference    -- (I, T) absent AND usd_reference has no bucket T
--                            --   (systemic blackout — every XLM-pivot asset NULL)
--
-- close_usd is always returned as a value-or-absent: a miss is a missing row
-- (NULL after the reader's LEFT JOIN), never an error and never a dropped row.

----------------------------------------------------------------------
-- prices.usd_reference — per-bucket USD reference availability.
-- The volume-weighted XLM/USDC close (= XLM's USD price under the USDC≡$1 peg)
-- per day bucket. A bucket's PRESENCE is the durable "USD reference is up at T"
-- signal the reader LEFT JOINs for systemic-blackout detection. Identity-resolved
-- (not asset_id) so it survives asset-id reassignment. Reads `close` (always
-- present from the backfill), independent of enrichment timing.
----------------------------------------------------------------------

CREATE VIEW IF NOT EXISTS prices.usd_reference AS
SELECT
    p.timestamp AS bucket,
    CAST(sum(toFloat64(p.close) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS xlm_usd
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS base  FINAL ON base.asset_id  = p.asset_id
INNER JOIN prices.assets AS quote FINAL ON quote.asset_id = p.quote_asset_id
WHERE base.asset_code = 'XLM' AND base.issuer_address = '' AND base.contract_address = ''
  AND quote.asset_code = 'USDC'
  AND quote.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND p.close > 0
GROUP BY p.timestamp;

----------------------------------------------------------------------
-- prices.price_usd_series — one USD close per (natural identity, day bucket).
-- The cross-source/cross-quote collapse: volume-weighted close_usd over every
-- candle of the asset in the bucket (ADR 0004 per-source rows merge at read
-- time). Only priced rows (close_usd > 0) — status 'ok'; misses are absent and
-- classified by the reader against usd_reference (see header).
----------------------------------------------------------------------

CREATE VIEW IF NOT EXISTS prices.price_usd_series AS
SELECT
    multiIf(
        a.contract_address != '', 'contract',
        a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
        'credit') AS asset_kind,
    a.asset_code       AS asset_code,
    a.issuer_address   AS issuer_address,
    a.contract_address AS contract_address,
    p.timestamp        AS bucket,
    CAST(sum(toFloat64(p.close_usd) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS close_usd
FROM prices.price_ohlcv_1d AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.close_usd > 0
GROUP BY asset_code, issuer_address, contract_address, bucket;

----------------------------------------------------------------------
-- Hourly-grain variants — identical shape/semantics to the daily views above,
-- reading price_ohlcv_1h (also forever-retained, also carries close_usd). For
-- read-time TVL keyed to a ledger's closed_at at hourly resolution. Filter by
-- bucket range so the predicate pushes down to the _1h scan (bounded by the
-- window, not full history); promote to a materialized table only if measured
-- read latency demands it (design note §6).
----------------------------------------------------------------------

CREATE VIEW IF NOT EXISTS prices.usd_reference_1h AS
SELECT
    p.timestamp AS bucket,
    CAST(sum(toFloat64(p.close) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS xlm_usd
FROM prices.price_ohlcv_1h AS p FINAL
INNER JOIN prices.assets AS base  FINAL ON base.asset_id  = p.asset_id
INNER JOIN prices.assets AS quote FINAL ON quote.asset_id = p.quote_asset_id
WHERE base.asset_code = 'XLM' AND base.issuer_address = '' AND base.contract_address = ''
  AND quote.asset_code = 'USDC'
  AND quote.issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
  AND p.close > 0
GROUP BY p.timestamp;

CREATE VIEW IF NOT EXISTS prices.price_usd_series_1h AS
SELECT
    multiIf(
        a.contract_address != '', 'contract',
        a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
        'credit') AS asset_kind,
    a.asset_code       AS asset_code,
    a.issuer_address   AS issuer_address,
    a.contract_address AS contract_address,
    p.timestamp        AS bucket,
    CAST(sum(toFloat64(p.close_usd) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS close_usd
FROM prices.price_ohlcv_1h AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.close_usd > 0
GROUP BY asset_code, issuer_address, contract_address, bucket;
