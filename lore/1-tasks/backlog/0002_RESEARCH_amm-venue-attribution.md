---
id: "0002"
title: "Attribute observed AMM contract IDs to venues (Soroswap / Aquarius / Phoenix)"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0001"]
tags: [priority-medium, effort-small, soroban, amm]
links:
  - "../active/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
  - "../../../docs/database-schema/amm-trades-schema.md"
history:
  - date: 2026-05-07
    status: backlog
    who: okarcz
    note: "Spawned from 0001 future work."
---

# Attribute observed AMM contract IDs to venues

## Summary

Task 0001 identified one `swap`-emitter and 29 `trade`-emitters in a 3.5-day
mainnet window but could not attribute them to specific venues without an
external registry. Confirm which of these contracts belong to Soroswap,
Aquarius, and/or Phoenix by cross-referencing public registries
(Soroswap factory address, Aquarius router address, Phoenix factory)
and/or Stellar Expert.

## Context

Per `0001/notes/S-amm-trades-schema-§11-1-resolved.md`, the BE indexer's
per-venue mapping is load-bearing. Without venue attribution we can't
populate `prices_amm_trades.venue` correctly.

## Implementation

- Fetch Soroswap / Aquarius / Phoenix public docs to find:
  - Factory / router contract addresses
  - How to enumerate pool contracts (factory event scan vs hardcoded list)
- Cross-check the contract IDs in `0001/notes/R-swap-topic-shapes.md`
  against those addresses.
- Document the mapping (contract → venue) in a new note here.

## Acceptance Criteria

- [ ] `swap`-emitter `CBQDHNBFBZYE...` attributed to a venue (or marked
      "unknown / non-target" with evidence)
- [ ] At least one of the 29 `trade`-emitters attributed to a venue
- [ ] Phoenix factory/router address documented (even if no Phoenix
      events were observed in the 0001 sample)
