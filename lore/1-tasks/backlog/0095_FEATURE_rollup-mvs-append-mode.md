---
id: "0095"
title: "Rollup MVs → APPEND mode (stop them wiping pre-rolled history)"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0090", "0064", "0094"]
tags: [layer-infra, priority-high, effort-small, milestone-M1, clickhouse, rollup, materialized-view, data-loss]
milestone: 1
links:
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: 2026-07-15
    status: backlog
    who: okarcz
    note: >
      Spawned from 0090 execution (Layer 2). Discovered during the pre-roll that
      the six mv_ohlcv_* rollup MVs are REPLACE-mode and were wiping the coarse
      tables every refresh. 0090 DROPs them as a stop-gap; this task recreates
      them correctly as APPEND.
---

# Rollup MVs → APPEND mode (stop them wiping pre-rolled history)

## Summary

The six `prices.mv_ohlcv_*` rollup MVs in `rollups.sql` are declared
`REFRESH EVERY <n> TO price_ohlcv_<coarse> AS SELECT … WHERE timestamp >= now()
- <window>` **with no `APPEND`**. A refreshable MV without `APPEND` *atomically
replaces* its target table on every refresh. So each MV overwrites its coarse
table with only the recent-window result — deleting all history (incl. any
pre-roll) every refresh. With live frozen the windows are empty, so they were
emptying the coarse tables outright.

Task 0090 **DROPped** the six MVs so the historical pre-roll can persist. This
task recreates them the right way.

## Context

Discovered in the 0090 pre-roll (2026-07-15): a successful pre-roll into the
coarse tables was wiped within a minute by `mv_ohlcv_1m_to_15m`
(`REFRESH EVERY 1 MINUTE`). Root cause of the long-standing "coarse tables empty"
symptom — deeper than 0090's original "pre-roll never run" diagnosis.

## Implementation

- Change each `mv_ohlcv_*` in `rollups.sql` to `REFRESH EVERY <n> APPEND TO …`
  so the refresh **inserts** the recent window (RMT collapses the re-inserted
  overlapping buckets by `version`) instead of replacing the whole table.
- Re-evaluate the refresh cadence vs window: `APPEND` re-inserting a 2h window
  every 1 minute = heavy write amplification (~120× duplicate buckets before
  merge). Consider a longer refresh interval or a tighter window so RMT merge
  load stays sane.
- Recreate the six MVs in prod (`DROP` + `CREATE … APPEND`) — shared cluster,
  owner sign-off.
- Verify: after recreate, a coarse table retains its pre-rolled history AND gains
  new live buckets, and is NOT emptied on refresh.

## Dependencies

- **Coupled to the live-freeze fix** ([[proto27-xdr26-live-freeze]] / 0064 / 0094):
  the MVs read live `prices.price_ohlcv_1m`. Until live ingestion writes fresh
  candles again, there is nothing for `APPEND` to add, so this is not urgent and
  should land alongside the live-path fix. Until then the coarse tables are
  pre-roll-only and static (acceptable — BE gets history now via 0090).

## Acceptance Criteria

- [ ] `rollups.sql` six `mv_ohlcv_*` use `REFRESH … APPEND …`; cadence/window
      re-evaluated for RMT merge load.
- [ ] MVs recreated in prod; a coarse table keeps pre-rolled history across a
      refresh AND picks up new live buckets (verified).
- [ ] No replace-mode refreshable MV remains on any `price_ohlcv_*` table.
