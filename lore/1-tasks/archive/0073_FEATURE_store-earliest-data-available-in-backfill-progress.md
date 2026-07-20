---
id: "0073"
title: "Store earliest_data_available on backfill_progress + populate it in the backfill push steps"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0040", "0051", "0053", "0106", "0108"]
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
  - date: 2026-07-02
    status: backlog
    who: claude
    note: >
      **Producer half folded into 0053.** The backfill is the natural producer
      of `earliest_data_available` (it is the code that "first lands a candle"),
      so the schema column + writer population moved into 0053's Step 4 dual
      progress-row work rather than being a separate follow-up. 0053 added the
      idempotent `ALTER TABLE prices.backfill_progress ADD COLUMN IF NOT EXISTS
      earliest_data_available Nullable(DateTime)` to init.sql and populates it as
      it lands older candles. **This task now owns only the 0040 read-side
      upgrade**: swap `/backfill/status` + the OHLCV `backfill_note` from the
      interim live per-asset `min(timestamp)` to the stored column.
  - date: 2026-07-20
    status: completed
    who: okarcz
    note: >
      **DONE — completed under task 0106** (PR #125 impl, PR #126 archive), which
      shipped this task's remaining read-side scope without ever closing this ID.
      Found and verified during the 0108 post-M1 grooming sweep.
      Evidence: `packages/prices-api/src/backfill/queries_ch.rs:30` selects the
      stored `earliest_data_available` from `backfill_progress FINAL`; no
      `min(timestamp)` remains anywhere in the backfill module. Carried on
      `ProgressRow` (queries_ch.rs:15), mapped into both DTO variants
      (handlers.rs:35, :42), declared at dto.rs:42, :53. The OHLCV
      `backfill_note` no longer runs a per-asset min() either — it derives its
      "from" date from the first already-fetched candle
      (`assets/handlers.rs:344-360`), so the interim scan is gone as required.
      Producer half had already landed in 0053. Archived, no work remaining.
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

- ~~**Schema**: add `earliest_data_available Nullable(DateTime)` to
  `prices.backfill_progress`~~ — **done in 0053** (idempotent `ALTER … ADD
  COLUMN IF NOT EXISTS` in init.sql).
- ~~**Writers**: the backfill lowers `earliest_data_available` toward older
  data each time it lands an older candle~~ — **done in 0053** (the combined
  single-pass backfill is the sole producer; the separate `sdex-cloud-push` /
  `soroban-amm-backfill` writers this task assumed were superseded by 0053).
- **0040 read API upgrade** (the remaining scope):
  - `/backfill/status` returns `sdex.earliest_data_available` /
    `soroban_amm.earliest_data_available` from the column.
  - OHLCV `backfill_note` switches from the per-asset `min(timestamp)` interim
    to the stored value (or keep per-asset min if a per-asset granularity is
    wanted — decide at impl).

## Acceptance Criteria

- [x] `backfill_progress.earliest_data_available` column exists (init.sql). — **done in 0053**
- [ ] The backfill populates/lowers it; integration test asserts it reflects
      the oldest landed candle. — **producer done in 0053; assertion tracked there**
- [x] `/backfill/status` exposes it per stream (no live min scan). — **done in 0106**
- [x] 0040 OHLCV `backfill_note` reads the stored value; interim min() removed. — **done in 0106**
      (resolved slightly differently than specced: the note reuses the first
      already-fetched candle rather than reading the stored column, which meets the
      intent — no extra scan on the hot path — at per-asset granularity.)
