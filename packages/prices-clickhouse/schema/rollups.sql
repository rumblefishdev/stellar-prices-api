-- prices rollup chain — PRODUCTION refreshable MV design (task 0051/0059/0095).
--
-- ⚠️ GENERATED. Every statement below is rendered by
-- `src/rollup_sql.rs` — the six fast MVs by `mv_ddl`, then the six
-- reconciliation MVs by `reconcile_mv_ddl`, each group fine to coarse — and
-- unit tests in `src/lib.rs` assert this file
-- equals that rendering whitespace-normalised. Edit the generator, re-render,
-- and commit both — an edit made here alone fails the build, and an edit made
-- there alone leaves the operator copy and the drift detector reading stale SQL.
--
-- NOT applied by prices-clickhouse-init (kept out of init.sql so schema-apply
-- stays version-agnostic). Refreshable MVs require ClickHouse ≥ 23.12 and the
-- allow_experimental_refreshable_materialized_view setting on older builds.
--
-- ⚠️ EDITING A BODY BELOW DOES NOT LAND ON A PROVISIONED TARGET (task 0142).
--   Every statement here is `CREATE MATERIALIZED VIEW IF NOT EXISTS`, and
--   `IF NOT EXISTS` does not redefine an object that already exists. On
--   ch-prod-01 — which holds all of them — re-applying this file after an edit
--   changes NOTHING and reports success. Unlike the plain views in views.sql
--   (task 0134) there is no `CREATE OR REPLACE` escape: a refreshable TO-table
--   MV must be DROPped and re-CREATEd, which takes that tier offline while it
--   is gone and re-opens every invariant below. That is an operator procedure,
--   not an apply:
--
--       docs/runbooks/0142-rollup-mv-reapply.md
--
--   The task 0286 re-CREATE of all six has its own procedure, which the 0142
--   runbook points at and which must be followed instead:
--
--       docs/runbooks/0286-candle-definitions-rollout.md
--
--   To see whether a target's live definitions still match this file:
--
--       cargo run -p prices-clickhouse --bin prices-clickhouse-drift
--
--   It is read-only (SELECTs against system.tables only) and exits non-zero on
--   drift. Run it after any edit here, and after any re-CREATE.
--
-- The six FAST MVs serve the LIVE path: each re-aggregates a bounded recent
-- window from the previous granularity's FINAL (post-dedup, post-enrichment)
-- and APPENDs the result. Historical/backfilled partitions fall outside the
-- window and are pre-rolled instead (see preroll.sql). The six RECONCILIATION
-- MVs after them repair what the fast windows missed (below).
--
-- WHAT A COARSE CANDLE MEANS (task 0286 / ADR 0287 §2–§5).
--   A candle's prices come only from the PRICE-FORMING trades of its own
--   bucket. One level up, that is the
--   `t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)`
--   gate on all four price aggregates — the second term because a PRE-0286 row
--   reads `pf_trade_count` from its DEFAULT (`trade_count`) and can carry a
--   stored price of 0, or of a few Decimal(38, 14) ticks, which is the same
--   quantisation noise `/ohlcv` has always refused (review WR-03): `open` is
--   the first child that had a price-forming fill,
--   `close` the last, `high`/`low` their extremes. A DUST-ONLY child — every
--   fill too small for its price to mean anything, written by the ingest with
--   `open = high = low = close = 0` — therefore reaches none of them. Before
--   0286 it reached all four: it was the `min(low)` of its bucket (a zero low
--   on every coarse tier) and, whenever it landed last, the `argMax` close.
--
--   A bucket with NO price-forming child has no price: each conditional
--   aggregate matches nothing and returns the type default 0 — the same "no
--   price" encoding the 1m tier writes. Its volume and `trade_count` are still
--   summed; dust trades happened, and `vwap` still weights every one of them.
--
--   `close_usd` is this bucket's own `close` re-priced by the LATEST PRICED
--   child's RATE (`close_usd / close`), never a carried product
--   (`argMaxIf(close_usd, …)`): carrying the product left `close` and
--   `close_usd` on different sub-buckets (the consequence task 0145
--   accepted); re-pricing the bucket's own close makes them same-bucket
--   again by construction. A child lends its rate only if `close_usd` AND
--   `close` both clear the same 1e-12 floor as the price gates: the ratio of
--   two values a few ticks wide (prod: 5e-14 / 4e-14 = 0.8) is quantisation
--   noise, and as the latest "priced" child it would re-price a healthy close
--   on every tier above. With no such child the rate is 0 and so is
--   `close_usd`, which the coarse sweep then prices — unless the bucket has no
--   price at all (`close = 0`), where 0 is the permanent, correct answer.
--
--   Both derived Decimals go through `ifNull(toDecimal128OrZero(toString(…),
--   14), 0)`: Decimal division silently overflows past a ~1.7e10 dividend on
--   26.3.10.60 — no exception, a wrong number — and `divideDecimal` throws on a
--   zero divisor. `vwap`'s fallback is an EXPLICIT zero because init.sql
--   declares the column NOT Nullable; the pre-0286 form only landed because
--   `insert_null_as_default` rewrote its NULL.
--
-- THE MONTH ROLLS FROM THE DAY (task 0286, BRIEF F10).
--   The month's MV reads `price_ohlcv_1d` and is renamed for it. A week is
--   attributed wholly to the month it STARTS in, so a week-fed month took its
--   close and extremes from whichever month owned the straddling week: a month
--   whose 1st is not a Monday lost its first days to the previous month, and
--   gained the previous month's tail. Re-creating it is the one step of the
--   rollout that also TRUNCATEs and re-rolls its target — see the 0286 runbook.
--
-- EACH MV WAITS FOR THE MV THAT WRITES ITS SOURCE (task 0143).
--   Every MV that reads a coarse tier carries `DEPENDS ON` the MV that writes
--   that tier, fully qualified (ClickHouse stores an unqualified name
--   qualified, and the drift check compares the stored text). Without it each
--   MV fired on its own clock: at 00:00 the day's MV and the two MVs that read
--   the day all fired together, and the week and month could read the day
--   before the day that just closed had been written. With it, a dependent's
--   slot waits until its dependency has refreshed for the same slot. Only the
--   MV reading the 1m tier has no dependency, because ingest writes that tier.
--   A STOPPED, failing or misspelled dependency blocks its dependents forever
--   in `system.view_refreshes.status = 'WaitingForDependencies'` with no
--   error, which is why the freshness probe watches that status. A live MV is
--   moved onto this clause in place with `ALTER TABLE … MODIFY REFRESH`, which
--   must repeat the whole clause (see the 0142 runbook).
--
-- AN HOURLY RECONCILIATION PASS REBUILDS ANY BUCKET THAT DISAGREES (task 0203).
--   The fast windows are measured from now(), so they slide forward with the
--   clock: a source row that arrives after its bucket left the window (a
--   stalled ingest catching up, 2026-08-13) was never rolled up, and a bucket
--   built from part of its children stayed partial for good. Every coarse
--   tier therefore has a second writer, refreshed hourly. It rolls the tier's
--   source over the last 7 days, aligned to the tier's bucket, with the SAME
--   aggregation as the fast MV, and appends ONLY the buckets whose
--   `trade_count` or `volume_base` differ from what the target holds FINAL
--   (or that the target lacks). It never compares `version`: enrichment and
--   the coarse sweep bump versions without changing completeness. The
--   appended row wins because its `sum(version)` covers more children than
--   the stale row did. The pass selects buckets by EVENT time, so arrival
--   order does not matter, and it is idempotent: a bucket that agrees is not
--   written. Each reconciliation MV waits for the one reading the tier below
--   it, so a repair climbs every tier in one pass. The lowest waits for
--   nothing, and no fast MV ever waits for a reconciliation MV, so the
--   backstop cannot delay fresh data. Window and cadence live in the
--   generator, once. An outage longer than 7 days still needs
--   schema/preroll-live-gap.sql, and a re-ingest must STOP the reconciliation
--   MVs first (runbook 0142 and the re-ingest runbooks).
--
-- DEPLOY ORDER, NOT NEGOTIABLE (task 0286 / BRIEF §4.9; the same statement is
--   in schema/init.sql): schema → enrichment + coarse sweep + prices-api → the
--   MV re-CREATE below → ingest LAST. A pre-0286 MV meeting a post-0286 ingest
--   turns a dust-only minute into a zero `low` across the whole coarse bucket,
--   and pre-0286 enrichment re-inserts rows without the pf columns, which then
--   silently take their DEFAULTs.
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
--   whole coarse table shares one monotonic scheme. Task 0286 did NOT change
--   it: there is no settle component and no second term.
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
--   The generator aligns the bounded INSERT forms the same way, so a pre-roll
--   range cannot be handed to the wide tiers raw either.
--
-- Correctness (task 0059):
--   - vwap references the summed aliases, never sum(…)/sum(…) (else
--     Code: 184 ILLEGAL_AGGREGATION from aggregate-in-aggregate). So does the
--     `close_usd` rate, which multiplies the `close` alias by an aggregate.
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
--           `preroll.sql` could accept `ts_bucket`; it keeps `timestamp` for
--           parity with the MVs, and names all eighteen target columns so that
--           position never decides anything.
--
--       (2) IN-QUERY RESOLUTION (the shadow). Inside the SELECT, the mandatory
--           `AS timestamp` alias shadows the source column `timestamp`, so a bare
--           `timestamp` in argMinIf/argMaxIf + the WHERE window resolves to the
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
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 2 HOUR, INTERVAL 15 MINUTE)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_15m_to_1h
REFRESH EVERY 15 MINUTE DEPENDS ON prices.mv_ohlcv_1m_to_15m APPEND
TO prices.price_ohlcv_1h AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 8 HOUR, INTERVAL 1 HOUR)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1h_to_4h
REFRESH EVERY 1 HOUR DEPENDS ON prices.mv_ohlcv_15m_to_1h APPEND
TO prices.price_ohlcv_4h AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 1 DAY, INTERVAL 4 HOUR)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_4h_to_1d
REFRESH EVERY 4 HOUR DEPENDS ON prices.mv_ohlcv_1h_to_4h APPEND
TO prices.price_ohlcv_1d AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 DAY)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1d_to_1w
REFRESH EVERY 1 DAY DEPENDS ON prices.mv_ohlcv_4h_to_1d APPEND
TO prices.price_ohlcv_1w AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 60 DAY, INTERVAL 1 WEEK)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1d_to_1M
REFRESH EVERY 1 DAY DEPENDS ON prices.mv_ohlcv_4h_to_1d APPEND
TO prices.price_ohlcv_1M AS
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 400 DAY, INTERVAL 1 MONTH)
GROUP BY timestamp, asset_id, quote_asset_id, source;

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_reconcile_1m_to_15m
REFRESH EVERY 1 HOUR APPEND
TO prices.price_ohlcv_15m AS
SELECT
    timestamp, asset_id, quote_asset_id, source, open, high,
    low, close, volume_base, volume_quote, volume_quote_usd, close_usd,
    vwap, trade_count, version, pf_trade_count, pf_volume, pf_price_volume
FROM (
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1m AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 15 MINUTE)
GROUP BY timestamp, asset_id, quote_asset_id, source
) AS s
WHERE (timestamp, asset_id, quote_asset_id, source, trade_count, volume_base) NOT IN (
    SELECT d.timestamp, d.asset_id, d.quote_asset_id, d.source, d.trade_count, d.volume_base
    FROM prices.price_ohlcv_15m AS d FINAL
    WHERE d.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 15 MINUTE)
);

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_reconcile_15m_to_1h
REFRESH EVERY 1 HOUR DEPENDS ON prices.mv_reconcile_1m_to_15m APPEND
TO prices.price_ohlcv_1h AS
SELECT
    timestamp, asset_id, quote_asset_id, source, open, high,
    low, close, volume_base, volume_quote, volume_quote_usd, close_usd,
    vwap, trade_count, version, pf_trade_count, pf_volume, pf_price_volume
FROM (
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_15m AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 HOUR)
GROUP BY timestamp, asset_id, quote_asset_id, source
) AS s
WHERE (timestamp, asset_id, quote_asset_id, source, trade_count, volume_base) NOT IN (
    SELECT d.timestamp, d.asset_id, d.quote_asset_id, d.source, d.trade_count, d.volume_base
    FROM prices.price_ohlcv_1h AS d FINAL
    WHERE d.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 HOUR)
);

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_reconcile_1h_to_4h
REFRESH EVERY 1 HOUR DEPENDS ON prices.mv_reconcile_15m_to_1h APPEND
TO prices.price_ohlcv_4h AS
SELECT
    timestamp, asset_id, quote_asset_id, source, open, high,
    low, close, volume_base, volume_quote, volume_quote_usd, close_usd,
    vwap, trade_count, version, pf_trade_count, pf_volume, pf_price_volume
FROM (
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1h AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 4 HOUR)
GROUP BY timestamp, asset_id, quote_asset_id, source
) AS s
WHERE (timestamp, asset_id, quote_asset_id, source, trade_count, volume_base) NOT IN (
    SELECT d.timestamp, d.asset_id, d.quote_asset_id, d.source, d.trade_count, d.volume_base
    FROM prices.price_ohlcv_4h AS d FINAL
    WHERE d.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 4 HOUR)
);

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_reconcile_4h_to_1d
REFRESH EVERY 1 HOUR DEPENDS ON prices.mv_reconcile_1h_to_4h APPEND
TO prices.price_ohlcv_1d AS
SELECT
    timestamp, asset_id, quote_asset_id, source, open, high,
    low, close, volume_base, volume_quote, volume_quote_usd, close_usd,
    vwap, trade_count, version, pf_trade_count, pf_volume, pf_price_volume
FROM (
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_4h AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 DAY)
GROUP BY timestamp, asset_id, quote_asset_id, source
) AS s
WHERE (timestamp, asset_id, quote_asset_id, source, trade_count, volume_base) NOT IN (
    SELECT d.timestamp, d.asset_id, d.quote_asset_id, d.source, d.trade_count, d.volume_base
    FROM prices.price_ohlcv_1d AS d FINAL
    WHERE d.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 DAY)
);

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_reconcile_1d_to_1w
REFRESH EVERY 1 HOUR DEPENDS ON prices.mv_reconcile_4h_to_1d APPEND
TO prices.price_ohlcv_1w AS
SELECT
    timestamp, asset_id, quote_asset_id, source, open, high,
    low, close, volume_base, volume_quote, volume_quote_usd, close_usd,
    vwap, trade_count, version, pf_trade_count, pf_volume, pf_price_volume
FROM (
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 WEEK)
GROUP BY timestamp, asset_id, quote_asset_id, source
) AS s
WHERE (timestamp, asset_id, quote_asset_id, source, trade_count, volume_base) NOT IN (
    SELECT d.timestamp, d.asset_id, d.quote_asset_id, d.source, d.trade_count, d.volume_base
    FROM prices.price_ohlcv_1w AS d FINAL
    WHERE d.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 WEEK)
);

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_reconcile_1d_to_1M
REFRESH EVERY 1 HOUR DEPENDS ON prices.mv_reconcile_4h_to_1d APPEND
TO prices.price_ohlcv_1M AS
SELECT
    timestamp, asset_id, quote_asset_id, source, open, high,
    low, close, volume_base, volume_quote, volume_quote_usd, close_usd,
    vwap, trade_count, version, pf_trade_count, pf_volume, pf_price_volume
FROM (
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS open,
    maxIf(t.high, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS high,
    minIf(t.low, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0 AND t.close >= toDecimal128('0.000000000001', 14)) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd >= toDecimal128('0.000000000001', 14) AND t.close >= toDecimal128('0.000000000001', 14))), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1d AS t FINAL
WHERE t.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 MONTH)
GROUP BY timestamp, asset_id, quote_asset_id, source
) AS s
WHERE (timestamp, asset_id, quote_asset_id, source, trade_count, volume_base) NOT IN (
    SELECT d.timestamp, d.asset_id, d.quote_asset_id, d.source, d.trade_count, d.volume_base
    FROM prices.price_ohlcv_1M AS d FINAL
    WHERE d.timestamp >= toStartOfInterval(now() - INTERVAL 7 DAY, INTERVAL 1 MONTH)
)
;
