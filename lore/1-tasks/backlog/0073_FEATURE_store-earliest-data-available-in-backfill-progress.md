---
id: "0073"
title: "Store earliest_data_available on backfill_progress + populate it in the backfill push steps"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0040", "0051"]
tags: ["phase-future", "effort-medium", "priority-medium"]
links: []
history:
  - date: 2026-06-30
    status: backlog
    who: claude
    note: >
      Spawned from 0040 Phase 3. Overview §4.5 specifies
      `earliest_data_available` as a STORED, O(1) value recorded by each
      backfill push step — but the column was never added to the ClickHouse
      `backfill_progress` table (init.sql) and the writers don't record it. The
      0040 read API works around it (omits it on /backfill/status; computes
      per-asset min(timestamp) for the OHLCV backfill_note). This task lands the
      column + writer population so the API can read it directly.
---

# Store earliest_data_available on backfill_progress

## Summary

Add `earliest_data_available` to `prices.backfill_progress` and have the SDEX
and Soroban-AMM backfill push steps record it, per overview §4.5. The 0040 read
API currently substitutes a live `min(timestamp)` (per-asset, for OHLCV's
`backfill_note`) and omits the per-stream value on `/backfill/status`.

## Context

§4.5 designed `earliest_data_available` as a **stored** per-stream timestamp
("recorded by the push step when it first lands a candle … not computed live via
MIN(timestamp). Returned as-is, so reads are O(1)"). It matters most for
`/backfill/status`, where the live value is a per-stream `min(timestamp)` over
the whole `price_ohlcv` column (timestamp is not the sort key → full scan,
expensive at scale). Same producer-gap shape as task 0072.

## Implementation

- **Schema**: add `earliest_data_available Nullable(DateTime)` to
  `prices.backfill_progress` in `packages/prices-clickhouse/schema/init.sql`
  (ALTER … ADD COLUMN; idempotent like the other init statements).
- **Writers**: the SDEX (`sdex-backfill`) and Soroban-AMM backfill push steps
  lower `earliest_data_available` toward older data each time they land an older
  candle (monotonic min; only update when the new oldest is earlier).
- **0040 read API upgrade**:
  - `/backfill/status` returns `sdex.earliest_data_available` /
    `soroban_amm.earliest_data_available` from the column.
  - OHLCV `backfill_note` switches from the per-asset `min(timestamp)` interim
    to the stored value (or keep per-asset min if a per-asset granularity is
    wanted — decide at impl).

## Acceptance Criteria

- [ ] `backfill_progress.earliest_data_available` column exists (init.sql).
- [ ] Both backfill push steps populate/lower it; integration test asserts it
      reflects the oldest landed candle.
- [ ] `/backfill/status` exposes it per stream (no live min scan).
- [ ] 0040 OHLCV `backfill_note` reads the stored value; interim min() removed.
