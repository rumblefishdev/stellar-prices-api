---
id: "0328"
title: "The board page recomputes the assignee from the last history entry and ignores the one board.json carries"
type: BUG
status: completed
assignee: akot
related_adr: []
related_tasks: ["0327", "0149", "0041"]
tags: ["lore-board", "tooling", "effort-small", "priority-medium"]
links:
  - "../../board.html"
  - "../../../tools/scripts/generate-lore-board.mjs"
history:
  - date: "2026-10-06"
    status: completed
    who: akot
    note: >
      PR #394 merged (d0399fd2) and deployed to GitHub Pages. getTaskAssignee
      returns t.assignee; a test in generate-lore-board.test.mjs pins it
      (14/14, red on the old page). The deployed board shows 0149, 0242 and
      0139 as akot.
  - date: "2026-10-06"
    status: active
    who: akot
    note: >
      Found on 0149: board.json says akot, the board page shows okarcz.
      [[0327]] fixed the generator only; the page has its own rule.
---

# The board page ignores the computed assignee

## Summary

[[0327]] made `getAssignee` in `generate-lore-board.mjs` honour `assignee:`,
then the newest-dated history entry, skipping `who: claude`. `board.json` has
carried the right value since. The page never reads it.
`getTaskAssignee` in `lore/board.html:934` still takes the `who` of the
**last** history entry. On a newest-first history that entry is the author.

## Evidence (2026-10-06)

- `board.json` on GitHub Pages: 0149 → `"assignee": "akot"`.
- The board page shows 0149 as `@okarcz`, from its 2026-08-05 spawn entry.
- 0327 checked its fix against the `board.json` diff (25 of 318 tasks moved),
  not against the page, so the page kept the old rule.

## Fix

- `getTaskAssignee(t)` returns `t.assignee`. The generator already applies the
  status rule (active and archive only) and the history fallback, so the page
  keeps no rule of its own.
- The same function feeds the filter counts, the card tag, the table and the
  modal (`board.html:883, 951, 999, 1052, 1086`), so all of them follow.

## Acceptance Criteria

- [x] `board.html` reads `assignee` from `board.json` and has no assignee
      logic of its own.
- [x] On the deployed board, 0149, 0242 and 0139 show `@akot`.
- [x] The assignee filter counts match `board.json`'s `assignee` values. By
      construction: the filter counts go through the same `getTaskAssignee`.
