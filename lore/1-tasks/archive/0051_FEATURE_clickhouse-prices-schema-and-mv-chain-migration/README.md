---
id: "0051"
title: "ClickHouse `prices.*` schema + materialised-view rollup chain migration"
type: FEATURE
status: completed
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
  - date: 2026-06-17
    status: active
    who: oski
    note: >
      Activated to implement the two now-unblocked steps (no live
      cluster needed): seed backfill_progress with the two canonical
      rows + assert in views_it.rs, and record the production
      apply/version-tracking strategy decision. Live mTLS apply
      (Steps 3–4) stays pending 0052 + 0063.
  - date: 2026-06-18
    status: active
    who: oski
    note: >
      **Descoped Step 3 (remote mTLS apply path).** Studying BE's
      infra-hetzner + db-clickhouse showed BE never applies schema over
      mTLS: db-clickhouse-init runs as a docker-compose sidecar ON the box
      over the plaintext Docker bridge as the `default` admin, and BE
      deliberately REMOVED its remote-DDL users (migration_admin/
      partition_admin, BE task 0241). mTLS there is only the runtime
      read/write transport for the remote api/indexer Lambdas. 0063
      confirms prices-api gets the same posture — box admin via loopback
      (`clickhouse-client --user=default` on the box) — so a remote
      mTLS DDL apply path is unnecessary and against the grain. 0051 now
      applies via loopback-admin like BE; no mTLS code to write. The 0052
      dependency drops; only the live apply remains, gated on 0063 access
      handover.
  - date: 2026-06-18
    status: blocked
    who: oski
    note: >
      Descope merged to develop via PR #46 (squash 1a31da6). With Step 3
      descoped and Steps 1–2 already shipped, the only remaining work is
      Step 4 — the live loopback-admin apply + MV smoke test — which needs
      0063 to create the `prices` database and hand over box admin access.
      No code work remains on 0051 itself; moving to blocked on 0063.
  - date: 2026-06-22
    status: blocked
    who: oski
    note: >
      **Step 4 live apply executed (operator, by hand).** 0063 created the
      `prices` database on the single Hetzner `production` box `ch-prod-01`
      and admin access is in hand, so the schema was applied over loopback as
      the `default` admin via Route A — `init.sql` → `seed.sql` → `views.sql`
      → `rollups.sql` streamed through `ssh … docker exec app-clickhouse-1
      clickhouse-client --multiquery` (CH 26.3.10). `price_ohlcv_1m` engine
      (`ReplacingMergeTree(version)`) + sort key
      (`asset_id, quote_asset_id, source, timestamp`) verified live against
      ADR 0003/0004. Provenance note `notes/G-live-schema-state.md` created.
      Remaining before done: paste the `SHOW TABLES`/seed-count outputs into
      the note and run the §6 MV propagation smoke test. Stays blocked-form
      in `blocked/` until those two close, then archive.
  - date: 2026-06-22
    status: completed
    who: oski
    note: >
      **Done.** Step 4 verification closed. Live object set captured (24
      objects: 12 base ReplacingMergeTree tables + 6 rollup MVs + 6 read
      views), `backfill_progress` seed = 2, `price_ohlcv_1m` engine + sort key
      verified vs ADR 0003/0004, and the refreshable MV chain smoke-verified
      (`_1m` fixture → `_15m`; cleanup also demonstrated replace-on-refresh
      semantics). All evidence in `notes/G-live-schema-state.md`. Both
      remaining acceptance criteria now [x]; no code changed (operational
      apply + verification only). Archiving.

# ClickHouse `prices.*` schema + materialised-view rollup chain migration

> **Rescoped 2026-06-17:** the schema + apply tooling are already
> shipped in `packages/prices-clickhouse` (tasks 0060/0061/0059). This
> task no longer authors DDL or builds a runner — it **applies the
> existing schema to the live Hetzner `prices` DB, seeds
> `backfill_progress`, and picks the production apply strategy.**
>
> **Re-scoped 2026-06-18:** the live apply is done **over loopback as
> the box `default` admin**, mirroring BE's docker-compose sidecar
> model — *not* over a remote mTLS DDL connection. Step 3's mTLS apply
> path is **descoped** (see Design Decisions → Emerged #5). mTLS (0052)
> stays the runtime read/write transport for the 0038/0039/0040
> Lambdas, exactly as in BE.

## Summary

Stand up the `prices.*` schema on the **live** Hetzner CH `prices`
database over loopback as the box `default` admin (BE's sidecar model),
using the schema + apply tooling already present in
`packages/prices-clickhouse`. Seed the two canonical
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

1. The init binary already connects via plaintext `CLICKHOUSE_URL`
   (localhost) — exactly the transport the live apply needs. What is
   missing is simply **running it against the live box's loopback**
   (on the box, or via SSH tunnel to `localhost:8123`) as the `default`
   admin, once 0063 hands over access. No new transport code is needed.
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

### Step 3: ~~Wire the init binary to the live mTLS endpoint~~ — DESCOPED (2026-06-18)

**No mTLS apply path is built.** BE applies schema on the box over the
plaintext Docker bridge as the `default` admin (a `db-clickhouse-init`
sidecar) and deliberately removed its remote-DDL users (BE task 0241);
mTLS there is only the runtime read/write transport for the remote
Lambdas. 0063 gives prices-api the same posture — box admin via loopback
— so the existing plaintext `prices-clickhouse-init` (or native
`clickhouse-client --queries-file`) **already is** the apply path. The
remaining work is operational (Step 4), not code. See Design Decisions →
Emerged #5. `prices_writer` stays `write_no_ddl`; DDL runs as `default`
admin on the box, not as the writer identity.

### Step 4: Apply against the live Hetzner CH `prices` DB

Once 0063 has provisioned the database + handed over box admin access,
run the existing plaintext `prices-clickhouse-init` (or feed
`init.sql`/`seed.sql`/`views.sql` to native `clickhouse-client
--queries-file`) against the box's loopback `localhost:8123` as the
`default` admin — on the box or through an SSH tunnel:

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

- [x] `prices.backfill_progress` seeded with the two canonical rows
      (`sdex_archive`, `soroban_amm`, `status='running'`), idempotently,
      asserted in `views_it.rs` — `schema/seed.sql` + `apply_seed`, wired
      into `prices-clickhouse-init`; verified end-to-end (re-run preserves
      `current_ledger`)
- [x] Production apply/version-tracking strategy decided + recorded in
      `notes/S-prod-apply-strategy.md` — wholesale-idempotent (as shipped)
- [x] Apply transport decided: **loopback-admin, no mTLS path** — the
      existing plaintext `prices-clickhouse-init` is the apply tool;
      remote mTLS DDL descoped (mirrors BE; Design Decisions → Emerged #5)
- [x] Schema applied to the live Hetzner `prices` DB (the single
      `production` box `ch-prod-01` — no separate dev/staging CH box), over
      loopback as the `default` admin — **applied 2026-06-22** via Route A
      (streamed `init`/`seed`/`views`/`rollups` SQL through `ssh … docker exec
      clickhouse-client --multiquery`). 24 objects verified (12 base tables +
      6 rollup MVs + 6 read-surface views), `backfill_progress` = 2,
      `price_ohlcv_1m` engine + sort key verified against ADR 0003/0004.
      Output captured in `notes/G-live-schema-state.md`.
- [x] Refreshable MV chain smoke-verified live — fixture `_1m`
      (`source='smoke'`) row propagated into `_15m`; fixture cleaned up, which
      also confirmed the MV replace-on-refresh semantics
      (`G-live-schema-state.md` §6).

## Implementation Notes

Steps 1–2 implemented (2026-06-17):

- `packages/prices-clickhouse/schema/seed.sql` — guarded `INSERT … SELECT
  arrayJoin([...]) WHERE task_name NOT IN (SELECT task_name FROM
  backfill_progress)`. Seeds `sdex_archive` + `soroban_amm` (`status='running'`,
  placeholder ledger bounds 0).
- `src/lib.rs` — `SEED_SQL` const + `apply_seed()` (sibling to `apply_init_sql`).
- `src/bin/prices-clickhouse-init.rs` — applies seed between tables and views.
- `tests/views_it.rs` — `SEED_SQL` added to `setup_scratch`; new
  `backfill_progress_seed_is_idempotent` test asserts exactly two streams and
  that re-running the seed preserves a row's `current_ledger`.
- Verified against ClickHouse 25.6: both integration tests pass, and the init
  binary run twice (with simulated progress in between) keeps 2 distinct
  streams and preserves `current_ledger=777`.
- `notes/S-prod-apply-strategy.md` records the wholesale-idempotent decision.

## Design Decisions

### Emerged

1. **Seed as a guarded `INSERT`, not a blind one.** `backfill_progress` is
   `ReplacingMergeTree(updated_at)` keyed by `task_name`, so a plain re-insert
   with a fresh `updated_at` would replace the live row and reset progress. The
   `NOT IN` guard makes re-apply a true no-op. (Plan said "guarded against
   duplicates"; this is the concrete mechanism.)
2. **Seed lives in `schema/seed.sql` + `apply_seed`, applied by the init
   binary** rather than inlined into `init.sql` — keeps data-seeding separate
   from DDL while staying on the single idempotent apply path.
3. **Placeholder ledger bounds = 0.** The spec only mandated `status='running'`;
   the backfill streams (0028/0053) fill real bounds as they advance.
4. **Converted 0051 to directory form** to hold `notes/S-prod-apply-strategy.md`.
5. **Descoped Step 3's remote mTLS apply path; apply over loopback as the
   box `default` admin instead** (chosen over building an mTLS DDL path on
   the init binary via 0052's client). Studying BE's `infra-hetzner` +
   `crates/db-clickhouse` showed BE applies schema with a `db-clickhouse-init`
   docker-compose **sidecar** running on the box over the plaintext Docker
   bridge as `default` — and BE *removed* its remote-DDL users
   (`migration_admin`/`partition_admin`, BE task 0241). mTLS in BE is solely
   the runtime read/write transport for the remote `api`/`indexer` Lambdas
   (`mtls::client_from_lambda_env`), never the schema path. Task 0063 grants
   prices-api the same posture (box admin via loopback — `clickhouse-client
   --user=default` on the box, confirmed in 0063 Steps 1+3), so a remote
   mTLS DDL connection is unnecessary and re-introduces exactly what BE
   retired. Consequence: **no mTLS code in 0051**; the existing plaintext
   `prices-clickhouse-init` is the apply tool, and the 0052 dependency drops.
   `prices_writer` stays `write_no_ddl`; DDL is the loopback `default` admin.

## Blocked on

- **0063** — Step 4 needs the live `prices` database created + **box
  admin access handed over** (loopback `default`) before the schema can
  be applied to the cluster. (Was 0050; moved to self-served 0063 after
  BE 0227 shipped.)
- **~~0052~~** — no longer a dependency: Step 3's mTLS apply path is
  descoped (Emerged #5), so 0051 needs no code from the mTLS crate.
- **Steps 1–3 are done / need no live cluster** — seed + strategy shipped,
  and the apply transport is the existing plaintext loopback path. Only the
  live apply (Step 4) waits on 0063.

## Out of scope

- Authoring the `prices.*` DDL / MV chain / views — **done in 0060 /
  0061 / 0059**; this task only applies + seeds + decides strategy.
- Remote mTLS DDL apply path — **descoped** (Emerged #5); schema is
  applied over loopback as the box admin, mirroring BE. mTLS (0052) is
  the runtime read/write transport for the 0038/0039/0040 Lambdas only.
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
