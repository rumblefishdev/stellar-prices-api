---
id: "0290"
title: "Index the Uniswap-v3-style concentrated-liquidity venue (factory CD3KRKGD…) — ~8.5k swaps a month we never see"
type: FEATURE
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0285", "0286", "0282"]
tags: [layer-indexing, priority-medium, effort-medium, amm, ingestion, data-correctness]
links:
  - "../active/0285_RESEARCH_pool-registry-does-not-match-what-is-trading/notes/S-classification-2026-09-17.md"
history:
  - date: 2026-09-17
    status: backlog
    who: okarcz
    note: >
      Spawned from 0285's classification. A pool family with its own factory
      emits Uniswap-v3-shaped swaps (16,933 over 2026-07-16 → 09-15) and no
      extractor of ours recognises it.
  - date: 2026-09-18
    status: active
    who: okarcz
    note: >
      Activated. Picked up because 0291 is blocked on 0286's schema and 0282 /
      0285 both wait on the 2026-09-19 measurement; this is the last known
      missing-trades gap from 0285 and is independent of the deploy freeze
      (research + extractor + tests; nothing to deploy until 0286 phase 1
      lands). Start by naming the venue and decoding its swap event.
---

# Index the Uniswap-v3-style concentrated-liquidity venue

## Summary

Since ~2026-04 (ledger ~61.49M) a factory at
`CD3KRKGDRVWPXVB3VXLUMQKMX6XZ6Q2H334IVZD4XXNAMKSRVQL5GLYF` (wasm `9F94C577`)
deploys concentrated-liquidity pools (wasm `003710B3`, newer `95A8E001`). Over
the live era they emitted **16,933 swaps** that reach no candle. See
[[0285]]'s note for the evidence.

## Context

- Pool constructor: `(factory, token0, token1, fee, tick_spacing,
  flash_executor, admin)`; factory emits `pool_created {fee, pool_address,
  sender, …}`.
- Pool event: `topics = [Symbol("swap")]`, `data = {amount0, amount1,
  liquidity, recipient, sender, sqrt_price_x96, tick}` — signed amounts, so
  direction comes from the sign.
- Three routers wrap these pools (`4EDD745F`, `9677074A`) and must **not** be
  indexed: every one of their swap txs also holds a pool swap.

## Implementation

- **Identify the venue by name** first — it decides the `source` label.
- Learn pools from `pool_created` (like `learn_factory` does for the other
  venues) and persist them ([[0291]]).
- Token pair comes from the pool's constructor/storage, not from the swap event
  — decide how to resolve it (factory event carries it? contract storage?).
- Map the swap to a tick using the signed amounts; decide whether price comes
  from amounts or from `sqrt_price_x96`, in line with ADR 0287's price-forming
  rule ([[0286]]).
- Backfill from the family's first pool, in the same run as [[0286]] phase 3
  if the timing allows.

## Acceptance Criteria

- [ ] The venue is named and has a `source` label.
- [ ] Its pools are learned from the factory and survive a cold start.
- [ ] Live candles for it match a raw count of its pool `swap` events for a full
      day.
- [ ] Its routers stay unindexed (a test pins it).
- [ ] History from the first pool is backfilled, or explicitly deferred with a
      reason.
