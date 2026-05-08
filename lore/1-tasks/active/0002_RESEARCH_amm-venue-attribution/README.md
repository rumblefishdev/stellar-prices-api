---
id: "0002"
title: "Attribute observed AMM contract IDs to venues (Soroswap / Aquarius / Phoenix)"
type: RESEARCH
status: active
related_adr: []
related_tasks: ["0001"]
tags: [priority-medium, effort-small, soroban, amm]
links:
  - "../../archive/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
  - "../../../../docs/database-schema/amm-trades-schema.md"
history:
  - date: 2026-05-07
    status: backlog
    who: okarcz
    note: "Spawned from 0001 future work."
  - date: 2026-05-08
    status: active
    who: okarcz
    note: "Activated for implementation."
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

Per `../../archive/0001_RESEARCH_dump-amm-swap-events/notes/S-amm-trades-schema-§11-1-resolved.md`,
the BE indexer's per-venue mapping is load-bearing. Without venue
attribution we can't populate `prices_amm_trades.venue` correctly.

## Approach (decided 2026-05-08)

**Registries + spot-check** — find canonical factory/router addresses for
each venue from public docs/GitHub, then spot-check the top emitters per
class (from `R-swap-topic-shapes.md`) against them. Cross-reference with
Stellar Expert labels for additional confirmation. Long-tail
unattributable contracts go into an "unknown" bucket with a follow-up
backlog task if material.

## Implementation

- Per-venue research notes (one R-note each):
  - `notes/R-soroswap-registry.md` — Soroswap factory/router/aggregator
  - `notes/R-aquarius-registry.md` — Aquarius factory/router/pools
  - `notes/R-phoenix-registry.md` — Phoenix factory/pools
- Cross-check observed top emitters from
  `R-swap-topic-shapes.md` against gathered addresses.
- Synthesize `notes/S-venue-attribution-mapping.md` with the
  contract → venue mapping table + unknown bucket.

## Acceptance Criteria

- [x] `swap`-emitter `CBQDHNBFBZYE...` attributed → **Aquarius router**
      (stellar.expert label + canonical address documented in
      `docs.aqua.network`).
- [x] At least one of the 29 `trade`-emitters attributed → all 29 are
      **Aquarius constant-product pools** (sample `CA6PUJLBYK...` matches
      canonical pool WASM hash; topic shape is venue-distinctive).
- [x] Phoenix factory/router documented:
      factory `CB4SVAWJA6TSRNOJZ7W2AWFW46D5VR4ZMFZKDIKXEINZCZEGZCJZCKMI`,
      multihop `CCLZRD4E72T7JCZCN3P7KNPYNXFYKQCL64ECLX7WP5GNVYPYJGU2IO2G`.
- [x] Top 5 `Symbol("swap")` emitters mapped — 3 attributed
      (1 Aquarius + 2 Phoenix), 2 deferred to follow-up as **unknown**
      with evidence. See `S-venue-attribution-mapping.md`.
- [x] `SoroswapPair` emitter cross-checked against Soroswap factory:
      sample pool deploys factory's pinned pair WASM hash
      (`18051456...0f73e`).

## Findings

See `notes/S-venue-attribution-mapping.md` for the full mapping and
indexer implications.

**Headline:**

| Topic / pattern | Attributed to | Notes |
|---|---|---|
| `Symbol("swap")` `CBQDHNBFBZYE...` | Aquarius router | Co-emits `add_pool` (factory-style) and `config_rewards` |
| `Symbol("swap")` `CBHCRSVX...` (4,128) | Phoenix XYK pool (XLM/USDC) | 1 of top 5 |
| `Symbol("swap")` `CBCZGGNO...` (440) | Phoenix XYK pool (XLM/PHO) | shares WASM with `CBHCRSVX...` |
| `Symbol("swap")` `CCR2CH4G...`, `CDMIM23W...` | **unknown** | not Aquarius / Phoenix / Soroswap; ~5,200 events combined |
| `Symbol("trade")` × 29 | Aquarius constant-product pools | venue-distinctive shape |
| `String("SoroswapPair")` × 79 | Soroswap pools | factory-derivable |

**Surprise (load-bearing):** Phoenix XYK pools emit **8 separate events
per swap** (6 for stable pools), all with `topic_0=Symbol("swap")` and a
field-name `topic_1`. The indexer must group by
`(tx_hash, op_index, contract_id)` to reconstruct one trade row.
Source: `phoenix-contracts/contracts/pool/src/contract.rs:1172-1185`.

## Future Work

- **0005 RESEARCH** — identify the two unknown `Symbol("swap")`
  emitters (`CCR2CH4G...`, `CDMIM23W...`). Likely a fourth Soroban DEX
  or stale internal contract; ~5,200 events / 4 days.
- **0003 DOCS** (existing backlog) — extend acceptance criteria to
  include Phoenix multi-event grouping rule and Soroswap two-topic
  filter pattern, in addition to the original §7/§11.1 update.
- **0004 RESEARCH** (existing backlog) — supersede; its questions are
  answered (Phoenix uses `Symbol("swap")` from pool contracts, hidden
  in the existing histogram, not a low-volume issue).
