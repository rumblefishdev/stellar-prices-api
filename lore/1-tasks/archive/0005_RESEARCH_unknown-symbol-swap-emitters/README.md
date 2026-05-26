---
id: '0005'
title: 'Identify unknown Symbol("swap") emitters CCR2CH4G... and CDMIM23W...'
type: RESEARCH
status: completed
related_adr: []
related_tasks: ['0002']
tags:
  [
    layer-research,
    priority-medium,
    effort-small,
    soroban,
    amm,
    venue-attribution,
  ]
links:
  - '../0002_RESEARCH_amm-venue-attribution/notes/S-venue-attribution-mapping.md'
  - '../0002_RESEARCH_amm-venue-attribution/notes/R-aquarius-registry.md'
history:
  - date: 2026-05-08
    status: backlog
    who: claude
    note: 'Spawned from 0002 future work.'
  - date: 2026-05-11
    status: active
    who: okarcz
    note: 'Activated for manual verification of the unknown emitters.'
  - date: 2026-05-11
    status: completed
    who: okarcz
    note: >
      Manually verified: all unknown Symbol("swap") emitters identified in
      0002 (CCR2CH4G..., CDMIM23W..., CCXRRORT..., CAUF4DFY..., and the
      long tail) do NOT belong to Soroswap, Aquarius, or Phoenix. They are
      out-of-scope for the indexer's per-venue mapping. Policy decision:
      exclude — do not index. See notes/S-unknown-emitters-non-target.md.
---

# Identify unknown `Symbol("swap")` emitters

## Summary

Task 0002 attributed every observed AMM contract emitter except a set of
`Symbol("swap")` emitters that did not match Aquarius router, Soroswap,
or Phoenix. This task manually verified those unknown contracts and
confirmed they are **non-target** — outside the {Soroswap, Aquarius,
Phoenix} set the BE indexer is required to track.

## Outcome

**Decision: exclude.** All unknown `Symbol("swap")` emitters listed in
0002's S-note are confirmed as non-target and will not be tracked by the
indexer. The per-venue mapping in 0002 is therefore complete with
respect to the three required venues.

See `notes/S-unknown-emitters-non-target.md` for the full list, the
manual verification summary, and the indexer policy.

## Acceptance Criteria

- [x] `CCR2CH4G...` confirmed non-target (manual verification)
- [x] `CDMIM23W...` confirmed non-target (manual verification)
- [x] `CCXRRORT...`, `CAUF4DFY...` and the long tail confirmed non-target
- [x] Policy decision documented for the BE team: **skip** (do not index
      as `venue: unknown`)

## Future Work

None. The Soroswap / Aquarius / Phoenix attribution from 0002 is the
complete target set. If a future Soroban DEX needs indexing, it will be
a new task.
