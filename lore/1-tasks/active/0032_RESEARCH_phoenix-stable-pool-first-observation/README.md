---
id: "0032"
title: "Capture Phoenix stable-pool first mainnet observation (WASM hash + 6-event decode)"
type: RESEARCH
status: active
related_adr: ["0001"]
related_tasks: ["0018"]
tags: [priority-low, effort-small, phoenix, stable-pool, schema-validation]
links:
  - "../../archive/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md"
  - "../../archive/0002_RESEARCH_amm-venue-attribution/notes/R-phoenix-registry.md"
  - "notes/R-phoenix-xyk-pool-interface.md"
  - "https://github.com/Phoenix-Protocol-Group/phoenix-contracts/blob/main/contracts/pool_stable/src/contract.rs"
history:
  - date: 2026-05-15
    status: backlog
    who: claude
    note: "Spawned from 0018 Appendix B item 3."
  - date: 2026-05-15
    status: active
    who: oski
    note: "Activated to start research."
---

# Phoenix stable-pool first observation

## Summary

Task 0018 §3 documents the Phoenix XYK pool 8-event swap grouping
and notes that the stable-pool variant emits 6 events (no
`actual received amount`, no `referral_fee_amount`). No mainnet
stable-pool address is currently known per archive task 0002
`R-phoenix-registry.md` — the upgrade script's `pools=()` array
only carries XYK addresses. This task captures the stable-pool
shape from a real mainnet event the first time one appears.

## Context

Until a stable pool is deployed and emits swap events, the
consumer's stable-pool decoder is source-only. The first
observation:

- Confirms the 6-event grouping with concrete values.
- Pins the stable-pool WASM hash so the consumer's venue lookup
  table can carry it.
- Verifies that the emission order in
  `contracts/pool_stable/src/contract.rs:1182-1189` matches the
  XYK source (it should — the only delta is the two omitted
  events).

## Implementation

1. Periodically (or on first failure of the prices-api consumer
   when it sees a 6-event `String("swap")` grouping it cannot
   pivot), scan the Phoenix factory
   (`CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI`)
   for `("create", "liquidity_pool")` events whose deployed
   contract carries the stable-pool WASM hash.
2. Once one is found, run the same `dump-swap-events
   --contract <stable_pool_id> --tx <hash> --show-xdr --pretty`
   flow as Phoenix XYK, save to
   `notes/evidence/phoenix_stable_pool_swap_decode.json`.
3. Update task 0018's G-note Appendix A extractor list with
   confirmed `PhoenixStablePoolExtractor` parameters (6-event
   group, field order, field types).

## Acceptance Criteria

- [ ] At least one mainnet Phoenix stable-pool deployment
      identified (WASM hash recorded)
      → **Negative result, 2026-05-15**: see
      [S-no-stable-pool-deployed.md](notes/S-no-stable-pool-deployed.md).
- [ ] One real stable-pool swap event grouping decoded and
      archived as evidence
      → Cannot satisfy; no stable pool exists to decode from.
- [x] Consumer's stable-pool decoder spec status documented
      (note: rephrased from "updated with observation" → "status
      documented" since no observation was possible).

## Findings (2026-05-15)

The Phoenix mainnet factory contains **11 pools, zero stable**.
Two distinct XYK WASM builds were found in production
(`167ab414...506c` ×10, `13b158655e...f2ca` ×1). The full inventory
is in
[notes/evidence/phoenix_pool_inventory_2026-05-15.txt](notes/evidence/phoenix_pool_inventory_2026-05-15.txt);
the analysis and consumer implications are in
[notes/S-no-stable-pool-deployed.md](notes/S-no-stable-pool-deployed.md).
Reference XYK interface and WASM hash are recorded in
[notes/R-phoenix-xyk-pool-interface.md](notes/R-phoenix-xyk-pool-interface.md).

## Notes

Low priority — task 0018's spec already covers the case from
source; this task is the confirmation step. Should re-prioritize
if the prices-api consumer fails on a 6-event grouping in the
wild before this lands.
