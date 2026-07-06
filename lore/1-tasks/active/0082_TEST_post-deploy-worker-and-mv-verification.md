---
id: "0082"
title: "Post-deploy verification — periodic workers write their tables + current_prices MV populates + alarms don't false-fire"
type: TEST
status: active
related_adr: ["0007"]
related_tasks: ["0070", "0056"]
tags: [layer-ops, milestone-M1, priority-medium, effort-small, aws, clickhouse, observability, post-deploy]
milestone: 1
links:
  - "../archive/0070_FEATURE_deploy-prices-ingestion-to-production-m1.md"
history:
  - date: 2026-07-06
    status: active
    who: okarcz
    note: >
      Activated right after the 0070 go-live to verify the periodic workers +
      MV + alarms while the freshly-deployed system is warm.
  - date: 2026-07-06
    status: backlog
    who: okarcz
    note: >
      Spawned from 0070 go-live. The ingestion path (ledger-processor) is verified
      end-to-end in prod; this closes the remaining post-deploy acceptance check —
      that each scheduled worker actually writes its table and the current_prices
      MV populates, and that the alarms are green under steady state.
---

# Post-deploy verification of periodic workers + MV + alarms

## Summary

The 0070 go-live proved live ledger ingestion (rows in `price_ohlcv_1m`, AMM
resolving, DLQ=0). This task closes the one deferred 0070 acceptance criterion:
confirm the EventBridge-scheduled workers each write their table on a real invoke
and the `current_prices` materialised view populates, and that no alarm is
false-firing.

## Context

Workers deployed by 0070's EventBridge stack, each on its own schedule:
`asset-discovery` → `assets`/`pool_registry`, `oracle` → `oracle_prices`,
`supply` → `asset_supply`, `enrichment` → USD columns, `cleanup` → retention.

## Implementation

- Either wait for each worker's schedule to fire, or `aws lambda invoke` each once,
  and confirm its target table gets fresh rows (SSH `clickhouse-client` on prod CH).
- Confirm `prices.current_prices` (MV) populates from the live `price_ohlcv_1m`.
- Confirm `lag_seconds` metric is emitting and the >60s alarm + per-Lambda
  error/duration alarms are `OK` (not `ALARM`) under steady state.
- Sanity-check `soroswap`-source rows appear in `price_ohlcv_1m` once a Soroswap
  swap lands (only `aquarius`/`phoenix` had rows in the first go-live window).

## Findings (2026-07-06, first invoke round)

Manual `aws lambda invoke` of each worker against prod:

| Worker | Result |
|--------|--------|
| `oracle` | ✅ `{"queried":3,"skipped":0,"written":3}` → `oracle_prices` |
| `asset-discovery` | ✅ `seeded 1660 assets` (pools_total 0 — empty payload gave no ledger range to scan) |
| `supply` | ⚠️ slow — aws CLI read-timeout + 3 retries at 60s each; no error logged. Verify by `asset_supply` freshness, not the sync invoke. |
| `enrichment` | ❌ **ACCESS_DENIED** → spawned **0083** |
| `cleanup` | ❌ **ACCESS_DENIED** → spawned **0083** |

Root cause (from prod `system.query_log`): `prices_writer` lacks `SELECT ON
system.parts` (cleanup) and `CREATE TABLE ON prices.*` (enrichment's peg-pivot
ref table). Core ingestion unaffected. Fix tracked in **0083** (BE RBAC grant +
enrichment temp-table redesign).

## Acceptance Criteria

- [~] Each periodic worker writes its table on a live invoke — `oracle` +
      `asset-discovery` ✅; `supply` pending freshness check; `enrichment` +
      `cleanup` **blocked on 0083** (RBAC).
- [ ] `current_prices` MV populated from live candles.
- [ ] `lag_seconds` + all deploy alarms `OK` under steady state (no false-fire).
- [ ] `soroswap`-source rows confirmed present in `price_ohlcv_1m`.
- [ ] `supply` confirmed writing `asset_supply` (via table freshness, not the
      timing-out sync invoke).
