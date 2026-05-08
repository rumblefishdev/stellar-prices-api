---
id: "0005"
title: "Identify unknown Symbol(\"swap\") emitters CCR2CH4G... and CDMIM23W..."
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0002"]
tags: [priority-medium, effort-small, soroban, amm, venue-attribution]
links:
  - "../archive/0002_RESEARCH_amm-venue-attribution/notes/S-venue-attribution-mapping.md"
  - "../archive/0002_RESEARCH_amm-venue-attribution/notes/R-aquarius-registry.md"
history:
  - date: 2026-05-08
    status: backlog
    who: claude
    note: "Spawned from 0002 future work."
---

# Identify unknown `Symbol("swap")` emitters

## Summary

Task 0002 attributed every observed AMM contract emitter except two
high-volume `Symbol("swap")` emitters that don't match Aquarius router,
Soroswap, or Phoenix. Identify them so the BE indexer's per-venue
mapping is complete.

## Context

From 0002's cross-check (`S-venue-attribution-mapping.md` §"Re-attribution
of `Symbol("swap")` emitters after correction"):

| Events | Contract | Status |
|---:|---|---|
| 2,706 | `CCR2CH4GQVCZHG7CHFVMNANCK45CU5DVKXZIIITDZQAU3CEJZ7RQH2MQ` | unknown |
| 2,480 | `CDMIM23WOUL5CZBKX3GOA3V5R5AMVIMTCP52KCDQORWELAPLJ27WZCHL` | unknown |
| 335 | `CCXRRORTOXXP53HEKJ6RCG7CDRWZAJHIS4N7PDL32PUNMNN7VWPJVQWS` | unknown |
| 229 | `CAUF4DFYSX52L2KJ4J7OFW3WDQMEUDVXNB7PG5VIC4VVOA3BCLWXDO2E` | unknown |
| (+ 30 more in long tail) | | |

These contracts emit `Symbol("swap")` like a router/aggregator (high
per-contract volume, single contract emitting many events) but:

- They are not the Aquarius router (`CBQDHNBFBZYE...`) — different
  WASM and creator (Aquarius fork verified).
- They are not in `soroswap/core` or `soroswap/aggregator`
  `mainnet.contracts.json` (Soroswap fork verified).
- They are not in the Phoenix factory's deployed-pool list (Phoenix
  fork verified).

**Hypotheses:**
1. Aquarius has multiple routers (one per pool type:
   constant_product / stable / concentrated). These could be the
   stable-pool router and concentrated-pool router. Check:
   `AquaToken/soroban-amm` repo for non-router contracts that emit
   `Symbol("swap")`.
2. A fourth Soroban DEX outside the {Soroswap, Aquarius, Phoenix}
   target set (DeFindex? Blend? AtomicSwapV2 from the histogram?).
3. Stale or internal/test deployments.

If (1), the indexer needs to extend Aquarius pool-type dispatch. If (2),
the BE team needs a policy decision on whether to index it. If (3),
they should be filtered out of the registry.

## Implementation

- Query stellar.expert API for `CCR2CH4G...`, `CDMIM23W...`,
  `CCXRRORT...`, `CAUF4DFY...` contract metadata: `package_name`,
  `repo`, `validation.status`, `creator`.
- If `repo` is set, follow it to identify the protocol.
- If `repo` is empty, inspect the WASM hash and search for that hash
  on stellar.expert (cross-references other contracts using the same
  WASM, which often clusters by protocol).
- Check the histogram in `R-swap-topic-shapes.md` for co-emitted topics
  by these contracts (e.g. `add_pool`, `config_rewards`,
  `update_reserves`) — co-emission patterns are a strong attribution
  signal.

## Acceptance Criteria

- [ ] `CCR2CH4G...` attributed (or marked "unknown / non-target" with
      WASM + creator evidence)
- [ ] `CDMIM23W...` attributed (or marked unknown with evidence)
- [ ] If either is an Aquarius variant, document the additional
      Aquarius pool-type and update
      `0002/notes/R-aquarius-registry.md`
- [ ] If both are non-target, document the policy decision (skip vs
      index as `venue: unknown`) for the BE team
