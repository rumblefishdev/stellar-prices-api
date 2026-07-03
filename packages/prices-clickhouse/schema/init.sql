-- prices ClickHouse schema — production DDL (task 0060, mirrors BE
-- crates/db-clickhouse/schema/init.sql layout).
--
-- Single source of truth for the `prices` database. Applied by:
--   - the prices-clickhouse-init binary (local dev / CI schema-apply)
--   - cargo run -p prices-clickhouse --bin prices-clickhouse-init
--   - docker-entrypoint-initdb.d on first ClickHouse start
--
-- All statements are idempotent (CREATE … IF NOT EXISTS); applying twice
-- is a no-op. Keep this file free of inline string literals containing `;`
-- and of block comments — the Rust applier splits naively on `;` and strips
-- `-- …` line comments only (see src/lib.rs::split_statements).
--
-- ## Design references
--   - ADR 0003 — price_ohlcv PK includes quote_asset_id
--   - ADR 0004 — per-source rows; cross-source merge at read time
--   - ADR 0007 — live data sink on shared Hetzner ClickHouse
--   - docs/database-schema/database-schema-overview.md — full design
--
-- ## Column-type contract (load-bearing)
-- The tables the SDEX/soroban backfill writer touches — `assets`,
-- `price_ohlcv_1m`, `backfill_sdex_ledgers` — MUST keep these exact column
-- names and types: the writer uses positional clickhouse::Row inserts
-- (packages/sdex-backfill/src/sink.rs). Do not reorder or retype without
-- updating the Row structs.
--
-- ## Engine assignment
--   - OHLCV fact tables       → ReplacingMergeTree(version), monthly partitions
--   - state / registry tables → ReplacingMergeTree(updated_at)
--   - oracle reference table  → MergeTree (append-only), monthly partitions
--
-- ## Rollup chain
-- The _15m … _1M granularities are populated in production by the refreshable
-- MV chain in schema/rollups.sql (mechanism finalised in task 0051). Backfill
-- streams instead PRE-ROLL the coarse granularities with an explicit
-- INSERT … SELECT … FROM _1m FINAL GROUP BY bucket (overview §3.2). The MV
-- chain is intentionally NOT part of init.sql so schema-apply stays version-
-- agnostic (refreshable MVs need ClickHouse ≥ 23.12 + an experimental flag).

CREATE DATABASE IF NOT EXISTS prices;

----------------------------------------------------------------------
-- Asset registry (ReplacingMergeTree, last-write-wins on updated_at)
-- §3.1. Populated by the backfill's AssetRegistry and, in production, the
-- Asset Discovery Lambda. asset_id is an app-assigned UInt32 surrogate.
----------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS prices.assets (
    asset_id         UInt32,
    asset_code       String,
    asset_type       String,
    issuer_address   String        DEFAULT '',
    contract_address String        DEFAULT '',
    sac_address      String        DEFAULT '',
    home_domain      String        DEFAULT '',
    is_active        UInt8         DEFAULT 1,
    created_at       DateTime      DEFAULT now(),
    updated_at       DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_code, issuer_address, contract_address)
SETTINGS index_granularity = 8192;

-- The SAC contract address that wraps a classic asset (task 0061 §12.4): the
-- §12.4 collapse is write-time, so a SAC-wrapped leg's price lives under the
-- classic identity. This column lets a read-time consumer resolve their SAC
-- contract address back to the classic asset (see prices.identity_by_contract).
-- Added to the base CREATE; idempotent ALTER for pre-0061 databases.
ALTER TABLE prices.assets ADD COLUMN IF NOT EXISTS sac_address String DEFAULT '' AFTER contract_address;

----------------------------------------------------------------------
-- Asset enrichment (ReplacingMergeTree, last-write-wins on updated_at) — §0067.
-- SINGLE-WRITER table: only the discovery/enrichment worker writes here. Split
-- out of `prices.assets` because that table is a full-row-replace RMT with TWO
-- writers (ledger processor + discovery); a full-row `write_assets` re-emit
-- would clobber any enrichment column set on the shared row back to its default.
-- Keeping enrichment in its own single-writer table (same pattern as
-- `asset_supply`) makes it survive, and read views LEFT JOIN it. `home_domain`
-- stays as a DEFAULT '' column on `assets` for back-compat but is no longer read.
----------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS prices.asset_metadata (
    asset_id     UInt32,
    home_domain  String        DEFAULT '',
    updated_at   DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_id)
SETTINGS index_granularity = 8192;

----------------------------------------------------------------------
-- 1-minute OHLCV candles, per-source rows (ADR 0004). Live writes from the
-- Prices Ledger Processor; backfill streams write here with source in
-- ('sdex','phoenix','soroswap','aquarius'). version = ledger_seq × 1000 +
-- intra-ledger order; ReplacingMergeTree(version) collapses duplicate PKs.
-- §3.2.
----------------------------------------------------------------------

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

-- Rolled granularities — identical shape/engine/partition/order to _1m
-- (CREATE … AS copies the full schema). Populated by the rollup chain
-- (schema/rollups.sql) live, or pre-rolled by the backfill.

CREATE TABLE IF NOT EXISTS prices.price_ohlcv_15m AS prices.price_ohlcv_1m;
CREATE TABLE IF NOT EXISTS prices.price_ohlcv_1h  AS prices.price_ohlcv_1m;
CREATE TABLE IF NOT EXISTS prices.price_ohlcv_4h  AS prices.price_ohlcv_1m;
CREATE TABLE IF NOT EXISTS prices.price_ohlcv_1d  AS prices.price_ohlcv_1m;
CREATE TABLE IF NOT EXISTS prices.price_ohlcv_1w  AS prices.price_ohlcv_1m;
CREATE TABLE IF NOT EXISTS prices.price_ohlcv_1M  AS prices.price_ohlcv_1m;

-- Historical USD close (task 0061). close_usd = oracle_usd × close, computed at
-- enrichment time (DEFAULT 0 until the enrichment pass fills it, mirroring
-- volume_quote_usd). Added to the base CREATE above so fresh AS-copies inherit
-- it; these idempotent ALTERs add it to databases created before 0061, where the
-- AS-copies do NOT inherit a post-hoc base-table ALTER — so apply per table.
ALTER TABLE prices.price_ohlcv_1m  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_15m ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_1h  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_4h  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_1d  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_1w  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_1M  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;

----------------------------------------------------------------------
-- Current per-asset state (§3.3). One row per asset. Written by the Current
-- Price Updater Lambda; not exercised by the backfill (current-state, not
-- historical). Included for schema completeness.
----------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS prices.current_prices (
    asset_id         UInt32,
    price_usd        Decimal(38, 14),
    price_xlm        Decimal(38, 14),
    change_24h_pct   Decimal(10, 4),
    change_7d_pct    Decimal(10, 4),
    volume_24h_usd   Decimal(38, 14),
    market_cap_usd   Decimal(38, 14),
    vwap_24h         Decimal(38, 14),
    sources          String,
    updated_at       DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_id)
SETTINGS index_granularity = 8192;

----------------------------------------------------------------------
-- Per-asset circulating supply (task 0039 supply worker). Its OWN
-- single-writer table so supply (slow, hourly) and price (fast, per-minute
-- MV) never fight over a shared ReplacingMergeTree row. The current_prices
-- MV LEFT JOINs this for market_cap_usd; NULL/absent supply → NULL market
-- cap (best-effort, general-overview §3.3). Sole writer = the supply worker.
----------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS prices.asset_supply (
    asset_id      UInt32,
    token_supply  Decimal(38, 14),
    fetched_at    DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(fetched_at)
ORDER BY (asset_id)
SETTINGS index_granularity = 8192;

----------------------------------------------------------------------
-- Oracle reference prices (§3.4). ReplacingMergeTree, monthly partitions.
-- Written by the Oracle Fetcher Lambda in production; the backfill writes
-- REFLECTOR/REDSTONE samples decoded from soroban events. raw_data keeps the
-- forensic JSON of the decoded event.
--
-- ReplacingMergeTree (dedup on the full sort key (asset_id, oracle_name,
-- timestamp)) so a re-run / crash-resume that re-decodes the same ledger does
-- not accumulate duplicate samples — matching the idempotent re-INSERT
-- guarantee the price_ohlcv tables get from ReplacingMergeTree(version). Read
-- with FINAL (or rely on background merges) for the collapsed view.
----------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS prices.oracle_prices (
    timestamp     DateTime      CODEC(DoubleDelta),
    asset_id      UInt32,
    oracle_name   LowCardinality(String),
    price_usd     Decimal(38, 14),
    raw_data      String        CODEC(ZSTD(3))
)
ENGINE = ReplacingMergeTree
PARTITION BY toYYYYMM(timestamp)
ORDER BY (asset_id, oracle_name, timestamp)
SETTINGS index_granularity = 8192;

----------------------------------------------------------------------
-- Backfill bookkeeping.
----------------------------------------------------------------------

-- One row per processed ledger sequence. Resume source: startup queries this
-- to skip already-done ledgers. ReplacingMergeTree dedups re-inserts.
CREATE TABLE IF NOT EXISTS prices.backfill_sdex_ledgers (
    sequence         UInt32
)
ENGINE = ReplacingMergeTree()
ORDER BY (sequence)
SETTINGS index_granularity = 8192;

-- Per-stream progress for GET /backfill/status (§3.5).
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
ORDER BY (task_name)
SETTINGS index_granularity = 8192;

-- earliest_data_available (overview §4.5; task 0073 producer-half folded into
-- 0053): stored timestamp of the oldest OHLCV row this stream has landed,
-- recorded by the backfill as it lands older candles — NOT computed live via
-- MIN(timestamp) (timestamp is not the sort key → full scan). Nullable: unset
-- until the stream lands its first candle. The `?timeframe=all` backfill_note
-- and /backfill/status read it as-is (O(1)).
ALTER TABLE prices.backfill_progress ADD COLUMN IF NOT EXISTS earliest_data_available Nullable(DateTime) AFTER last_push_at;

-- newest_data_available (task 0053): companion to earliest_data_available — the
-- timestamp of the MOST RECENT OHLCV row this stream has landed. Together the
-- pair is the covered time-window of the stream, and both ends advance
-- monotonically per-partition in the forward single-pass (direction-agnostic,
-- unlike the ledger-directional current_ledger). Nullable: unset until the
-- stream lands its first candle. Read as-is (O(1)); never a live MAX() scan.
ALTER TABLE prices.backfill_progress ADD COLUMN IF NOT EXISTS newest_data_available Nullable(DateTime) AFTER earliest_data_available;

-- ---------------------------------------------------------------------
-- Asset Discovery high-water-mark (task 0054). One row per worker tracking
-- the highest ledger sequence the hourly discovery scan has processed, so
-- the next invocation resumes at last_ledger + 1 rather than re-scanning.
-- Single-writer = the asset-discovery worker. ReplacingMergeTree on the
-- worker key; read with FINAL.
-- ---------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS prices.discovery_state (
    worker        LowCardinality(String),   -- 'asset-discovery'
    last_ledger   UInt64,                    -- highest ledger sequence scanned
    updated_at    DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (worker)
SETTINGS index_granularity = 8192;

-- ---------------------------------------------------------------------
-- Unresolved AMM pools (task 0053, decision #3). One row per
-- (contract_id, source): a Soroban contract that emitted a swap-shaped event
-- while absent from the venue registry, so the swap could not be classified to
-- a venue/pool and its volume was dropped. On a clean forward-discovery
-- backfill (AMM window starting at Soroban activation) this table is EMPTY.
-- A still_unresolved=1 row is a genuine extractor gap to investigate:
-- sample_topics carries the event shape; first/last_ledger + swap_count size
-- the dropped volume. still_unresolved=0 means the pool registered later in the
-- run (only its early swaps were dropped). source is 'backfill' or 'live' — the
-- live processor may append the same shape. ReplacingMergeTree(version) with
-- version = last_ledger collapses re-runs on the (contract_id, source) key.
-- ---------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS prices.unresolved_pools (
    contract_id      String,
    source           LowCardinality(String),
    first_ledger     UInt32,
    last_ledger      UInt32,
    swap_count       UInt64,
    sample_topics    String        CODEC(ZSTD(3)),
    still_unresolved UInt8         DEFAULT 1,
    version          UInt64,
    updated_at       DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(version)
ORDER BY (contract_id, source)
SETTINGS index_granularity = 8192;

-- ---------------------------------------------------------------------
-- Discovered AMM pool registry (task 0053, decision #4). One row per pool the
-- forward-discovery backfill classified from a factory event — the persisted
-- output of the in-window registry so a partial re-backfill (a mid-history
-- window) or the live processor can LOAD it instead of re-deriving from Soroban
-- activation (this inverts task 0069: registry-as-output, not required-input).
-- venue = 'soroswap' | 'phoenix' | 'aquarius'. token0/token1 are the Soroswap
-- pair tokens (needed because a Soroswap swap event omits them); pool_type /
-- wasm_hash are Phoenix pool details; both default empty for venues that don't
-- use them. ReplacingMergeTree(updated_at) on contract_id collapses re-runs;
-- read with FINAL.
-- ---------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS prices.pool_registry (
    contract_id   String,
    venue         LowCardinality(String),
    token0        String        DEFAULT '',
    token1        String        DEFAULT '',
    pool_type     UInt32        DEFAULT 0,
    wasm_hash     String        DEFAULT '',
    updated_at    DateTime      DEFAULT now()
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (contract_id)
SETTINGS index_granularity = 8192;
