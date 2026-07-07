---
id: "0084"
title: "supply-worker times out at the 300s Lambda limit before completing a full asset walk"
type: BUG
status: completed
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
  - date: 2026-07-07
    status: active
    who: okarcz
    note: >
      Fix code-complete (prepare-only, not deployed). Staleness self-checkpoint
      (`load_stalest_credit_assets`, order by `fetched_at`, never-fetched first)
      + a 240 s wall-clock walk budget (`SUPPLY_TIME_BUDGET_SECS`) so no single
      invoke needs the full registry and a run can't hit the 300 s timeout;
      successive hourly runs round-robin all assets. Supply EventBridge target
      set `retryAttempts: 0` to kill the async-retry storm. 5 unit pass +
      Docker-gated IT for the ordering; `--features lambda`/clippy/fmt + infra
      `tsc` clean. Remaining: post-deploy confirmation (clean run, full coverage,
      `supply-errors` → OK).
  - date: 2026-07-07
    status: active
    who: okarcz
    note: >
      High-effort code review (PR #90) found + fixed 4 issues. #1 dead-asset
      starvation: Ok(None) assets now write supply 0 (task-accepted market_cap=0)
      so their fetched_at advances and they rotate out of the stalest front
      instead of leading every run forever; transient Err still skipped (no
      value clobber). #2 `retryAttempts:0` was on the EventBridge target
      (delivery), not function-error retries — moved to
      `fn.configureAsyncInvoke` so the async-retry storm is actually bounded.
      #3 `SUPPLY_TIME_BUDGET_SECS` clamped to [10, 290] (compile-time invariant
      < 300 s) so a misconfig can't re-time-out or write nothing. #4 ORDER BY
      `coalesce(fetched_at, toDateTime(0))` so stalest-first no longer depends on
      the `join_use_nulls` default. New `absent` stat. 5 unit + const-assert
      invariant; clippy/fmt/tsc clean.
  - date: 2026-07-07
    status: active
    who: okarcz
    note: >
      Deployed to prod (PR #90 merged to develop; rebuilt all worker bootstraps,
      diff-reviewed, `make deploy-production-eventbridge` → UPDATE_COMPLETE 58 s).
      Verified live: post-deploy REPORT `Duration≈240128 ms Succeeded` (was
      `300000 ms Status: timeout` pre-deploy); `written:945 deferred:4054
      deadline_hit:true`; `asset_supply` count 3434→4280 climbing, `absent`
      zero-writes present. Zero Errors datapoints post-deploy. 3/4 AC confirmed;
      only `supply-errors` alarm→OK pending the 1 h window (re-check ~12:00, then
      archive). Registry ≥5000 (max_assets cap) → full coverage ~5–6 hourly runs.
  - date: 2026-07-07
    status: completed
    who: okarcz
    note: >
      DONE. All 4 acceptance criteria confirmed in prod. Final one:
      `supply-errors` alarm cleared ALARM→OK at 11:31:53 UTC (zero Errors since
      the pre-deploy timeouts rolled off the 1 h window). Fix = staleness
      self-checkpoint + 240 s wall-clock budget + Ok(None)→write-0 rotation +
      `configureAsyncInvoke(retryAttempts:0)` + budget clamp + join-nulls-safe
      ordering (PR #90, incl. 4 code-review fixes). Deployed + live-verified.
      Archived.
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

> **Legend.** `[x]` = code-complete + tested. `[ ]` **(operational)** = mechanism
> implemented + tested; confirmed only by a real post-deploy scheduled run.

- [x] A scheduled supply run completes without timing out — a wall-clock budget
      (`SUPPLY_TIME_BUDGET_SECS`, default 240 s < the 300 s Lambda limit) stops the
      Horizon walk early and flushes. **CONFIRMED in prod 2026-07-07:** post-deploy
      `REPORT` lines show `Duration ≈ 240128 ms … Succeeded` (no `Status: timeout`),
      vs the pre-deploy `Duration: 300000.00 ms … Status: timeout` on the same
      function minutes earlier.
- [x] `asset_supply` covers the full active asset set (no partial-walk gap) — the
      walk loads the **stalest** assets first (`load_stalest_credit_assets`,
      ordered by `asset_supply.fetched_at` with never-fetched leading), so
      successive hourly runs round-robin the whole registry. **CONFIRMED in prod:**
      each run does ~945 assets and defers the rest (`written:945 deferred:4054
      deadline_hit:true`); `asset_supply FINAL` count climbed 3434→4280 across
      runs; `absent` zero-writes present (`zero_rows:5`). Registry is ≥5000
      (max_assets cap hit) so full coverage round-robins over ~5–6 hourly runs.
- [x] `supply-errors` alarm returns to `OK` — **CONFIRMED 2026-07-07 11:31:53 UTC**
      (`StateValue: OK`). The 1 h `Sum(Errors)` window rolled off the last
      pre-deploy timeout (~10:35) with zero errors since, so it cleared ALARM→OK
      on the first clean evaluation.
- [x] Async-retry storm bounded — function-error async retries set to 0 via
      `fn.configureAsyncInvoke` (the correct layer; the EventBridge target
      `retryAttempts` only governs delivery — code-review finding #2), and with
      clean completion there is no error to retry. Prod runs Succeeded, so no
      re-drive occurred.

## Implementation Notes

Code-complete + **deployed to prod 2026-07-07** (`Prices-production-EventBridge`,
58 s, `UPDATE_COMPLETE`); verified live (see Acceptance Criteria).

- **`packages/supply-worker/src/lib.rs`** —
  - `SupplyRunConfig { time_budget, max_assets }` + `from_env()`
    (`SUPPLY_TIME_BUDGET_SECS` / `SUPPLY_MAX_ASSETS_PER_RUN`); defaults 240 s /
    5000.
  - `load_credit_assets` → **`load_stalest_credit_assets(client, limit)`**: LEFT
    JOINs the latest `fetched_at` per asset (`GROUP BY max`), `ORDER BY
    fetched_at ASC` (never-fetched = epoch-0 default → first), `LIMIT ?`.
  - `run_supply(..., cfg)` walks under an `Instant`-based deadline; on budget-hit
    it stops starting fetches, flushes the pending batch, and reports
    `deferred` + `deadline_hit` on `SupplyStats`.
- **`src/main.rs`** — builds `SupplyRunConfig::from_env()` at cold start, passes
  it in, logs `deferred`/`deadline_hit`.
- **`infra/…/lambda-baseline.ts`** — new optional `targetRetryAttempts` prop on
  the worker target.
- **`infra/…/eventbridge-stack.ts`** — supply sets `SUPPLY_TIME_BUDGET_SECS=240`
  + `targetRetryAttempts: 0`.
- **Tests** — unit: config-default-under-timeout. IT (Docker-gated): stalest-first
  ordering (never-fetched leads the already-fetched) + `limit` slice cap. 5 unit
  pass; `cargo check --features lambda` / clippy / fmt clean; infra `tsc` clean.

## Design Decisions

### From Plan

1. **Batch + checkpoint across invocations (fix option 1).** No single invoke
   walks the whole registry; each does a bounded slice and the next resumes.

### Emerged

2. **Staleness self-checkpoint, not an external cursor.** The plan suggested a
   cursor in `discovery_state`/SSM. Instead the checkpoint is *implicit* in the
   data: order by `asset_supply.fetched_at` and refresh the stalest first —
   writing them stamps a fresh `fetched_at` so they fall to the back next run.
   No cursor to store/advance, no offset drift when assets are added/removed, and
   it self-heals (a missed asset stays stalest and leads the next run). Chosen
   because `asset_supply` already carries `fetched_at` (its RMT version).
3. **Time budget is the primary bound; `max_assets` is a query-size cap.** A
   wall-clock deadline (240 s) directly prevents the timeout regardless of
   per-asset Horizon latency, which a fixed count cannot. `max_assets` (5000)
   only keeps the loaded list bounded as the registry grows.
4. **`retryAttempts: 0` added as an optional shared-helper prop**, used only by
   supply — surgical, leaves the other workers' default (2) untouched.
