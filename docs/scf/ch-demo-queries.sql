-- SCF Milestone 1 — ClickHouse demo / evidence queries (stellar-prices-api)
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
--     Expect: largest_gap_candles <= 2 for the liquid majors.
--     A gap only exists where no trade occurred in that minute — a quiet
--     market, not a broken indexer. See the evidence doc's note under AC 3.
SELECT
    a.asset_code,
    p.source,
    count()            AS candles_24h,
    max(p.gap_minutes) AS largest_gap_candles,
    min(p.timestamp)   AS first_candle,
    max(p.timestamp)   AS last_candle
FROM (
    SELECT
        asset_id,
        source,
        timestamp,
        dateDiff('minute', lagInFrame(timestamp) OVER w, timestamp) AS gap_minutes
    FROM prices.price_ohlcv_1m FINAL
    WHERE timestamp >= now() - INTERVAL 24 HOUR
    WINDOW w AS (PARTITION BY asset_id, source ORDER BY timestamp)
) AS p
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE a.asset_code IN ('XLM', 'USDC', 'EURC', 'AQUA', 'BTC', 'ETH')
GROUP BY a.asset_code, p.source
ORDER BY a.asset_code, p.source;

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

-- (6) Depth of history actually in the store. Cross-checks the
--     earliest_data_available value that GET /v1/backfill/status reports.
--     Expect: days_of_history >= ~180 for sdex; the AMM sources reach Soroban
--     activation (2024-02-20), i.e. ~880 days.
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

-- (10) Oracle reference prices are ingested but never set a price.
--      Shown only to demonstrate the reference feed is live.
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
