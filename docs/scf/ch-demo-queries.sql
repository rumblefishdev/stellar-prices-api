-- SCF Milestones 1 and 2 — ClickHouse demo / evidence queries
-- (stellar-prices-api)
--
-- Queries (1)-(9) back milestone-1-evidence.md. Queries (10)-(22), from the
-- MILESTONE 2 banner onward, back milestone-2-evidence.md.
--
-- Run by the operator against PRODUCTION ClickHouse over mTLS, from their own
-- shell. Outputs are pasted back into milestone-1-evidence.md (replacing the
-- <TODO: paste output> markers) and shown on camera during the video.
--
-- These queries are READ-ONLY. They never write, drop, or alter anything.
--
-- Connection: see docs/runbooks/running-ingestion-components.md for the mTLS
-- client-certificate invocation. Database: `prices` on ch-prod-01
-- (ch.sorobanscan.rumblefish.dev).
--
-- Query numbering matches the figure numbering in milestone-1-evidence.md §5.

-- ---------------------------------------------------------------------------
-- AC 2 — schema matches the design
-- ---------------------------------------------------------------------------

-- (1) Every table, materialised view, and read view in the prices database.
--     Expect: base tables + mv_current_prices + the 6 mv_ohlcv_* rollup MVs
--             + 6 read views.
--
--     NOTE — the 6 mv_ohlcv_* rollup MVs (mv_ohlcv_1m_to_15m … mv_ohlcv_1w_to_1M)
--     ARE PRESENT and running in APPEND mode. They were briefly dropped after a
--     replace-mode incident (in replace mode a bounded refresh overwrote coarse
--     history the backfill had pre-rolled); task 0095 recreated them in APPEND
--     mode on 2026-07-17, so they roll live candles forward without clobbering
--     history. This matches the evidence document (AC 2 and section 6).
SHOW TABLES FROM prices;

-- (2) The 1-minute candle table DDL.
--     Expect: ReplacingMergeTree(version)
--             ORDER BY (asset_id, quote_asset_id, source, timestamp)
--             PARTITION BY toYYYYMM(timestamp)
SHOW CREATE TABLE prices.price_ohlcv_1m;

-- (2b) Optional — rollups are database objects, not application code.
--      Returns mv_current_prices AND the 6 mv_ohlcv_* rollup MVs, all
--      MaterializedView engine (see the note on query (1)). The DDL that defines
--      them is packages/prices-clickhouse/schema/rollups.sql.
SELECT name, engine
FROM system.tables
WHERE database = 'prices' AND engine = 'MaterializedView'
ORDER BY name;

-- ---------------------------------------------------------------------------
-- AC 3 — 24 h of continuous 1-minute candles for >= 20 major assets
-- ---------------------------------------------------------------------------

-- (3) How many distinct assets have candles in the last 24 h?
--     Expect: >= 20.
SELECT count(DISTINCT asset_id) AS assets_with_candles
FROM prices.price_ohlcv_1m FINAL
WHERE timestamp >= now() - INTERVAL 24 HOUR;

-- (4) Per-asset, per-source coverage and largest gap for the named majors.
--     Expect: the deepest markets (XLM, AQUA on sdex) stay within the criterion's
--     two-candle bound (largest_gap_minutes <= 2; the exact value drifts 1-2
--     day to day). Thinner majors (BTC/ETH/EURC) show larger gaps that track
--     their lower trade frequency — a quiet market, not a broken indexer (a candle
--     exists only where a trade occurred that minute). USDC is sparse as a BASE
--     because it is almost always a quote asset. See the evidence doc's note
--     under AC 3.
--
--     Two correctness details, both REQUIRED:
--       * Ticker codes are NOT unique on Stellar — anyone can issue an asset
--         named 'BTC'/'USDC'/etc. The `canonical` CTE pins each ticker to its
--         most-active asset_id first; a plain asset_code join lets an illiquid
--         impostor's gap dominate max().
--       * lagInFrame's 3rd arg (default = the current row's timestamp): without
--         it the first candle in the window has no predecessor and lagInFrame
--         returns the 1970 epoch, so its gap reads as ~29.7M minutes.
WITH canonical AS (
    SELECT asset_code, argMax(asset_id, cnt) AS asset_id
    FROM (
        SELECT a.asset_code, p.asset_id, count() AS cnt
        FROM prices.price_ohlcv_1m AS p FINAL
        INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
        WHERE p.timestamp >= now() - INTERVAL 24 HOUR
          AND a.asset_code IN ('XLM', 'USDC', 'EURC', 'AQUA', 'BTC', 'ETH')
        GROUP BY a.asset_code, p.asset_id
    )
    GROUP BY asset_code
)
SELECT
    c.asset_code,
    p.source,
    count()            AS candles_24h,
    max(p.gap_minutes) AS largest_gap_minutes,
    min(p.timestamp)   AS first_candle,
    max(p.timestamp)   AS last_candle
FROM (
    SELECT
        asset_id,
        source,
        timestamp,
        dateDiff('minute', lagInFrame(timestamp, 1, timestamp) OVER w, timestamp) AS gap_minutes
    FROM prices.price_ohlcv_1m FINAL
    WHERE timestamp >= now() - INTERVAL 24 HOUR
    WINDOW w AS (PARTITION BY asset_id, source ORDER BY timestamp)
) AS p
INNER JOIN canonical AS c ON c.asset_id = p.asset_id
GROUP BY c.asset_code, p.source
ORDER BY c.asset_code, p.source;

-- (5) Candles by source — proves both the SDEX path and the Soroban AMM
--     extractors are live. Expect: sdex, plus whichever of
--     soroswap / aquarius / phoenix traded in the window.
SELECT
    source,
    count()                  AS candles_24h,
    count(DISTINCT asset_id) AS assets
FROM prices.price_ohlcv_1m FINAL
WHERE timestamp >= now() - INTERVAL 24 HOUR
GROUP BY source
ORDER BY candles_24h DESC;

-- ---------------------------------------------------------------------------
-- AC 6 — earliest_data_available reaches ~6 months back
-- ---------------------------------------------------------------------------

-- (6) Depth of history actually INGESTED in the store.
--     Expect: every source ~820-885 days, back to roughly Soroban activation
--     (2024-02-20) — ~5x the ~180-day (six-month) bar.
--
--     NB — this is the ingested/queryable depth, which is the honest number.
--     GET /backfill/status reports sdex.earliest_data_available ~2015-11: that
--     is the earliest ledger available TO BACKFILL (the archive floor), NOT the
--     earliest candle ingested. The SDEX pre-Soroban tail is still backfilling
--     toward that floor (current_ledger sits at the Soroban-activation boundary).
--
--     NOTE — query the COARSE table, not price_ohlcv_1m. `1m` is a transient
--     feeder on a 7-day retention (cleanup-worker/src/lib.rs drops its monthly
--     partitions); `price_ohlcv_{1h,4h,1d,1w,1M}` are retained forever and are
--     the permanent store of record. Asking `1m` for six months of history
--     returns only the last few days and looks like a failure.
SELECT
    source,
    min(timestamp)                         AS earliest_candle,
    max(timestamp)                         AS latest_candle,
    dateDiff('day', min(timestamp), now()) AS days_of_history
FROM prices.price_ohlcv_1d FINAL
GROUP BY source
ORDER BY source;

-- ---------------------------------------------------------------------------
-- Supporting context (optional on camera; useful if a reviewer asks)
-- ---------------------------------------------------------------------------

-- (7) Live ingestion frontier — how far behind the network tip are we?
--     The durable cursor (task 0064) is versioned on `ledger`, so it can only
--     move forward.
SELECT
    ledger                                     AS cursor_ledger,
    updated_at                                 AS cursor_updated_at,
    dateDiff('second', updated_at, now())      AS cursor_age_sec
FROM prices.ingest_cursor FINAL;

-- (8) Freshness of the candle frontier by source.
SELECT
    source,
    max(timestamp)                              AS newest_candle,
    dateDiff('second', max(timestamp), now())   AS behind_sec
FROM prices.price_ohlcv_1m FINAL
GROUP BY source
ORDER BY source;

-- (9) Backfill progress, dual-stream — the table behind GET /v1/backfill/status.
SELECT *
FROM prices.backfill_progress FINAL
ORDER BY task_name;

-- (10) Oracle reference prices are ingested as a cross-reference and never set
--      a CANDLE price. They do set the published current price for the small
--      number of assets that cannot be priced from trades — one asset, USDC, as
--      of 2026-09-08 — and `current_prices.method` names the provenance on every
--      row (`traded`, `oracle`, or unset). Query (26) below counts them.
--      Shown here to demonstrate the reference feeds are live.
--      Expect two oracles: reflector (SEP-40, also drives quote->USD conversion)
--      and redstone. Neither ever sets a candle price.
SELECT
    oracle_name,
    count()                                    AS rows,
    count(DISTINCT asset_id)                   AS assets,
    max(timestamp)                             AS newest,
    dateDiff('second', max(timestamp), now())  AS behind_sec
FROM prices.oracle_prices FINAL
GROUP BY oracle_name;

-- (11) The AMM pool registry that Soroswap swap decoding depends on
--      (Soroswap swap events carry no token addresses — the pool must be
--      resolved to its token pair).
SELECT venue, count() AS pools
FROM prices.pool_registry FINAL
GROUP BY venue
ORDER BY venue;

-- ===========================================================================
-- MILESTONE 2 — Public API (added 2026-09-07, task 0128)
-- ===========================================================================
--
-- Everything below backs a claim in milestone-2-evidence.md. Same rules as
-- above: READ-ONLY, run by the operator against PRODUCTION ClickHouse over
-- mTLS, database `prices`.
--
-- Most of Milestone 2's evidence is HTTP rather than SQL, because the
-- deliverable is an API. The commands for those live next to their claims in
-- milestone-2-evidence.md §5. What follows is the part a reviewer can only
-- check from the database side.
--
-- Query numbering continues from the Milestone 1 set.

-- ---------------------------------------------------------------------------
-- AC 4 — VWAP verifiable against raw price_ohlcv rows
-- ---------------------------------------------------------------------------

-- (10) The published row for one asset, as the API serves it.
--      Compare `sources` and `vwap_24h` against query (11) recomputed by hand.
--      Expect: `sources` names each venue with its own price and 24h volume.
SELECT asset_id, price_usd, price_xlm, vwap_24h, volume_24h_usd,
       change_24h_pct, sources, method, updated_at
FROM prices.current_prices FINAL
WHERE asset_id = 4;

-- (11) The raw candles the published figure is derived from.
--      This is the capture that task 0123 re-aggregated in plain Python,
--      independent of the materialised view's own SQL. Pin the window: the MV
--      refreshes every minute, so an unpinned comparison proves nothing.
--      Expect: ~29,000 rows for six assets over 24 h.
SELECT timestamp, asset_id, quote_asset_id, source,
       close, close_usd, volume_quote_usd
FROM prices.price_ohlcv_1m FINAL
WHERE asset_id IN (4, 5, 70, 108, 430, 741)
  AND timestamp >= toDateTime('2026-08-25 13:22:00')
  AND timestamp <= toDateTime('2026-08-26 13:22:00');

-- (12) Ties across quote legs — why "the latest priced close" is a SET.
--      Expect: several assets with cnt > 1. This is common, not exotic, and it
--      is why the reconciliation asserts set membership rather than equality.
SELECT asset_id, max(timestamp) AS newest_priced, count() AS cnt
FROM prices.price_ohlcv_1m FINAL
WHERE close_usd > 0
  AND timestamp >= now() - INTERVAL 1 HOUR
GROUP BY asset_id
HAVING cnt > 1
ORDER BY cnt DESC
LIMIT 20;

-- ---------------------------------------------------------------------------
-- AC 5 — earliest_data_available <= 2022-01-01, reconciled four ways
-- ---------------------------------------------------------------------------

-- (13) What the API reports, read from the row the endpoint reads.
--      Expect: sdex.earliest_data_available = 2015-11-18 03:47:00.
SELECT task_name, status, current_ledger, target_ledger,
       last_push_at, completed_at, earliest_data_available
FROM prices.backfill_progress FINAL
ORDER BY task_name;

-- (14) What the candles actually contain — the independent check.
--      The stored value above is a monotonic high-water mark and CANNOT correct
--      itself downward, so an overstatement would be permanent and invisible.
--      That is the whole reason this query exists.
--      Expect: 2015-11-18, matching (13).
SELECT min(timestamp) AS oldest_sdex_candle,
       max(timestamp) AS newest_sdex_candle,
       count()        AS daily_candles
FROM prices.price_ohlcv_1d
WHERE source = 'sdex';

-- (15) Oldest ACTIVE partition on every candle tier — the third view.
--      Expect: 201511 on all seven tiers.
SELECT table, min(partition) AS oldest_partition
FROM system.parts
WHERE database = 'prices'
  AND table LIKE 'price_ohlcv_%'
  AND active
GROUP BY table
ORDER BY table;

-- (16) Continuity, month by month, across the criterion's window.
--      Depth alone is not coverage. Expect: days_with_candles equal to the
--      calendar length of every month from 2022-01 onward, leap day included.
SELECT toYYYYMM(timestamp)            AS month,
       uniqExact(toDate(timestamp))   AS days_with_candles,
       count()                        AS candles,
       uniqExact(asset_id)            AS assets
FROM prices.price_ohlcv_1d
WHERE source = 'sdex'
  AND timestamp >= toDate('2022-01-01')
  AND timestamp <  today()
GROUP BY month
ORDER BY month;

-- ---------------------------------------------------------------------------
-- AC 6 — the USDC exclusion, stated with the data behind it
-- ---------------------------------------------------------------------------

-- (17) Why USDC cannot serve as the spot-check asset.
--      Expect: zero candles. USDC is our top-preference QUOTE asset, so pairs
--      canonicalise as base=X/quote=USDC and USDC essentially never appears as
--      a base leg. With no candles of its own, the published series is filled
--      from the peg. See milestone-2-evidence.md §5 AC 6 and task 0265.
SELECT count() AS usdc_base_leg_candles
FROM prices.price_ohlcv_1d
WHERE asset_id = (SELECT asset_id FROM prices.assets FINAL
                  WHERE asset_code = 'USDC'
                    AND issuer_address = 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN'
                  LIMIT 1);

-- (18) The control: the substitute spot-check assets DO have measured trades
--      on the dates the report compares against Binance.
--      Expect: real trade_count and non-zero close_usd on each date.
SELECT toDate(timestamp) AS day, asset_id, source,
       close, close_usd, trade_count
FROM prices.price_ohlcv_1d
WHERE asset_id = 4
  AND toDate(timestamp) IN ('2022-01-03', '2022-06-15', '2023-03-11',
                            '2024-07-01', '2026-06-15')
ORDER BY day, source;

-- (19) ⚠️ The caveat the report states rather than hides: USD coverage is thin,
--      and thinnest where it is oldest. The spot-check assets were chosen from
--      the priced majority, deliberately.
--      Expect: ~36% of 2022 daily SDEX candles carry close_usd = 0, ~13% of
--      2021, and effectively all of 2015-2020.
SELECT toYear(timestamp)                              AS year,
       count()                                        AS candles,
       countIf(close_usd = 0)                         AS unpriced,
       round(100 * countIf(close_usd = 0) / count(), 2) AS unpriced_pct
FROM prices.price_ohlcv_1d
WHERE source = 'sdex'
GROUP BY year
ORDER BY year;

-- ---------------------------------------------------------------------------
-- Work items without a numbered criterion (evidence doc §6)
-- ---------------------------------------------------------------------------

-- (20) The minimum-volume source threshold, and why it had to be CONDITIONAL.
--      Applied unconditionally a $100 floor would have blanked the VWAP on
--      96.5% of priced assets, because most venues carry a dollar a day or
--      less. Expect: the vast majority of venues in the two lowest buckets.
SELECT multiIf(v <= 1, '1. <= $1',
               v <= 10, '2. $1-10',
               v <= 100, '3. $10-100',
               v <= 1000, '4. $100-1k',
               v <= 10000, '5. $1k-10k',
                           '6. > $10k')  AS bucket,
       count()                            AS venues
FROM (
    SELECT asset_id, source, sum(volume_quote_usd) AS v
    FROM prices.price_ohlcv_1m FINAL
    WHERE timestamp >= now() - INTERVAL 24 HOUR
      AND close_usd > 0
    GROUP BY asset_id, source
)
GROUP BY bucket
ORDER BY bucket;

-- (21) The outlier filter can empty the source set while a price still
--      publishes — a documented property, not a defect. The median
--      interpolates on an even count, so four values 1,1,3,3 give a median of
--      2 and every element deviates by 50%.
--      Expect: a non-zero count here is EXPECTED. See §6.2 and task 0238.
SELECT count() AS priced_but_no_sources
FROM prices.current_prices FINAL
WHERE price_usd > 0
  AND (sources = '' OR sources = '{}');

-- (22) Aquarius appears as a named source with real volume (§6.3).
--      Expect: aquarius present alongside sdex/soroswap/phoenix.
SELECT source, count() AS candles, round(sum(volume_quote_usd), 2) AS volume_24h_usd
FROM prices.price_ohlcv_1m FINAL
WHERE timestamp >= now() - INTERVAL 24 HOUR
GROUP BY source
ORDER BY volume_24h_usd DESC;

-- (26) Provenance of every published price. The `method` column is what stops
--      an oracle-derived number being read as a traded one.
--      Expect: `traded` dominant, `oracle` on the handful that cannot be traded-
--      priced, and a blank group for rows written before the column existed.
SELECT
    method,
    count() AS assets
FROM prices.current_prices FINAL
GROUP BY method
ORDER BY assets DESC;
