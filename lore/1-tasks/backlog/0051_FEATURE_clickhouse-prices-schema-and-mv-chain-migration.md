---
id: "0051"
title: "ClickHouse `prices.*` schema + materialised-view rollup chain migration"
type: FEATURE
status: backlog
related_adr: ["0003", "0004", "0007"]
related_tasks: ["0060", "0061", "0059", "0063", "0052", "0011", "0038", "0046", "0050"]
tags: [layer-database, priority-high, effort-medium, milestone-M1, clickhouse, hetzner, schema, migrations, ddl]
milestone: 1
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../../docs/database-schema/clickhouse-prod-schema.sql"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md"
  - "./0050_FEATURE_be-side-prep-sns-mtls-prices-db-provisioning.md"
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
  - date: 2026-06-17
    status: backlog
    who: oski
    note: >
      Live-apply dependency repointed from 0050 → 0063. BE 0227
      (Hetzner deploy) shipped and prices-api is getting admin access,
      so the empty prices database + scoped users are now provisioned
      by the self-served task 0063 rather than by BE under 0050.
      Authoring + Docker-CH integration testing remain unblocked and
      can start now. Open item flagged: schema-apply needs a
      DDL-capable identity (prices_writer is write_no_ddl) — settle
      with 0063.
  - date: 2026-06-17
    status: backlog
    who: oski
    note: >
      **Major rescope.** Inspecting the codebase showed the schema is
      already shipped in packages/prices-clickhouse (tasks 0060/0061/
      0059): init.sql has every §3 table (assets, price_ohlcv_1m + 6
      rolled granularities, current_prices, oracle_prices,
      backfill_progress), rollups.sql has the full MV chain (built as
      REFRESH-EVERY refreshable MVs, not incremental — a deliberate
      0060/0059 design), views.sql has the read surface (0061), and the
      prices-clickhouse-init binary applies it all idempotently with a
      Docker integration test (views_it.rs). The original "author
      numbered DDL + build a versioned schema-apply runner" scope is
      therefore largely DONE. 0051 now narrows to the genuine remainder:
      apply that schema to the LIVE Hetzner prices DB over mTLS, seed
      backfill_progress, and decide the production apply strategy.
---

# ClickHouse `prices.*` schema + materialised-view rollup chain migration

> **Rescoped 2026-06-17:** the schema + apply tooling are already
> shipped in `packages/prices-clickhouse` (tasks 0060/0061/0059). This
> task no longer authors DDL or builds a runner — it **applies the
> existing schema to the live Hetzner `prices` DB over mTLS, seeds
> `backfill_progress`, and picks the production apply strategy.**

## Summary

Stand up the `prices.*` schema on the **live** Hetzner CH `prices`
database over HTTPS-mTLS, using the schema + apply tooling already
present in `packages/prices-clickhouse`. Seed the two canonical
`backfill_progress` rows (`sdex_archive`, `soroban_amm`) per §3.5,
which no existing migration does. Decide and document how schema is
applied/tracked in production.

## Context — what already exists vs what remains

`packages/prices-clickhouse` (task 0060, with 0061 + 0059) already
ships, and a Docker integration test (`tests/views_it.rs`) covers it:

- **Tables** (`schema/init.sql`): `assets` (`ReplacingMergeTree(updated_at)`),
  `price_ohlcv_1m` + the 6 rolled granularities (`_15m … _1M`,
  `ReplacingMergeTree(version)`, sort key per ADR 0003/0004),
  `current_prices`, `oracle_prices`, `backfill_progress`.
- **MV rollup chain** (`schema/rollups.sql`): `mv_ohlcv_1m_to_15m …
  _1w_to_1M`, built as **refreshable MVs** (`REFRESH EVERY …`) — a
  deliberate 0060/0059 design, not the incremental cascade the original
  ADR 0007 §3.4 sketch implied. MV version propagation is task 0059.
- **Read-surface views** (`schema/views.sql`, task 0061).
- **Apply tooling**: `apply_init_sql` / `apply_sql` + the
  `prices-clickhouse-init` binary (applies init + views always,
  rollups opt-in via `--rollups`), idempotent (`CREATE … IF NOT EXISTS`).

What is **not** done — the remaining 0051 work:

1. The init binary connects via plaintext `CLICKHOUSE_URL` (localhost).
   Nothing applies the schema to the **live Hetzner `prices` DB over
   Caddy:443 mTLS**.
2. `backfill_progress` is created but **never seeded** with the two
   canonical rows (no `INSERT … sdex_archive / soroban_amm` anywhere).
3. No decision recorded on the **production apply / version-tracking
   strategy** (wholesale-idempotent — as shipped, mirroring BE — vs the
   numbered-migrations + `schema_migrations` table the original plan
   sketched).

## Implementation Plan

### Step 1: Seed `backfill_progress`

Add the two canonical rows (`sdex_archive`, `soroban_amm`) with
`status='running'` per §3.5. Make it idempotent
and part of the standard apply path — either an `INSERT` block in
`init.sql` guarded against duplicates (the table is
`ReplacingMergeTree`, so a stable key + re-insert is safe) or a small
`schema/seed.sql` applied by the init binary. Cover it in
`views_it.rs` (`SELECT count() FROM prices.backfill_progress` = 2).

### Step 2: Decide the production apply / version-tracking strategy

Record the decision in `notes/S-prod-apply-strategy.md`:

- **Recommended — wholesale idempotent** (as already shipped): the
  `prices-clickhouse-init` binary applies `init.sql` + `views.sql`
  (+ `--rollups`) on demand; every statement is `CREATE … IF NOT
  EXISTS`. Mirrors BE's single-`init.sql` model; no `schema_migrations`
  table needed. Schema changes ship as edits to the SQL files + a
  re-run.
- **Alternative — numbered migrations + `schema_migrations`**: only if
  we hit a change that idempotent re-apply can't express safely
  (column drop, engine change). Defer until such a change exists; if
  adopted, spawn it as its own task rather than pre-building it here.

### Step 3: Wire the init binary to the live mTLS endpoint

The binary today builds a plaintext `client(&cfg)`. Add an
mTLS-capable apply path that uses **0052's `client_mtls`** so the same
schema can be applied to Caddy:443. Resolve the DDL-identity question
from 0063: `prices_writer` is `write_no_ddl`, so apply schema under an
admin/DDL-capable cert (short-lived) — **not** the writer identity.

### Step 4: Apply against the live Hetzner CH `prices` DB

Once 0063 has provisioned the database + a DDL-capable credential:

- Apply against **dev** first; verify the table/MV/view set with
  `SHOW TABLES FROM prices` + a `SHOW CREATE TABLE prices.price_ohlcv_1m`
  engine/ORDER BY check, and `backfill_progress` count = 2.
- Smoke the MV chain (refreshable): insert a `_1m` fixture row, trigger/
  wait a refresh, confirm it propagates up the chain.
- Apply against **staging** then **prod** once dev is clean.
- Capture each env's `SHOW TABLES FROM prices` output in
  `notes/G-live-schema-state.md` for provenance.

## Acceptance Criteria

Already satisfied by `prices-clickhouse` (0060/0061/0059) — kept for
provenance:

- [x] `prices.*` DDL exists and is idempotent (`schema/init.sql`),
      engines + sort keys per ADR 0003/0004/0007
- [x] MV rollup chain `mv_ohlcv_1m_to_15m … _1w_to_1M` exists
      (`schema/rollups.sql`, refreshable-MV design)
- [x] Apply tooling + Docker integration test
      (`prices-clickhouse-init`, `tests/views_it.rs`)

Remaining for this task:

- [ ] `prices.backfill_progress` seeded with the two canonical rows
      (`sdex_archive`, `soroban_amm`, `status='running'`), idempotently,
      asserted in `views_it.rs`
- [ ] Production apply/version-tracking strategy decided + recorded in
      `notes/S-prod-apply-strategy.md`
- [ ] Init binary can apply over mTLS via 0052's `client_mtls`, under a
      DDL-capable identity (not `prices_writer`)
- [ ] Schema applied to the live Hetzner `prices` DB for at least dev;
      table/MV/view set + `backfill_progress` count verified; output
      captured in `notes/G-live-schema-state.md`
- [ ] Refreshable MV chain smoke-verified live (fixture `_1m` row
      propagates up after refresh)

## Blocked on

- **0063** — needs the live `prices` database + endpoint + a
  DDL-capable credential before Steps 3–4 can apply against the
  cluster. (Was 0050; moved to self-served 0063 after BE 0227 shipped.)
- **0052** — Step 3 uses its `client_mtls` for the live apply path.
- **Steps 1–2 are unblocked now** — seeding `backfill_progress` and the
  strategy note need only the local Docker CH and can start immediately.

## Out of scope

- Authoring the `prices.*` DDL / MV chain / views — **done in 0060 /
  0061 / 0059**; this task only applies + seeds + decides strategy.
- Numbered-migration runner + `schema_migrations` table — deferred
  unless a non-idempotent change forces it (see Step 2); spawn as its
  own task if/when adopted.
- MV version-propagation correctness — task **0059**.
- Schema evolution beyond Tranche 1 — separate migrations per-need.
- Backfill of historical data — see 0027 / 0028 (SDEX) and 0053
  (Soroban AMM).

## Notes

- Engines and sort keys are not free design choices: ADR 0003,
  ADR 0004, and ADR 0007 §3.3 lock them in. Any DDL change needs a new
  ADR — but DDL authoring now lives in `prices-clickhouse`, not here.
- The MV chain shipped as **refreshable MVs** (`REFRESH EVERY …`), a
  0060/0059 design choice that diverges from ADR 0007 §3.4's
  incremental-cascade sketch. That divergence is owned by those tasks;
  0051 is faithful to what shipped, not to the original sketch.
