-- Minimal seed for a local /price load test (the `prices` database created by
-- docker-compose's init.sql). Idempotent-ish: ReplacingMergeTree collapses
-- duplicate rows on merge / FINAL.
--
--   curl --data-binary @packages/prices-api/loadtest/seed.sql \
--     'http://localhost:8123/?database=prices'
--
-- (split into one request per statement if your client doesn't multi-statement)
--
-- GOTCHA: if the refreshable MV `prices.mv_current_prices` (schema/current.sql)
-- has been applied to your local CH, it REPLACES `current_prices` every minute
-- and will wipe this manual seed. For load testing, start from a clean CH
-- (`docker compose down -v && docker compose up -d clickhouse`) so only the
-- schema (no MV) is present, OR drop the MV first:
--   DROP VIEW IF EXISTS prices.mv_current_prices;

-- `assets` derives asset_id from the identity (task 0139); current_prices
-- names the same id through the same expression.
INSERT INTO prices.assets
    (asset_code, asset_type, issuer_address, contract_address)
VALUES ('XLM', 'native', '', '');

INSERT INTO prices.current_prices
    (asset_id, price_usd, vwap_24h, volume_24h_usd, updated_at)
VALUES (xxh3(concat('XLM', ':', '', ':', '')), 0.5, 0.51, 1234.5, now());
