---
id: '0033'
title: 'Lock Soroswap Pair swap event emit site to source (verbatim quote in 0018 G-note)'
type: DOCS
status: backlog
related_adr: []
related_tasks: ['0018']
tags:
  [
    layer-research,
    priority-low,
    effort-small,
    soroswap,
    source-cross-ref,
    doc-completeness,
  ]
links:
  - '../archive/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md'
  - 'https://github.com/soroswap/core'
history:
  - date: 2026-05-15
    status: backlog
    who: claude
    note: 'Spawned from 0018 Appendix B item 4.'
---

# Soroswap Pair swap event emit site — verbatim source quote

## Summary

Task 0018 §1 (Soroswap) confirmed the decoded `SoroswapPair swap`
shape against a real mainnet sample WASM-verified as a canonical
Soroswap pair, and against the Uniswap-V2-style Map layout
described in archive task 0001's wider-sample summary. It did
**not** quote the actual emit call from `soroswap/core` verbatim,
unlike the parallel registry notes for Aquarius and Phoenix.

This task closes that loop: fetch
`soroswap/core/contracts/pair/src/event.rs` (or whatever the
actual filename is) and quote the
`env.events().publish(("SoroswapPair", "swap"), SwapEvent {…})`
call site so the G-note's §1.5 has the same source-level
guarantee the other two AMM sections do.

## Context

The decoded sample plus the WASM-hash identity match
(`hashes.pair = 18051456…f73e`) already establishes correctness
empirically. The verbatim quote upgrades that to
"empirical + source identity" — same level as Aquarius §2 and
Phoenix §3 — and protects against future Soroswap contract
upgrades silently changing the field names.

## Implementation

1. Browse `https://github.com/soroswap/core` to locate the pair
   contract's event emission site (likely
   `contracts/pair/src/event.rs` or analogous; the registry
   note links `contracts/factory/src/event.rs` so the convention
   is one-file-per-contract-events module).
2. Fetch the file and locate the `swap` emit call. Quote it
   verbatim (with filename + line numbers) into the archived
   task 0018's G-note §1.5 — file `notes/G-amm-swap-event-shapes.md`.
3. If the source's emit shape diverges from the decoded sample
   (it shouldn't), update §1.1 with the actual field-name
   ordering.

## Acceptance Criteria

- [ ] `soroswap/core` Pair swap emit call located and quoted in
      §1.5 of the (archived) task 0018 G-note.
- [ ] Confirmed: source and decoded sample agree (or, if not,
      the divergence is flagged and explained).

## Notes

Lightweight — one WebFetch + a small G-note edit. The G-note
file will be in `lore/1-tasks/archive/0018_…/notes/`. Editing
archived task files is allowed for late-arriving corrections
(per archive `CLAUDE.md` it's a reference store, not frozen).
