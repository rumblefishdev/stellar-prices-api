---
id: "0039"
title: "Prices periodic workers — 4 EventBridge-Scheduler-triggered ClickHouse Lambdas (price updater, oracle, discovery, cleanup; rollup eliminated)"
type: FEATURE
status: active
related_adr: ["0003", "0004", "0006", "0007"]
related_tasks: ["0011", "0038", "0045", "0047"]
tags: [layer-indexing, priority-high, effort-large, lambda, scheduler, rust, aws, ingestion, clickhouse, hetzner]
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "../backlog/0011_FEATURE_bootstrap-cdk-with-ssm-platform-lookups.md"
  - "../backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md"
  - "./0038_FEATURE_prices-ledger-processor-lambda.md"
history:
  - date: 2026-05-18
    status: backlog
    who: oski
    note: >
      Drafted to fill the gap between the live ingestion Lambda
      (0038) and the read-side API (0040). The general-overview
      §2.1 and §5.3 / §5.4 list five EventBridge-Scheduler-driven
      Lambdas that maintain derived tables and the asset
      registry; none of them was represented by an existing task.
      Sequenced AFTER 0038 because every worker either reads
      from or rolls up `price_ohlcv` rows that the Ledger
      Processor produces — running the periodic workers before
      ingestion is live yields empty/garbage outputs.
  - date: 2026-05-18
    status: blocked
    who: oski
    by: ["0011", "0038"]
    note: >
      Moved to blocked/ — 0011 provides Lambda + EventBridge +
      Secrets Manager CDK scaffolding; 0038 must be producing
      `price_ohlcv` rows before the periodic workers have
      useful input. Promote only once both are archived.
  - date: 2026-05-18
    status: blocked
    who: okarcz
    note: >
      Redesign pending. Task 0044's research (synthesis §3) and
      ADR 0007 (proposed) call for major rewrite — the OHLCV
      Rollup Lambda is **deleted entirely** (replaced by a CH
      MaterializedView chain `1m → 15m → ... → 1M`); Current
      Price Updater, Oracle Fetcher, Asset Discovery, Cleanup
      Worker retargeted from RDS to ClickHouse. Cleanup becomes
      `ALTER TABLE … DROP PARTITION` per per-granularity table.
      Hold rewrite until ADR 0007 accepted.
  - date: 2026-05-20
    status: blocked
    who: okarcz
    note: >
      ADR 0007 accepted via task 0045's closure (agreement record
      at archive/0045_.../notes/G-be-agreement-record.md is the
      cross-team contract). Architectural uncertainty resolved;
      remaining gates are engineering: BE 0227 (Hetzner mTLS
      endpoint) + task 0047 (throughput verification GREEN/YELLOW).
      OHLCV Rollup Lambda confirmed deleted in the rewrite (per
      ADR 0007 §3.4 — CH MV chain replaces it). Task stays
      blocked; rewrite begins after 0038 lands.
  - date: 2026-06-25
    status: active
    who: oski
    note: >
      Promoted blocked → active. Blocker 0038 landed (PR #34 merged
      to develop) and 0011 is archived, so both gates are cleared.
      Body/plan rewrite to the ClickHouse 4-worker scope (see the
      2026-06-25 validation below) lands on the impl branch, not
      this status-only push.
---

# Prices periodic workers — 4 EventBridge-Scheduler-triggered ClickHouse Lambdas

> **Scope corrected 2026-06-25** against ADR 0007 (accepted) + the live
> ClickHouse schema. The original 5-Lambda / RDS-Postgres design is
> superseded: the **OHLCV Rollup Lambda is eliminated** (replaced by the
> live CH refreshable-MV chain) and the surviving four retarget from
> RDS/`sqlx`/VPC to ClickHouse/mTLS/no-VPC. See **Architecture validation
> (2026-06-25)** below for the per-worker verdict and evidence.

## Summary

Implement the **four** EventBridge-Scheduler-driven Lambdas that survive
the ADR 0007 ClickHouse refactor: **Current Price Updater** (rate 1 min),
**Oracle Fetcher** (rate 5 min, Reflector via Soroban RPC
`simulateTransaction`), **Asset Discovery** (rate 1 hour), and **Cleanup
Worker** (cron 02:00 UTC daily). All four are Rust binaries on
`provided.al2` via `lambda_runtime` per ADR 0006, run **outside any VPC**
(ADR 0007 §6), and read/write the `prices.*` database in BE's Hetzner
ClickHouse over HTTPS-mTLS (the same `prices-clickhouse::mtls` sink seam
0038 uses), **not** RDS. The **OHLCV Rollup Lambda is dropped** — rollups
are the live CH materialized-view chain in
`packages/prices-clickhouse/schema/rollups.sql` (task 0051, already on
prod).

## Context

The general-overview doc §2.1 lists these five workers as
Prices-API-budgeted components alongside the Ledger Processor and
the API handlers. §5.3 specifies each worker's trigger, source,
and output; §5.4 fixes the EventBridge Scheduler rule expressions:

```
oracle-ingest:     rate(5 minutes)  → Lambda "oracle-worker"
asset-discovery:   rate(1 hour)     → Lambda "discovery-worker"
price-update:      rate(1 minute)   → Lambda "price-updater"
retention-cleanup: cron(0 2 * * *)  → Lambda "cleanup-worker"
# ohlcv-rollup (rate 15m) — REMOVED: now the CH MV chain (rollups.sql)
```

The Ledger Processor (0038) is event-driven (S3 PutObject), not
schedule-driven — it lives outside this task.

Why this is one task, not four: all four share the same deployment
shape (`provided.al2` + EventBridge Scheduler rule + ClickHouse mTLS
client + Secrets Manager, **no VPC**), the same CDK stack structure,
and the same observability harness. Splitting them would create four
copies of the same scaffolding. They can be implemented incrementally
within one task — each worker is one binary + one rule + one set of
acceptance criteria — but the deployment, CI, and CDK scaffolding is
built once.

## Architecture validation (2026-06-25)

Re-validated every planned worker against **ADR 0007** (accepted
2026-05-20) and the live `packages/prices-clickhouse/schema/`. The
original §5.3/§5.4 design predates the RDS→ClickHouse flip; this is the
reconciled scope.

| Worker | Verdict | Basis |
|--------|---------|-------|
| **OHLCV Rollup** `rate(15m)` | ❌ **ELIMINATED** | ADR 0007 §3.4 "Rollup Lambda eliminated." Replaced by the refreshable-MV chain `1m→15m→…→1M` in `schema/rollups.sql`; task **0051** archived (chain live on prod 2026-06-22). |
| **Current Price Updater** `rate(1m)` | ✅ KEEP (→CH) | Cross-source VWAP compute → `prices.current_prices` (`ReplacingMergeTree`, `schema/init.sql`). External compute, not a pure MV. ⚠️ open Q below. |
| **Oracle Fetcher** `rate(5m)` | ✅ KEEP (→CH) | Fetches Reflector via Soroban RPC `simulateTransaction` (external I/O — cannot be an MV) → `prices.oracle_prices`. |
| **Asset Discovery** `rate(1h)` | ✅ KEEP (→CH) | Scans ledgers for new assets → `prices.assets`. NOT folded into 0038 (which only `load_registry()` reads). ⚠️ overlaps task **0054** (minimal T1 carve-out) — absorb/extend, don't duplicate. |
| **Cleanup/Retention** `cron daily` | ✅ KEEP (thin) | `ALTER TABLE … DROP PARTITION` per per-granularity table (ADR 0007 §3.3). No declarative `TTL` in schema → retention stays procedural. |

**Cross-cutting retargets (apply to all four):**

- **RDS/`sqlx` → ClickHouse.** Reuse 0038's `prices-clickhouse::mtls`
  sink seam; no Postgres, no UPSERT — `ReplacingMergeTree` INSERT +
  read-time `FINAL`/`argMax` (ADR 0007 §3, Consequences).
- **No VPC / NAT / SG / RDS-IAM** (ADR 0007 §6). The Step-7 CDK wiring
  drops all VPC + RDS-IAM scaffolding; creds are the mTLS cert/key in
  Secrets Manager (2 secrets/env), same as 0038.
- Net Lambda count: **4, not 5**.

**Open design questions (resolve at each worker's impl start, not now):**

1. **Current Price Updater — Lambda vs. view.** A `current_price_usd`
   read view already exists (`schema/views.sql`). Confirm the 1-min
   write-Lambda into `current_prices` is still warranted over a
   read-time/MV approach before building it.
2. **Asset Discovery vs. task 0054.** Decide whether 0039 absorbs 0054's
   minimal discovery binary or 0054 ships first and 0039 extends it.

## Implementation Plan

### Step 1: Shared crate scaffolding

Add `packages/periodic-workers/` (or four sibling binary crates
under `packages/`) sharing a common library for: the **ClickHouse
mTLS client** (reuse 0038's `prices-clickhouse::mtls` seam — Secrets
Manager cert/key fetch via the Parameters/Secrets extension, **no
`sqlx`, no RDS pool**), structured CloudWatch logging, `lambda_runtime`
entrypoint boilerplate, and a small "ran at" telemetry helper.

### Step 2: Current Price Updater (`price-updater`)

- Trigger: EventBridge Scheduler `rate(1 minute)`.
- Behaviour (§5.3, §5.5): read latest per-asset 1-min candles
  from `prices.price_ohlcv_1m` (CH), compute VWAP across sources
  using the §5.5 formula (with the §5.5 outlier-detection rule and
  the `min_volume_usd` threshold), and **INSERT** into
  `prices.current_prices` (`ReplacingMergeTree`, `schema/init.sql`) —
  no Postgres UPSERT; dedup is engine-side, read via `FINAL`.
- CH `current_prices` is column-shaped, not a Postgres JSONB blob;
  use its actual column layout in `schema/init.sql`. (See open Q#1 —
  confirm Lambda vs. the existing `current_price_usd` view first.)

### ~~Step 3: OHLCV Rollup (`rollup-worker`)~~ — ELIMINATED

**Removed per ADR 0007 §3.4.** Rollups are the live ClickHouse
refreshable-MV chain `mv_ohlcv_1m_to_15m → … → 1w_to_1M` in
`packages/prices-clickhouse/schema/rollups.sql` (built/verified by task
0051, on prod since 2026-06-22; correctness hardening tracked in 0059).
No Lambda, no EventBridge rule, no acceptance criterion for rollup in
this task. Nothing to build here — left as a tombstone so the §5.4 rule
list isn't re-added by mistake.

### Step 4: Oracle Fetcher (`oracle-worker`)

- Trigger: EventBridge Scheduler `rate(5 minutes)`.
- Behaviour (§5.3, §2.2): call Reflector's oracle contract via
  Soroban RPC `simulateTransaction`. Write results to
  `oracle_prices` (§3.4) keyed by `(timestamp, asset_id,
  oracle_name)` — partition pattern matches `price_ohlcv`.
- **Failure stance** (§2.2, §5.3): non-critical. A failed
  fetch logs + emits a metric but does not block other
  workers; the column "shows `null` or last known value"
  downstream.
- Soroban RPC endpoint + Reflector contract address read from
  Secrets Manager (per §2.1 row).

### Step 5: Asset Discovery (`discovery-worker`)

- Trigger: EventBridge Scheduler `rate(1 hour)`.
- **First check task 0054** (minimal T1 Asset Discovery carve-out) —
  absorb or extend it rather than building a parallel binary (open Q#2).
- Behaviour (§5.3): scan recent ledger account entries for new
  classic asset issuances and new SEP-41 / Soroban token
  contract deployments. INSERT into `prices.assets`
  (`ReplacingMergeTree`, `schema/init.sql`) keyed on
  `(asset_code, issuer_address, contract_address)`; dedup engine-side.
- Reads from the same `stellar-ledger-data/` S3 bucket as 0038
  (no separate ingestion path); selects the last hour of ledgers by
  S3 key prefix or by `closed_at` lookup against `prices.*` (CH).
- Soroswap / Aquarius API integration (§2.2) for pool pair
  metadata is in scope here — pool registries inform the
  Ledger Processor (0038) about which contracts to extract
  swaps from. Coordinate the pool-registry hand-off with the
  0037 Phoenix pool registry surface.

### Step 6: Cleanup Worker (`cleanup-worker`)

- Trigger: EventBridge Scheduler `cron(0 2 * * *)` (02:00 UTC
  daily).
- Behaviour (§3.6 Retention Policy, §5.3): delete expired
  fine-grained candles per the §3.6 policy, drop old monthly
  partitions on `price_ohlcv` / `oracle_prices`, and create
  upcoming partitions (2 months ahead per §3.2 comment).
- Idempotent: re-running on the same day is a no-op.

### Step 7: CDK + EventBridge wiring

In the `infra/` CDK app:

- One Lambda function definition per worker (four), using 0038's
  conventions: `provided.al2`, ARM64, **no VPC / no RDS-IAM** (ADR
  0007 §6), Secrets Manager read of the mTLS cert/key bundle.
- One EventBridge Scheduler rule per worker with the §5.4
  expressions verbatim (rollup rule omitted).
- DLQ + retry policy per worker (defer the exact DLQ shape to
  impl time; default to 2 retries + DLQ).
- CloudWatch alarms: per-worker error rate + duration p95;
  Oracle worker alarm explicitly informational (per §2.2's
  "failures do not block primary ingestion").

### Step 8: Tests

- Unit per worker: feed a fixture state and assert the worker's
  output. `price-updater`: assert VWAP + outlier exclusion +
  `min_volume_usd` thresholding. `cleanup-worker`: assert partition
  drops + creates against a fixture date. (No rollup tests — the MV
  chain's correctness is covered by tasks 0051/0059.)
- Integration: run each worker against a **local Docker ClickHouse**
  (the prices schema, same harness 0038's e2e uses) and snapshot the
  result. Not Postgres.

## Acceptance Criteria

- [ ] **Four** Rust Lambda binaries built against `provided.al2`
      (ARM64), deployed via the `infra/` CDK app — no VPC, no RDS.
- [ ] **Four** EventBridge Scheduler rules created with the §5.4
      cron/rate expressions verbatim (no rollup rule).
- [ ] `price-updater` produces a `prices.current_prices` row per
      tracked asset within ≤2 minutes of a 1-min OHLCV row landing
      from 0038. VWAP formula matches §5.5; outlier and
      `min_volume_usd` rules applied. (Pending open Q#1.)
- [ ] `oracle-worker` calls Reflector via Soroban RPC, writes
      `prices.oracle_prices` rows, and emits an alarm-without-blocking
      on RPC failure.
- [ ] `discovery-worker` inserts new assets into `prices.assets`
      keyed on §3.1 without duplicating existing rows (engine-side
      dedup via `ReplacingMergeTree` + `FINAL`). (Reconciled with 0054.)
- [ ] `cleanup-worker` `DROP PARTITION`s the oldest stale monthly
      partition per per-granularity table and creates the
      2-months-ahead partition; idempotent on same-day re-run.
- [ ] Per-worker CloudWatch alarms wired (error rate, duration
      p95); Oracle alarm marked informational.
- [ ] Integration harness covers all four workers against a local
      Docker **ClickHouse** mirror of the `prices.*` schema.
- [x] ~~`rollup-worker`~~ — eliminated (CH MV chain; ADR 0007 §3.4).

## Dependencies (both cleared 2026-06-25)

- **0011** — Lambda + EventBridge + Secrets Manager CDK scaffolding
  (no RDS/VPC under ADR 0007). **Archived (done).**
- **0038** — live ingestion producing `prices.price_ohlcv_1m`. Every
  worker either reads it (price-updater, cleanup) or maintains tables
  that feed the live path (discovery → asset registry). **PR #34
  merged to develop 2026-06-25.**

## Out of scope

- API read handlers — see 0040.
- Historical backfill — see ADR 0001 (Stream 1) and ADR 0005
  (Stream 2). Periodic workers operate only on data already
  in the `prices.*` ClickHouse database.
- Reflector contract deployment, key management beyond Secrets
  Manager.
- Cross-oracle merging (Chainlink, RedStone, Band, etc.) — the
  §3.4 schema supports it but only Reflector is in-scope for
  the §5.3 oracle worker.

## Notes

- Four workers in one task is a deliberate scoping call: the
  scaffolding (CDK Lambda + EventBridge rule + Secrets Manager
  mTLS + CH client + CW alarms) is identical across all four, and
  the per-worker logic is small. If any worker grows beyond
  ~300 lines of impl logic during build, split it out into its
  own task at that point — don't pre-emptively fragment.
- Per ADR 0006 §Decision the first Rust binary lands with the
  Ledger Processor; this task is the second wave and reuses
  the same `cargo lambda` packaging + CI patterns established
  by 0038.
- Oracle worker's "non-critical" stance (§2.2) is load-bearing
  — make sure its alarm severity and runbook reflect that, or
  on-call will be paged for what the design says should be
  ignorable.
