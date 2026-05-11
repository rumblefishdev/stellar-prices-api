---
id: "0003"
title: "Update amm-trades-schema.md §7 and §11.1 with empirical topic findings"
type: DOCS
status: completed
related_adr: []
related_tasks: ["0001", "0002", "0005"]
tags: [priority-medium, effort-small, schema-validation]
links:
  - "0001_RESEARCH_dump-amm-swap-events/notes/S-amm-trades-schema-§11-1-resolved.md"
  - "0002_RESEARCH_amm-venue-attribution/notes/S-venue-attribution-mapping.md"
  - "0005_RESEARCH_unknown-symbol-swap-emitters/notes/S-unknown-emitters-non-target.md"
  - "../../../docs/database-schema/amm-trades-schema.md"
history:
  - date: 2026-05-07
    status: backlog
    who: okarcz
    note: "Spawned from 0001 future work."
  - date: 2026-05-08
    status: backlog
    who: claude
    note: >
      Scope expanded after 0002. Original scope (two decoder shapes)
      was incomplete. Add Phoenix String("swap") multi-event grouping,
      Soroswap two-topic filter, ScVal-kind-based dispatch, and the
      contract → venue registry derived from canonical factories.
  - date: 2026-05-11
    status: active
    who: okarcz
    note: "Activated for implementation."
  - date: 2026-05-11
    status: completed
    who: claude
    note: >
      All 9 acceptance criteria met. §7 rewritten with four-decoder
      reality and ScVal-kind dispatch; added "Per-venue decoder
      reference", "Pool enumeration", and "Per-venue payload notes"
      subsections covering Phoenix multi-event grouping, Soroswap
      two-topic filter, Soroswap no-inline-tokens rule, and Phoenix
      fee shape. §11.1 + §11.2 marked resolved with lore archive
      references; §11.3 (decimals normalisation) remains open pending
      BE input. §4 "topics JSONB" rationale updated. No DDL changes.
---

# Update amm-trades-schema.md §7 and §11.1 with empirical findings

## Summary

Replace the hypothetical wording in §7 step 3 ("if any AMM uses a
different symbol the indexer's per-venue mapping handles it") and the
open question §11.1 with the empirical findings from tasks 0001 + 0002:
**four distinct decoder shapes are in use**, distinguished by
`(topics[0].kind, topics[0].value)` rather than by the normalised
topic_0 string alone, and Phoenix is event-multiplexed (8 events =
1 trade) so the filter alone is insufficient.

## Context

Lore 0001 / `S-amm-trades-schema-§11-1-resolved.md` documents the
original two decoder shapes (`Symbol("swap")` router-style and
`Symbol("trade")` Aquarius-pool).

Lore 0002 / `S-venue-attribution-mapping.md` extends this with:
- Soroswap two-topic shape:
  `[String("SoroswapPair") + Symbol("swap")]` with a uniswap-v2 Map
  payload — token addresses are NOT inline.
- Phoenix multi-event grouping:
  8 events per swap (6 for stable pools), all
  `[String("swap"), String(<field>)]` with scalar-per-event payloads;
  must group by `(tx_hash, op_index, contract_id)` to reconstruct
  one trade row.
- ScVal-kind-based dispatch:
  Phoenix vs Aquarius can be distinguished by `topics[0].kind`
  (`String` vs `Symbol`) — both with value `"swap"`.
- Canonical venue registry from public factories:
  Soroswap factory `CA4HEQTL...`, Aquarius router `CBQDHNBFBZYE...`,
  Phoenix factory `CB4SVAWJA6...`. Pool enumeration via factory
  events listed in the synthesis note.

## Implementation

- Update §7 step 3 with the four-decoder reality and the
  ScVal-kind-based dispatch.
- Add the multi-event grouping rule for Phoenix (group by
  `(tx_hash, op_index, contract_id)` for `topics[0]=String("swap")`).
- Add the two-topic filter rule for Soroswap (require
  `topics[1]=Symbol("swap")` to keep, drop on `sync` / `deposit` /
  `withdraw`).
- Replace §11.1 with the resolved finding + link to lore 0001 + 0002.
- Add a "Per-venue decoder reference" subsection: table of
  `(topics[0].kind, topics[0].value, topics[1]?, decoder, fee column
  source)` for all four shapes.
- Add a "Pool enumeration" subsection: per-venue factory address +
  pool-creation event topic + data shape.
- Phoenix `fee` semantics: pools don't emit `commission_amount` — only
  `spread_amount`. Document the chosen fill strategy (recommend:
  compute from `total_fee_bps × offer_amount`).
- Do NOT change the DDL; it is correct as-is.

## Acceptance Criteria

- [x] §7 step 3 reflects four-decoder reality and ScVal-kind-based
      dispatch (no longer hypothetical)
- [x] Phoenix multi-event grouping rule documented (8 events → 1 row,
      group key `(tx_hash, op_index, contract_id)`)
- [x] Soroswap two-topic filter rule documented (must check
      `topics[1]=Symbol("swap")` to drop sync/deposit/withdraw)
- [x] Soroswap "no inline tokens" rule documented (token addresses
      come from per-pool cache, populated at pool-discovery time)
- [x] Phoenix `fee` column fill strategy documented (as informational
      note; fee not stored in current schema)
- [x] §11.1 marked resolved with references to lore 0001 + 0002
      archive paths
- [x] "Per-venue decoder reference" table added (under §7)
- [x] "Pool enumeration" subsection added with factory addresses +
      pool-creation event topics (under §7)
- [x] No DDL changes (verified with `git diff`)

## Outcome

Schema doc `docs/database-schema/amm-trades-schema.md` updated:

- **§4 "What is deliberately omitted"** — `topics` JSONB rationale
  tightened: the `venue` column already encodes the decoder shape
  (`topics[0]` is no longer always `"swap"` once `Symbol("trade")` and
  `String("SoroswapPair")` are considered), but storing topics is
  still redundant.
- **§7 "Scope of the BE indexer's filter"** — rewrote step 3 to
  dispatch on `(topics[0].kind, topics[0].value)` and (for Soroswap)
  `topics[1]`. Step 4 now flags Phoenix's 8-event → 1-trade grouping.
  Added three subsections:
  - "Per-venue decoder reference" — four-row table covering Aquarius
    router, Aquarius constant-product pool, Phoenix XYK/stable pool,
    Soroswap pool. Cross-references lore 0005 on the non-target
    allowlist policy.
  - "Pool enumeration" — per-venue factory/router addresses and
    pool-creation topics, plus a second table of additional canonical
    addresses (Soroswap router/aggregator, WASM hashes, Phoenix
    multihop).
  - "Per-venue payload notes" — Phoenix multi-event grouping,
    Soroswap two-topic filter, Soroswap no-inline-tokens, Phoenix fee
    shape (informational).
- **§11 "Open questions for the BE team"** — items 1 and 2 marked
  resolved with full lore archive references. Item 3 (decimals
  normalisation) remains the only open question pending BE input.

**DDL untouched** — the schema in §3 is unchanged. Storage estimates,
write semantics, partition strategy, and the cross-service contract
are all unaffected by this revision.

## Implementation Notes

- Single-commit doc revision: `docs/database-schema/amm-trades-schema.md`
  grew from 328 → 437 lines (139 insertions, 30 deletions).
- Re-used the existing H2/H3 heading convention (no §X.Y numbering in
  body, but §X.Y references in prose).
- Lore archive paths in the schema doc are repo-relative to keep them
  stable if the schema doc is ever vendored or relocated within the
  repo.
