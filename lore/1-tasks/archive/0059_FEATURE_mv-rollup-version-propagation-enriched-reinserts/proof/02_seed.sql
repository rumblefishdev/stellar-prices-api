-- Experiment 1 seed: one 15-minute bucket (2026-01-01 00:00) fed by 15
-- one-minute rows, each in its OWN INSERT statement — i.e. 15 separate
-- inserted blocks, the way live per-ledger ingestion actually arrives.
-- Series: asset_id=1, quote_asset_id=2, source='sdex'.
-- Per minute i (0..14): volume_base=10, volume_quote=1000, volume_quote_usd=0,
--   open=100+i, high=101+i, low=100, close=100+i+0.5, trade_count=1,
--   version=i+1 (monotonic, mimics ledger_seq-derived version).
-- Correct full-bucket sums: volume_base=150, volume_quote=15000,
--   volume_quote_usd=0, trade_count=15, open=100, close=114.5, high=115,
--   low=100, version=15.

INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:00:00',1,2,'sdex',100,101,100,100.5,10,1000,0,100,1,1);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:01:00',1,2,'sdex',101,102,100,101.5,10,1000,0,100,1,2);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:02:00',1,2,'sdex',102,103,100,102.5,10,1000,0,100,1,3);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:03:00',1,2,'sdex',103,104,100,103.5,10,1000,0,100,1,4);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:04:00',1,2,'sdex',104,105,100,104.5,10,1000,0,100,1,5);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:05:00',1,2,'sdex',105,106,100,105.5,10,1000,0,100,1,6);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:06:00',1,2,'sdex',106,107,100,106.5,10,1000,0,100,1,7);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:07:00',1,2,'sdex',107,108,100,107.5,10,1000,0,100,1,8);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:08:00',1,2,'sdex',108,109,100,108.5,10,1000,0,100,1,9);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:09:00',1,2,'sdex',109,110,100,109.5,10,1000,0,100,1,10);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:10:00',1,2,'sdex',110,111,100,110.5,10,1000,0,100,1,11);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:11:00',1,2,'sdex',111,112,100,111.5,10,1000,0,100,1,12);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:12:00',1,2,'sdex',112,113,100,112.5,10,1000,0,100,1,13);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:13:00',1,2,'sdex',113,114,100,113.5,10,1000,0,100,1,14);
INSERT INTO prices.price_ohlcv_1m VALUES ('2026-01-01 00:14:00',1,2,'sdex',114,115,100,114.5,10,1000,0,100,1,15);
