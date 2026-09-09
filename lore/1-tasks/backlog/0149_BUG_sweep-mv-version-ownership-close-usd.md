---
id: "0149"
title: "Two writers with incompatible version arithmetic own close_usd — the 0114 sweep's repair is overwritten inside the MV window"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0144", "0146", "0148", "0114", "0095", "0143"]
tags:
  ["priority-medium", "effort-medium", "clickhouse", "data-correctness", "materialized-view", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/repair.rs"
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 7) — finding 3ii-b, confirmed on
      the prod CH pin (TEST E). Deliberately ordered after [[0146]], which
      removes most of its consequence.
---

# Sweep vs MV: incompatible version arithmetic on `close_usd`

## Summary

Two writers update `close_usd` on the same coarse rows and claim the row by
different rules:

- The [[0114]] coarse sweep wins by **`version + 1`** (`repair.rs:20-22`).
- The rollup MVs write at **`version = sum(version)`** over their sub-rows —
  and every enrichment event underneath adds 1 to that sum.

Two enrichment events and the MV overtakes the sweep. Worse, the MV gets far
more attempts: `mv_ohlcv_15m_to_1h` re-appends the same hour **every 15 minutes
for 8 hours** (`REFRESH EVERY 15 MINUTE`, window `now() - INTERVAL 8 HOUR`).
The sweep gets one.

Confirmed on CH **26.3.10.60** ([[0144]] `repro/04_sweep_durability.sql`,
TEST E):

```
after the sweep repairs it        close_usd 0.171   version 401
after the next MV refresh         close_usd 0       version 402
what the view publishes now       (empty)
```

So the repair path we already built is defeated inside the re-aggregation
window — the bucket vanishes from `price_usd_series*` again, which is BE's
observation exactly.

## Why this is ordered late

**[[0146]] removes most of the consequence.** Once the MVs use `argMaxIf`, the
re-append writes a *correct* value rather than a zero — so being overtaken by
the MV stops being harmful inside the window. And outside the window the MV
does not write at all, so [[0148]]'s historical sweep is unaffected either way.

That leaves this as a latent trap rather than a live defect: **two writers with
incompatible version arithmetic on one column**, with no mechanism preventing
the next such writer from colliding the same way. Worth closing on its own
terms, not urgent.

Fixing only this and not [[0146]] would leave the zero-propagation completely
intact — the sweep would win a race to publish a value the MV is about to
recompute as zero anyway.

## Implementation

Options to weigh, none obviously correct yet:

- Give the sweep a version domain the MV cannot reach (e.g. a large offset), at
  the cost of making `version` no longer a plain event count.
- Have the sweep write through the same path as the MV so there is one writer.
- Make the sweep skip rows still inside an MV's re-aggregation window entirely,
  and rely on [[0146]] for those — smallest change, but encodes the window
  boundaries in two places.
- Reconsider whether `close_usd` should be MV-carried at all rather than always
  derived by the sweep.

Related: [[0143]] records that there is no `DEPENDS ON` anywhere in the cascade,
so ordering between tiers is already unsynchronised; any fix here should not
assume refresh ordering it does not control.

## Acceptance Criteria

- [ ] A single documented owner for `close_usd` on coarse rows, or a version
      scheme under which the sweep's repair provably survives an MV refresh.
- [ ] Regression test on CH 26.3.10.60 reproducing TEST E and showing the
      repair now survives.
- [ ] The `version` column's semantics restated in the schema header if they
      change.
- [ ] Confirmed not to reintroduce the [[0095]] replace-mode invariants problem.
