---
id: "0047"
title: "Cross-tenant throughput verification — can shared Hetzner CH+Caddy absorb combined BE + prices-api read/write load?"
type: RESEARCH
status: backlog
related_adr: ["0007"]
related_tasks: ["0045", "0046", "0044"]
tags: [layer-research, priority-high, effort-medium, hetzner, clickhouse, throughput, capacity, cross-team]
links:
  - "../blocked/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "../blocked/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-conversation-brief.md"
  - "../active/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/active/0216_RESEARCH_hetzner-clickhouse-deploy/README.md"
  - "../../../../soroban-block-explorer/lore/1-tasks/active/0227_FEATURE_infra-hetzner-ansible-playbook.md"
history:
  - date: 2026-05-19
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0045's BE agreement record (Cluster B6 TBD).
      BE flagged the risk that a single Caddy:443 + single CH
      instance may not absorb the combined query/write load of both
      tenants under peak conditions. Task 0046 measured storage and
      row volume (proven light); this task measures throughput,
      concurrency, IOPS, and CPU contention. ADR 0007 stays
      `proposed` until this task resolves the gate.
---

# Cross-tenant throughput verification on shared Hetzner CH

## Summary

Verify that the shared Hetzner ClickHouse data plane (single CH
instance behind Caddy:443) can sustain the **combined** read/write
load of both tenants — BE's `default.*` query traffic + prices-api's
write traffic from 6 Lambdas + prices-api's read traffic from the API
gateway — under peak realistic conditions.

This is the final gate on ADR 0007 transitioning `proposed →
accepted`. Task 0046 proved storage and row volume are non-issues;
this task proves the connection / concurrency / IOPS / CPU layer is
also non-issue (or surfaces what needs tuning).

## Context

Task 0045's BE agreement record (Cluster B6 outcome) flagged the
shared-host throughput question as the only "TBD that could change
the architecture". Specifically:

- BE's existing read load (queries against `default.*`) is not yet
  quantified — we have row counts but not query rates.
- prices-api adds:
  - **Writes:** ~6 Lambdas × 3 envs, batched HTTP `INSERT` into
    `prices.*` tables, peak ~10-15 req/s per env (per the brief
    §3.2 ask 5).
  - **Reads:** API gateway → handler Lambdas → CH at ~100 req/s
    per key (per overview §2.1), aggregating to roughly comparable
    peak per env.
- CH is a single instance on a single physical host with bounded
  CPU / RAM / IOPS / `max_concurrent_queries`. Caddy is a single
  process with bounded `max_keepalive_conns`.
- The MV chain (`price_ohlcv_1m → 15m → 1h → 4h → 1d → 1w → 1M`)
  runs on every 1m row, contributing background CPU + IO load.

A failure mode here forces the **sidecar-CH fallback** (Option 4 in
task 0044's I-note) — prices-api would run its own Hetzner box,
keeping the shared-S3 / shared-mTLS-CA pattern but separating the
data plane.

## Research plan

### Step 1: Quantify BE's current load

Read-only step. Once BE's CH is live on the production box:

- `system.query_log`: avg/p95/p99 QPS, query duration, query memory.
- `system.metric_log`: CPU utilization, IO throughput, concurrent
  queries.
- `system.parts`: storage growth rate, merge frequency, merge IO.

If BE's CH isn't production-live yet, work against the local
docker-compose CH (`localhost:8123`) with the BE backfill data
already loaded — the query patterns are the same shape; just the
hardware is different.

### Step 2: Synthesize prices-api's projected load

Build a write-load and read-load model:

| Lambda | Trigger | Write rate per env | CH op |
|---|---|---:|---|
| Ledger Processor (0038) | S3 event | ~1 INSERT per 5s (one per ledger) | INSERT into `price_ohlcv_1m` per swap/trade event |
| Current Price Updater (0039) | EventBridge 1min | 1 batch INSERT per minute | UPSERT into `current_prices` |
| Oracle Fetcher (0039) | EventBridge 5min | 1 INSERT per 5min | INSERT into `oracle_prices` (also driven by REFLECTOR events via Ledger Processor) |
| Asset Discovery (0039) | EventBridge 1hr | 1 small INSERT per hour | UPSERT into `assets` |
| Cleanup Worker (0039) | EventBridge daily | 1 ALTER TABLE per day | DROP PARTITION (cheap) |
| API read handlers (0040) | API Gateway | up to ~100 req/s per key | SELECT from `price_ohlcv_*` / `current_prices` / `oracle_prices` |

Aggregate per env, then × 3 envs (dev/staging/prod) where prod
dominates.

### Step 3: Identify failure modes and their thresholds

For each subsystem, surface the limit and how close we'd run:

| Subsystem | Limit | Current BE load | + prices-api projected | Headroom |
|---|---|---|---|---|
| Caddy `max_keepalive_conns` | TBD per BE | TBD | TBD | TBD |
| CH `max_concurrent_queries` | default 100 | TBD | TBD | TBD |
| CH IO throughput (NVMe ~3 GB/s read, ~1.5 GB/s write) | hardware-bound | TBD | TBD | TBD |
| CH CPU (8-32 cores typical) | hardware-bound | TBD | TBD | TBD |
| MV chain CPU (background) | bounded fraction | n/a | depends on event rate | TBD |
| Caddy throughput | bounded by single process | TBD | TBD | TBD |

### Step 4: Load test (if feasible)

If BE's CH is production-live or a staging mirror exists:

- Use `clickhouse-benchmark` or a custom Rust load generator.
- Drive concurrent reads (BE-shape queries) + writes (prices-api-shape
  inserts) at projected peak rates.
- Measure: query latency p50/p95/p99, error rate, CH CPU/RAM/IO
  utilization, Caddy CPU/memory.
- Compare against headroom — does prices-api's add fit comfortably?

If load-test infrastructure isn't available, fall back to:

- Analytical model from §3 + per-query benchmark on local CH for the
  10 worst-case query shapes.
- Decide based on margins (if projected load is <30% of any limit,
  comfortable; >70% requires tuning or fallback).

### Step 5: Recommendations

Output one of three:

1. **GREEN — proceed with ADR 0007 as-proposed.** Shared host
   absorbs combined load with comfortable margins.
2. **YELLOW — proceed with tuning.** Shared host works but specific
   settings need adjustment (e.g. bump `max_concurrent_queries`,
   tune Caddy keepalive, schedule MV merges off-peak). Document
   the tuning asks for BE.
3. **RED — fallback to sidecar CH.** Shared host cannot absorb the
   load without degrading BE. ADR 0007 supersedes to the Option 4
   sidecar path. Estimated cost delta: +~€39-69/mo for a second
   Hetzner box (one box covers all 3 envs).

## Acceptance Criteria

- [ ] `notes/G-throughput-verification.md` — single report with:
  BE current load (from `system.query_log` etc.), prices-api projected
  load model (per §2), per-subsystem headroom table (§3), load-test
  results if available (§4), color-coded recommendation (§5).
- [ ] If GREEN or YELLOW: list concrete tuning asks for BE
  (Caddy / CH settings), cross-link from task 0045's agreement record.
- [ ] If RED: spawn ADR amendment + task to scope the sidecar-CH
  build-out. Update 0045's agreement record.
- [ ] Reproducible: the analytical model and any load-test setup
  documented so another engineer can re-run after BE 0227 ships.

## Blocked on

- **BE 0216 + 0227** — Hetzner CH must be live enough to query
  `system.query_log` (or at least docker-compose CH backfilled with
  BE data is available for analytical work).
- **Task 0046** — already merged; the empirical write-volume numbers
  feed §2.

## Out of scope

- Storage capacity — already covered by task 0046.
- Cost-share number — separate D12 follow-up (more BE-side estimation).
- BE's own query optimization — they own that.
- Caddy tuning beyond the surface ask — BE owns Caddy config.

## Notes

- This task is the final gate on ADR 0007 → accepted. The brief, the
  agreement record, and ADR 0007 all reference each other; closing
  this task closes the loop.
- If BE's CH is not yet production-live, much of the analytical work
  can be done against the local docker-compose CH (already loaded
  with 10k ledgers of backfill from task 0046). Defer the final
  load test until BE's production CH is queryable.
- The "RED" outcome is genuinely possible. The empirical
  storage-light finding from 0046 does **not** imply throughput-light;
  oracle updates touch ~19 rows per event with frequent INSERT
  batching, and the MV chain has a non-trivial CPU footprint.
