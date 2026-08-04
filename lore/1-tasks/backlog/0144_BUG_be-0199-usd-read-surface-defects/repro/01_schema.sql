CREATE DATABASE IF NOT EXISTS prices;

-- Verbatim from packages/prices-clickhouse/schema/init.sql
CREATE TABLE IF NOT EXISTS prices.assets (
    asset_id         UInt32,
    asset_code       String,
    issuer_address   String,
    contract_address String,
    asset_type       LowCardinality(String),
    updated_at       DateTime DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_code, issuer_address, contract_address);

CREATE TABLE IF NOT EXISTS prices.price_ohlcv_1m (
    timestamp        DateTime      CODEC(DoubleDelta),
    asset_id         UInt32,
    quote_asset_id   UInt32,
    source           LowCardinality(String),
    open             Decimal(38, 14),
    high             Decimal(38, 14),
    low              Decimal(38, 14),
    close            Decimal(38, 14),
    volume_base      Decimal(38, 14) DEFAULT 0,
    volume_quote     Decimal(38, 14) DEFAULT 0,
    volume_quote_usd Decimal(38, 14) DEFAULT 0,
    close_usd        Decimal(38, 14) DEFAULT 0,
    vwap             Decimal(38, 14),
    trade_count      UInt32        DEFAULT 0,
    version          UInt64
)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, quote_asset_id, source, timestamp)
SETTINGS index_granularity = 8192;

CREATE TABLE IF NOT EXISTS prices.price_ohlcv_15m AS prices.price_ohlcv_1m;
CREATE TABLE IF NOT EXISTS prices.price_ohlcv_1h  AS prices.price_ohlcv_1m;
CREATE TABLE IF NOT EXISTS prices.price_ohlcv_1d  AS prices.price_ohlcv_1m;

CREATE TABLE IF NOT EXISTS prices.current_prices (
    asset_id       UInt32,
    price_usd      Decimal(38, 14) DEFAULT 0,
    price_xlm      Decimal(38, 14) DEFAULT 0,
    change_24h_pct Decimal(10, 4)  DEFAULT 0,
    change_7d_pct  Decimal(10, 4)  DEFAULT 0,
    volume_24h_usd Decimal(38, 14) DEFAULT 0,
    market_cap_usd Decimal(38, 14) DEFAULT 0,
    vwap_24h       Decimal(38, 14) DEFAULT 0,
    sources        String          DEFAULT '',
    updated_at     DateTime        DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_id);

-- Verbatim from packages/prices-clickhouse/schema/views.sql
CREATE OR REPLACE VIEW prices.price_usd_series_1h AS
SELECT
    multiIf(
        a.contract_address != '', 'contract',
        a.asset_code = 'XLM' AND a.issuer_address = '', 'native',
        'credit') AS asset_kind,
    if(a.contract_address != '', '', a.asset_code)     AS asset_code,
    if(a.contract_address != '', '', a.issuer_address) AS issuer_address,
    a.contract_address AS contract_address,
    p.timestamp        AS bucket,
    CAST(sum(toFloat64(p.close_usd) * toFloat64(p.volume_base)) / nullIf(sum(toFloat64(p.volume_base)), 0) AS Decimal(38, 14)) AS close_usd
FROM prices.price_ohlcv_1h AS p FINAL
INNER JOIN prices.assets AS a FINAL ON a.asset_id = p.asset_id
WHERE p.close_usd > 0
GROUP BY asset_kind, asset_code, issuer_address, contract_address, bucket;
