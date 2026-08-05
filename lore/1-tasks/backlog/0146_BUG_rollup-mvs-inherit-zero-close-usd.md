---
id: "0146"
title: "All six rollup MVs zero a coarse row's close_usd when its newest sub-bucket is un-enriched"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0144", "0145", "0142", "0137", "0148", "0149", "0095", "0136"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "materialized-view", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 3) — BE 0199 finding 3ii,
      reproduced on the prod CH pin. The single highest-value fix in the chain
      and the only one with a real delivery problem.
---

# Rollup MVs inherit `close_usd = 0` from an un-enriched sub-bucket

## Summary

Every rollup tier carries the USD close forward with

```sql
argMax(close_usd, t.timestamp)            AS close_usd
```

(`rollups.sql:90, 111, 132, 153, 174, 195` — all six MVs). `argMax` takes the
value from the **latest** sub-bucket. When that sub-bucket is not yet enriched
its `close_usd` is `0`, so the coarse row inherits 0 and **discards the priced
sub-buckets underneath it**. A partly-enriched hour does not roll up as partly
priced; it rolls up as unpriced.

Downstream, `price_usd_series*`'s `WHERE close_usd > 0` then drops the bucket
entirely — which is the disappearing-bucket half of BE's yXLM observation
([[0144]] finding 3ii, their 14:13 reading).

Reproduced on CH **26.3.10.60** with a single writer and no version
interaction — [[0144]] `repro/03_tests.sql`, TEST A:

```
timestamp             close   volume_base   close_usd_asis   close_usd_if_guarded
2026-08-04 13:00:00   0.172         42000                0                  0.171
```

## Why this one is hard: delivery, not the fix

The fix is one function per MV. Landing it is the problem.

All six are `CREATE MATERIALIZED VIEW IF NOT EXISTS … REFRESH … APPEND TO …`.
**`IF NOT EXISTS` does not redefine an existing object**, and there is no
`CREATE OR REPLACE` form for a refreshable `TO`-table MV. Editing the file and
re-applying it on ch-prod-01 changes nothing and reports success — that is
[[0142]], which must land first.

The only route is DROP + re-CREATE, and that is not free:

- **The DROP window is a data-loss window** — [[0090]]/[[0095]] are the
  precedent. Any re-CREATE must preserve APPEND + `sum(version)` + aligned
  windows or it silently reintroduces the replace-mode wipe.
- **A dropped MV stops rolling up while it is gone** — [[0136]] went nine days
  unnoticed with no alarm. [[0137]]'s freshness alarm should be deployed before
  the first DROP.

## Implementation

1. **[[0142]] first, but only its cheapest deliverable** — drift detection
   comparing `system.tables.create_table_query` against `rollups.sql`. It
   touches no production object and converts a silent no-op into a loud one.
   Do not let 0142 grow into "convert all six" before this ships.
2. **[[0137]] freshness alarm deployed** before any DROP window opens.
3. `argMaxIf(close_usd, t.timestamp, close_usd > 0)` at all six sites.
4. DROP + re-CREATE **one MV at a time**, with the [[0095]] invariants as a
   pre-flight checklist and expected freshness recovery stated per step. Model
   the procedure on [[0136]]'s per-table recovery runbook.
5. **Document the decoupling in the file header**: `close` and `close_usd` may
   now come from different sub-buckets. That is the right trade — an
   approximately-right USD close beats a fabricated zero — but two columns
   silently ceasing to be same-row is exactly what bites a future reader.
6. Regression test on 26.3.10.60 reproducing TEST A.

## Ordering notes

- Ships **after** [[0145]] (same fix, no delivery problem, has a deadline).
- Once this lands, rows *inside* each MV's re-aggregation window self-heal —
  the MV re-appends a correct value instead of a zero. That is what demotes
  [[0149]] (the version race) from blocker to hygiene.
- Rows *outside* the windows stay frozen and are [[0148]]'s problem.

## Acceptance Criteria

- [ ] [[0142]] drift detection in place; a divergence between `rollups.sql` and
      the live definitions is visible rather than silent.
- [ ] [[0137]] freshness alarm deployed before the first DROP.
- [ ] All six MVs use `argMaxIf`; APPEND + `sum(version)` + aligned windows
      verifiably preserved after re-CREATE.
- [ ] No coarse row carries `close_usd = 0` while `close > 0` and a priced
      sub-bucket exists underneath it — regression test on 26.3.10.60.
- [ ] `close` / `close_usd` decoupling disclosed in the header.
- [ ] Per-MV freshness confirmed recovered after each re-CREATE.
