-- prices coarse PRE-ROLL — INCREMENTAL, NON-TRUNCATING, scoped to the
-- Soroban-era AMM sources corrected by the events-sourced reprice (task 0097).
--
-- STATUS: pre-flight COMPLETE (2026-07-17, prod). Both former OPEN QUESTIONs are
--    settled by measurement — see §0. Params are known:
--      start_ts = '2024-02-20 17:00:10'   (close of ledger 50457424)
--      end_ts   = '2026-07-06 09:35:16'   (close of ledger 63352611)
--    Not yet executed against prod.
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
-- RESOLVED (2026-07-17) — RMT version TIES: why phoenix gets a DELETE first.
--   `version = max(ledger_sequence*1000 + operation_index)` over a bucket's rows.
--   RMT keeps max(version); on a TIE it keeps "the last row in the selection",
--   which is NOT a contractual guarantee. Per source, measured:
--     * soroswap — coarse rows do NOT EXIST (absent from price_ohlcv_1d
--       entirely). Every row this script inserts lands on a NEW key, unopposed.
--       No delete needed.
--     * aquarius — coarse exists and its values are UNCHANGED by the reprice, so
--       a tie is harmless either way. No delete needed.
--     * phoenix  — coarse exists (1d tip 2026-07-09) and IS STALE: task 0097
--       recovered 5,175 seven-event swaps. Those swaps mostly sit MID-bucket, so
--       they raise volume/trade_count WITHOUT raising the bucket's max ledger →
--       the corrected row carries the SAME version as the stale one. On that tie
--       the fix might silently NOT land in coarse — the store of record — while
--       `1m` looks perfect. STAGE 0 therefore DELETEs phoenix rows in-window
--       first, making the outcome deterministic instead of a coin flip.
--
-- RESOLVED (2026-07-17) — buckets STRADDLING {end_ts} are harmless.
--   `system.tables` has NO `mv_ohlcv*` rows: the six replace-mode rollup MVs were
--   dropped in 0090 (restoring them as APPEND is task 0095), so NOTHING maintains
--   live-era coarse today. There is no competing row for the bucket spanning
--   {end_ts}, hence nothing our partial slice can clobber and nothing that can
--   outrank it. Accepted residual: the 1w/1M buckets containing {end_ts} reflect
--   only their <= {end_ts} slice until the next pre-roll extends them. For
--   soroswap the 2026-07-06 day/week/month are partial for a second reason
--   anyway — live emitted NO soroswap candles until the 0096 fix deployed
--   2026-07-15, so [63352612, deploy] is an uncovered gap owned by task 0099.
--
-- =====================================================================
-- §0 PRE-FLIGHT — ALREADY RUN (2026-07-17, prod). Recorded for re-runs.
-- =====================================================================
--
-- 0.1 {start_ts} = 2024-02-20 17:00:10 — close of activation ledger 50457424.
-- 0.2 {end_ts}   = 2026-07-06 09:35:16 — close of ledger 63352611 (SDEX live floor - 1).
--     SELECT sequence, min(closed_at) FROM default.ledgers
--      WHERE sequence IN (50457424, 63352611) GROUP BY sequence;
--     (`min` collapses the ledgers RMT, as source.rs does; closed_at is a DateTime.)
--
-- 0.3 Live-era coarse maintenance: NONE.
--     SELECT name FROM system.tables WHERE database='prices' AND name LIKE 'mv_ohlcv%';
--     -> 0 rows (MVs dropped in 0090). 1d tips: aquarius/phoenix/sdex all
--        2026-07-09; soroswap ABSENT.
--
-- 0.4 Repriced `1m` verified in-window (sum(trade_count) vs run tick counts):
--       soroswap 536,318 == 536,318 ticks  (EXACT)
--       aquarius 3,842,335 vs 3,842,273    (+62,  0.0016% pre-existing excess)
--       phoenix    242,218 vs   242,201    (+17)
--     The excesses are pre-existing `1m` rows from earlier runs, NOT double
--     counting (soroswap started from zero rows and matches exactly). Tracked in
--     task 0100; not a pre-roll blocker.
--
-- 0.5 Disk: 577G available on /var/lib/docker (need >= 20G). OK.
--
-- 0.6 Cleanup rule `prices-production-cleanup` is DISABLED (verified) and MUST
--     stay disabled until §5 verifies. `price_ohlcv_1m` is a 7-day transient
--     feeder — if cleanup fires before the pre-roll, the repriced candles are
--     dropped before reaching the coarse store of record (the 0090 incident).
--
-- PARAMS: pass both explicitly, e.g.
--   clickhouse-client --param_start_ts='2024-02-20 17:00:10' \
--                     --param_end_ts='2026-07-06 09:35:16' \
--                     --queries-file preroll-amm-reprice.sql
-- =====================================================================

-- =====================================================================
-- STAGE 0 — DELETE stale phoenix coarse rows in-window, so the corrected
--   re-roll cannot lose an RMT version tie (see RESOLVED note above).
--   ONLY phoenix: soroswap has no rows to contest, aquarius's are unchanged.
--   `source` is part of every table's key
--   (ORDER BY (asset_id, quote_asset_id, source, timestamp)), so these DELETEs
--   cannot touch sdex/aquarius/soroswap — the expensive pre-Soroban SDEX tail
--   included.
--
--   `mutations_sync = 2` makes each DELETE SYNCHRONOUS: it must finish before
--   STAGE 1 re-inserts, or the two race and the result is exactly the
--   nondeterminism this stage exists to remove. Mutations only affect parts that
--   existed when they were submitted, but do not rely on that — wait.
--
--   If a DELETE errors, STOP: an emptied coarse level with no re-insert is a
--   history hole. Re-run this file from STAGE 0 (idempotent: deleting already
--   deleted rows is a no-op).
-- =====================================================================
--
-- close_usd is GUARDED — `argMaxIf(close_usd, t.timestamp, close_usd > 0)`,
-- never a bare argMax (task 0145, from the BE 0199 report via 0144).
--
--   `close_usd` is baked by a separate, LAGGING enrichment pass onto a
--   non-nullable `Decimal(38,14) DEFAULT 0` column (init.sql), so "not yet
--   enriched" and "no USD price exists" are the SAME value: zero. An unguarded
--   argMax therefore hands the coarse bucket that zero whenever its NEWEST
--   sub-bucket happens to be un-enriched — discarding every priced sub-bucket
--   underneath it. At pre-roll scale, over spans where enrichment is by
--   definition incomplete at pre-roll time, that manufactures a whole estate of
--   zeroed coarse rows, which then age out of the MV re-aggregation windows
--   where only the 0114 sweep can still reach them (task 0148).
--
-- CONSEQUENCE, deliberately accepted: `close` and `close_usd` may now come from
-- DIFFERENT sub-buckets. The two columns are no longer guaranteed same-row, so
-- do NOT assume `close_usd ~= close * rate_at(close's own bucket)` when reading
-- these tables. An approximately-right USD close beats a fabricated zero, but
-- the decoupling is silent and will bite a reader who has not been told.
--
-- NOT fixed by this guard: if EVERY sub-bucket in the range is un-enriched,
-- argMaxIf matches no rows and returns the Decimal default — 0 again. That is
-- correct here (there is genuinely no priced value to carry forward), but it
-- means a 0 in these tables still cannot be read as "worth nothing". Task 0151
-- owns that representational problem.

ALTER TABLE prices.price_ohlcv_15m DELETE
WHERE source = 'phoenix'
  AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
SETTINGS mutations_sync = 2;

ALTER TABLE prices.price_ohlcv_1h DELETE
WHERE source = 'phoenix'
  AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
SETTINGS mutations_sync = 2;

ALTER TABLE prices.price_ohlcv_4h DELETE
WHERE source = 'phoenix'
  AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
SETTINGS mutations_sync = 2;

ALTER TABLE prices.price_ohlcv_1d DELETE
WHERE source = 'phoenix'
  AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
SETTINGS mutations_sync = 2;

ALTER TABLE prices.price_ohlcv_1w DELETE
WHERE source = 'phoenix'
  AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
SETTINGS mutations_sync = 2;

ALTER TABLE prices.price_ohlcv_1M DELETE
WHERE source = 'phoenix'
  AND timestamp >= {start_ts:DateTime} AND timestamp < {end_ts:DateTime}
SETTINGS mutations_sync = 2;

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
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
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
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
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
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-01-01' AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source;

-- =====================================================================
-- STAGE 2 — coarse chain 15m -> 1h -> 4h -> 1d -> 1w -> 1M, each level FROM the
--   previous, scoped to the same sources and window. Run STAGE 1 first.
--
--   FINAL IS MANDATORY — do NOT "drop FINAL if the quota bites" (0090's advice,
--   and what an earlier draft of this file said). It was safe only when the
--   target level was freshly TRUNCATEd. `15m` now holds this run's
--   aquarius/soroswap rows ON TOP OF the pre-existing rows from the earlier
--   pre-roll, so a non-FINAL read sums BOTH copies and DOUBLE-COUNTS — silently,
--   in the store of record. Bound memory by CHUNKING, never by dropping dedup.
--
--   1h/4h/1d are MONTH-chunked, matching `PARTITION BY toYYYYMM` exactly: one
--   partition per statement. Measured on ch-prod-01 (5.59 GiB quota):
--     * full window   -> Code: 241 (While executing ReplacingSorted)
--     * year-chunked  -> 2024 OK, 2025 -> Code: 241 (reading 202512 .bin)
--   A year-bounded FINAL opens all 12 monthly partitions at once and must read
--   them in primary-key order across EVERY source — including the large `sdex`
--   rows, which the `source` filter cannot prune (source is the 3rd key column,
--   not a prefix, so it filters but never seeks). Month-chunking is the only
--   bound that matches the physical layout. Safe for 1h/4h/1d: none of those
--   buckets straddle a month boundary.
--
--   1w/1M stay single full-window statements: their buckets DO straddle month
--   AND year boundaries, so chunking them would split a bucket and write two
--   partials. They read the small 1d/1w tables, so memory is not a concern.
--
--   `SETTINGS max_threads = 4` further caps the concurrent per-part read buffers
--   that dominate this stage's memory. NOTE: legal ONLY because this file is run
--   via clickhouse-client. Our own tooling sends reads with `readonly=1` (long
--   SQL -> POST), where any SETTINGS clause is rejected with Code: 164.
--
--   Idempotent: re-running a month re-inserts identical rows that RMT collapses
--   (same key, same values, same version), so a partial run is safe to repeat
--   from the top.
-- =====================================================================

-- ---------------------------------------------------------------------
-- 1h <- 15m — MONTH-chunked (one partition per statement)
-- ---------------------------------------------------------------------

-- 2024-02
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < '2024-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-03
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-03-01' AND t.timestamp < '2024-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-04
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-04-01' AND t.timestamp < '2024-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-05
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-05-01' AND t.timestamp < '2024-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-06
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-06-01' AND t.timestamp < '2024-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-07
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-07-01' AND t.timestamp < '2024-08-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-08
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-08-01' AND t.timestamp < '2024-09-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-09
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-09-01' AND t.timestamp < '2024-10-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-10
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-10-01' AND t.timestamp < '2024-11-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-11
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-11-01' AND t.timestamp < '2024-12-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-12
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-12-01' AND t.timestamp < '2025-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-01
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-01-01' AND t.timestamp < '2025-02-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-02
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-02-01' AND t.timestamp < '2025-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-03
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-03-01' AND t.timestamp < '2025-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-04
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-04-01' AND t.timestamp < '2025-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-05
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-05-01' AND t.timestamp < '2025-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-06
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-06-01' AND t.timestamp < '2025-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-07
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-07-01' AND t.timestamp < '2025-08-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-08
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-08-01' AND t.timestamp < '2025-09-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-09
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-09-01' AND t.timestamp < '2025-10-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-10
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-10-01' AND t.timestamp < '2025-11-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-11
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-11-01' AND t.timestamp < '2025-12-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-12
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-12-01' AND t.timestamp < '2026-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-01
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-01-01' AND t.timestamp < '2026-02-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-02
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-02-01' AND t.timestamp < '2026-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-03
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-03-01' AND t.timestamp < '2026-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-04
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-04-01' AND t.timestamp < '2026-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-05
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-05-01' AND t.timestamp < '2026-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-06
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-06-01' AND t.timestamp < '2026-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-07
INSERT INTO prices.price_ohlcv_1h
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-07-01' AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- ---------------------------------------------------------------------
-- 4h <- 1h — MONTH-chunked (one partition per statement)
-- ---------------------------------------------------------------------

-- 2024-02
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < '2024-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-03
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-03-01' AND t.timestamp < '2024-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-04
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-04-01' AND t.timestamp < '2024-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-05
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-05-01' AND t.timestamp < '2024-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-06
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-06-01' AND t.timestamp < '2024-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-07
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-07-01' AND t.timestamp < '2024-08-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-08
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-08-01' AND t.timestamp < '2024-09-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-09
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-09-01' AND t.timestamp < '2024-10-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-10
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-10-01' AND t.timestamp < '2024-11-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-11
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-11-01' AND t.timestamp < '2024-12-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-12
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-12-01' AND t.timestamp < '2025-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-01
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-01-01' AND t.timestamp < '2025-02-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-02
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-02-01' AND t.timestamp < '2025-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-03
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-03-01' AND t.timestamp < '2025-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-04
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-04-01' AND t.timestamp < '2025-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-05
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-05-01' AND t.timestamp < '2025-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-06
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-06-01' AND t.timestamp < '2025-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-07
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-07-01' AND t.timestamp < '2025-08-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-08
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-08-01' AND t.timestamp < '2025-09-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-09
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-09-01' AND t.timestamp < '2025-10-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-10
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-10-01' AND t.timestamp < '2025-11-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-11
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-11-01' AND t.timestamp < '2025-12-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-12
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-12-01' AND t.timestamp < '2026-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-01
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-01-01' AND t.timestamp < '2026-02-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-02
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-02-01' AND t.timestamp < '2026-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-03
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-03-01' AND t.timestamp < '2026-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-04
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-04-01' AND t.timestamp < '2026-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-05
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-05-01' AND t.timestamp < '2026-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-06
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-06-01' AND t.timestamp < '2026-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-07
INSERT INTO prices.price_ohlcv_4h
SELECT toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-07-01' AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- ---------------------------------------------------------------------
-- 1d <- 4h — MONTH-chunked (one partition per statement)
-- ---------------------------------------------------------------------

-- 2024-02
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < '2024-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-03
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-03-01' AND t.timestamp < '2024-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-04
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-04-01' AND t.timestamp < '2024-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-05
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-05-01' AND t.timestamp < '2024-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-06
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-06-01' AND t.timestamp < '2024-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-07
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-07-01' AND t.timestamp < '2024-08-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-08
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-08-01' AND t.timestamp < '2024-09-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-09
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-09-01' AND t.timestamp < '2024-10-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-10
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-10-01' AND t.timestamp < '2024-11-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-11
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-11-01' AND t.timestamp < '2024-12-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2024-12
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2024-12-01' AND t.timestamp < '2025-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-01
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-01-01' AND t.timestamp < '2025-02-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-02
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-02-01' AND t.timestamp < '2025-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-03
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-03-01' AND t.timestamp < '2025-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-04
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-04-01' AND t.timestamp < '2025-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-05
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-05-01' AND t.timestamp < '2025-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-06
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-06-01' AND t.timestamp < '2025-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-07
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-07-01' AND t.timestamp < '2025-08-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-08
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-08-01' AND t.timestamp < '2025-09-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-09
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-09-01' AND t.timestamp < '2025-10-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-10
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-10-01' AND t.timestamp < '2025-11-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-11
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-11-01' AND t.timestamp < '2025-12-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2025-12
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2025-12-01' AND t.timestamp < '2026-01-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-01
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-01-01' AND t.timestamp < '2026-02-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-02
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-02-01' AND t.timestamp < '2026-03-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-03
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-03-01' AND t.timestamp < '2026-04-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-04
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-04-01' AND t.timestamp < '2026-05-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-05
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-05-01' AND t.timestamp < '2026-06-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-06
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-06-01' AND t.timestamp < '2026-07-01'
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- 2026-07
INSERT INTO prices.price_ohlcv_1d
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= '2026-07-01' AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- ---------------------------------------------------------------------
-- 1w <- 1d (FULL WINDOW — weeks straddle month/year boundaries, never chunk)
-- ---------------------------------------------------------------------

INSERT INTO prices.price_ohlcv_1w
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

-- ---------------------------------------------------------------------
-- 1M <- 1w (FULL WINDOW — reads a tiny table)
-- ---------------------------------------------------------------------

INSERT INTO prices.price_ohlcv_1M
SELECT toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
       asset_id, quote_asset_id, source,
       argMin(open, t.timestamp) AS open, max(high) AS high, min(low) AS low,
       argMax(close, t.timestamp) AS close,
       sum(volume_base) AS volume_base, sum(volume_quote) AS volume_quote,
       sum(volume_quote_usd) AS volume_quote_usd,
       argMaxIf(close_usd, t.timestamp, close_usd > 0) AS close_usd,
       volume_quote / nullIf(volume_base, 0) AS vwap,
       sum(trade_count) AS trade_count, max(version) AS version
FROM prices.price_ohlcv_1w AS t FINAL
WHERE t.source IN ('aquarius', 'phoenix', 'soroswap')
  AND t.timestamp >= {start_ts:DateTime} AND t.timestamp < {end_ts:DateTime}
GROUP BY timestamp, asset_id, quote_asset_id, source
SETTINGS max_threads = 4;

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
