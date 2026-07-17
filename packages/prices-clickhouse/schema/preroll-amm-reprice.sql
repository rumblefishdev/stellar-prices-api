-- prices coarse PRE-ROLL — INCREMENTAL, NON-TRUNCATING, scoped to the
-- Soroban-era AMM sources corrected by the events-sourced reprice (task 0097).
--
-- ⚠️ DRAFT — NOT YET RUN AGAINST PROD. Two open decisions are marked
--    `OPEN QUESTION` below and MUST be settled (with the pre-flight measurements
--    in §0) before this is executed. Do not run it on the strength of the
--    header comments alone.
--
-- WHY THIS EXISTS — neither existing script fits:
--   * `preroll.sql` is a FULL rebuild expecting TRUNCATE-d coarse tables. Running
--     it would wipe every already-pre-rolled coarse row — the exact 0090
--     history-loss incident. NEVER use it here.
--   * `preroll-incremental.sql` is bounded to the pre-Soroban SDEX tail
--     `[genesis, activation)` — the complement of the range 0097 repriced. It
--     would re-roll the wrong side of the boundary and touch nothing 0097 wrote.
--   (The 0097 runbook `events-sourced-amm-reprice.md` §4 currently points at the
--   pre-Soroban script — that reference is WRONG and is fixed alongside this file.)
--
-- WHAT 0097 CHANGED, AND SO WHAT THIS RE-ROLLS:
--   The reprice rewrites `price_ohlcv_1m` for the AMM sources only
--   (`aquarius`, `phoenix`, `soroswap`) over `[activation, SDEX-live-floor)` —
--   ledgers 50457424..63352611. Soroswap is the headline: it had ~zero candles
--   before the 0096 extractor fix (swap action sits in topic[1], not topic[0]),
--   so ~824k historical swaps produced nothing. The coarse tables still reflect
--   that gap and must be re-rolled from the corrected `1m`.
--
-- SAFETY — why this cannot disturb SDEX coarse:
--   Every OHLCV table is `ORDER BY (asset_id, quote_asset_id, source, timestamp)`
--   with `source` IN the key, so an `sdex` row and an `aquarius` row for the same
--   minute+pair are DISTINCT rows, never RMT-merge candidates. Scoping every
--   statement to `source IN ('aquarius','phoenix','soroswap')` therefore makes
--   SDEX coarse — including the expensive pre-Soroban tail — untouchable here.
--
-- MEMORY (0090 + 0097 findings): ch-prod-01 enforces a ~5.59 GiB per-query quota
--   (0097's coverage probe hit it as MEMORY_LIMIT_EXCEEDED on a full-range scan).
--   The limit is PER-QUERY, so STAGE 1 is chunked by year. Note the source filter
--   does NOT prune by primary key — `source` is the 3rd key column, not a prefix,
--   so a source-only predicate cannot seek; it filters after granule selection.
--   Year-chunking (PARTITION BY toYYYYMM) is what actually bounds the read. If a
--   statement still exceeds the quota: split that year in halves, or drop FINAL
--   on the dup-free intermediate stages (1h/4h/1d/1w/1M) as 0090 did — they read
--   single-pass GROUP BY output and carry no duplicates.
--
-- IDEMPOTENCY: re-runnable; RMT collapses the duplicate-PK rows a second run adds.
--   Never TRUNCATE.
--
-- Correctness (task 0059): the bucket key is aliased `AS timestamp` and SHADOWS
--   the source `timestamp`; argMin/argMax MUST reference the qualified
--   `t.timestamp` (FROM ... AS t) or open/close/close_usd tie-break to an
--   arbitrary row. Column order matches the target table (INSERT ... SELECT maps
--   by position); kept identical to preroll.sql / preroll-incremental.sql.
--
-- =====================================================================
-- OPEN QUESTION 1 — RMT version TIES on corrected aquarius/phoenix rows.
--   `version = max(ledger_sequence*1000 + operation_index)` over a candle's ticks
--   (prices-ingest-core/src/bucket.rs:48,81). RMT keeps max(version); on a TIE it
--   keeps "the last row in the selection", which is NOT a contractual guarantee
--   about our insert winning.
--     * soroswap: SAFE regardless — there are ~no pre-existing rows to tie with,
--       so these are clean inserts on new keys.
--     * aquarius/phoenix: a tie with DIFFERENT values is possible where two pools
--       of the same venue map to the SAME canonical pair in the same minute (e.g.
--       Aquarius stable + constant-product on one pair) and the previously-
--       unresolved pool's trades all sit EARLIER than the already-counted pool's
--       last trade. Then max(version) is unchanged while volume grew, so the
--       corrected row does not reliably outrank the stale one.
--   DECIDE BEFORE RUNNING: accept (narrow, and our insert is last so it wins in
--   practice), or scope this pre-roll to `source = 'soroswap'` only, or verify
--   empirically post-run per §5. Measurement in §0.4 sizes the exposure.
--
-- OPEN QUESTION 2 — buckets STRADDLING `{end_ts}` (1w / 1M especially).
--   `{end_ts}` is an arbitrary ledger close time, not bucket-aligned. Any bucket
--   spanning it is rolled here from its `< {end_ts}` slice ONLY, so it is a
--   PARTIAL aggregate of a bucket whose remainder is live-era data.
--   Whether that partial can clobber a complete row depends on what maintains
--   coarse for the live era RIGHT NOW — and per 0090 the six replace-mode
--   `mv_ohlcv_*` MVs were DROPPED (restoring them as APPEND is task 0095). If
--   nothing currently maintains live coarse, there is no competing row and the
--   straddling bucket is simply incomplete until the next pre-roll; if something
--   does, the live row carries HIGHER versions (later ledgers) and wins, leaving
--   our repriced slice out of that bucket.
--   DECIDE BEFORE RUNNING: confirm §0.3, then either accept the residual (as the
--   pre-Soroban script does at its activation boundary) or repair the straddling
--   buckets afterwards by re-rolling them unbounded from full `15m`.
-- =====================================================================

-- =====================================================================
-- §0 PRE-FLIGHT — run these FIRST; they supply the params and settle the
--   OPEN QUESTIONs above. Record the outputs in the 0097 task notes.
-- =====================================================================
--
-- 0.1 — {start_ts}: close time of the activation ledger (50457424).
--   SELECT toDateTime(closed_at) FROM default.ledgers WHERE sequence = 50457424;
--
-- 0.2 — {end_ts}: close time of the SDEX live floor - 1 (63352611).
--   SELECT toDateTime(closed_at) FROM default.ledgers WHERE sequence = 63352611;
--
-- 0.3 — OPEN QUESTION 2: does anything maintain live-era coarse today?
--   SELECT name FROM system.tables WHERE database = 'prices' AND name LIKE 'mv_ohlcv%';
--   -- plus the newest coarse row per source, to see if it tracks the live tip:
--   SELECT source, max(timestamp) FROM prices.price_ohlcv_1d
--    WHERE source IN ('aquarius','phoenix','soroswap') GROUP BY source;
--
-- 0.4 — OPEN QUESTION 1: how many aquarius/phoenix coarse rows could tie?
--   -- Same-source, same-pair, same-minute keys already present in coarse that
--   -- the reprice also rewrote. Zero here ⇒ OPEN QUESTION 1 is moot.
--   SELECT source, count() FROM prices.price_ohlcv_1m FINAL
--    WHERE source IN ('aquarius','phoenix')
--      AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
--    GROUP BY source;
--
-- 0.5 — disk headroom (the 0097 runbook's Checkpoint C):  df -h /var/lib/docker
--
-- PARAMS: pass both explicitly, e.g.
--   clickhouse-client --param_start_ts='2024-02-20 00:00:00' \
--                     --param_end_ts='2026-07-08 00:00:00' \
--                     --queries-file preroll-amm-reprice.sql
-- =====================================================================

-- =====================================================================
-- STAGE 1 — 15m <- 1m, CHUNKED BY YEAR (heavy; FINAL to dedup re-ingests).
--   The Soroban era spans activation (~2024-02) to the SDEX live floor
--   (~2026-07), so: a partial 2024 from {start_ts}, a full 2025, and a partial
--   2026 to {end_ts}. Every statement is scoped to the AMM sources.
-- =====================================================================

-- 2024, from activation
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < '2025-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2025
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-01-01' AND t.timestamp < '2026-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- 2026, to the SDEX live floor
INSERT INTO prices.price_ohlcv_15m
SELECT toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-01-01' AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- =====================================================================
-- STAGE 2 — coarse chain 15m -> 1h -> 4h -> 1d -> 1w -> 1M, each level FROM the
--   previous, scoped to the same sources and window. Run STAGE 1 to completion
--   first. These read progressively smaller tables, so one bounded statement per
--   level is fine (year-chunk 1h<-15m like STAGE 1 if the quota bites; 1h/4h/1d
--   buckets never straddle a calendar year so that is safe. Do NOT year-chunk
--   1w/1M — their buckets DO straddle year boundaries).
--   See OPEN QUESTION 2 re: the bucket straddling {end_ts}.
-- =====================================================================

INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1w
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1M
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMax(close_usd, t.timestamp) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1w AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- =====================================================================
-- §5 VERIFY (after the chain completes; before re-enabling cleanup)
-- =====================================================================
--
-- 5.1 — soroswap now present at every granularity (the 0096/0097 headline).
--   Pre-reprice this was 0 rows at every level. Any level still 0 ⇒ that stage
--   did not run.
--   SELECT '1m'  AS g, count() FROM prices.price_ohlcv_1m  FINAL WHERE source='soroswap'
--     AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
--   UNION ALL SELECT '15m', count() FROM prices.price_ohlcv_15m FINAL WHERE source='soroswap'
--     AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
--   UNION ALL SELECT '1h',  count() FROM prices.price_ohlcv_1h  FINAL WHERE source='soroswap'
--     AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
--   UNION ALL SELECT '1d',  count() FROM prices.price_ohlcv_1d  FINAL WHERE source='soroswap'
--     AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime};
--
-- 5.2 — conservation: rolled volume must equal 1m volume per source. A coarse
--   total BELOW 1m means buckets were lost or an RMT tie kept a stale row
--   (OPEN QUESTION 1); ABOVE means double-counting.
--   SELECT source, sum(volume_base) FROM prices.price_ohlcv_1m FINAL
--    WHERE source IN ('aquarius','phoenix','soroswap')
--      AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
--    GROUP BY source;
--   -- then the same against price_ohlcv_1d; the two must match per source.
--
-- 5.3 — SDEX coarse untouched (the safety property this file rests on). Capture
--   this BEFORE the run too, and diff.
--   SELECT count(), sum(volume_base) FROM prices.price_ohlcv_1d FINAL WHERE source='sdex';
