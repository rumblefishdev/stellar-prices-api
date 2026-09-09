-- prices rollup chain — PRODUCTION refreshable MV design (task 0051/0059/0095).
--
-- NOT applied by prices-clickhouse-init (kept out of init.sql so schema-apply
-- stays version-agnostic). Refreshable MVs require ClickHouse ≥ 23.12 and the
-- allow_experimental_refreshable_materialized_view setting on older builds.
--
-- ⚠️ EDITING A BODY BELOW DOES NOT LAND ON A PROVISIONED TARGET (task 0142).
--   Every statement here is `CREATE MATERIALIZED VIEW IF NOT EXISTS`, and
--   `IF NOT EXISTS` does not redefine an object that already exists. On
--   ch-prod-01 — which holds all six — re-applying this file after an edit
--   changes NOTHING and reports success. Unlike the plain views in views.sql
--   (task 0134) there is no `CREATE OR REPLACE` escape: a refreshable TO-table
--   MV must be DROPped and re-CREATEd, which takes that tier offline while it
--   is gone and re-opens every invariant below. That is an operator procedure,
--   not an apply:
--
--       docs/runbooks/0142-rollup-mv-reapply.md
--
--   To see whether a target's live definitions still match this file:
--
--       cargo run -p prices-clickhouse --bin prices-clickhouse-drift
--
--   It is read-only (SELECTs against system.tables only) and exits non-zero on
--   drift. Run it after any edit here, and after any re-CREATE.
--
-- These MVs serve the LIVE path only: each re-aggregates a bounded recent
-- window from the previous granularity's FINAL (post-dedup, post-enrichment)
-- and APPENDs the result. Historical/backfilled partitions fall outside the
-- window and are pre-rolled instead (see preroll.sql).
--
-- APPEND, NOT REPLACE (task 0095 — this is the load-bearing correctness fix).
--   A refreshable MV WITHOUT `APPEND` *atomically replaces its whole target
--   table* on every refresh (CH `CREATE VIEW` ref). Paired with the bounded
--   `WHERE timestamp >= now() - <window>` below, replace mode overwrites the
--   coarse table with ONLY the recent window each tick — deleting all
--   pre-rolled history (and, when live was frozen, emptying it outright). That
--   is exactly the production data-loss 0090 found and DROPped these MVs to
--   stop. `REFRESH … APPEND` inserts the window instead, leaving older rows
--   untouched; the `ReplacingMergeTree(version)` target collapses the
--   re-inserted overlapping buckets by version. 0059 decided this in advance
--   (G-rollup-version-propagation-decision.md, "durability & refresh mode").
--
-- STRICTLY-INCREASING version = sum(version), NOT max(version) (task 0059 #5).
--   Under APPEND the target stays on RMT version dedup, so the projected
--   version decides which re-inserted row wins. `max(version)` is INSUFFICIENT:
--   correcting an EARLY row in a bucket bumps that row's version but leaves the
--   bucket `max` unchanged, so the stale and corrected rollup rows tie on
--   version and RMT's tie-break is not contractual (the same tie 0097's
--   phoenix re-roll had to DELETE around). `sum(version)` strictly increases
--   under every real mutation — a correction raises one addend, a later bucket
--   row adds a positive addend — so the freshest/fullest aggregation always
--   wins. It is also self-protecting at window edges: a partial bucket sums
--   FEWER source versions than the complete one, so a complete bucket outranks
--   any partial re-roll of itself. (Proof observed sum 120→121 where max tied
--   15→15.) preroll.sql / preroll-live-gap.sql project sum(version) too, so the
--   whole coarse table shares one monotonic scheme.
--
-- WINDOW LOWER BOUND ALIGNED TO THE COARSE BUCKET (task 0095).
--   Each WHERE lower bound is `toStartOfInterval(now() - <window>, INTERVAL
--   <coarse-grain>)`, not a raw `now() - <window>`. A raw bound falls mid
--   coarse-bucket, so the OLDEST bucket in the window would be re-aggregated
--   from only its in-window source slice — a PARTIAL bucket. Aligning the bound
--   to the coarse-bucket start guarantees the oldest bucket is rebuilt COMPLETE
--   from all its source rows, so a refresh never appends a truncated bucket
--   into pre-rolled history. (Only the NEWEST bucket, at now(), is legitimately
--   partial; it wins nothing and is superseded — higher sum(version) — by the
--   next refresh. Same self-healing straddle preroll-live-gap.sql documents.)
--
-- Correctness (task 0059):
--   - vwap references the summed aliases, never sum(…)/sum(…) (else
--     Code: 184 ILLEGAL_AGGREGATION from aggregate-in-aggregate).
--   - The bucket key MUST be aliased `AS timestamp`, and that forced name is —
--     separately — what makes the `t.` qualifier below mandatory. Two distinct
--     ClickHouse name mechanisms collide here (task 0071):
--
--       (1) INSERT ROUTING (by name). A `TO`-table MV routes its result by
--           matching each SELECT-output column NAME to the TARGET table's column
--           of the same name (target = `price_ohlcv_15m`; the source table is not
--           consulted). So the bucket output must be named `timestamp` to land in
--           `price_ohlcv_15m.timestamp`. A differently-named bucket (`ts_bucket`)
--           is rejected: `Code: 8 THERE_IS_NO_COLUMN` (verified on CH 26.3.10.60).
--           NB a plain `INSERT … SELECT` routes by POSITION instead — which is why
--           `preroll.sql` accepts `ts_bucket`; it keeps `timestamp` only for
--           parity with the MVs.
--
--       (2) IN-QUERY RESOLUTION (the shadow). Inside the SELECT, the mandatory
--           `AS timestamp` alias shadows the source column `timestamp`, so a bare
--           `timestamp` in argMin/argMax + the WHERE window resolves to the
--           CONSTANT bucket-start value, not the per-row time — tie-breaking
--           open/close/close_usd to an arbitrary row (task 0059 full-chain test).
--           Reading the QUALIFIED source `t.timestamp` (FROM … AS t) is the fix.
--
--     Because (1) forces the name that causes (2), renaming the bucket is NOT an
--     option for the MVs — `t.`-qualification is the only available remedy.

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1m_to_15m
REFRESH EVERY 1 MINUTE APPEND
TO prices.price_ohlcv_15m AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 2 HOUR, INTERVAL 15 MINUTE)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_15m_to_1h
REFRESH EVERY 15 MINUTE APPEND
TO prices.price_ohlcv_1h AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 8 HOUR, INTERVAL 1 HOUR)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1h_to_4h
REFRESH EVERY 1 HOUR APPEND
TO prices.price_ohlcv_4h AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 1 DAY, INTERVAL 4 HOUR)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_4h_to_1d
REFRESH EVERY 4 HOUR APPEND
TO prices.price_ohlcv_1d AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 DAY)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1d_to_1w
REFRESH EVERY 1 DAY APPEND
TO prices.price_ohlcv_1w AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 60 DAY, INTERVAL 1 WEEK)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1w_to_1M
REFRESH EVERY 1 DAY APPEND
TO prices.price_ohlcv_1M AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
    asset_id, quote_asset_id, source,
    argMin(open,  t.timestamp)                AS open,
    max(high)                                 AS high,
    min(low)                                  AS low,
    argMax(close, t.timestamp)                AS close,
    sum(volume_base)                          AS volume_base,
    sum(volume_quote)                         AS volume_quote,
    sum(volume_quote_usd)                     AS volume_quote_usd,
    argMax(close_usd, t.timestamp)            AS close_usd,
    volume_quote / nullIf(volume_base, 0) AS vwap,
    sum(trade_count)                          AS trade_count,
    sum(version)                              AS version
FROM prices.price_ohlcv_1w AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 400 DAY, INTERVAL 1 MONTH)
GROUP BY timestamp, asset_id, quote_asset_id, source;
