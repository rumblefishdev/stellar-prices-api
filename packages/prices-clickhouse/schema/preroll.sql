-- prices coarse-granularity PRE-ROLL — deterministic, full-range.
--
-- ⚠️ GENERATED. Every statement below is rendered by
-- `src/rollup_sql.rs::rollup_insert(tier, "prices", &Bounds::Full, None)`, and a
-- unit test in `src/lib.rs` asserts this file equals that rendering
-- whitespace-normalised. It is the same SELECT the six MVs in `rollups.sql`
-- carry, so a coarse candle means one thing however it was built. Edit the
-- generator, re-render, commit both.
--
-- Used by the backfill / sizing measurement (task 0060) to populate the
-- _15m … _1M tables from a fully-written _1m, instead of the bounded-window
-- live MV chain (rollups.sql). Each granularity re-aggregates the previous
-- one's FINAL over the WHOLE range, producing exactly one row per
-- (bucket, asset, quote, source) — the production-correct rollup result.
--
-- Run AFTER the _1m backfill completes (the applier in src/lib.rs::apply_sql
-- splits on `;`). Re-runnable: ReplacingMergeTree(version) collapses the
-- duplicate-PK rows a second run would add. For a clean measurement, TRUNCATE
-- the coarse tables first (the runbook does this).
--
-- WHAT A COARSE CANDLE MEANS (task 0286 / ADR 0287 §2–§5). A candle's prices
-- come only from the PRICE-FORMING trades of its own bucket, so all four price
-- aggregates are gated on `t.pf_trade_count > 0`: a DUST-ONLY child — every
-- fill too small for its price to mean anything, written by the ingest with
-- `open = high = low = close = 0` — contributes to none of them. Before 0286
-- it was the `min(low)` of its bucket and, whenever it landed last, the
-- `argMax` close. A bucket with no price-forming child has no price at all
-- (every conditional aggregate returns the type default 0), and keeps its
-- volume and `trade_count`; `vwap` still weights every fill, dust included.
--
-- The MONTH rolls from the DAY (BRIEF F10): a week belongs wholly to the month
-- it STARTS in, so a week-fed month took its close and extremes from whichever
-- month owned the straddling week.
--
-- Correctness (task 0059): the bucket key is aliased `AS timestamp`, which
-- SHADOWS the source `timestamp` column. argMinIf/argMaxIf must reference the
-- QUALIFIED source column `t.timestamp` (FROM … AS t); the bare `timestamp`
-- would resolve to the constant bucket-start alias and tie-break open / close /
-- close_usd to an arbitrary row instead of the true first / last by time.
--
-- version = sum(version), NOT max(version) (task 0095). The coarse tables share
-- one monotonic version scheme with the APPEND rollup MVs (rollups.sql): a
-- fuller aggregation sums more source versions than a partial one, so a complete
-- bucket always outranks a partial re-roll of itself under RMT. Mixing schemes
-- (preroll max vs MV sum) would let a partial MV bucket outrank a complete
-- pre-rolled one, because sum ≫ max for any multi-row bucket. Task 0286 left
-- this alone — no settle component, no second term.
--
-- close_usd is a RATE, re-priced by the bucket's own close (task 0286, ADR 0287
-- §5): `close × argMaxIf(close_usd / close, t.timestamp, close_usd > 0 AND
-- close > 0)`.
--
--   This SUPERSEDES task 0145's `argMaxIf(close_usd, t.timestamp, close_usd >
--   0)` rather than sitting beside it, and with it the consequence 0145
--   accepted: that `close` and `close_usd` could come from DIFFERENT
--   sub-buckets. They are same-bucket again by construction — the coarse row
--   carries its own close, valued at the newest exchange rate any priced child
--   observed. The reason 0145 existed is unchanged and still handled: `close_usd`
--   is baked by a separate, LAGGING enrichment pass onto a non-nullable
--   `Decimal(38,14) DEFAULT 0` column, so "not yet enriched" and "no USD price
--   exists" are the same value — zero — and an UNGUARDED argMax would hand the
--   coarse bucket that zero whenever its newest sub-bucket happened to be
--   un-enriched. The `close_usd > 0 AND close > 0` predicate is what skips it.
--
--   NOT fixed by this: if EVERY sub-bucket in the range is un-enriched, the
--   rate matches no rows, comes back 0, and so does close_usd. That is correct
--   here (there is genuinely no priced value to carry forward), but it means a
--   0 in these tables still cannot be read as "worth nothing". Task 0151 owns
--   that representational problem.
--
-- Both derived Decimals go through `ifNull(toDecimal128OrZero(toString(…), 14),
-- 0)`: Decimal division silently overflows past a ~1.7e10 dividend on
-- 26.3.10.60 — no exception, a wrong number — which is exactly the scale a
-- full-range pre-roll reaches. `vwap`'s fallback is an EXPLICIT zero because
-- init.sql declares the column NOT Nullable.
--
-- Task 0286: every INSERT below names all EIGHTEEN target columns, and projects
-- them. A positional `INSERT … SELECT` of fewer columns than the target has
-- fails with Code 20 — but a NAMED list that omits one does NOT: ClickHouse
-- fills it from its DEFAULT, and `pf_trade_count DEFAULT trade_count` would
-- report a dust-only bucket as fully price-forming. The three pf columns are
-- summed from the children here, so they mean what the 1m tier means.

INSERT INTO prices.price_ohlcv_15m
    (timestamp, asset_id, quote_asset_id, source,
     open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd,
     vwap, trade_count, version, pf_trade_count,
     pf_volume, pf_price_volume)
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 15 MINUTE) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0) AS open,
    maxIf(t.high, t.pf_trade_count > 0) AS high,
    minIf(t.low, t.pf_trade_count > 0) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd > 0 AND t.close > 0)), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1m AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1h
    (timestamp, asset_id, quote_asset_id, source,
     open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd,
     vwap, trade_count, version, pf_trade_count,
     pf_volume, pf_price_volume)
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0) AS open,
    maxIf(t.high, t.pf_trade_count > 0) AS high,
    minIf(t.low, t.pf_trade_count > 0) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd > 0 AND t.close > 0)), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_15m AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_4h
    (timestamp, asset_id, quote_asset_id, source,
     open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd,
     vwap, trade_count, version, pf_trade_count,
     pf_volume, pf_price_volume)
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 4 HOUR) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0) AS open,
    maxIf(t.high, t.pf_trade_count > 0) AS high,
    minIf(t.low, t.pf_trade_count > 0) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd > 0 AND t.close > 0)), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1h AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1d
    (timestamp, asset_id, quote_asset_id, source,
     open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd,
     vwap, trade_count, version, pf_trade_count,
     pf_volume, pf_price_volume)
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 DAY) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0) AS open,
    maxIf(t.high, t.pf_trade_count > 0) AS high,
    minIf(t.low, t.pf_trade_count > 0) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd > 0 AND t.close > 0)), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_4h AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1w
    (timestamp, asset_id, quote_asset_id, source,
     open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd,
     vwap, trade_count, version, pf_trade_count,
     pf_volume, pf_price_volume)
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 WEEK) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0) AS open,
    maxIf(t.high, t.pf_trade_count > 0) AS high,
    minIf(t.low, t.pf_trade_count > 0) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd > 0 AND t.close > 0)), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1d AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source;

INSERT INTO prices.price_ohlcv_1M
    (timestamp, asset_id, quote_asset_id, source,
     open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd,
     vwap, trade_count, version, pf_trade_count,
     pf_volume, pf_price_volume)
SELECT
    toStartOfInterval(t.timestamp, INTERVAL 1 MONTH) AS timestamp,
    asset_id, quote_asset_id, source,
    argMinIf(t.open, t.timestamp, t.pf_trade_count > 0) AS open,
    maxIf(t.high, t.pf_trade_count > 0) AS high,
    minIf(t.low, t.pf_trade_count > 0) AS low,
    argMaxIf(t.close, t.timestamp, t.pf_trade_count > 0) AS close,
    sum(t.volume_base) AS volume_base,
    sum(t.volume_quote) AS volume_quote,
    sum(t.volume_quote_usd) AS volume_quote_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(close) * argMaxIf(toFloat64(t.close_usd) / toFloat64(t.close), t.timestamp, t.close_usd > 0 AND t.close > 0)), 14), 0) AS close_usd,
    ifNull(toDecimal128OrZero(toString(toFloat64(volume_quote) / nullIf(toFloat64(volume_base), 0)), 14), toDecimal128(0, 14)) AS vwap,
    sum(t.trade_count) AS trade_count,
    sum(t.version) AS version,
    sum(t.pf_trade_count) AS pf_trade_count,
    sum(t.pf_volume) AS pf_volume,
    sum(t.pf_price_volume) AS pf_price_volume
FROM prices.price_ohlcv_1d AS t FINAL
GROUP BY timestamp, asset_id, quote_asset_id, source
;
