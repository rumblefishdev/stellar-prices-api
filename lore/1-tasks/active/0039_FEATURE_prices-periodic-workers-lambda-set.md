---
id: "0039"
title: "Prices periodic workers — 3 EventBridge-Scheduler-triggered ClickHouse Lambdas (oracle, discovery, cleanup; rollup + price-updater eliminated)"
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
      Body/plan rewrite to the ClickHouse worker scope (see the
      2026-06-25 validation below) lands on the impl branch, not
      this status-only push.
  - date: 2026-06-25
    status: active
    who: oski
    note: >
      Resolved open Q#1: Current Price Updater Lambda ELIMINATED →
      refreshable `current_prices` MV + new single-writer
      `prices.asset_supply` table (market_cap = price × hourly supply,
      via JOIN). Scope now THREE Lambdas (oracle, discovery, cleanup);
      two former Lambdas are MVs. Design hinges on the CH
      ReplacingMergeTree single-writer-per-row invariant — surfaced a
      pre-existing clobber risk on `assets.home_domain` (the ledger
      processor's `write_assets` re-emits full rows), spawned as task
      0067.
---

# Prices periodic workers — 4 EventBridge-Scheduler-triggered ClickHouse Lambdas

> **Scope corrected 2026-06-25** against ADR 0007 (accepted) + the live
> ClickHouse schema. The original 5-Lambda / RDS-Postgres design is
> superseded. **Two** of the five planned Lambdas are eliminated, replaced
> by ClickHouse-native refreshable materialized views: the **OHLCV Rollup
> Lambda** (→ the `rollups.sql` MV chain) and the **Current Price Updater
> Lambda** (→ a `current_prices` MV + a new `asset_supply` table; see open
> Q#1 resolution). The surviving **three** retarget from RDS/`sqlx`/VPC to
> ClickHouse/mTLS/no-VPC. See **Architecture validation (2026-06-25)** below
> for the per-worker verdict and evidence.

## Summary

Implement the **three** EventBridge-Scheduler-driven Lambdas that survive
the ADR 0007 ClickHouse refactor: **Oracle Fetcher** (rate 5 min, Reflector
via Soroban RPC `simulateTransaction`), **Asset Discovery** (rate 1 hour),
and **Cleanup Worker** (cron 02:00 UTC daily). All three are Rust binaries
on `provided.al2` via `lambda_runtime` per ADR 0006, run **outside any VPC**
(ADR 0007 §6), and read/write the `prices.*` database in BE's Hetzner
ClickHouse over HTTPS-mTLS (the same `prices-clickhouse::mtls` sink seam
0038 uses), **not** RDS.

Two former Lambdas become CH-native and are **not** built here: the **OHLCV
Rollup** is the live MV chain in `packages/prices-clickhouse/schema/rollups.sql`
(task 0051, on prod), and the **Current Price Updater** becomes a refreshable
MV writing `prices.current_prices` every minute, with `market_cap_usd`
computed from a new hourly-refreshed `prices.asset_supply` table (Step 2).

## Context

The general-overview doc §2.1 lists these five workers as
Prices-API-budgeted components alongside the Ledger Processor and
the API handlers. §5.3 specifies each worker's trigger, source,
and output; §5.4 fixes the EventBridge Scheduler rule expressions:

```
oracle-ingest:     rate(5 minutes)  → Lambda "oracle-worker"
asset-discovery:   rate(1 hour)     → Lambda "discovery-worker"
retention-cleanup: cron(0 2 * * *)  → Lambda "cleanup-worker"
# ohlcv-rollup  (rate 15m) — REMOVED: now the CH MV chain (rollups.sql)
# price-update  (rate 1m)  — REMOVED: now the current_prices MV (Step 2)
```

The Ledger Processor (0038) is event-driven (S3 PutObject), not
schedule-driven — it lives outside this task.

Why this is one task, not three: all three share the same deployment
shape (`provided.al2` + EventBridge Scheduler rule + ClickHouse mTLS
client + Secrets Manager, **no VPC**), the same CDK stack structure,
and the same observability harness. Splitting them would create three
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
| **Current Price Updater** `rate(1m)` | ❌ **ELIMINATED** | Open Q#1 resolved (2026-06-25): 5 of 6 `current_prices` columns are SQL-derivable from `price_ohlcv_1m` + rollups (§5.5 outlier filter is plain `quantileExact` SQL), so the 1-min Lambda → a refreshable MV. The only external input — `market_cap_usd = price × token_supply` (§3.3, supply from Soroban `total_supply`/Horizon) — moves to a new `prices.asset_supply` table the MV JOINs. See Step 2. |
| **Oracle Fetcher** `rate(5m)` | ✅ KEEP (→CH) | Fetches Reflector via Soroban RPC `simulateTransaction` (external I/O — cannot be an MV) → `prices.oracle_prices`. |
| **Asset Discovery** `rate(1h)` | ✅ KEEP (→CH) | Scans ledgers for new assets → `prices.assets`; **also fetches `token_supply` → new `prices.asset_supply`** (Step 5). NOT folded into 0038 (which only `load_registry()` reads). ⚠️ overlaps task **0054** (minimal T1 carve-out) — absorb/extend, don't duplicate. |
| **Cleanup/Retention** `cron daily` | ✅ KEEP (thin) | `ALTER TABLE … DROP PARTITION` per per-granularity table (ADR 0007 §3.3). No declarative `TTL` in schema → retention stays procedural. |

**Cross-cutting retargets (apply to all three Lambdas):**

- **RDS/`sqlx` → ClickHouse.** Reuse 0038's `prices-clickhouse::mtls`
  sink seam; no Postgres, no UPSERT — `ReplacingMergeTree` INSERT +
  read-time `FINAL`/`argMax` (ADR 0007 §3, Consequences).
- **No VPC / NAT / SG / RDS-IAM** (ADR 0007 §6). The Step-7 CDK wiring
  drops all VPC + RDS-IAM scaffolding; creds are the mTLS cert/key in
  Secrets Manager (2 secrets/env), same as 0038.
- **Single-writer-per-`ReplacingMergeTree`-row invariant** (load-bearing).
  CH `ReplacingMergeTree` replaces the *whole* row for a sort key on merge —
  there is no column-level merge. So no two writers may target the same row.
  This is why `market_cap_usd` is **not** written by the discovery Lambda
  directly into `current_prices` (the per-minute MV would clobber it, and
  vice-versa) and supply is **not** added to `prices.assets` (the ledger
  processor's `write_assets` re-emits full rows with `updated_at = now()` —
  `prices-ingest-core/src/writer.rs:141` — and would clobber it). Instead
  supply gets its own single-writer table; the MV reads it via JOIN.
- Net Lambda count: **3, not 5** (rollup + price-updater are MVs).

**Open design questions:**

1. ~~Current Price Updater — Lambda vs. view.~~ **RESOLVED 2026-06-25 →
   eliminate the Lambda; use a refreshable MV + `asset_supply` table.**
   Rationale: §5.5's outlier rule (inter-source median + % threshold) is
   plain ClickHouse SQL (`quantileExact(0.5)` + filtered weighted average),
   and `?min_volume_usd=` is a *read-time* param re-weighted from the
   `sources` JSON (overview §5.5 layering table, L3) — so nothing on the
   write path needs imperative code **except** `market_cap_usd`, which is
   external (`token_supply` via Soroban `total_supply`/Horizon, §3.3) and
   `NULL`-able by design. Supply is slow-moving, so it rides the hourly
   discovery worker into a dedicated `prices.asset_supply` table that the
   `current_prices` MV multiplies by the live price. See Step 2.
2. **Asset Discovery vs. task 0054.** Decide whether 0039 absorbs 0054's
   minimal discovery binary or 0054 ships first and 0039 extends it. (Still
   open — now also covers where the `asset_supply` fetch lands.)

## Implementation Plan

### Step 1: Shared crate scaffolding

Add `packages/periodic-workers/` (or three sibling binary crates
under `packages/`) sharing a common library for: the **ClickHouse
mTLS client** (reuse 0038's `prices-clickhouse::mtls` seam — Secrets
Manager cert/key fetch via the Parameters/Secrets extension, **no
`sqlx`, no RDS pool**), structured CloudWatch logging, `lambda_runtime`
entrypoint boilerplate, and a small "ran at" telemetry helper.

### Step 2: Current prices — refreshable MV + `asset_supply` (NO Lambda)

Replaces the former `price-updater` Lambda (open Q#1 resolved). Two parts,
each preserving the single-writer-per-row invariant:

**2a. `current_prices` refreshable MV (sole writer of `current_prices`).**

- Add to `packages/prices-clickhouse/schema/rollups.sql` (or a sibling
  `current.sql`): `CREATE MATERIALIZED VIEW prices.mv_current_prices REFRESH
  EVERY 1 MINUTE TO prices.current_prices AS SELECT …`.
- Computes the 5 SQL-derivable columns from `price_ohlcv_1m FINAL` + the
  rollup tables: `price_usd`/`price_xlm` (`argMax(close…, timestamp)`),
  `change_24h_pct`/`change_7d_pct` (latest vs. 24h/7d-ago close),
  `volume_24h_usd` (trailing-24h sum), `vwap_24h` (§5.5 cross-source
  weighted price with the inter-source-median outlier filter via
  `quantileExact(0.5)` + a filtered weighted average), and the per-source
  `sources` JSON.
- `market_cap_usd = price_usd * s.token_supply` via
  `LEFT JOIN prices.asset_supply AS s FINAL ON s.asset_id = …` — `NULL`
  when supply is absent (§3.3 allows it). Price factor is minute-fresh;
  supply factor lags ≤1h. **The MV is the only writer of `current_prices`.**
- ⚠️ Perf check before committing: a 1-min MV recomputing a 24h-trailing
  median-filtered VWAP across all assets×sources is heavier than the
  bounded-window rollup MVs. `EXPLAIN`/time it for the expected asset count;
  fallback is a thin scheduled `INSERT…SELECT` (still no per-asset Lambda).

**2b. `prices.asset_supply` table (sole writer = the hourly discovery worker).**

```sql
CREATE TABLE IF NOT EXISTS prices.asset_supply (
    asset_id     UInt32,
    token_supply Decimal(38, 14),
    fetched_at   DateTime DEFAULT now()
) ENGINE = ReplacingMergeTree(fetched_at) ORDER BY (asset_id);
```

- New schema object in `schema/init.sql`. Populated only by Step 5's supply
  fetch — never by the MV, never by the ledger processor. This dedicated
  table is what lets supply (slow) and price (fast) each have a single
  writer, instead of fighting over a shared `current_prices`/`assets` row.
- The existing `current_price_usd` read view (`schema/views.sql`) is
  unchanged — it remains the read surface over `current_prices`.

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
- **Supply fetch (feeds the Step 2 `current_prices` MV).** For each
  tracked asset, fetch `token_supply` — Soroban `total_supply` via RPC
  `simulateTransaction` for SEP-41/contract tokens, Horizon `/assets` for
  classic — and INSERT into `prices.asset_supply` (sole writer). Supply is
  slow-moving, so hourly is sufficient; `market_cap_usd` is then recomputed
  per-minute by the MV. Does **not** write `current_prices` or a supply
  column on `assets` (single-writer invariant).
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

- One Lambda function definition per worker (**three**: oracle,
  discovery, cleanup), using 0038's conventions: `provided.al2`, ARM64,
  **no VPC / no RDS-IAM** (ADR 0007 §6), Secrets Manager read of the mTLS
  cert/key bundle. No Lambda for current-prices (it's the Step 2 MV).
- One EventBridge Scheduler rule per worker with the §5.4
  expressions verbatim (rollup **and** price-update rules omitted — both
  are MVs now).
- DLQ + retry policy per worker (defer the exact DLQ shape to
  impl time; default to 2 retries + DLQ).
- CloudWatch alarms: per-worker error rate + duration p95;
  Oracle worker alarm explicitly informational (per §2.2's
  "failures do not block primary ingestion").

### Step 8: Tests

- Unit per worker: feed a fixture state and assert the worker's
  output. `discovery-worker`: assert new-asset INSERT + `asset_supply`
  rows. `cleanup-worker`: assert partition drops + creates against a
  fixture date. (No rollup tests — covered by 0051/0059.)
- **`current_prices` MV (Step 2):** seed `price_ohlcv_1m` + `asset_supply`
  fixtures, refresh the MV, assert the §5.5 VWAP + outlier exclusion +
  `market_cap_usd = price × supply` (and `NULL` when supply absent). SQL
  test, not a Lambda test.
- Integration: run each worker + the MV against a **local Docker
  ClickHouse** (the prices schema, same harness 0038's e2e uses) and
  snapshot the result. Not Postgres.

## Acceptance Criteria

- [ ] **Three** Rust Lambda binaries (oracle, discovery, cleanup) built
      against `provided.al2` (ARM64), deployed via the `infra/` CDK app —
      no VPC, no RDS.
- [ ] **Three** EventBridge Scheduler rules created with the §5.4
      cron/rate expressions verbatim (no rollup, no price-update rule).
- [ ] `prices.asset_supply` table created; `mv_current_prices` refreshable
      MV is the **sole** writer of `current_prices`, refreshes every minute,
      and computes `market_cap_usd = price × asset_supply.token_supply`
      (`NULL` when supply absent). §5.5 VWAP + median-outlier filter match.
- [ ] `oracle-worker` calls Reflector via Soroban RPC, writes
      `prices.oracle_prices` rows, and emits an alarm-without-blocking
      on RPC failure.
- [ ] `discovery-worker` inserts new assets into `prices.assets` keyed on
      §3.1 without duplicating existing rows, **and** writes `token_supply`
      into `prices.asset_supply` (its sole writer). (Reconciled with 0054.)
- [ ] `cleanup-worker` `DROP PARTITION`s the oldest stale monthly
      partition per per-granularity table and creates the
      2-months-ahead partition; idempotent on same-day re-run.
- [ ] Single-writer invariant holds: no two writers target the same
      `current_prices` / `assets` / `asset_supply` row (verified by writer
      audit — MV owns `current_prices`, discovery owns `asset_supply`).
- [ ] Per-worker CloudWatch alarms wired (error rate, duration
      p95); Oracle alarm marked informational.
- [ ] Integration harness covers the three workers + the `current_prices`
      MV against a local Docker **ClickHouse** mirror of the `prices.*`
      schema.
- [x] ~~`rollup-worker`~~ — eliminated (CH MV chain; ADR 0007 §3.4).
- [x] ~~`price-updater`~~ — eliminated (refreshable MV + `asset_supply`;
      open Q#1, 2026-06-25).

## Dependencies (both cleared 2026-06-25)

- **0011** — Lambda + EventBridge + Secrets Manager CDK scaffolding
  (no RDS/VPC under ADR 0007). **Archived (done).**
- **0038** — live ingestion producing `prices.price_ohlcv_1m`. The
  `current_prices` MV and the cleanup worker read it; discovery maintains
  the asset registry + supply that feed the live path. **PR #34 merged to
  develop 2026-06-25.**

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

## Future Work

- **0067** (BUG, backlog) — `assets` enrichment columns (`home_domain`, and
  any future on-`assets` column) are clobbered by the ledger processor's
  full-row `write_assets` re-emit. Surfaced by this task's single-writer
  analysis; 0039 dodges it for supply via the dedicated `asset_supply`
  table, but `home_domain` still needs the writer-ownership fix.

## Notes

- Three workers in one task is a deliberate scoping call: the
  scaffolding (CDK Lambda + EventBridge rule + Secrets Manager
  mTLS + CH client + CW alarms) is identical across all three, and
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
