---
id: "0148"
title: "Repair the historical close_usd estate the argMax fix cannot reach"
type: BUG
status: completed
related_adr: []
related_tasks: ["0144", "0146", "0145", "0114", "0149", "0136", "0088", "0286"]
tags:
  ["priority-medium", "effort-medium", "clickhouse", "data-correctness", "enrichment", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/repair.rs"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 4). The [[0146]] fix corrects
      coarse rows from the moment it lands; it does not retroactively repair
      rows already zeroed and aged out of the MV re-aggregation windows.
  - date: 2026-09-25
    status: completed
    who: okarcz
    note: >
      Closed into [[0286]]. Its phase 3 re-ingests all history, and the
      runbook's new §7b-2 (PR #353) re-prices every coarse tier over the
      full range with coarse-repair after the 1m drain. The per-tier check
      is 0286's "Coarse history re-priced" criterion. No separate repair
      is run.
---

# Repair the frozen historical `close_usd` estate

## Summary

[[0146]] stops the rollup MVs writing `close_usd = 0` over priced sub-buckets.
It fixes the future, not the past:

- Rows **inside** an MV's re-aggregation window self-heal — the MV re-appends a
  correct value on its next refresh.
- Rows **outside** the window are frozen at whatever they were when they aged
  out, and no MV will ever touch them again.

That frozen estate is what this task repairs. [[0144]] query D sizes it per
tier; **query E** narrows it to the rows outside the windows, which is the
actual work list.

## The good news on ordering

The [[0114]] coarse sweep already exists to re-derive `close_usd` on coarse
rows, and [[0144]] found it gets clobbered by the MVs' `sum(version)`
arithmetic ([[0149]]). **That clobbering only happens inside the
re-aggregation windows** — precisely the region this task does *not* target. So
the historical repair can proceed **without** [[0149]] landing first.

Confirm this rather than assume it: verify on a sample that a swept historical
row still carries the repaired value after a full MV refresh cycle.

## Implementation

- Run [[0144]] queries D and E on prod to produce the work list per tier
  (`_1h`, `_4h`, `_1d`, `_1w`, `_1M`, `_15m`).
- Drive the [[0114]] sweep over that span. Heavy-repair gotchas from [[0097]]
  apply: `FINAL` is mandatory, chunk by month, RMT ties need DELETE-first.
- Sample-verify durability after a full MV refresh cycle before declaring done.
- **Coordinate with [[0136]]'s 2026-07-21→08-03 gap pre-roll and [[0088]]
  pass 2's pre-roll.** Do not sweep a span that is about to be re-pre-rolled —
  and do not sweep before [[0145]] lands, or the pre-roll will re-introduce
  zeros into the span just repaired.
- Any span that cannot be repaired (genuinely no priced sub-bucket ever
  existed) should be **explicitly written off and recorded**, not left
  ambiguous.

## Acceptance Criteria

- [ ] Queries D and E run on prod; the estate sized per tier and recorded.
- [ ] [[0145]] and [[0146]] both landed before the sweep runs.
- [ ] Sweep executed over the frozen span; residual count re-measured.
- [ ] Durability verified after a full MV refresh cycle on a sample, not
      assumed from [[0149]]'s window argument.
- [ ] Un-repairable spans explicitly written off with the reason recorded.
- [ ] No collision with the [[0136]] / [[0088]] pre-roll schedules.

## Completion record (2026-09-25)

Closed without its own run: superseded by [[0286]]. Phase 3 rewrites the
whole `price_ohlcv_1m` history, and runbook step §7b-2
(`docs/runbooks/0286-reingest-history.md`, PR #353) then re-prices all six
coarse tiers from 2015-01 to two months back with `coarse-repair` in plain
mode. The acceptance check lives in 0286's "Coarse history re-priced"
criterion (per tier, before and after).

Found 2026-09-24: without that step the 0286 runbook left coarse history
unpriced. Enrichment prices `1m` only
(`infra/src/lib/stacks/eventbridge-stack.ts:483`), and the coarse sweep looks
back two months (`:627`).
