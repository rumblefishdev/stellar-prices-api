---
id: "0240"
title: "0118 review leftovers — the duplicated $100 literal, validation as a type, and a measurement repeated five times"
type: REFACTOR
status: backlog
related_adr: []
related_tasks: ["0118", "0119"]
tags: [layer-backend, layer-database, priority-low, effort-small, cleanup, vwap]
milestone: 3
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../packages/prices-api/src/assets/handlers.rs"
history:
  - date: 2026-08-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0118]]'s `/code-review`. Fourteen findings were reported;
      six were fixed in the PR (including the correctness one) and the rest
      deferred here. None is a bug — they are drift risks and duplication.
---

# 0118 review leftovers

## Summary

Three of the deferred `/code-review` findings are worth doing together, since
all three are about the same thing: a fact that exists in more than one place
and can drift. The remaining findings from that review (a redundant
`is_finite()`, a redundant rebinding, a duplicated test matrix) are noise and
are deliberately **not** carried here.

## Context

1. **`MIN_VOLUME_USD` is two literals in `current.sql`** — `max(src_volume >
   100) OVER (…)` in `per_source_kept` and `WHERE NOT asset_has_funded OR
   src_volume > 100` in `per_source_funded`. They must agree: retuning one and
   missing the other makes `asset_has_funded` answer a different question than
   the filter applies, so a venue in the band between the two values arms the
   flag and is then filtered out — emptying `sources` on exactly the assets the
   conditional arm protects, in a band no fixture pins. A `100 AS
   min_volume_usd` alias in the existing `WITH` chain gives one definition.
   (The cross-language third copy is already gone: the strict-filter fix
   removed `SYSTEM_MIN_VOLUME_USD` from `dto.rs`.)
2. **`min_volume_usd` validation is hand-wired into two handlers** rather than
   living in the typed query param, contrary to the policy `ListParams`' own
   doc states. The next route to grow the param — `POST /prices/batch` already
   shares `PriceResponse::from_row` — must remember the third call site;
   forgetting it returns 200 with everything excluded instead of [[0119]]'s
   400. A validating `Deserialize` on a `MinVolumeUsd` newtype removes both
   sites and the possibility of omission.
3. **The 2026-08-27 blast-radius measurement is written out in full in five
   places** — the `current.sql` header, fixture 20's comment, its assertion
   message, the runbook, and the task. The runbook explicitly keeps the query
   for re-measurement; a re-run updates one copy and leaves four asserting the
   old figures. Keep the narrative once and have the code comments point at it,
   the way `current.sql` already points at other tasks.

## Implementation

- Single `min_volume_usd` alias in the MV's `WITH` chain, referenced by both
  predicates. Needs a `DROP` + re-`CREATE` — fold it into the next MV redeploy
  rather than spending a cycle on it alone (see [[0238]], which will need one).
- `MinVolumeUsd` newtype with a validating `Deserialize`; delete
  `min_volume_error` and both call sites.
- Trim four of the five copies of the measurement to a pointer.

## Acceptance Criteria

- [ ] The threshold has one definition in `current.sql`, and the fixtures still
      pass
- [ ] Invalid `min_volume_usd` is rejected by the type, with no per-handler
      call site left, and the existing 400 tests still pass
- [ ] The measurement narrative exists once, with pointers from the code
