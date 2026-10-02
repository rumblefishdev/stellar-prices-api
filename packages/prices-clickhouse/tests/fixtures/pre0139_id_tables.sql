-- The 12 asset-id tables as they stood before task 0139 (UInt32 ids),
-- verbatim from schema/init.sql at efcd9254. Test data for the rekey ITs,
-- which rewrite `prices.` to a scratch database. Omitted from the original:
-- CREATE DATABASE and the asset_symbol table (no asset id).

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
    -- DEPRECATED (task 0067): enrichment moved to the single-writer
    -- `prices.asset_metadata`. Neither written nor read — do NOT wire a writer
    -- here (that re-arms the two-writer RMT clobber). Kept only to avoid a
    -- destructive DROP; safe to remove once no env references it.
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
-- ('sdex','phoenix','soroswap','aquarius','sushiswap','comet'). version =
-- ledger_seq × 1000 + intra-ledger order; ReplacingMergeTree(version) collapses
-- duplicate PKs. §3.2.
--
-- Price semantics (task 0286, ADR 0287): open/high/low/close come ONLY from the
-- bucket's PRICE-FORMING fills — open is the first such fill and close the last,
-- in fill order; high/low are their extremes. A bucket with none has no price
-- (all four zero) but keeps its volumes and trade_count. pf_trade_count,
-- pf_volume and pf_price_volume are that price-forming subset's count, base
-- volume and Σ price × base volume. Their DEFAULT expressions give a pre-0286
-- row its OLD meaning (every fill was price-forming); only a re-ingest changes
-- that (0286 phase 3).
--
-- DEPLOY ORDER for 0286 (task §4.9 — this order, not any other):
--   1. this schema,
--   2. enrichment worker + the coarse sweep + prices-api,
--   3. the rollup MV re-CREATE,
--   4. ingest LAST (ledger-processor, backfill binaries).
-- Reason: a pre-0286 MV takes min(low) over a dust-only minute and turns the new
-- zero low into a zero low for the whole coarse bucket, and pre-0286 enrichment
-- re-inserts a candle row WITHOUT the pf columns, so they fall back to the
-- DEFAULT expressions above and a dust-only minute starts reporting itself as
-- fully price-forming. Both are silent. Shipping ingest last means nothing
-- writes a zero-priced row until everything downstream understands one.
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
    version          UInt64,
    pf_trade_count   UInt32          DEFAULT trade_count,
    pf_volume        Decimal(38, 14) DEFAULT volume_base,
    pf_price_volume  Decimal(38, 14) DEFAULT volume_quote
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
-- volume_quote_usd).
--
-- ⚠️ That 0 is a SENTINEL with four meanings, kept on purpose (ADR 0292): not yet
-- enriched; never priceable (no oracle or reference market for the quote asset);
-- genuinely zero (assumed not to occur); and — since ADR 0287 — "this bucket has
-- no price" (`pf_trade_count = 0`, so `close = 0` and `rate × 0 = 0`, permanent
-- and correct). No aggregate may read it as a number: every reader and its guard
-- is listed in docs/database-schema/close-usd-zero-guardrails.md, and a new one
-- belongs there in the same PR. Added to the base CREATE above so fresh AS-copies inherit
-- it; these idempotent ALTERs add it to databases created before 0061, where the
-- AS-copies do NOT inherit a post-hoc base-table ALTER — so apply per table.
ALTER TABLE prices.price_ohlcv_1m  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_15m ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_1h  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_4h  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_1d  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_1w  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;
ALTER TABLE prices.price_ohlcv_1M  ADD COLUMN IF NOT EXISTS close_usd Decimal(38, 14) DEFAULT 0 AFTER volume_quote_usd;

-- Price-forming aggregates (task 0286, ADR 0287). Added to the base CREATE
-- above so fresh AS-copies inherit them; these idempotent ALTERs add them to
-- databases created before 0286, where the AS-copies do NOT inherit a post-hoc
-- base-table ALTER — so apply per table, exactly like close_usd above. They run
-- AFTER the close_usd block on purpose: a pre-0061 database must get close_usd
-- into its mid-table position first.
--
-- The DEFAULT expressions are what make this migration safe on a live database.
-- ClickHouse computes a DEFAULT over other columns on READ for parts written
-- before the ALTER, so every existing candle keeps reporting the pre-0286
-- meaning (every fill was price-forming) instead of reading zero. Nothing is
-- rewritten and no history is re-rolled in phase 1 (task 0286 §4.9).
ALTER TABLE prices.price_ohlcv_1m
    ADD COLUMN IF NOT EXISTS pf_trade_count UInt32 DEFAULT trade_count AFTER version,
    ADD COLUMN IF NOT EXISTS pf_volume Decimal(38, 14) DEFAULT volume_base AFTER pf_trade_count,
    ADD COLUMN IF NOT EXISTS pf_price_volume Decimal(38, 14) DEFAULT volume_quote AFTER pf_volume;
ALTER TABLE prices.price_ohlcv_15m
    ADD COLUMN IF NOT EXISTS pf_trade_count UInt32 DEFAULT trade_count AFTER version,
    ADD COLUMN IF NOT EXISTS pf_volume Decimal(38, 14) DEFAULT volume_base AFTER pf_trade_count,
    ADD COLUMN IF NOT EXISTS pf_price_volume Decimal(38, 14) DEFAULT volume_quote AFTER pf_volume;
ALTER TABLE prices.price_ohlcv_1h
    ADD COLUMN IF NOT EXISTS pf_trade_count UInt32 DEFAULT trade_count AFTER version,
    ADD COLUMN IF NOT EXISTS pf_volume Decimal(38, 14) DEFAULT volume_base AFTER pf_trade_count,
    ADD COLUMN IF NOT EXISTS pf_price_volume Decimal(38, 14) DEFAULT volume_quote AFTER pf_volume;
ALTER TABLE prices.price_ohlcv_4h
    ADD COLUMN IF NOT EXISTS pf_trade_count UInt32 DEFAULT trade_count AFTER version,
    ADD COLUMN IF NOT EXISTS pf_volume Decimal(38, 14) DEFAULT volume_base AFTER pf_trade_count,
    ADD COLUMN IF NOT EXISTS pf_price_volume Decimal(38, 14) DEFAULT volume_quote AFTER pf_volume;
ALTER TABLE prices.price_ohlcv_1d
    ADD COLUMN IF NOT EXISTS pf_trade_count UInt32 DEFAULT trade_count AFTER version,
    ADD COLUMN IF NOT EXISTS pf_volume Decimal(38, 14) DEFAULT volume_base AFTER pf_trade_count,
    ADD COLUMN IF NOT EXISTS pf_price_volume Decimal(38, 14) DEFAULT volume_quote AFTER pf_volume;
ALTER TABLE prices.price_ohlcv_1w
    ADD COLUMN IF NOT EXISTS pf_trade_count UInt32 DEFAULT trade_count AFTER version,
    ADD COLUMN IF NOT EXISTS pf_volume Decimal(38, 14) DEFAULT volume_base AFTER pf_trade_count,
    ADD COLUMN IF NOT EXISTS pf_price_volume Decimal(38, 14) DEFAULT volume_quote AFTER pf_volume;
ALTER TABLE prices.price_ohlcv_1M
    ADD COLUMN IF NOT EXISTS pf_trade_count UInt32 DEFAULT trade_count AFTER version,
    ADD COLUMN IF NOT EXISTS pf_volume Decimal(38, 14) DEFAULT volume_base AFTER pf_trade_count,
    ADD COLUMN IF NOT EXISTS pf_price_volume Decimal(38, 14) DEFAULT volume_quote AFTER pf_volume;

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
    updated_at       DateTime      DEFAULT now(),
    method           LowCardinality(String) DEFAULT '',
    as_of            DateTime      DEFAULT toDateTime(0),
    price_status     LowCardinality(String) DEFAULT ''
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY (asset_id)
SETTINGS index_granularity = 8192;

-- Price provenance (task 0178). Extends [[0165]]'s price_usd_series vocabulary
-- to the tip surface, so a consumer can tell a MEASURED 1.0000 from a filled
-- one — without it, `price_usd` repeats the `close_usd = 0` mistake of one
-- value meaning several things.
--
--   'traded' — a real aggregate of candles some pricing tier already priced.
--   'oracle' — a measured depeg-aware rate from prices.usd_rate. USDC ONLY;
--              see the allowlist warning in current.sql before widening it.
--   ''       — the "unavailable" SENTINEL, not a vocabulary word: the asset has
--              no priced candle in the window, so `price_usd` is the 0 sentinel
--              and no method applies. This column is non-nullable like every
--              other column on this table, so absence must be a value.
--   'peg'    — reserved by 0165 and deliberately NOT emitted here. This surface
--              reads the real rate, so "no measured rate was available" is
--              never true of it.
--
-- ⚠️ Do NOT conflate with prices.usd_rate.method, which is a RATE-provenance
-- enum ('oracle'/'peg'/'pivot'/'pivot2') answering a different question. 0165
-- made them distinct deliberately (views.sql:171-174).
--
-- Idempotent ALTER for databases created before 0178, mirroring the close_usd
-- pattern above.
ALTER TABLE prices.current_prices ADD COLUMN IF NOT EXISTS method LowCardinality(String) DEFAULT '' AFTER updated_at;

-- The price's own age and what kind of price it is (task 0216). `updated_at` is
-- when this snapshot was REFRESHED — every row carries a timestamp a minute old
-- no matter how old the price is — so before these two columns no consumer
-- could apply a freshness policy at all.
--
--   as_of        — the timestamp of the candle `price_usd` was read from, i.e.
--                  the SAME predicate `price_usd` itself is chosen by. For the
--                  oracle arm it is the rate reading's own timestamp.
--   price_status — 'priced'   the price is the asset's newest price-forming
--                             candle (every oracle row reads this too).
--                  'carried'  a real priced close exists, but a NEWER
--                             price-forming candle has not been priced yet.
--                  'unpriced' `price_usd` is the 0 sentinel: no priced candle
--                             in the window, and `method` is '' for the same
--                             reason.
--                  ''         the "not yet rewritten" SENTINEL, not a
--                             vocabulary word: a row this table carries from
--                             before the MV was re-created. It can only be seen
--                             between this ALTER and that re-CREATE.
--
-- Both are non-nullable like every other column here, so absence must be a
-- value — and that is exactly the trap to respect on as_of. `maxIf(timestamp,
-- …)` over a window with no matching candle returns the DateTime DEFAULT, not
-- NULL: 1970-01-01, an ordinary-looking 56-year-old price sitting beside a 0.
-- The MV forces the epoch deliberately whenever there is no price
-- (current.sql's as_of projection) so the API can map exactly that value to ''
-- rather than publishing an age that is a coincidence of an empty aggregate.
--
-- Idempotent ALTERs for databases created before 0216, mirroring the method
-- pattern above. Kept as two statements, one per column, for the same reason
-- the method ALTER is its own statement: each is independently re-runnable.
ALTER TABLE prices.current_prices ADD COLUMN IF NOT EXISTS as_of DateTime DEFAULT toDateTime(0) AFTER method;
ALTER TABLE prices.current_prices ADD COLUMN IF NOT EXISTS price_status LowCardinality(String) DEFAULT '' AFTER as_of;

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

