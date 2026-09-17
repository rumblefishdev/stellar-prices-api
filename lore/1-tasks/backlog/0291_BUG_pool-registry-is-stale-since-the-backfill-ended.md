---
id: "0291"
title: "pool_registry has not learned a pool since 2026-07-06 — live forgets newer pools on every cold start and drops their trades"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0285", "0286", "0282", "0078", "0101"]
tags: [layer-indexing, priority-high, effort-small, amm, aquarius, soroswap, ingestion, data-correctness, clickhouse]
links:
  - "../active/0285_RESEARCH_pool-registry-does-not-match-what-is-trading/notes/S-classification-2026-09-17.md"
  - "../../../packages/prices-ledger-processor/src/main.rs"
history:
  - date: 2026-09-17
    status: backlog
    who: okarcz
    note: >
      Spawned from 0285. The table's only writer is the history backfill, whose
      data ends 2026-07-06; the live processor only reads it. 22 Aquarius pools
      (34,684 trades) and 10 Soroswap pools created since are missing. Blocks
      0286 phase 3 for the live-era AMM months (its precondition 9).
---

# `pool_registry` is stale since the backfill ended

## Summary

`prices.pool_registry` was last written **2026-08-11**, by the history backfill,
whose data ends **2026-07-06**. The live processor loads it at cold start and
never writes it (`prices-ledger-processor/src/main.rs:80-94`). A pool created
later is known to live only while a container that saw its factory event is
warm; after any cold start its trades are silently dropped — an unknown
contract's `trade` is not even counted as unresolved.

Missing today (live era, see [[0285]]'s note):

| venue | pools | events |
| --- | --- | --- |
| aquarius | 22 | 34,684 `trade` (~5% of Aquarius) |
| soroswap | 10 | 267 `swap` |

## Why it matters now

- **[[0286]] phase 3** reprices live-era AMM months from the registry after
  `DROP PARTITION` — with a stale registry it would delete candles for these
  pools and not put them back. **Fix before phase 3 reaches 2026-07.**
- [[0282]] and [[0101]] use the registry as their denominator.

## Implementation

- **One-off:** seed the missing pools from the factory events since
  2026-07-06 (`add_pool`, `new_pair`, Phoenix `create`) — `events-backfill`
  already persists what it learns; check whether a registry-only run over
  2026-07-06 → now is enough.
- **Durable:** have the live processor persist pools it learns from factory
  events (it already does the same for assets, task 0132), so a cold start
  never forgets one.
- **Observable:** count and alarm on `trade`/`swap` events from contracts the
  live path cannot classify, including `trade` (today only `swap` is counted,
  and live never writes `unresolved_pools`) — this also closes [[0282]]'s open
  observability criterion.

## Acceptance Criteria

- [ ] The 22 Aquarius and 10 Soroswap pools are in `pool_registry`.
- [ ] A pool created after deploy is in the table within one reconcile run, and
      survives a forced cold start (test + prod check).
- [ ] Unclassifiable pool events are counted and alarmed on the live path.
- [ ] Aquarius raw (registry-joined, all pool wasms) vs stored for a full day
      after the fix shows no stored > raw residue.
