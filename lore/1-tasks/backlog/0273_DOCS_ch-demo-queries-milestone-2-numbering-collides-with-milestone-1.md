---
id: "0273"
title: "ch-demo-queries.sql has two queries numbered (10) and two numbered (11) — the Milestone 2 block restarts the sequence its own header says it continues"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0128"]
tags: [layer-docs, priority-low, effort-small, scf, evidence]
links:
  - "../../../docs/scf/ch-demo-queries.sql"
history:
  - date: 2026-09-09
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0128]] future work. Found while refreshing the Milestone 2
      evidence package for submission; offered three times during that task and
      deferred each time because it blocks nothing.
---

# `ch-demo-queries.sql` numbering collides between the two milestones

## Summary

`docs/scf/ch-demo-queries.sql` is the reviewer-facing query set behind both SCF
evidence packages. The Milestone 2 block, appended under its own banner, **starts
again at (10)** — a number Milestone 1 already uses — and then **jumps from (22)
to (26)**. Its own header claims the opposite:

> `-- Query numbering continues from the Milestone 1 set.`

So a reviewer told to "see query (10)" finds two different queries, and (23),
(24) and (25) do not exist.

## Context

Milestone 1 runs (1) through (11). The Milestone 2 banner sits at line 202 and
its first query is (10) again — "The published row for one asset, as the API
serves it" — colliding with M1's (10), "Oracle reference prices are ingested as a
cross-reference". (11) collides the same way. The final query is (26), four
numbers past its predecessor.

[[0128]]'s own acceptance criteria recorded the M2 additions as "queries
(10)-(22)", so the collision was written down as if it were the intended range
rather than caught.

**Nothing cross-references these numbers** — `milestone-2-evidence.md` does not
cite query numbers, and neither does the M1 package — so renumbering is safe and
touches one file.

⚠️ **The Milestone 1 package is frozen.** Its numbering must not move. Only the
M2 block below the banner is in scope.

## Implementation

- Renumber the Milestone 2 block **(12) through (24)**, continuing M1's (11) and
  closing the (23)-(25) hole in one pass.
- Leave every query body untouched; this is a comment-header change.
- Re-read the banner's "numbering continues from the Milestone 1 set" line and
  confirm it is finally true.

## Acceptance Criteria

- [ ] No query number appears twice in the file.
- [ ] The Milestone 2 block runs (12)-(24) with no gaps.
- [ ] Milestone 1's (1)-(11) are unchanged.
- [ ] No query body, table name or column name is edited.
- [ ] `grep -c '^-- ([0-9]' docs/scf/ch-demo-queries.sql` equals the number of
      queries, and the sequence is strictly increasing.
