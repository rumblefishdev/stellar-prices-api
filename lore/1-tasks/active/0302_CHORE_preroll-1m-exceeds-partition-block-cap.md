---
id: "0302"
title: "The full-range 1M pre-roll exceeds max_partitions_per_insert_block and cannot run as generated"
type: CHORE
status: active
assignee: okarcz
related_adr: ["0287"]
related_tasks: ["0286"]
tags: [layer-backend, priority-low, effort-small, clickhouse, rollups, runbook]
links:
  - "../active/0286_BUG_candles-are-built-from-dust-fills-in-the-wrong-order.md"
  - "../../../packages/prices-clickhouse/src/rollup_sql.rs"
  - "../../../packages/prices-clickhouse/schema/preroll.sql"
  - "../../../docs/runbooks/0286-candle-definitions-rollout.md"
history:
  - date: "2026-11-25"
    status: active
    who: okarcz
    note: >
      Implemented on chore/0302_… (6699881f): SETTINGS
      max_partitions_per_insert_block = 1000 on all six full-range
      statements; new 131-month CH test shows each is refused without it.
      PR to follow.
  - date: "2026-11-25"
    status: active
    who: okarcz
    note: "Activated for implementation (quick-fix queue)."
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

- [x] The full-range `preroll.sql` runs to completion against a database
      holding more than 100 monthly partitions, as `prices_admin`, with no
      per-query setting supplied by the caller.
      ✅ 2026-11-25, locally on ClickHouse 26.3.10.60 (the prod engine), as
      `default`: `rollup_pf_it::the_full_range_preroll_writes_more_than_a_hundred_months_as_shipped`
      seeds 131 months and applies the shipped file with no caller setting.
      Not re-run as `prices_admin` on prod: the same setting was accepted from
      `prices_admin` as a URL parameter on 2026-09-22, and a query-level
      `SETTINGS` clause goes through the same settings constraints.
- [x] The generator/file equality test still passes and covers the new clause.
      ✅ `lib.rs` `preroll_sql_is_exactly_the_generators_full_range_inserts` now
      renders with `Some(rollup_sql::FULL_RANGE_SETTINGS)`; 118/118 unit tests.
- [x] The five other full-range statements are either fixed the same way or
      shown not to need it, with the reason recorded here.
      ✅ Fixed the same way. Shown to need it: in the integration test each of
      the six statements, with the clause stripped, is refused with
      `TOO_MANY_PARTS`. Every tier writes the same monthly partitioning from a
      full-range source, so any of them crosses 100 once history does.

## Implementation Notes

Branch `chore/0302_preroll-1m-exceeds-partition-block-cap`, commit `6699881f`.

- `rollup_sql.rs`: `pub const FULL_RANGE_SETTINGS = "max_partitions_per_insert_block = 1000"`,
  rendered through the existing `settings` argument of `rollup_insert`.
- `schema/preroll.sql`: all six INSERTs end in the clause; header names the
  new rendering and explains the cap (task 0302 paragraph).
- `lib.rs`: the file == generator test renders with the constant.
- `tests/rollup_pf_it.rs`: one new CH test (131 months; bare statement refused
  per tier, shipped statement writes > 100 partitions; then the whole file).
  The seed insert itself needs the raised cap.
- `docs/runbooks/0286-candle-definitions-rollout.md` step 5c: says the file
  runs as written since 0302; the URL-parameter form stays for older checkouts.
- Also green: `rollup_chain_it`, `preroll_close_usd_guard_it`. clippy clean for
  the change (one existing warning in `current_mv_it`).

## Design Decisions

### Emerged

1. **All six statements, not only 1M.** The test proves each one is refused
   without the clause, so there is nothing to gain from leaving five unrunnable.
2. **1000, the value the rollout used.** 1000 months is 83 years of history;
   the cap only guards against an accidental insert that fans out wildly.
3. **Bounded pre-rolls untouched.** `preroll-live-gap.sql` and 0286 phase 3
   write one or two months per run; adding the clause there would only hide a
   mistake that widens their bounds.

## Issues Encountered

- The first version of the test split `preroll.sql` on `;` and broke on
  semicolons inside the header comments. It now starts each statement at its
  `INSERT INTO`.
