---
id: "0084"
title: "supply-worker times out at the 300s Lambda limit before completing a full asset walk"
type: BUG
status: active
related_adr: []
related_tasks: ["0070", "0082", "0039"]
tags: [layer-ops, priority-medium, effort-medium, aws, lambda, horizon, worker, post-deploy]
links:
  - "../../../packages/supply-worker/src"
history:
  - date: 2026-07-06
    status: backlog
    who: okarcz
    note: >
      Found by 0082 post-deploy verification. supply-worker runs the full 300s
      Lambda timeout and ends `Status: timeout` on every invocation (scheduled +
      the 2 async retries), writing `asset_supply` only partially (1164 of 1685
      assets) before being killed. Memory is fine (~48/512 MB) — it's wall-clock
      bound on serial per-asset Horizon calls. Degraded (not go-live-blocking:
      supply feeds market-cap-style enrichment, not core pricing).
  - date: 2026-07-06
    status: active
    who: okarcz
    note: Promoted to active to start work on the supply-worker timeout fix.
---

# supply-worker times out before completing the asset walk

## Summary

`prices-production-supply` never finishes: every invocation hits the **300 s**
Lambda timeout (`Status: timeout`, no "run complete" log), writing `asset_supply`
only partially. It walks ~1,685 assets via `horizon.stellar.org` serially, so the
wall-clock exceeds the timeout. Async retries re-run the same event 2× (also
timing out) → ~15 min wasted compute per trigger and a permanently-firing
`supply-errors` alarm.

## Evidence (2026-07-06, prod)

- Every `REPORT` line: `Duration: 300000.00 ms … Status: timeout`.
- `asset_supply` = 1164 rows vs `assets` = 1685 (partial coverage).
- Max memory ~48 MB / 512 MB — not memory-bound; pure I/O wall-clock.

## Fix options

1. **Batch + checkpoint across invocations.** Process N assets per invoke, persist
   a cursor (e.g. in `discovery_state` or an SSM param), resume next schedule —
   so no single invoke needs the whole walk. Preferred; bounds each run.
2. **Parallelize the Horizon fetches** (bounded concurrency) so a full walk fits
   in one invoke. Simpler but fragile as the asset count grows + Horizon rate limits.
3. Raise the Lambda timeout toward the 15-min max — a stopgap, not a fix; breaks
   again as assets grow.

Lean: (1), optionally + (2) for throughput. Also cap async-retry attempts so a
slow run doesn't triple compute.

## Acceptance Criteria

- [ ] A scheduled supply run completes without timing out (logs a clean
      completion, not `Status: timeout`).
- [ ] `asset_supply` covers the full active asset set (no partial-walk gap).
- [ ] `supply-errors` alarm returns to `OK` under steady state.
- [ ] Async-retry storm bounded (no 3× full-timeout re-runs per trigger).
