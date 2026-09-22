---
id: "0299"
title: "lore_validate reports 1,238 errors and 1,109 of them are one YAML quirk — the 129 real findings are unreadable"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0100"]
tags: [layer-tooling, priority-low, effort-small, lore, tooling, developer-experience]
links:
  - "../../_template.md"
history:
  - date: "2026-09-21"
    status: backlog
    who: okarcz
    note: >
      Found while rescoping [[0100]]: quoting that file's two history dates was
      needed to make it validate, which raised the question of why the rest of
      the tree does not. Measured across all 421 lore files — 1,238 errors in
      389 of them, of which 1,109 are a single YAML typing quirk on `date`.
      The tool is therefore unusable, and 129 genuine findings (including
      `status: done`, which the archive rule already depends on) are buried in
      the noise. Filed as CHORE, not BUG — nothing is broken in production;
      a tool we cannot read is simply a tool nobody runs.
---

# The lore validator is all noise, so nobody runs it

## Summary

`lore-framework_validate` currently reports **1,238 errors across 389 of 421
files** — effectively "your whole knowledge base is broken", which is the same
as reporting nothing. **1,109 of those errors (90%) are one YAML typing quirk**
on the `date` field. Underneath them sit ~129 genuine findings nobody can see.

## Context

### The quirk

YAML infers types for unquoted scalars, and `YYYY-MM-DD` is one of the patterns
it treats specially:

```yaml
date: 2026-09-15     # parsed as a DATE value
date: "2026-09-15"   # parsed as a STRING
```

The validator's schema wants a string —
`DateSchema = z.string().regex(/^\d{4}-\d{2}-\d{2}$/)` — so every unquoted date
comes back as `history.N.date: Expected string, received date`. Nothing is
wrong with the *content*; the schema and the YAML parser simply disagree about
what was written.

### ⚠️ The schema is NOT ours to change

The validator ships in **`lore-framework-mcp`, a third-party npm package**
pinned to `1.2.10` by the plugin manifest (author: Mariusz Korzekwa). It is not
code in this repo, so "just relax the schema" is an **upstream** request, not a
local fix. Any plan that depends on the schema changing is a plan that blocks
on someone outside the team.

### The templates keep re-creating it

Both our own `lore/1-tasks/_template.md:11` and the plugin's
`lore-framework-tasks/SKILL.md:132` emit `date: YYYY-MM-DD` **unquoted**. (The
plugin's own skill contradicts itself — line 147 of the same file shows a
quoted example.) So this is not historical sediment that can be cleaned once:
**every new task re-introduces it** until a template changes. That makes the
template fix the only step that actually stops the bleeding.

### What is hiding under the noise

The 129 non-date errors, which are the reason this is worth doing at all:

| count | finding |
| --- | --- |
| 34 | `type:` is `CHORE` (18), `TEST` (10) or `PERF` (6) — all in daily use, none in the schema's enum (`BUG`/`FEATURE`/`RESEARCH`/`REFACTOR`/`DOCS`) |
| 17 | file has no `history` block at all |
| 16 | note `type` is `G`/`S` where the schema wants `generation`/`synthesis` |
| 14 | `spawned_from` is an array where the schema wants a string |
| 11 | `status: done` instead of `completed` (5 top-level, 6 in history) |
| 4 | `type:` missing entirely |
| ~33 | long tail — `status: proposed` on a wiki note, `history[].status: applied`, `history[].by` as a string |

The `status: done` rows matter most: that rule is already load-bearing (the
archive step depends on it) and it is currently invisible.

## Implementation

Ordered so the bleeding stops before the cleanup — the reverse order would rot
again while the cleanup is in review.

1. **Quote the date in `lore/1-tasks/_template.md`** (and any sibling
   template). One line, ours to change, stops new files drifting.
2. **Mechanical quote pass** over the tree:
   `sed -E 's/^(\s*- date: )([0-9]{4}-[0-9]{2}-[0-9]{2})$/\1"\2"/'`.
   ⚠️ Touches ~389 files, so it must land as its own commit on `develop` at a
   moment nothing is in flight — it will conflict with any open branch that
   edits a task file. Coordinate before running it.
3. **Re-run the validator and triage the ~129 survivors** — decide per class
   whether the repo conforms or the schema is wrong.
4. **File the schema mismatches upstream** as one issue against
   `lore-framework-mcp`: accept a date *or* a string for `date`, and widen
   `TASK_TYPES` to include `CHORE`/`TEST`/`PERF`. Both are the schema being out
   of step with how the framework is actually used — this task file is itself
   `type: CHORE` and would be rejected by it.
5. **Decide the fate of the genuinely-wrong files** — `status: done`,
   `spawned_from` arrays, missing `history`. These are ours and worth fixing
   regardless of what upstream does.

## Acceptance Criteria

- [ ] `lore/1-tasks/_template.md` emits a quoted date; a task created from it
      validates clean without hand-editing.
- [ ] `lore-framework_validate` over the whole tree returns zero `date` errors.
- [ ] The remaining errors are triaged: each class either fixed in-repo or
      recorded here as an accepted upstream divergence.
- [ ] An upstream issue exists against `lore-framework-mcp` for the `date` type
      and the `TASK_TYPES` enum, linked from this file.
- [ ] The validator's output is short enough that a person reads it — the
      standing test being that running it is worth doing before a task lands.
