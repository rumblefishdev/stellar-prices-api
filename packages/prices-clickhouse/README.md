# prices-clickhouse

Schema + connection layer for the `prices` ClickHouse database. Mirrors BE's
`crates/db-clickhouse` layout: a single embedded `schema/init.sql` is the source
of truth, applied idempotently by the `prices-clickhouse-init` binary.

This crate stands up the schema and hands out a configured `clickhouse::Client`.
It deliberately owns **no** row structs or writers — the backfill / extractor
crates (`sdex-backfill`, the venue extractors) own their own write path.

## Layout

```
packages/prices-clickhouse/
├── Cargo.toml
├── README.md
├── schema/
│   ├── init.sql       # DATABASE + all prices.* tables (the source of truth)
│   ├── rollups.sql    # production refreshable-MV rollup chain (applied separately)
│   └── preroll.sql    # deterministic full-range _1m → _15m…_1M re-aggregate (measurement)
└── src/
    ├── lib.rs                         # Config, client(), apply_init_sql / apply_sql, embedded SQL
    └── bin/prices-clickhouse-init.rs  # CLI schema applier
```

## Tables (`prices.*`)

| Table | Engine | Partition | Written by |
|-------|--------|-----------|-----------|
| `assets` | `ReplacingMergeTree(updated_at)` | — | backfill registry / Asset Discovery |
| `price_ohlcv_1m` | `ReplacingMergeTree(version)` | `toYYYYMM(timestamp)` | backfill (per-source) / Ledger Processor |
| `price_ohlcv_15m`…`_1M` | `ReplacingMergeTree(version)` | `toYYYYMM(timestamp)` | rollup chain / backfill pre-roll |
| `current_prices` | `ReplacingMergeTree(updated_at)` | — | Current Price Updater (not backfilled) |
| `oracle_prices` | `ReplacingMergeTree` | `toYYYYMM(timestamp)` | backfill REFLECTOR/REDSTONE / Oracle Fetcher |
| `backfill_sdex_ledgers` | `ReplacingMergeTree` | — | backfill (resume cursor) |
| `backfill_progress` | `ReplacingMergeTree(updated_at)` | — | backfill streams |

The `assets`, `price_ohlcv_1m`, and `backfill_sdex_ledgers` column layouts are a
**contract** with `sdex-backfill/src/sink.rs` (positional `clickhouse::Row`
inserts) — do not retype/reorder without updating those structs.

## Quick start (local)

```bash
# 1. Bring up local ClickHouse (prices-api docker-compose) — db 'prices'
docker compose up -d clickhouse

# 2. Apply the schema
export CLICKHOUSE_URL=http://localhost:8123
export CLICKHOUSE_USER=default
export CLICKHOUSE_PASSWORD=clickhouse
cargo run -p prices-clickhouse --bin prices-clickhouse-init

# 2b. (optional) also create the production refreshable-MV rollup chain
cargo run -p prices-clickhouse --bin prices-clickhouse-init -- --rollups

# 3. Verify (expect 12 tables in db 'prices')
docker exec <ch-container> clickhouse-client -q \
  "SELECT count() FROM system.tables WHERE database='prices'"
```

## Rollups: MV chain vs. pre-roll

- **Live path** — `rollups.sql` defines 6 refreshable MVs (`_1m → _15m → … →
  _1M`), each re-aggregating a bounded recent window from the previous
  granularity's `FINAL`. Mechanism finalised in task 0051; needs CH ≥ 23.12.
- **Backfill / historical** — `preroll.sql` re-aggregates the whole range from
  `_1m FINAL` into each coarser table (one row per bucket). Used by the task
  0060 sizing measurement so the coarse tables are populated deterministically,
  independent of the live MVs' time window.

## Notes

- `init.sql` is intentionally MV-free so schema-apply stays version-agnostic.
- Schema design: `docs/database-schema/database-schema-overview.md`, ADRs 0003 /
  0004 / 0007.
