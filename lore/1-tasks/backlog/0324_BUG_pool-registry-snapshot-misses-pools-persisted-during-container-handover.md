---
id: "0324"
title: "The ledger processor reads the pool registry once, at start — a pool another environment persists after that start has its trades dropped until the environment is replaced"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0291", "0078"]
tags: [layer-indexing, priority-medium, effort-small, amm, aquarius, ingestion, data-correctness]
links:
  - "../../../packages/prices-ledger-processor/src/main.rs"
  - "../../../packages/prices-ledger-processor/src/metrics.rs"
history:
  - date: "2026-10-01"
    status: backlog
    who: stkrolikiewicz
    note: >
      Found on 2026-09-30 through the unregistered-pool alarm, on the first
      pool the processor learned live since 0291.
---

# The ledger processor reads the pool registry once, at start

## Summary

`prices-ledger-processor` loads `prices.pool_registry` once per execution
environment, at init. Since 0291 the processor also writes the pools it learns
back to that table. A pool persisted by one environment after the next one has
loaded the registry stays unknown to the next one, which drops the pool's
trades as unregistered until it is replaced, about two hours later. It happened
once, on 2026-09-30, and cost three trades. Re-ingest them, and let the
processor pick up a pool persisted after its start.

## Context

- **Where.** `packages/prices-ledger-processor/src/main.rs:90-96` loads the
  asset registry and the pool registry in one `tokio::join!` at init. Nothing
  reloads them during the environment's life.
- **What happened (2026-09-30, CEST).** Aquarius pool
  `CD2CU3DRKJAFLB2ZQFFBQ4WL4Q5XZ36CMTP3PXFLHBNFWFXR3JAGNKLH` was created at
  16:52:42. Environment `38586303ed694be7b9f3dae3dda92cc9` learned it and
  persisted it at 16:52:44. Environment `6f13304421a04b9183ddb099883b266d` had
  initialised at 16:52:09, before that write, and processed the ledgers that
  followed. It dropped the pool's trades as `UnregisteredPoolEvents`: one in the
  17:00 hour, two in the 18:00 hour.
  `prices-production-ledger-processor-unregistered-pool` went to ALARM at 17:01,
  18:05 and 18:40, and was back to OK by 18:55. Environment
  `cbefc1bee02949cba008623cab150521` started at 19:14 and loaded the pool. No
  event has been dropped since (checked 2026-10-01 08:20).
- **The metric's comment misses this path.** `metrics.rs:61-63` names two ways
  a pool can be unregistered: a new factory, or a factory event missed before a
  cold start. This is a third: a pool persisted by another environment after
  this one loaded the registry.
- **How often.** It needs a new pool to be created during the short overlap of
  two environments at a handover. Once in the eight days since 0291's
  persistence went live on 2026-09-22. Each hit drops the new pool's trades for
  the rest of that environment's life.

## Implementation Plan

1. **Re-ingest the dropped trades.** The pool's trades between 16:52 and 19:15
   CEST on 2026-09-30, through the existing re-ingest path for a ledger range.
   Check the pool's candles for that window afterwards.
2. **Pick up late pools.** Options:
   - Read through on a miss: before dropping a trade from a pool-shaped
     contract missing from the in-memory registry, read that contract's row
     from `prices.pool_registry` and keep it if found. Misses are rare, so the
     read costs nothing in steady state, and the window closes.
   - Reload the registry on a timer: simpler, but only narrows the window.
3. **Update the comment** in `metrics.rs:61-63`, and the alarm description if it
   repeats it, to the causes that remain.

## Acceptance Criteria

- [ ] The trades dropped on 2026-09-30 (3 `UnregisteredPoolEvents`) are in
      `prices`, and the pool's candles for that window are rebuilt
- [ ] A trade for a pool persisted after the processor loaded its registry is
      ingested, not dropped — covered by a test
- [ ] The `UnregisteredPoolEvents` comment names only the causes that remain
