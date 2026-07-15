---
id: "0096"
title: "Backfill must preload pool_registry — Soroswap historical coverage gap"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0078", "0079", "0053", "0088", "0090"]
tags: [layer-indexing, priority-high, effort-medium, milestone-M1, backfill, clickhouse, amm, soroswap, pool-registry, data-loss]
milestone: 1
links:
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-07-15
    status: backlog
    who: okarcz
    note: >
      Surfaced during the 0090 pre-roll: prod coarse tables have ZERO soroswap
      candles despite the registry being seeded. The backfill-side twin of the
      live-only 0078 fix.
  - date: 2026-07-15
    status: active
    who: okarcz
    note: >
      Promoted to active to begin implementation (backfill pool_registry
      preload). Branch cut off develop.
---

# Backfill must preload pool_registry — Soroswap historical coverage gap

## Summary

The historical backfill produces **zero `soroswap` candles**, so the coarse
tables (and `price_ohlcv_1m`) have no Soroswap prices at all. Aquarius and
Phoenix are present; Soroswap is entirely missing. This is the **backfill-side
twin of task 0078** (which fixed the *live* processor to preload
`prices.pool_registry`) — the backfill has no equivalent preload.

## Evidence (prod ch-prod-01, 2026-07-15)

- `price_ohlcv_1m` sources: `sdex` 530M, `aquarius` 2.14M, `phoenix` 156k,
  **`soroswap` 0**.
- `pool_registry FINAL` by venue: `aquarius` 488, `phoenix` 19, **`soroswap`
  221** — so the registry IS seeded (task 0079).
- `unresolved_pools FINAL`: 138 pools / 7,887 swaps, all `source='backfill'`;
  **`unresolved_that_are_soroswap = 0`**.

So Soroswap swaps are neither resolved (0 candles) NOR recorded as unresolved —
they are **invisible to the backfill**, not merely unattributed.

## Likely root cause (to confirm in code)

A Soroswap swap event **omits the pair tokens** (per the `pool_registry` schema
note), so it can only become a candle if the pool is known at processing time.
The backfill (`sdex-backfill`, combined mode, task 0053) relies on **in-window
forward-discovery** of pools from factory events; it does **not load the
pre-seeded `pool_registry`** (only the live processor does, via 0078). So
Soroswap pools created before the backfill window — or whose factory/swap events
the extractor doesn't recognize — are never known, and their swaps are dropped
without even an `unresolved_pools` row. Aquarius/Phoenix survive because their
events carry enough to resolve via forward-discovery.

To confirm: check whether `sdex-backfill` loads `pool_registry` at start, and
whether the Soroswap swap-event shape (0018) is actually reached in the backfill
dispatch. That the swaps produce NO `unresolved_pools` row points at a
recognition/dispatch gap, not just a missing-tokens gap.

## Implementation (sketch)

- Make the backfill **preload `prices.pool_registry`** into its in-memory
  registry at start (mirror the 0078 live fix), so pre-existing Soroswap pools
  resolve.
- Verify Soroswap swap events are recognized and dispatched in the backfill path
  (not only live); if a genuinely-unknown pool is hit it must land in
  `unresolved_pools` (visible), never be silently dropped.
- Re-run the Soroswap-affected range (bounded, per the 0090 runbook: disable
  cleanup → backfill → pre-roll → re-enable) to fill the missing candles.

## Acceptance Criteria

- [ ] Root cause confirmed in code (registry-not-loaded vs swap-shape not
      dispatched vs both).
- [ ] Backfill preloads `pool_registry`; unknown pools go to `unresolved_pools`
      (never silently dropped).
- [ ] Re-run yields non-zero `soroswap` candles in `price_ohlcv_1m` + coarse
      tables for the backfilled range (verified per-source).

## Notes

- Does NOT block 0090 (pre-roll/cleanup are correct for the data that exists) —
  this is an upstream backfill-coverage gap. Related coverage memory:
  [[amm-historical-pool-discovery-gap]], [[amm-live-pool-registry-preload-gap]].
- Likely also affects the live path for any Soroswap pool not covered by 0078's
  preload; verify once live is unfrozen ([[proto27-xdr26-live-freeze]]).
