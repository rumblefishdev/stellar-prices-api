---
id: "0179"
title: "Rollup leading indicators — pending-mutation age, part counts and view_refreshes exceptions"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0137", "0136", "0109", "0134"]
tags:
  ["priority-medium", "effort-small", "clickhouse", "observability", "milestone-M2"]
milestone: 2
links: []
history:
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0137]]. The primary freshness signal (per-tier
      `now() - max(timestamp)`) shipped without these three leading indicators
      because they need `system.mutations` and `system.view_refreshes`, whose
      readability by the scoped mTLS user is unmeasured — and the runtime users
      are XML-managed by BE, so we cannot `GRANT` them ourselves ([[0134]]).
      0137 was deliberately designed to need no `system.*` access at all so it
      could ship first.
---

# Rollup leading indicators

## Summary

[[0137]] alarms when a rollup tier's newest bucket ages past its bound — the
signal that would have caught [[0136]]'s nine-day freeze. That is the *lagging*
indicator: by the time it fires, the data is already stale.

0136 also left three **leading** indicators, each of which would have fired days
earlier:

- any row in `system.mutations` with `is_done = 0` older than ~1 h — in 0136
  these sat for **13 days**;
- any `prices` table above ~1,000 active parts (`parts_to_delay_insert`), well
  before the 5,000 throw limit;
- a non-empty `exception` on any row of `system.view_refreshes`.

## Context

The blocker is access, not design. Measure first:

```sql
-- Run as the scoped mTLS user (prices_writer), NOT as `default`.
SELECT count() FROM system.mutations;
SELECT count() FROM system.view_refreshes;
```

If either is unreadable, that half is blocked on a BE request to widen the
XML-managed grants — worth batching with any other grant need rather than
raising alone.

⚠️ **`system.parts` is already readable** (0137 measured against it locally and
the task-0137 §Access note records the grant), so the **part-count** indicator is
unblocked today even if the other two are not. It is the cheapest of the three
and can ship alone.

## Implementation

- Extend `rollup-freshness-probe` rather than adding a fourth probe — it already
  runs every 15 minutes, already holds a CH client, and already has dead-probe
  cover in the [[0112]] `workerHealth` array.
- Publish as additional metrics under the existing `Prices/Rollup` namespace so
  the IAM grant needs no change: e.g. `PendingMutationAgeSeconds`,
  `MaxActivePartsPerTable` (dimension `Table`), `ViewRefreshExceptions`.
- ⚠️ **Overlaps [[0109]]'s guard**, which already has to watch `system.mutations`.
  Settle ownership before building — the 0137 acceptance criterion was written
  as "here or in 0109, without duplicating each other".

## Acceptance Criteria

- [ ] Readability of `system.mutations` and `system.view_refreshes` by the scoped
      mTLS user is **measured** and recorded either way.
- [ ] Part-count indicator ships (unblocked regardless of the above).
- [ ] Mutation-age and `view_refreshes` indicators ship, or are recorded as
      blocked on a named BE grant request.
- [ ] No duplication with [[0109]] — ownership of the `system.mutations` watch is
      settled and written down in both tasks.
- [ ] Thresholds recorded with rationale, consistent with [[0137]]'s bounds.
