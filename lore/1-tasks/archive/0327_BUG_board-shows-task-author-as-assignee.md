---
id: "0327"
title: "The board shows a task's author as its assignee when history is written newest-first"
type: BUG
status: completed
assignee: akot
related_adr: []
related_tasks: ["0041", "0139", "0242"]
tags: ["lore-board", "tooling", "effort-small", "priority-medium"]
links: []
history:
  - date: "2026-10-05"
    status: completed
    who: akot
    note: >
      getAssignee honours `assignee:`, else the newest-dated history entry,
      skipping `who: claude`. 13 table-driven tests. Board diff: 25 of 318
      tasks change assignee, 0139 and 0242 now akot.
  - date: "2026-10-05"
    status: active
    who: akot
    note: >
      Task created. 0139 showed okarcz and 0242 stkrolikiewicz on the board,
      though both are worked by akot.
---

# The board shows a task's author as its assignee when history is written newest-first

## Summary

`getAssignee` in `tools/scripts/generate-lore-board.mjs` takes `who` from the
last element of `history`, assuming entries are appended oldest-first. Many
tasks prepend instead (0139, 0242, 0294, 0296), so the board names whoever
created the task. It also ignores the `assignee:` frontmatter field that
several tasks already carry (0101, 0315, 0323).

## Fix

- Use `assignee:` from frontmatter when present.
- Otherwise take `who` from the history entry with the latest `date`, whatever
  the order. The board's own frontmatter parser strips quotes, so every date
  is a `YYYY-MM-DD` string and compares as one.
- Unchanged: `backlog` and `blocked` tasks get no assignee.

## Acceptance Criteria

- [x] 0139 and 0242 show `akot` on the board (in a local `board.json`; the
      live board after merge)
- [x] A task with history in either order shows its newest `who`
- [x] A task with `assignee:` shows that value
- [x] Every other task's assignee is unchanged, or the change is explained
      (diff of `board.json` before/after)

## Design Decisions

### Emerged

1. **Date ties follow the file's direction.** A file whose first date is
   later than its last is newest-first, so the earlier of two same-day
   entries wins there; otherwise the later one does.
2. **`who: claude` is never the assignee.** Without this, 0294–0296 would
   have flipped to `claude`. Entries without `who` are skipped too.
3. **The script runs only when invoked directly**, so the test can import
   `getAssignee`.

## Board diff (318 tasks, 25 changed)

- 0139 okarcz → akot, 0242 stkrolikiewicz → akot.
- 0070, 0078, 0120: the newest entry is a different person than the
  oldest.
- 20 tasks, almost all archived, that showed `claude` now show the person
  from their newest human entry. 0074 has only `claude` entries and shows
  none. Some old aliases surface as written (`oski`, `oskar`, `operator`):
  that is the data, not the code.
