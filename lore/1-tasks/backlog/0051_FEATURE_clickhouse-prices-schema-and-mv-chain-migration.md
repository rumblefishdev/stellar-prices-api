---
id: '0051'
title: 'ClickHouse `prices.*` schema + materialised-view rollup chain migration'
type: FEATURE
status: backlog
related_adr: ['0003', '0004', '0007']
related_tasks: ['0050', '0011', '0038', '0046']
tags:
  [
    layer-database,
    priority-high,
    effort-medium,
    milestone-M1,
    clickhouse,
    hetzner,
    schema,
    migrations,
    ddl,
  ]
milestone: 1
links:
  - '../../../docs/prices-api-general-overview.md'
  - '../../../docs/database-schema/clickhouse-prod-schema.sql'
  - '../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md'
  - '../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md'
  - '../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md'
  - '../archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md'
  - './0050_FEATURE_be-side-prep-sns-mtls-prices-db-provisioning.md'
history:
  - date: 2026-05-21
    status: backlog
    who: okarcz
    note: >
      Spawned during Tranche 1 task-set creation. The §3 schema
      (assets, per-granularity OHLCV tables, MV chain, current_prices,
      oracle_prices, backfill_progress) is fully specified in the
      design doc and partially mirrored in
      docs/database-schema/clickhouse-prod-schema.sql, but no task
      owns the act of applying it to the Hetzner CH cluster. 0011
      stops at AWS CDK; 0038 assumes the schema exists. This task
      fills the gap.
---

# ClickHouse `prices.*` schema + materialised-view rollup chain migration

## Summary

Author the full `prices.*` DDL (Section 3 tables + MV rollup chain)
and the migration tooling that applies it idempotently to the
Hetzner CH cluster's `prices` database over HTTPS-mTLS. Seeds the
two canonical `backfill_progress` rows (`sdex_archive`,
`soroban_amm`) per §3.5. Output is the source of truth for the
ingestion, periodic-worker, push, and read paths.

## Context

Per §3 of the general-overview doc, all live prices data lives in
the Hetzner CH `prices` database. The schema comprises:

- `prices.assets` — `ReplacingMergeTree(updated_at)`, surrogate
  `asset_id`, sort key `(asset_code, issuer_address, contract_address)`.
- Seven per-granularity OHLCV tables — `prices.price_ohlcv_1m`,
  `_15m`, `_1h`, `_4h`, `_1d`, `_1w`, `_1M` — all
  `ReplacingMergeTree(version)`, monthly partitions, sort key
  `(asset_id, quote_asset_id, source, timestamp)` per ADR 0003 +
  ADR 0004.
- Six materialised views — `mv_ohlcv_1m_to_15m`, `_15m_to_1h`,
  `_1h_to_4h`, `_4h_to_1d`, `_1d_to_1w`, `_1w_to_1M` — replacing
  the previously-scheduled OHLCV Rollup Lambda (ADR 0007 §3.4).
- `prices.current_prices` — `ReplacingMergeTree(updated_at)`.
- `prices.oracle_prices` — `MergeTree`, monthly partitions.
- `prices.backfill_progress` — `ReplacingMergeTree(updated_at)`,
  seeded with two rows per §3.5.

`docs/database-schema/clickhouse-prod-schema.sql` already mirrors
BE's production CH schema as a reference; this task produces the
_prices_-side equivalent inside the `prices` database and ships
the migration tooling that applies it.

## Implementation Plan

### Step 1: Pick a migration runner

Choose between:

- **Hand-rolled SQL files + a Rust binary** (`schema-apply`)
  that connects via the 0052 ClickHouse client and runs `.sql`
  files in numeric order, recording applied versions in a small
  `prices.schema_migrations` table. Mirrors BE's
  `db-clickhouse/migrations/` shape.
- An external tool like `clickhouse-driver`/`refinery` if a
  ready-made CH-aware runner exists in the Rust ecosystem at
  impl time.

Recommend hand-rolled: keeps the tooling surface small, matches
BE's pattern, no extra dependency. Decide at impl time and
record the decision in a short `notes/S-migration-runner.md`.

### Step 2: Author migration files

Write the DDL in numbered files under `db/migrations/clickhouse/`:

```
001_assets.sql
002_price_ohlcv_1m.sql
003_price_ohlcv_higher_granularities.sql
004_mv_rollup_chain.sql
005_current_prices.sql
006_oracle_prices.sql
007_backfill_progress.sql
008_seed_backfill_progress_rows.sql
009_schema_migrations.sql           (the tracking table itself; created first in practice)
```

Each file is idempotent (`CREATE TABLE IF NOT EXISTS`,
`CREATE MATERIALIZED VIEW IF NOT EXISTS`). Each DDL statement
mirrors §3 of the design doc verbatim, with the engine and sort
key choices grounded in ADR 0003 (PK includes quote_asset_id),
ADR 0004 (multi-source merge columns), and ADR 0007 (per-source
rows + MV chain).

### Step 3: Implement `schema-apply` runner

Small Rust binary depending on the 0052 ClickHouse client crate.
Behaviour:

1. Connect via mTLS to Caddy:443 using the env-scoped cert + key.
2. Ensure `prices.schema_migrations(version UInt32, applied_at
DateTime DEFAULT now()) ENGINE = ReplacingMergeTree(applied_at)
ORDER BY (version)` exists (bootstrapping).
3. Read the highest applied version; for each file with a higher
   numeric prefix, execute its statements in order, then INSERT
   the new version.
4. Exit non-zero on any DDL failure; print which file failed.

### Step 4: Integration test

Use a Docker `clickhouse-server` (no mTLS, plaintext) to apply
the migrations end-to-end. Assert:

- `SHOW TABLES FROM prices` returns the 7 OHLCV tables + 3
  metadata tables + 6 MVs.
- `SHOW CREATE TABLE prices.price_ohlcv_1m` matches the §3.2
  schema verbatim (regex match on engine + ORDER BY clause).
- `SELECT count() FROM prices.backfill_progress` returns 2.
- Re-running the runner against the same DB is a no-op.

### Step 5: Apply against the Hetzner CH `prices` DB

Once 0050 has provisioned the database and credentials:

- Apply against dev env first; verify with `clickhouse-client
--secure --host=caddy.example.com:443` + the prices-api user.
- Run the Step 4 assertions against the live `prices` DB.
- Apply against staging and prod once dev is clean.

## Acceptance Criteria

- [ ] `db/migrations/clickhouse/` contains all numbered DDL
      files; each is idempotent and matches §3 verbatim
- [ ] `schema-apply` Rust binary applies the migration set
      against a Docker CH from a clean slate without error
- [ ] Re-running the binary is a no-op (no schema drift, no
      duplicate rows in `schema_migrations`)
- [ ] Integration test asserts table list, engine signatures,
      sort keys, and partition expressions
- [ ] Applied against the live Hetzner CH `prices` database
      for at least the dev env; `SHOW TABLES FROM prices`
      output captured in a `notes/G-dev-schema-state.md` for
      provenance
- [ ] `prices.backfill_progress` shows two seeded rows
      (`sdex_archive`, `soroban_amm`) with `status='running'`
- [ ] MV chain backfills: insert a fixture 1-min row, observe
      it cascades through 15m → 1h → 4h → 1d → 1w → 1M

## Blocked on

- **0050** — needs the `prices` database, user, and Hetzner CH
  endpoint to exist before the runner can apply against the live
  cluster. Docker-CH integration test does not depend on 0050;
  authoring + Docker testing can start in Week 1 in parallel.
- **0052** — needs the shared ClickHouse client crate so the
  runner is not the only thing importing the `clickhouse` Rust
  crate directly. Could be relaxed if 0052 slips: a self-contained
  client in the migration runner is acceptable for v1.

## Out of scope

- Schema evolution beyond Tranche 1 — column adds / drops as the
  product matures are separate migrations spawned per-need.
- `default.*` cross-DB views — none required for Tranche 1
  (§3 lists none); ADR 0007 §3.7 documents the policy if/when
  one is needed.
- Backfill of historical data — see 0027 / 0028 (SDEX) and 0053
  (Soroban AMM).
- Performance tuning beyond what §3 sort keys specify.

## Notes

- Engines and sort keys are not free design choices: ADR 0003,
  ADR 0004, and ADR 0007 §3.3 lock them in. Treat any deviation
  in DDL authoring as needing a new ADR.
- The MV chain replaces the previously-scheduled OHLCV Rollup
  Lambda (ADR 0007 §3.4). If a future tranche revisits per-MV
  cost or correctness, that decision lives in its own ADR; this
  task is faithful to ADR 0007.
