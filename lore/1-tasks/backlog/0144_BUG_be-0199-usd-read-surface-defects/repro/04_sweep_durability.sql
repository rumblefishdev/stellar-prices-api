-- TEST E: is the 0114 coarse sweep's repair durable inside the MV's
-- 8-hour re-aggregation window?
--
-- Sequence, all on the 13:00 _1h row for (asset 1, quote 2, sdex):
--   1. MV appends it:      close_usd = 0     version = sum(_15m versions) = 400
--   2. coarse sweep repairs it (repair.rs: version + 1):
--                          close_usd = 0.171 version = 401
--   3. enrichment lands on TWO of the _1m rows under the 13:00/13:15 sub-buckets;
--      mv_ohlcv_1m_to_15m re-appends those two _15m rows at version + 1 each,
--      so _15m now sums to 402. The 13:45 sub-bucket stays unpriced (its quote
--      is exotic — the permanent floor).
--   4. mv_ohlcv_15m_to_1h re-appends the hour: still argMax -> 0, now version 402.
TRUNCATE TABLE prices.price_ohlcv_1h;

INSERT INTO prices.price_ohlcv_1h
    (timestamp, asset_id, quote_asset_id, source, open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES
    ('2026-08-04 13:00:00', 1, 2, 'sdex', 0.170, 0.172, 0.169, 0.1720, 42000, 7146, 0,    0,     0.17, 170, 400),
    ('2026-08-04 13:00:00', 1, 2, 'sdex', 0.170, 0.172, 0.169, 0.1720, 42000, 7146, 7146, 0.171, 0.17, 170, 401);

SELECT 'after the sweep repairs it (version 401 beats the MV row at 400)' AS t FORMAT TSVRaw;
SELECT timestamp, close_usd, version FROM prices.price_ohlcv_1h FINAL FORMAT PrettyCompactMonoBlock;

INSERT INTO prices.price_ohlcv_1h
    (timestamp, asset_id, quote_asset_id, source, open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES
    ('2026-08-04 13:00:00', 1, 2, 'sdex', 0.170, 0.172, 0.169, 0.1720, 42000, 7146, 0, 0, 0.17, 170, 402);

SELECT 'after the next 15-minute MV refresh re-appends the same hour' AS t FORMAT TSVRaw;
SELECT timestamp, close_usd, version FROM prices.price_ohlcv_1h FINAL FORMAT PrettyCompactMonoBlock;

SELECT 'what the view publishes now' AS t FORMAT TSVRaw;
SELECT asset_code, bucket, close_usd FROM prices.price_usd_series_1h FORMAT PrettyCompactMonoBlock;
