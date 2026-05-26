---
id: "0032"
title: "Capture Phoenix stable-pool first mainnet observation (WASM hash + 6-event decode)"
type: RESEARCH
status: completed
related_adr: ["0001"]
related_tasks: ["0018", "0034", "0035", "0036"]
tags: [layer-research, priority-low, effort-small, phoenix, stable-pool, schema-validation, negative-result]
links:
  - "../../archive/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md"
  - "../../archive/0002_RESEARCH_amm-venue-attribution/notes/R-phoenix-registry.md"
  - "notes/R-phoenix-xyk-pool-interface.md"
  - "notes/S-no-stable-pool-deployed.md"
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
  - date: 2026-05-15
    status: completed
    who: oski
    note: >
      Closed as negative-result synthesis. Surveyed all 11 Phoenix
      factory pools via query_pools(); zero stable pools deployed.
      Side-finding: two distinct XYK WASMs in production (367ab414...
      x10, 13b158655e... x1, the PHO/USDC pool) with identical
      Soroban interface and meta strings. 3 follow-ups spawned:
      0034 (consumer multi-WASM tolerance, priority-medium),
      0035 (periodic re-survey, priority-low),
      0036 (237-byte WASM delta investigation, priority-low).
      Artifacts: 1 R-note, 1 S-note, 1 evidence file.
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

## Implementation Notes

What was actually done (vs the original plan):

- Inspected the Phoenix factory's Soroban interface; found
  `query_pools() -> Vec<Address>` exposed as a view function.
- Invoked `query_pools` on mainnet → 11 pool addresses returned.
- Fetched each pool's WASM via `stellar contract fetch` and SHA-256'd
  it locally. Grouped: 10 pools share one hash, 1 pool has a
  different hash.
- Classified the non-matching pool by calling `query_config`,
  `query_pool_info`, `query_version`, and resolving its token
  contracts' `symbol()` / `name()`. Result: PHO/USDC, still XYK.
- Wrote 1 R-note (XYK reference + factory inventory), 1 S-note
  (negative-result synthesis + 3 consumer implications), 1 evidence
  file (reproducible hash listing).
- Spawned 3 backlog tasks: 0034, 0035, 0036.

## Design Decisions

### From Plan

1. **Survey the Phoenix factory before assuming a stable pool
   exists.** The task spec called this out as the gating step.

### Emerged

2. **Used `query_pools()` view function instead of scanning
   `("create", "liquidity_pool")` events** as the task originally
   prescribed. The factory exposes a direct getter for the deployed
   pool set — one RPC call vs scanning thousands of ledgers. Same
   result, much cheaper.
3. **Recorded the XYK reference R-note before any stable pool was
   found.** The original plan implied notes would be created only
   when a stable pool was observed. Recording the XYK baseline
   up-front gave the survey a "known negative" filter and turned out
   to be exactly what was needed when the negative result emerged.
4. **Did not edit task 0018's archived G-note.** Acceptance criterion
   3 ("Consumer's stable-pool decoder spec updated with observation")
   was reinterpreted as "status documented" via the S-note here,
   because there is no observation to fold into 0018's spec. Modifying
   an already-archived task's content felt wrong; keeping the negative
   finding in 0032's directory keeps the chain of custody clear and
   greppable.
5. **Recommended `pool_type + event_count` as the consumer's XYK-vs-
   stable discriminator** instead of WASM-hash matching. Motivated by
   the side-finding of two XYK WASMs in production — a hash-based
   classifier would silently drop the PHO/USDC pool today. See
   [S-note §So what?](notes/S-no-stable-pool-deployed.md).
6. **Did not run `dump-swap-events` on the PHO/USDC pool to confirm
   the 8-event grouping holds for the second XYK build.** That
   verification is real but it's defensive — separated into task
   0036 to keep this task's scope on the original "stable pool"
   question.

## Issues Encountered

- **Mainnet RPC required BYO endpoint.** `stellar network ls` lists
  `mainnet` but `stellar contract fetch ... --network mainnet` errors
  with "Invalid URL Bring Your Own". Worked around by passing
  `--rpc-url https://mainnet.sorobanrpc.com` and the network
  passphrase explicitly. Not a regression; just the current Stellar
  CLI policy for paid/public RPC choice.

## Future Work

Spawned as backlog tasks (see frontmatter `related_tasks`):

- **[0034](../../backlog/0034_FEATURE_consumer-multi-xyk-wasm-tolerance.md)**
  — Consumer must tolerate ≥2 XYK WASM builds (priority-medium).
- **[0035](../../backlog/0035_RESEARCH_phoenix-factory-periodic-resurvey.md)**
  — Periodic Phoenix factory re-survey; replaces 0032 as ongoing
  concern (priority-low).
- **[0036](../../backlog/0036_RESEARCH_phoenix-xyk-237b-wasm-delta.md)**
  — Investigate 237-byte XYK WASM delta; confirm 8-event grouping
  holds on second build (priority-low).

## Notes

Low priority — task 0018's spec already covers the case from
source; this task was the confirmation step. The stable-pool
decoder remains source-only; this is now an explicit decision
rather than a deferred TODO.
