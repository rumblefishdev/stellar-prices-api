-- Experiment 2 (enrichment) + Experiment 3 (the fix).
-- Run after 01_schema.sql + 02_seed.sql.

-- Exp 2: task-0026 enrichment re-INSERTs minute 00:06 with volume_quote_usd
-- filled (oracle 0.5 x volume_quote 1000 = 500) and version = 7 + 1 = 8.
INSERT INTO prices.price_ohlcv_1m VALUES
  ('2026-01-01 00:06:00',1,2,'sdex',106,107,100,106.5,10,1000,500,0.5,1,8);

-- _1m FINAL now reflects the correction (v8 > v7); bucket truth = 500.
-- _15m_draft FINAL does NOT (its winner is the v15 partial = minute 14 only):
--   the enrichment partial carries max(version)=8 < 15, so it loses. The
--   correction is invisible at _15m.  <-- finding #3

-- Exp 3 (the fix): re-aggregate the WHOLE bucket from _1m FINAL.
-- NB: vwap references the aliases (volume_quote_usd / volume_base) rather than
-- re-summing — re-summing trips the same ILLEGAL_AGGREGATION as the draft
-- (finding #1). Project BOTH max(version) and sum(version) to show the
-- tie-break problem.
SELECT
  argMin(open,timestamp)              AS open,
  max(high)                           AS high,
  min(low)                            AS low,
  argMax(close,timestamp)             AS close,
  sum(volume_base)                    AS volume_base,
  sum(volume_quote)                   AS volume_quote,
  sum(volume_quote_usd)               AS volume_quote_usd,
  volume_quote_usd / nullIf(volume_base,0) AS vwap,
  sum(trade_count)                    AS trade_count,
  max(version)                        AS max_version,   -- 15 — unchanged by
                                                        -- enriching an early minute
  sum(version)                        AS sum_version    -- 121 (was 120) — strictly
                                                        -- increases on any correction
FROM prices.price_ohlcv_1m FINAL
GROUP BY toStartOfInterval(timestamp, INTERVAL 15 MINUTE), asset_id, quote_asset_id, source;

-- Version-projection caveat (finding #5): a scheduled re-aggregate into a
-- ReplacingMergeTree(version) target that projects version = max(version)
-- emits the SAME version (15) before and after enriching an early minute, so
-- the stale and corrected rollup rows tie. ReplacingMergeTree then falls back
-- to insertion-order tie-breaking — not a guarantee. Robust options:
--   (a) true Refreshable MV (atomic target replace; no version dedup relied on)
--   (b) project a strictly-increasing version (e.g. sum(version), or a refresh epoch).
