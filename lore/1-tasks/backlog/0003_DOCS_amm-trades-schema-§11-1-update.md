---
id: "0003"
title: "Update amm-trades-schema.md §7 and §11.1 with empirical topic findings"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0001", "0002"]
tags: [priority-medium, effort-small, schema-validation]
links:
  - "../archive/0001_RESEARCH_dump-amm-swap-events/notes/S-amm-trades-schema-§11-1-resolved.md"
  - "../active/0002_RESEARCH_amm-venue-attribution/notes/S-venue-attribution-mapping.md"
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

- [ ] §7 step 3 reflects four-decoder reality and ScVal-kind-based
      dispatch (no longer hypothetical)
- [ ] Phoenix multi-event grouping rule documented (8 events → 1 row,
      group key `(tx_hash, op_index, contract_id)`)
- [ ] Soroswap two-topic filter rule documented (must check
      `topics[1]=Symbol("swap")` to drop sync/deposit/withdraw)
- [ ] Soroswap "no inline tokens" rule documented (token addresses
      come from per-pool cache, populated at pool-discovery time)
- [ ] Phoenix `fee` column fill strategy documented
- [ ] §11.1 marked resolved with references to lore 0001 + 0002
      archive paths
- [ ] "Per-venue decoder reference" table added
- [ ] "Pool enumeration" subsection added with factory addresses +
      pool-creation event topics
- [ ] No DDL changes (verify with `git diff`)
