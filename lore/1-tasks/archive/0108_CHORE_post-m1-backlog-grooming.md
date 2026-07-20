---
id: "0108"
title: "Post-M1 backlog grooming — close done/superseded tasks, salvage their content"
type: CHORE
status: completed
related_adr: []
related_tasks: ["0073", "0068", "0065", "0036", "0035", "0030", "0062", "0072", "0101", "0088"]
tags: ["layer-ops", "priority-medium", "effort-small", "housekeeping", "lore"]
links: []
history:
  - date: 2026-07-20
    status: active
    who: okarcz
    note: >
      Milestone 1 delivered, so the backlog was swept for tasks that are already
      done, duplicated, or answered by later work. Seven closures identified and
      each verified against the code (not against the task's own prose) before
      being proposed. 0047 was reviewed for closure and deliberately KEPT — see
      §Kept open.
  - date: 2026-07-20
    status: completed
    who: okarcz
    note: >
      Done. 7 tasks archived (0030, 0035, 0036, 0062, 0065, 0068, 0073);
      backlog 23 → 17 (7 closed, 1 spawned). Content salvaged into 0072 (0068's
      §5.5 outlier filter + DROP/CREATE redeploy gotcha + TO(...) footgun) and
      0101 (0065's cross-invocation minute-boundary residual, folded into its
      existing minute-alignment rule). 0109 spawned for the backfill preflight
      guard and linked from 0088.
      Two findings beyond the planned scope, both recorded on their tasks rather
      than fixed here: 0085's gate has EXPIRED (it said "do before the 0053
      backfill runs" — that backfill has run, so the timeout risk is now live,
      and a comment at ch_enrich.rs:293 contradicts the code); and CI builds 6
      Lambda crates but verifies only 5 — enrichment-worker is built and never
      checked — which is worse than 0077 describes.
      No code, schema, or prod changes. Triage of the remainder is in the PR body.
---

# Post-M1 backlog grooming

## Summary

The backlog carried 23 tasks after Milestone 1 shipped. Seven are closable:
three were silently completed by later tasks, one is a duplicate, and three have
no remaining consumer. This task archives them with evidence, and salvages the
content that would otherwise be lost into the tasks that inherit it.

Grooming only — **no implementation work**. The five ready-to-do tasks
(0093, 0077, 0075, 0098, 0105) stay in the backlog and keep their own scope.

## Context

Each closure was verified by reading the code, because several tasks describe a
state the repo has since moved past. The two that matter most:

- **0073** was fully implemented under **0106** (PR #125/#126, archived) — a
  different task ID, so nothing ever closed 0073.
- **0065** was fixed in the backfill rewrite and even has a regression test
  named after its exact failure mode, but the task was never updated.

## Closures

| # | Verdict | Evidence |
|---|---------|----------|
| **0073** store/expose `earliest_data_available` | **done** | Shipped as 0106. `packages/prices-api/src/backfill/queries_ch.rs:30` selects the stored column; no `min(timestamp)` remains in the module. OHLCV `backfill_note` reads the first returned candle (`assets/handlers.rs:344`), not an interim scan. |
| **0068** current_prices MV v2 columns | **duplicate** | Same four DEFAULT columns, same MV, as 0072. Content salvaged into 0072 (below); 0072 additionally owns the `/price` pass-through switch. |
| **0065** cross-chunk intra-minute candles | **done** | `packages/events-backfill/src/run.rs:198` holds run-level accumulator state and flushes via `flush_older_than`, keeping the boundary minute open. Regression test `minute_split_across_calls_is_summed_once_not_undercounted` (`run.rs:544`). Live path uses one accumulator per contiguous run (`prices-ledger-processor/src/reconcile.rs:100`). Residual salvaged into 0101. |
| **0036** Phoenix 237-byte XYK WASM delta | **superseded** | The question was whether the second XYK build alters event emission. **0099 answered it**: Phoenix emits variable-length swap groups (7-event as well as 8-event); the `n >= 8` gate dropped ~2.1%. Fixed and deployed 2026-07-17 11:57:52. |
| **0035** Phoenix stable-pool re-survey | **won't-do** | Speculative monitoring with no consumer — the seeder does not seed Phoenix `stable` at all (`packages/pool-registry-seed/src/lib.rs:88` maps it to `None`). If wanted later this is a scheduled check, not a task. |
| **0030** BE `topics_xdr` column-naming | **won't-do** | Cross-repo courtesy note to `soroban-block-explorer`; no prices-api change and no blocker. The decode assumption it warns about is long since settled on our side. |
| **0062** enrichment progress via rows-affected | **won't-do (gated)** | Hard-gated on a `clickhouse` crate upgrade that has not happened — still 0.13, whose `query().execute()` discards `written_rows` (`packages/enrichment-worker/src/ch_enrich.rs:285`). Purely a cost optimisation; the correctness hazard in the area was already fixed under 0061. Will re-surface naturally with the crate bump. |

## Content salvage (do before archiving)

Closing these files must not lose information:

- **0068 → 0072**: port the §5.5 inter-source median-outlier filter on `vwap_24h`,
  and the "a refreshable MV's definition is fixed at create time, so redeploy is
  `DROP VIEW` + re-`CREATE`" gotcha. 0072 has neither.
- **0065 → 0101**: the residual **cross-invocation** minute split. Within one run
  the accumulator handles it; two separate runs whose ranges split a minute each
  emit a partial candle and RMT keeps only the higher-version one. 0101 is the
  task that runs bounded reprice windows, so that is where a split boundary can
  actually bite — it already carries a minute-alignment requirement for `--end`.
- **0036 → archive note only**; nothing to salvage.

## Kept open (reviewed, not closed)

- **0047** cross-tenant throughput on shared Hetzner CH — proposed for closure on
  the grounds that the full system has run against `ch-prod-01` alongside BE for
  weeks without a contention incident. **Rejected by the owner (2026-07-20):**
  "ran without incident" is an inference, not the measurement the task asks for,
  and the task was deliberately scoped as a *joint* look at
  `system.query_log`/`metric_log` with the BE team. Stays in backlog for that.

## Spawned

- **0109** — preflight guard in `sdex-backfill`: refuse to start (or loudly warn)
  when the `prices-production-cleanup` EventBridge rule is ENABLED. Named as a
  follow-up in 0088's cleanup-incident section but never made a task. It is the
  fix that structurally prevents a recurrence; the precondition currently lives
  only in runbook prose, which demonstrably did not survive the gap between the
  0090 re-run and the 2026-07-15 tail start.

## Acceptance Criteria

- [x] Seven tasks archived with a dated history note carrying the closure reason
      and its code evidence.
- [x] 0068's outlier-filter spec + DROP/CREATE gotcha present in 0072.
- [x] 0065's cross-invocation residual present in 0101.
- [x] 0109 created in backlog, linked from 0088.
- [x] 0047 left in backlog, with the review outcome recorded on it.
- [x] Index regenerated; backlog count reflects the closures (23 → 17).

## Out of scope

- Implementing any backlog task. 0093 / 0077 / 0075 / 0098 / 0105 are triaged as
  ready but are not touched here.
- Running 0105's `DROP TABLE` statements, or any other prod SQL.
