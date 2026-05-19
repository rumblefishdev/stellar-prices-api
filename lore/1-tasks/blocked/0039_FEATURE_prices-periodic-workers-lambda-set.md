---
id: "0039"
title: "Prices periodic workers — 5 EventBridge-Scheduler-triggered Lambdas (price updater, rollup, oracle, discovery, cleanup)"
type: FEATURE
status: blocked
related_adr: ["0003", "0004", "0006"]
related_tasks: ["0011", "0038"]
tags: [layer-indexing, priority-high, effort-large, lambda, scheduler, rust, aws, ingestion]
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../backlog/0011_FEATURE_bootstrap-cdk-with-ssm-platform-lookups.md"
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
---

# Prices periodic workers — 5 EventBridge-Scheduler-triggered Lambdas

## Summary

Implement the five periodic Lambdas listed in the general-overview
doc §2.1 / §5.3 / §5.4: **Current Price Updater** (rate 1 min),
**OHLCV Rollup** (rate 15 min), **Oracle Fetcher** (rate 5 min,
Reflector via Soroban RPC `simulateTransaction`), **Asset
Discovery** (rate 1 hour), and **Cleanup Worker** (cron 02:00 UTC
daily). All five are Rust binaries on `provided.al2` via
`lambda_runtime` per ADR 0006, share the CDK Lambda + EventBridge
stacks provisioned by 0011, and operate against the cloud RDS
that 0038's Ledger Processor populates.

## Context

The general-overview doc §2.1 lists these five workers as
Prices-API-budgeted components alongside the Ledger Processor and
the API handlers. §5.3 specifies each worker's trigger, source,
and output; §5.4 fixes the EventBridge Scheduler rule expressions:

```
oracle-ingest:     rate(5 minutes)  → Lambda "oracle-worker"
asset-discovery:   rate(1 hour)     → Lambda "discovery-worker"
ohlcv-rollup:      rate(15 minutes) → Lambda "rollup-worker"
price-update:      rate(1 minute)   → Lambda "price-updater"
retention-cleanup: cron(0 2 * * *)  → Lambda "cleanup-worker"
```

The Ledger Processor (0038) is event-driven (S3 PutObject), not
schedule-driven — it lives outside this task.

Why this is one task, not five: all five share the same
deployment shape (`provided.al2` + EventBridge Scheduler rule +
RDS access + Secrets Manager), the same CDK stack structure, and
the same observability harness. Splitting them into five tasks
would create five copies of the same scaffolding. They can be
implemented incrementally within one task — each worker is one
binary + one rule + one set of acceptance criteria — but the
deployment, CI, and CDK scaffolding is built once.

## Implementation Plan

### Step 1: Shared crate scaffolding

Add `packages/periodic-workers/` (or five sibling binary crates
under `packages/`) sharing a common library for: RDS connection
pooling via `sqlx`, Secrets Manager lookup of the DB password,
structured CloudWatch logging, `lambda_runtime` entrypoint
boilerplate, and a small "ran at" telemetry helper.

### Step 2: Current Price Updater (`price-updater`)

- Trigger: EventBridge Scheduler `rate(1 minute)`.
- Behaviour (§5.3, §5.5): read latest per-asset 1-min candles
  from `price_ohlcv` partitioned on the current month, compute
  VWAP across sources using the §5.5 formula (with the §5.5
  outlier-detection rule and the `min_volume_usd` threshold),
  and UPSERT into `current_prices` (schema §3.3).
- Writes per-source `sources` JSONB column per the §3.3 example
  (numeric values serialised as strings to preserve
  NUMERIC(28,14) precision).

### Step 3: OHLCV Rollup (`rollup-worker`)

- Trigger: EventBridge Scheduler `rate(15 minutes)`.
- Behaviour (§5.3): re-derive 15m / 1h / 4h / 1d / 1w / 1M
  candles from the underlying 1m rows. Writes via the same
  UPSERT contract as 0038 (§5.2 "Write semantics") but using
  the **whole-row replace** variant (not incremental merge),
  because rollups operate on already-aggregated windows.
- PK shape per ADR 0003.

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
- Behaviour (§5.3): scan recent ledger account entries for new
  classic asset issuances and new SEP-41 / Soroban token
  contract deployments. UPSERT into `assets` (§3.1) on the
  `(asset_code, issuer_address, contract_address)` unique key.
- Reads from the same `stellar-ledger-data/` S3 bucket as 0038
  (no separate ingestion path); selects the last hour of
  ledgers by S3 key prefix or by `closed_at` lookup against
  the prices RDS.
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

### Step 7: CDK + EventBridge wiring (depends on 0011)

In `infra/aws-cdk/`:

- One Lambda function definition per worker, all using the
  conventions established by 0038 (`provided.al2`, VPC, RDS
  IAM, Secrets Manager read).
- One EventBridge Scheduler rule per worker with the §5.4
  expressions verbatim.
- DLQ + retry policy per worker (defer the exact DLQ shape to
  impl time; default to 2 retries + DLQ).
- CloudWatch alarms: per-worker error rate + duration p95;
  Oracle worker alarm explicitly informational (per §2.2's
  "failures do not block primary ingestion").

### Step 8: Tests

- Unit per worker: feed a fixture DB state and assert the
  worker's output. `price-updater`: assert VWAP + outlier
  exclusion + `min_volume_usd` thresholding. `rollup-worker`:
  assert window math for at least one window pair (1m→15m).
  `cleanup-worker`: assert partition drops + creates against
  a fixture date.
- Integration: same Docker-Postgres harness as 0038. Run each
  worker against a known DB state and snapshot the result.

## Acceptance Criteria

- [ ] Five Rust Lambda binaries built against `provided.al2`,
      deployed via CDK from 0011's `infra/aws-cdk/` app.
- [ ] Five EventBridge Scheduler rules created with the §5.4
      cron/rate expressions verbatim.
- [ ] `price-updater` produces a `current_prices` row per
      tracked asset within ≤2 minutes of a 1-min OHLCV row
      landing from 0038. VWAP formula matches §5.5; outlier
      and `min_volume_usd` rules applied.
- [ ] `rollup-worker` produces 15m / 1h / 4h / 1d / 1w / 1M
      rows derived from 1m rows; whole-row UPSERT semantics
      verified by re-running on the same window.
- [ ] `oracle-worker` calls Reflector via Soroban RPC, writes
      `oracle_prices` rows, and emits an alarm-without-blocking
      on RPC failure.
- [ ] `discovery-worker` UPSERTs new assets in `assets` table
      on the §3.1 unique key without duplicating existing rows.
- [ ] `cleanup-worker` drops the oldest stale monthly partition
      and creates the 2-months-ahead partition; idempotent on
      same-day re-run.
- [ ] Per-worker CloudWatch alarms wired (error rate, duration
      p95); Oracle alarm marked informational.
- [ ] Integration test harness covers all five workers against
      a Docker-Postgres mirror of the 0011 schema.

## Blocked on

- **0011** — Lambda + EventBridge CDK stacks, Secrets Manager
  for DB creds + Reflector contract address + Soroswap/Aquarius
  API keys, RDS itself. Without 0011 there is no place to
  deploy these workers and no DB to read/write.
- **0038** — live ingestion must be running first. Every worker
  here either reads `price_ohlcv` (price-updater, rollup,
  cleanup) or maintains tables that feed into the live path
  (discovery → asset registry → Ledger Processor extraction).
  Running periodic workers against an empty `price_ohlcv` yields
  no useful output and obscures real bugs behind "no data" noise.

## Out of scope

- API read handlers — see 0040.
- Historical backfill — see ADR 0001 (Stream 1) and ADR 0005
  (Stream 2). Periodic workers operate only on data already
  in the cloud RDS.
- Reflector contract deployment, key management beyond Secrets
  Manager.
- Cross-oracle merging (Chainlink, RedStone, Band, etc.) — the
  §3.4 schema supports it but only Reflector is in-scope for
  the §5.3 oracle worker.

## Notes

- Five workers in one task is a deliberate scoping call: the
  scaffolding (CDK Lambda + EventBridge rule + Secrets Manager
  + RDS access + CW alarms) is identical across all five, and
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
