---
id: "0055"
title: "`GET /backfill/status` endpoint — Tranche 1 isolated read handler"
type: FEATURE
status: backlog
related_adr: ["0006", "0007"]
related_tasks: ["0011", "0050", "0051", "0052", "0040"]
tags: [layer-backend, priority-high, effort-small, milestone-M1, api, lambda, axum, rust, clickhouse, read-endpoint]
milestone: 1
links:
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../blocked/0040_FEATURE_prices-api-gateway-and-read-handlers.md"
  - "./0051_FEATURE_clickhouse-prices-schema-and-mv-chain-migration.md"
  - "./0052_FEATURE_clickhouse-mtls-client-shared-crate.md"
history:
  - date: 2026-05-21
    status: backlog
    who: okarcz
    note: >
      Spawned during Tranche 1 task-set creation. §4.5
      `GET /backfill/status` is an explicit Tranche 1 acceptance
      criterion ("`GET /backfill/status` endpoint live and
      returning valid progress data"), but 0040 bundles the full
      public API surface (assets, ohlcv, batch, oracles, …) which
      is T2 scope. Carve out just /backfill/status so it can ship
      on the T1 timeline without dragging T2 work forward.
---

# `GET /backfill/status` endpoint — Tranche 1 isolated

## Summary

Build the minimum viable axum-on-Lambda surface that serves
`GET /backfill/status` per the design doc §4.5 response shape.
The endpoint reads both `prices.backfill_progress` rows
(`sdex_archive`, `soroban_amm`), computes `progress_pct` and
`ledgers_remaining` at read time, and returns the nested JSON
structure. Includes the API Gateway route, usage plan, API key
auth, and 30s response cache.

This carve-out ships ahead of the full public API (0040) so
Tranche 1's acceptance criterion #4 (backfill status is
queryable, freshness alarm wired) can be met on schedule.

## Context

§4.5 of the design doc specifies the canonical response shape:

```json
{
  "realtime_tip_ledger": 57234198,
  "sdex": { "status": "running", "current_ledger": …, "progress_pct": …, "last_push_at": …, "earliest_data_available": … },
  "soroban_amm": { "status": "completed", "last_push_at": …, "completed_at": …, "earliest_data_available": … }
}
```

Per ADRs 0001 and 0005 the row reflects most-recent-push state
(not live heartbeat). The endpoint reads it as-is; no live-cadence
inference happens here.

0040 covers the full public API surface (Tranche 2). The
`/backfill/status` route is the only T1-required endpoint, so
shipping it standalone is cheaper than waiting for the full T2
work to converge.

Once 0040 lands, it should subsume this Lambda — either by
folding the handler into the larger axum app, or by keeping it
as a dedicated function (same shape, no API change). Either
approach is fine; the decision lives in 0040.

## Implementation Plan

### Step 1: axum + Lambda scaffolding

Add `packages/backfill-status-api/` (binary crate). Depends on:

- `lambda_runtime` + `lambda_http` — Lambda HTTP integration.
- `axum` — handler framework (one route is sufficient).
- 0052 shared CH client.
- `serde` / `serde_json` — response serialisation.

### Step 2: Handler logic

For the single GET handler:

1. Read both rows from `prices.backfill_progress` using a single
   `SELECT … FINAL FROM prices.backfill_progress` query so the
   ReplacingMergeTree's latest version per `task_name` is
   selected.
2. Resolve `realtime_tip_ledger`: read `MAX(timestamp)` from
   the most-recent `prices.price_ohlcv_1m` partition, convert
   back to ledger sequence via the
   `(ledger_seq, closed_at)` mapping table BE maintains — or,
   simpler for T1, store the current tip in a small
   `prices.ingest_state` row updated by 0038 on each invocation.
   Pick the approach at impl time; document in
   `notes/S-realtime-tip-resolution.md`.
3. Compute `progress_pct = (target_ledger - current_ledger) /
   (target_ledger - start_ledger) * 100` and
   `ledgers_remaining = current_ledger - start_ledger` at read
   time (matches §4.5).
4. Serialise to the §4.5 JSON envelope and return 200.

### Step 3: API Gateway route + auth

In the 0011 CDK app:

- New API Gateway REST API resource: `/v1/backfill/status`.
- Usage plan + API key requirement attached.
- Response cache: 30s TTL per §6.
- Throttling: 100 req/s per key per §6.
- Lambda integration to the new function.

### Step 4: Tests

- Unit: handler with mocked CH client returning fixture rows;
  assert response envelope matches §4.5 exactly.
- Integration: against a Docker CH with 0051's schema +
  fixture `backfill_progress` rows, hit the handler and
  validate the response body.
- Schema test: the response must validate against the OpenAPI
  spec stub (full spec lands in 0040; T1 stub covers this one
  route).

### Step 5: CloudWatch wiring

- Standard latency / error-rate alarms via the 0011 conventions.
- The `sdex.last_push_at` freshness alarm itself lives in 0056
  (not in this task) — this task only ensures the field is
  populated correctly so 0056's alarm has something to read.

## Acceptance Criteria

- [ ] `packages/backfill-status-api` Lambda binary builds and
      deploys via CDK from 0011's stack
- [ ] `GET /v1/backfill/status` returns a §4.5-shape response
      with valid `sdex` and `soroban_amm` objects after 0051
      has seeded `backfill_progress` and either backfill stream
      has run at least once
- [ ] API key auth enforced: requests without a valid key
      return 401
- [ ] Response cache verified: consecutive requests within 30s
      return `X-Cache: Hit`
- [ ] `progress_pct` and `ledgers_remaining` computed correctly
      against hand-checked fixture rows
- [ ] OpenAPI stub for this one route is produced; consumed by
      the 0040 spec when it lands
- [ ] Integration test passes in CI against the Docker CH
      fixture

## Blocked on

- **0011** — API Gateway + Lambda + Secrets Manager CDK scaffolding.
- **0050** — Hetzner CH endpoint + mTLS material provisioning.
- **0051** — `prices.backfill_progress` table must exist with
  the two seeded rows.
- **0052** — shared mTLS CH client.

## Out of scope

- The other 6 endpoints from §4 — those are 0040.
- Live CLI progress surfacing (operator-visible only per
  §5.6 freshness subsection).
- Backfill orchestration (push triggers, retries) — separate
  concern.
- The `sdex.last_push_at` freshness CloudWatch alarm itself —
  see 0056.

## Notes

- Keep this endpoint deliberately tiny. The temptation to fold
  in adjacent read functionality should be resisted; 0040 owns
  the full read surface. This task exists because §4.5 is a T1
  acceptance criterion and waiting for 0040 would block T1
  delivery.
- The `realtime_tip_ledger` resolution path is the only
  non-trivial design choice; document the chosen approach in a
  short S-note so 0040 can adopt the same convention.
