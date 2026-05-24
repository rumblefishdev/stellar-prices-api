-- Prices API — local ClickHouse schema for SDEX backfill
-- Applied automatically by docker-entrypoint-initdb.d on first start.
-- Idempotent: CREATE … IF NOT EXISTS on every object.

CREATE DATABASE IF NOT EXISTS prices;

-- §3.1 — Asset registry (ReplacingMergeTree, last-write-wins on updated_at)
CREATE TABLE IF NOT EXISTS prices.assets (
    asset_id         UInt32,
    asset_code       FixedString(12),
    asset_type       Enum8('classic' = 1, 'soroban' = 2),
    issuer_address   FixedString(56),
    contract_address FixedString(56),
    home_domain      String        DEFAULT '',
    is_active        UInt8         DEFAULT 1,
    created_at       DateTime      DEFAULT now(),
    updated_at       DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_code, issuer_address, contract_address)
SETTINGS index_granularity = 8192;

-- §3.2 — 1-minute OHLCV candles, per-source rows (ADR 0004)
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
    volume_quote_usd Decimal(38, 14) DEFAULT 0,
    vwap             Decimal(38, 14),
    trade_count      UInt32        DEFAULT 0,
    version          UInt64
)
ENGINE = ReplacingMergeTree(version)
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, quote_asset_id, source, timestamp)
SETTINGS index_granularity = 8192;

-- Backfill tracking — one row per processed ledger sequence.
-- Serves as the resume source: startup queries this table to find
-- which ledgers are already done (same pattern as BE's `ledgers` table).
-- MergeTree (append-only); ReplacingMergeTree dedup handles re-inserts.
CREATE TABLE IF NOT EXISTS prices.backfill_sdex_ledgers (
    sequence         UInt32
)
ENGINE = ReplacingMergeTree()
ORDER BY (sequence)
SETTINGS index_granularity = 8192;

-- §3.5 — Backfill progress for GET /backfill/status (cloud-side concern,
-- but included here so the schema is complete for local development).
CREATE TABLE IF NOT EXISTS prices.backfill_progress (
    task_name        LowCardinality(String),
    start_ledger     UInt64,
    target_ledger    UInt64,
    current_ledger   UInt64,
    status           Enum8('running' = 1, 'paused' = 2, 'completed' = 3, 'error' = 4)
                     DEFAULT 'running',
    last_push_at     Nullable(DateTime),
    started_at       DateTime      DEFAULT now(),
    completed_at     Nullable(DateTime),
    updated_at       DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (task_name);
