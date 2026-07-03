---
id: "0080"
title: "Verify Aquarius concentrated-liquidity pool swap-event shape against AquariusPoolExtractor"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0079", "0018"]
tags: [layer-research, priority-medium, effort-small, amm, aquarius, pool-registry, extractor, decode]
links:
  - "../active/0079_FEATURE_pool-registry-seed-from-soroswap-api.md"
history:
  - date: 2026-07-03
    status: backlog
    who: okarcz
    note: >
      Spawned from 0079: the live Soroswap API reports 20 Aquarius pools with
      poolType 'concentrated' (alongside 269 xyk + 41 stable). The 0079 seeder
      deliberately holds these back because AquariusPoolExtractor is documented
      only for constant-product + stableswap (inline-token `trade` event); a
      concentrated-liquidity pool may emit a different shape → mis-decoded prices.
---

# Verify Aquarius concentrated-pool swap-event shape

## Summary

Determine whether the 20 mainnet Aquarius `concentrated` pools emit the same
inline-token `trade` event that `AquariusPoolExtractor` already decodes. If they
do, they can be added to the `pool_registry` seed (0079); if not, document the
divergence and scope an extractor variant before seeding them.

## Context

- 0079's API seeder classifies venue-aware: Aquarius `xyk`+`stable` are seeded
  (extractor doc confirms both use the inline-token `trade` shape), but
  `concentrated` (20 pools) is skipped to avoid seeding a pool whose swaps might
  mis-decode into wrong OHLCV. Unseeded, their live swaps go to
  `unresolved_pools` (no data) rather than producing bad data.
- The extractor lives at `packages/aquarius-extractor/src/lib.rs`; its header
  documents constant-product / stableswap only. Reference decode methodology:
  task 0018 (per-AMM swap event shapes).

## Implementation

1. Pick a real `concentrated` Aquarius pool (from the API list) and dump a real
   swap tx's events (same method as 0018 — `dump-swap-events --contract … --tx …
   --show-xdr --pretty` against a Galexie range containing the tx).
2. Compare the event shape (topics + data) to the `trade` event
   `AquariusPoolExtractor::extract_one` expects (inline `sold_token` /
   `bought_token` addresses + amounts).
3. If identical → widen the 0079 seeder to include `concentrated` (`classify_pool_type`
   for `aquarius` adds `"concentrated"`), add a regression sample, re-seed.
4. If it diverges → document the shape, keep it excluded, and spawn a task to add
   a concentrated-pool extractor variant.

## Acceptance Criteria

- [ ] One real Aquarius concentrated-pool swap decoded + archived as evidence.
- [ ] Decision recorded: same shape (→ include in seed) or divergent (→ exclude +
      extractor-variant task).
- [ ] If include: 0079 seeder updated; the 20 pools land in `pool_registry`.
