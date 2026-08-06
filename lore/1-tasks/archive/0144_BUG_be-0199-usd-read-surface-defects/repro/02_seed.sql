-- Identities. asset_id 1 = yXLM (credit), 9 = XLM (native), 2 = XLM as quote.
-- asset_id 7 is DELIBERATELY shared by two natural identities, reproducing the
-- 0139 condition (assets is RMT(updated_at) ORDER BY natural identity, so FINAL
-- keeps BOTH rows).
INSERT INTO prices.assets (asset_id, asset_code, issuer_address, contract_address, asset_type, updated_at) VALUES
    (1, 'yXLM', 'GARDNV3QAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', '', 'credit',  '2026-08-01 00:00:00'),
    (9, 'XLM',  '',                                                     '', 'classic', '2026-08-01 00:00:00'),
    (2, 'USDC', 'GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN', '', 'credit', '2026-08-01 00:00:00'),
    (7, 'DUPA', 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA1', '', 'credit', '2026-08-01 00:00:00'),
    (7, 'DUPB', 'GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA2', '', 'credit', '2026-08-01 00:00:00');

-- ── Test A input ────────────────────────────────────────────────────────────
-- One hour of _15m sub-buckets for yXLM/XLM on sdex. The two EARLIER
-- sub-buckets are enriched; the two LATER ones are not (enrichment runs
-- rate(1 hour), so the tail of a fresh hour is routinely unpriced).
INSERT INTO prices.price_ohlcv_15m
    (timestamp, asset_id, quote_asset_id, source, open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES
    ('2026-08-04 13:00:00', 1, 2, 'sdex', 0.170, 0.172, 0.169, 0.1700, 10000, 1700, 1700, 0.1700, 0.17, 40, 100),
    ('2026-08-04 13:15:00', 1, 2, 'sdex', 0.170, 0.172, 0.169, 0.1710, 12000, 2052, 2052, 0.1710, 0.171, 50, 100),
    ('2026-08-04 13:30:00', 1, 2, 'sdex', 0.170, 0.172, 0.169, 0.1690, 11000, 1859, 0, 0, 0.169, 45, 100),
    ('2026-08-04 13:45:00', 1, 2, 'sdex', 0.170, 0.172, 0.169, 0.1720,  9000, 1548, 0, 0, 0.172, 35, 100);

-- ── Test B input ────────────────────────────────────────────────────────────
-- BE's measured yXLM case, at _1h grain. The 13:00 bucket holds a real-volume
-- row that is not yet enriched and a 0.764-unit dust print that is. The 12:00
-- bucket is the control: same dust print, but everything enriched.
INSERT INTO prices.price_ohlcv_1h
    (timestamp, asset_id, quote_asset_id, source, open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES
    ('2026-08-04 12:00:00', 1, 2, 'sdex',   0.170, 0.172, 0.169, 0.1700, 42038, 7146, 7146, 0.17000, 0.17, 300, 100),
    ('2026-08-04 12:00:00', 1, 9, 'soroswap', 1.30, 1.31, 1.30, 1.3085,  0.764,    1,    1, 1.30850, 1.3085, 1, 100),
    ('2026-08-04 13:00:00', 1, 2, 'sdex',   0.170, 0.172, 0.169, 0.1700, 42038, 7146, 7146, 0,       0.17, 300, 100),
    ('2026-08-04 13:00:00', 1, 9, 'soroswap', 1.30, 1.31, 1.30, 1.3085,  0.764,    1,    1, 1.30850, 1.3085, 1, 100);

-- ── Test C input ────────────────────────────────────────────────────────────
-- Native XLM's trailing minutes. Every candle up to 13:58 is enriched; the two
-- newest are not — 13:59 because enrichment has not run yet, 14:00 because its
-- quote is exotic (no oracle, not USDC/USDT/XLM) and so will NEVER be enriched.
INSERT INTO prices.price_ohlcv_1m
    (timestamp, asset_id, quote_asset_id, source, open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES
    ('2026-08-04 13:57:00', 9, 2, 'sdex', 0.42, 0.43, 0.41, 0.4200, 100000, 42000, 42000, 0.4200, 0.42, 90, 100),
    ('2026-08-04 13:58:00', 9, 2, 'sdex', 0.42, 0.43, 0.41, 0.4210,  90000, 37890, 37890, 0.4210, 0.421, 80, 100),
    ('2026-08-04 13:59:00', 9, 2, 'sdex', 0.42, 0.43, 0.41, 0.4205,  95000, 39947, 0, 0, 0.4205, 85, 100),
    ('2026-08-04 14:00:00', 9, 7, 'sdex', 3.10, 3.20, 3.05, 3.1500,     12,    38,     0, 0,      3.15,  1, 100);

-- ── Test D input ────────────────────────────────────────────────────────────
-- One candle, on the asset_id that two natural identities share.
INSERT INTO prices.price_ohlcv_1d
    (timestamp, asset_id, quote_asset_id, source, open, high, low, close,
     volume_base, volume_quote, volume_quote_usd, close_usd, vwap, trade_count, version) VALUES
    ('2026-08-03 00:00:00', 7, 2, 'sdex', 1.00, 1.10, 0.90, 1.0500, 5000, 5250, 5250, 1.0500, 1.05, 20, 100);
