---
id: "0069"
title: "Asset Discovery — Soroswap/Aquarius pool-registry maintenance"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0039", "0054", "0037"]
tags: ["phase-future", "effort-small", "priority-low", "discovery", "amm"]
links: []
history:
  - date: 2026-06-26
    status: backlog
    who: claude
    note: "Spawned from 0039 Step 5 future work — the additive pool-registry piece was not delivered in PR #56 (oracle/supply/cleanup/MV + discovery-via-0054 shipped instead)."
---

# Asset Discovery — Soroswap/Aquarius pool-registry maintenance

## Summary

Extend the Asset Discovery worker (shipped by task 0054, reused by 0039) with
Soroswap / Aquarius pool-pair registry maintenance. Pool registries tell the
Ledger Processor (0038) which AMM contracts to extract swaps from; without
periodic maintenance, newly-created pools on those protocols are missed on the
live path.

## Context

0039 Step 5 (Q#2 → Option A) reused 0054's discovery binary as-is and only
*planned* to add this pool-registry maintenance on top. The 0039 implementation
(PR #56) delivered the oracle, supply, cleanup workers and the `current_prices`
MV, plus discovery via 0054 — but the pool-registry extension was not built. It
is carried out of 0039 as this standalone follow-up. Related: the AMM
historical-pool-discovery gap (factory-registry seeding) and the 0037 Phoenix
pool-registry surface.

## Implementation

- Add periodic Soroswap / Aquarius pool-pair discovery to the discovery worker
  (or a sibling step on the same `rate(1h)` rule).
- Persist the discovered pools to the registry the Ledger Processor reads to
  decide which contracts to extract swaps from.
- Coordinate the registry hand-off with the 0037 Phoenix pool-registry surface
  so all three AMM protocols share one registry contract/shape.

## Acceptance Criteria

- [ ] New Soroswap/Aquarius pools created in-window are added to the pool
      registry within one discovery cycle.
- [ ] The Ledger Processor picks up the maintained registry (extracts swaps
      from the newly-registered pools).
- [ ] Registry shape is consistent with the 0037 Phoenix surface.
