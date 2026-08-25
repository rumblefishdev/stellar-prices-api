---
id: "0218"
title: "The coarse-table sweep has never executed — it sits behind the 1m pass's `?` and is starved by the Lambda deadline"
type: BUG
status: active
related_adr: []
related_tasks: ["0215", "0114", "0111", "0026"]
tags: ["priority-high", "effort-small", "enrichment", "observability", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/main.rs"
history:
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      Spawned from 0215's post-fix verification. Found by reading the CloudWatch
      log of three consecutive invocations: no "enrichment pass complete" line
      and no "coarse sweep complete" line in any of them. Not a regression from
      the Caddy fix — the sweep was unreachable before it too, for a different
      reason.
  - date: 2026-08-24
    status: active
    who: okarcz
    note: >
      Activated. 0111 deployed to prod 2026-08-24 08:03:24 UTC and the sweep is
      now REACHED: "coarse sweep complete" appears on both post-deploy
      invocations with tables_swept=6, tables_failed=0, tables_skipped=0,
      rows_enriched=204,769 then 204,428, start_month=202607 end_month=202608.
      The 1m pass now returns in ~7.2 s (duration_ms in its own log line) and
      the invocation finishes in 26.7-29.6 s against the 300 s timeout, so
      remaining_ms is no longer saturated at 0.
      ⚠️ That satisfies AC 1 ONLY. The task's own argument stands: this is
      "starved by design, not by accident" — the sweep still sits after the 1m
      pass in one fixed budget, so the next growth in the 1m pass silently
      starves it again. ACs 2, 3 and 4 are untouched: a never-reached stage is
      still indistinguishable from one that ran and found nothing, the sweep's
      budget can still be driven to zero by the preceding stage, and nothing is
      published on an invocation that hits the deadline. Option 1 (separate
      EventBridge rule + Lambda) remains the preferred fix and is the only one
      that survives future growth.
---

# The coarse sweep is unreachable, and has been on both sides of the fix

## Summary

`main.rs:167` is `let stats = pass.run().await?;`. The recurring coarse-table
sweep — [[0114]]'s remedy for the coarse tables carrying no USD values — sits
**after** that `?`. It has therefore never executed in production:

| period | why it never runs |
|---|---|
| ← 2026-08-21 14:46 | `run()` returned `Err(BadResponse(""))` on every invocation ([[0215]]); `?` propagated and the sweep was skipped |
| 2026-08-21 14:46 → | `run()` no longer errors, but the 1m pass consumes the whole budget and the Lambda is killed **inside** it (`Status: timeout`, `Duration: 300000.00 ms`) |

Confirmed by absence in CloudWatch across three consecutive attempts
(`RequestId 198d7653-…`, 15:17-15:36 UTC): neither
`"enrichment pass complete"` nor `"coarse sweep complete"` appears.

## The budget arithmetic cannot rescue it

`main.rs:195-203`:

```rust
const MARGIN_MS: u64 = 60_000;
let remaining_ms = lambda_deadline_ms.saturating_sub(now_ms).saturating_sub(MARGIN_MS);
let budget_ms = sweep_budget_secs.saturating_mul(1_000).min(remaining_ms);
```

The design is sound in isolation — the sweep defers rather than blowing the
timeout. But it assumes the 1m pass *returns*. A pass that runs to the hard
deadline leaves `remaining_ms` saturated at **0**, so even if the code were
reached the sweep would do nothing. The two failure modes compound.

⚠️ **The `time_budget_secs: 120` in the startup log is aspirational.** It is
logged from config at cold start (`main.rs:135`), so `"coarse sweep config
enabled=true tables=5 max_batches=20 time_budget_secs=120"` appears on every
invocation and reads like the sweep is configured and running. It is neither.

## Why this is worth its own task rather than a note on 0111

[[0111]] will free the budget, and that is necessary — but it is not sufficient
and it is not the whole defect:

1. **The failure is silent by construction.** The sweep's own errors are
   deliberately swallowed (`"coarse sweep failed (non-fatal)"`), which is correct
   for a best-effort stage — but there is no signal distinguishing *swept
   nothing*, *swept and failed*, and *never reached*. All three look identical
   from outside.
2. **It is starved by design, not by accident.** Any stage placed after an
   unbounded stage in a fixed budget gets whatever is left, which is zero when
   the first stage is time-bound. Fixing 0111 makes it work *today*; the next
   growth in the 1m pass silently starves it again.
3. **It gates [[0114]]**, which [[0111]] itself calls "the more serious defect"
   that "outranks this task".

## Implementation — options to cost

1. **Separate schedule.** Move the sweep to its own EventBridge rule and Lambda
   so it has an independent budget and cannot be starved by the 1m pass. Cleanest
   and removes the coupling permanently.
2. **Run it first, bounded.** Give the sweep its 120 s before the 1m pass rather
   than after. Preserves one Lambda, but inverts which stage absorbs the squeeze.
3. **Keep the order, add a floor.** Reserve the sweep's budget up front and make
   the 1m pass respect the reduced deadline. Smallest change; still one Lambda.

Option 1 is preferred and is the only one that survives future growth in the 1m
pass. Note that all three depend on the pass not running to the hard deadline —
so [[0111]] is a prerequisite for the sweep doing useful work, whichever is
chosen.

## Acceptance Criteria

- [ ] `"coarse sweep complete"` appears in CloudWatch on a recurring schedule,
      with `rows_enriched` recorded before/after.
- [ ] A stage that is never *reached* is distinguishable in logs and metrics from
      one that ran and found nothing — verified by inducing, not inferred.
- [ ] The sweep's budget cannot be reduced to zero by the preceding stage.
- [ ] `EnrichmentPassDurationMs` and the sweep's own metric are both published on
      an invocation that hits the Lambda deadline, so a starved run is visible.

## Out of scope

- The full-table scan that consumes the budget — that is [[0111]].
- The coarse tables' missing USD values themselves — that is [[0114]].
