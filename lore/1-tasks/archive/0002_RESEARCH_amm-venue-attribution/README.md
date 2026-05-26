---
id: "0002"
title: "Attribute observed AMM contract IDs to venues (Soroswap / Aquarius / Phoenix)"
type: RESEARCH
status: completed
related_adr: []
related_tasks: ["0001"]
tags: [layer-research, priority-medium, effort-small, soroban, amm]
links:
  - "../0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
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
  - date: 2026-05-08
    status: completed
    who: claude
    note: >
      All 5 acceptance criteria met. 5 R-notes / 1 Q-note / 1 S-note;
      ~960 lines of research. 5/5 directly observable canonical addresses
      verified against 0001 event evidence; 9 of 11 Phoenix pools match
      String("swap") emitters in the 4-day sample. Topic-kind correction
      surfaced (Phoenix uses String, not Symbol) — load-bearing for
      indexer §7. Spawned 0005 (unknown emitters), expanded 0003 (schema
      doc updates), superseded 0004 (Phoenix detection answered).
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

Per `../0001_RESEARCH_dump-amm-swap-events/notes/S-amm-trades-schema-§11-1-resolved.md`,
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

| Topic | Attributed to | Notes |
|---|---|---|
| `Symbol("swap")` `CBQDHNBFBZYE...` (11,947) | Aquarius router | Co-emits `add_pool` (factory-style) and `config_rewards` |
| `Symbol("swap")` `CCR2CH4G...` (2,706) + `CDMIM23W...` (2,480) | **unknown** | router-style emission; not Aquarius / Phoenix / Soroswap |
| `Symbol("swap")` long tail (32 contracts) | mostly unknown | unattributed for now |
| `String("swap")` × 9 distinct emitters (5,704 events) | Phoenix XYK pools | 9 of 11 known Phoenix pools — match 100% |
| `Symbol("trade")` × 29 | Aquarius constant-product pools | venue-distinctive shape |
| `String("SoroswapPair")` × 79 | Soroswap pools | factory-derivable |
| `String("SoroswapRouter")` / `String("SoroswapAggregator")` | Soroswap router / aggregator | direct registry match |

**Verified 2026-05-08:** registry addresses cross-checked against task
0001's event evidence. 5/5 directly observable canonical addresses match
emitter `contract_id`s exactly. 9 of 11 Phoenix pool addresses match
`String("swap")` emitters in the 4-day sample (the other 2 had zero
volume). See `notes/S-venue-attribution-mapping.md` §"Cross-check vs
0001 sample".

**Surprise (load-bearing):** Phoenix XYK pools emit **8 separate events
per swap** (6 for stable pools) using `topics[0] = String("swap")`
(NOT `Symbol("swap")` — corrected from initial source-code reading;
on-chain bytes are authoritative). The indexer must group by
`(tx_hash, op_index, contract_id)` to reconstruct one trade row.
Filter dispatch is therefore on `(topics[0].kind, topics[0].value)`:
`Symbol("swap")` → Aquarius decoder, `String("swap")` → Phoenix decoder.

## Implementation Notes

- 7 files in `notes/`: 1 question note, 3 per-venue research notes,
  1 synthesis note (post cross-check), plus task README. ~960 lines
  total.
- Three parallel research forks (Soroswap / Aquarius / Phoenix), each
  finding canonical addresses from public docs/GitHub and
  cross-referencing against task 0001's top emitters.
- Cross-check pass: re-ran `dump-swap-events` against the wider sample
  to verify registry addresses match observed `contract_id`s and to
  enumerate the full top-35 `Symbol("swap")` emitter list. Found
  9/11 Phoenix pools as `String("swap")` emitters in the 4-day window.
- No code changes. Task is purely research artefacts.

## Design Decisions

### From Plan

1. **Registries + spot-check** approach (vs full attribution or
   factory-event scan). Chosen with the user before research started:
   optimises for known-good attribution + a clear unknown bucket
   without trying to attribute the long tail.
2. **Per-venue R-notes + single S-note**: parallel research forks each
   write their own R-note; a synthesis note pulls them together.
   Standard lore RESEARCH-task layout.

### Emerged

3. **Re-ran `dump-swap-events` for cross-check**: the user asked to
   verify registry addresses against actual emitter `contract_id`s.
   Required a fresh data extraction step not in the original plan.
   Worth it — surfaced the topic-kind correction (item 4) which
   would otherwise have been baked into the schema doc as a wrong
   premise.
4. **Topic-kind correction (Symbol → String for Phoenix)**: cross-check
   revealed Phoenix pools emit `ScVal::String("swap")`, not
   `ScVal::Symbol("swap")` as the Phoenix fork's source-code reading
   claimed. Cause: Rust source uses `&str` tuple `("swap","sender")`
   which `IntoVal` compiles to `String`. The on-chain bytes are
   authoritative. Updated R-phoenix-registry.md, S-note, README;
   propagated to backlog 0003 acceptance criteria.
5. **R-swap-topic-shapes.md (task 0001) was wrong about "44 distinct
   `Symbol("swap")` emitters"**: it grouped on the histogram's
   normalised `topic_0` string and silently mixed Symbol with String.
   Correct split is 35 + 9. Documented as a correction in 0002's
   S-note rather than editing the archived 0001 note.
6. **ScVal-kind-based dispatch as schema design simplification**: the
   topic-kind correction unexpectedly enabled distinguishing Phoenix
   from Aquarius without a contract registry — the indexer can branch
   on `topics[0].kind`. Documented as a load-bearing finding for §7.

## Issues Encountered

- **Phoenix source-code reading vs on-chain bytes**: the Phoenix fork
  read `phoenix-contracts/contracts/pool/src/contract.rs` and concluded
  the topic kind was `Symbol`. The actual deployed WASM emits `String`.
  Lesson: when source code uses `&str` literals in event-publish tuples,
  trust the wire format, not the type-name in the source. Always
  verify the ScVal kind empirically against a real event.
- **R-swap-topic-shapes.md ranking conflated topic kinds**: the original
  parent task's wider-sample table ranked emitters by `topic_0` after
  normalisation, hiding the Symbol/String split. This is why "44
  Symbol(swap) emitters" was wrong. Detection only happened during
  this task's cross-check. Fix: 0002's synthesis explicitly documents
  the split; future histograms should retain the ScVal-kind to avoid
  the same trap.
- **Phoenix factory + multihop not directly observed**: factory events
  are rare (only on pool creation) and the multihop didn't emit
  `topic_0=swap` in the 4-day window. Attribution rests on
  second-order verification (pools attested, factory address from
  source-of-truth file). Acceptable for this task; flagged in the
  S-note in case a stricter on-chain proof is needed later.

## Future Work

All future work has been spawned as backlog tasks (no prose-only
TODOs):

- **0005 RESEARCH** (spawned) — identify the two unknown
  `Symbol("swap")` emitters (`CCR2CH4G...`, `CDMIM23W...`). Likely a
  fourth Soroban DEX or stale internal contract; ~5,200 events / 4 days.
- **0003 DOCS** (scope expanded) — schema doc updates now cover the
  four-decoder reality, ScVal-kind dispatch, Phoenix multi-event
  grouping, Soroswap two-topic filter, and per-venue factory
  enumeration. Acceptance criteria grew from 3 to 9.
- **0004 RESEARCH** (superseded by 0002, archived) — Phoenix detection
  answered. Phoenix is not low-volume; it emits `String("swap")` from
  9 attested mainnet pools, 5,704 events in the 4-day window.
