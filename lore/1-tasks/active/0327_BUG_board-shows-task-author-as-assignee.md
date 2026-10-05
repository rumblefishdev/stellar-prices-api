---
id: "0327"
title: "The board shows a task's author as its assignee when history is written newest-first"
type: BUG
status: active
assignee: akot
related_adr: []
related_tasks: ["0041", "0139", "0242"]
tags: ["lore-board", "tooling", "effort-small", "priority-medium"]
links: []
history:
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
  the order. YAML parses unquoted dates as `Date` and quoted ones as strings,
  so normalise both before comparing. On a tie, the later array position wins.
- Unchanged: `backlog` and `blocked` tasks get no assignee.

## Acceptance Criteria

- [ ] 0139 and 0242 show `akot` on the board
- [ ] A task with history in either order shows its newest `who`
- [ ] A task with `assignee:` shows that value
- [ ] Every other task's assignee is unchanged, or the change is explained
      (diff of `board.json` before/after)
