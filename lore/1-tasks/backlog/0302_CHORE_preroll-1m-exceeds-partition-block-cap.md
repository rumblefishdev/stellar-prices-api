---
id: "0302"
title: "The full-range 1M pre-roll exceeds max_partitions_per_insert_block and cannot run as generated"
type: CHORE
status: backlog
related_adr: ["0287"]
related_tasks: ["0286"]
tags: [layer-backend, priority-low, effort-small, clickhouse, rollups, runbook]
links:
  - "../active/0286_BUG_candles-are-built-from-dust-fills-in-the-wrong-order.md"
  - "../../../packages/prices-clickhouse/src/rollup_sql.rs"
  - "../../../packages/prices-clickhouse/schema/preroll.sql"
  - "../../../docs/runbooks/0286-candle-definitions-rollout.md"
history:
  - date: "2026-09-22"
    status: backlog
    who: okarcz
    note: >
      Spawned from 0286's phase-1 rollout, where step 5c failed on production
      with Code 252. Worked around on the day with a URL parameter; the
      generated statement is still unrunnable as written.
---

# The full-range 1M pre-roll cannot run as generated

## Summary

The last statement of `schema/preroll.sql` rebuilds `price_ohlcv_1M` from
`price_ohlcv_1d` over the whole range. `price_ohlcv_*` is partitioned
`toYYYYMM(timestamp)`, the chain has ~131 months of history, and ClickHouse
caps a single INSERT block at `max_partitions_per_insert_block = 100`. So the
statement fails:

```
Code: 252. DB::Exception: Too many partitions for single INSERT block
(more than 100). (TOO_MANY_PARTS)
```

It is not a data problem — 131 partitions on a table is well inside
ClickHouse's own "under 1000..10000" guidance. The cap is on one insert
*block*, and the limit only grows further out of reach as the chain ages.

## Context

Hit on production 2026-09-22 during [[0286]] phase 1, step 5c, with
`price_ohlcv_1M` already TRUNCATEd — so the month tier was empty and viewless
until it was resolved. The rollout used
`?max_partitions_per_insert_block=1000` as a URL parameter rather than editing
the SQL, because `preroll.sql` is rendered from
`packages/prices-clickhouse/src/rollup_sql.rs` and pinned byte-for-byte to it
by a unit test.

⚠️ Only the **full-range** pre-roll is affected. [[0286]] phase 3 runs bounded
per-month pre-rolls that write one or two partitions per block and never
approach the cap.

## Implementation

- Render `SETTINGS max_partitions_per_insert_block = 1000` on the full-range
  1M statement in `rollup_sql.rs`, regenerate `preroll.sql`, update the
  file-equals-generator test.
- Decide whether the other five full-range statements need it. They write into
  the same monthly partitioning from the same source, so they very likely do —
  the rollout never reached them because only 1M is rebuilt in phase 1.
- Keep the runbook's URL-parameter form as the escape hatch for an operator
  running an older checkout.

## Acceptance Criteria

- [ ] The full-range `preroll.sql` runs to completion against a database
      holding more than 100 monthly partitions, as `prices_admin`, with no
      per-query setting supplied by the caller.
- [ ] The generator/file equality test still passes and covers the new clause.
- [ ] The five other full-range statements are either fixed the same way or
      shown not to need it, with the reason recorded here.
