---
id: "0145"
title: "All four pre-roll scripts carry the unguarded argMax(close_usd) — 121 sites that will bake zeros into the 0088 and 0136 pre-rolls"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0144", "0146", "0088", "0136", "0114", "0131"]
tags:
  ["priority-high", "effort-small", "clickhouse", "data-correctness", "pre-roll", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/preroll.sql"
  - "../../../packages/prices-clickhouse/schema/preroll-incremental.sql"
  - "../../../packages/prices-clickhouse/schema/preroll-live-gap.sql"
  - "../../../packages/prices-clickhouse/schema/preroll-amm-reprice.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (scope correction C1). The defect BE's
      0199 report located in the six rollup MVs is also in every pre-roll
      script — 121 further sites. Time-critical: [[0088]] pass 2 and [[0136]]'s
      gap pre-roll both run this logic at span scale.
---

# Pre-roll scripts carry the same unguarded `argMax(close_usd, …)`

## Summary

[[0144]] reproduced that `argMax(close_usd, t.timestamp)` with no `> 0` guard
makes a coarse row inherit `0` whenever its newest sub-bucket is not yet
enriched — discarding the priced sub-buckets underneath it. That task names the
six MVs in `rollups.sql`. **The identical expression is in every pre-roll script
as well**, and those are the scripts about to be run over large historical spans.

| File | Unguarded sites |
|---|---|
| `preroll.sql` | 6 |
| `preroll-incremental.sql` | 14 |
| `preroll-live-gap.sql` | 6 |
| `preroll-amm-reprice.sql` | 95 |
| **total** | **121** |

`argMaxIf(close_usd` appears nowhere in the schema.

## Why this is urgent rather than merely large

Two pre-rolls are queued against exactly this code:

- **[[0088]] pass 2** finishes ~2026-08-09/10 and needs its output pre-rolled.
- **[[0136]]**'s 2026-07-21→08-03 freeze gap needs a bounded incremental
  pre-roll.

Run either against today's scripts and it manufactures a fresh estate of coarse
rows whose `close_usd` is 0 despite priced sub-buckets underneath — at backfill
scale, over spans where enrichment is by definition incomplete at pre-roll time.
Those rows then age out of the MV re-aggregation windows, at which point only
the [[0114]] sweep can reach them ([[0148]]).

Fixing the scripts is the cheapest link in the whole [[0144]] chain: they are
plain SQL scripts, not provisioned objects, so there is **no [[0142]] no-op
trap, no DROP window and no freshness exposure**. It merges and it is live.

## Implementation

- Replace `argMax(close_usd, t.timestamp)` with
  `argMaxIf(close_usd, t.timestamp, close_usd > 0)` at all 121 sites.
- **Check `preroll-amm-reprice.sql` for a generator first.** 95 near-identical
  unrolled blocks is not hand-written; if a generator exists, fix it and
  regenerate rather than editing the output.
- Note in each file header that `close` and `close_usd` may now come from
  different sub-buckets — the same disclosure [[0146]] owes `rollups.sql`.
  An approximately-right USD close beats a fabricated zero, but the two columns
  silently ceasing to be same-row will bite a future reader.
- Regression test on CH **26.3.10.60** (the prod pin): a span whose newest
  sub-bucket is unpriced must pre-roll to the latest *priced* close, not 0.
  [[0144]]'s `repro/03_tests.sql` TEST A is the shape to copy.

## Acceptance Criteria

- [ ] No `argMax(close_usd` remains in any `preroll*.sql`; a guard test asserts
      this so the pattern cannot be reintroduced.
- [ ] If `preroll-amm-reprice.sql` is generated, the generator is fixed too.
- [ ] Regression test on 26.3.10.60 reproducing the zero-inheritance, green.
- [ ] Merged **before** the [[0088]] pass-2 pre-roll and the [[0136]] gap
      pre-roll are run.
- [ ] Header disclosure of the `close` / `close_usd` decoupling in each file.
