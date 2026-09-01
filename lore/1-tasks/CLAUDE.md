# Tasks

All tasks by lifecycle status. Shared ID sequence (NNNN).

## IMPORTANT: Always use `/lore-framework-tasks` skill

**When updating tasks (status changes, completing, archiving, spawning follow-ups), you MUST invoke the `/lore-framework-tasks` skill FIRST.** This is critical for maintaining consistent task lifecycle and documentation. Never update task status manually without the skill.

## Directories

- [backlog/](backlog/CLAUDE.md) — future work, not yet started
- [active/](active/CLAUDE.md) — currently in progress
- [blocked/](blocked/CLAUDE.md) — waiting on dependencies
- [archive/](archive/CLAUDE.md) — completed

## Format

`NNNN_TYPE_slug.md` or `NNNN_TYPE_slug/` (directory for complex tasks)

## Task Size

**Keep README.md short** (~50-100 lines): summary, status, context.

Heavy content goes into `notes/` subdirectory. **Convert to directory when task grows beyond ~150 lines.**

## Task documents land on `develop`, branches carry code

**Analysis, measurements, decisions and spawned tasks go to `develop` as soon as they exist — not to the feature branch.** They are `docs(lore-NNNN):` changes with no code in them, so they merge cleanly and need no review cycle. The branch carries the implementation, its tests, and the completion entry.

Why, from one day's evidence (2026-08-28):

- `fix/0210_soroban-asset-symbol` sat **a week** holding a correct re-scope nobody could see. `develop`'s copy still showed the plan that re-scope had disproven, so a later pass re-derived the same conclusion from scratch — and briefly committed the **opposite** recommendation to `develop` before finding the branch.
- [[0120]] listed two long-archived tasks in `by:`, so Tranche 2's largest item read as triple-blocked when only one blocker remained.
- [[0210]]'s promotion moved the file and flipped `status` without a history entry; [[0165]] had the same drift a week earlier.

A finding somebody else's task depends on must not wait for a merge. The identity-split defect in [[0242]] was found inside [[0210]]'s branch work and belonged on `develop` the moment it was measured, because it changes what [[0139]] and the ingest path are about.

Practical consequence worth knowing: this also removes most task-file merge conflicts, since two people editing the same task document on different branches is where they come from.

**Exception:** implementation notes that only make sense beside the code they describe can land with that code.

## Note Prefixes

| Prefix | Type | Use for |
|--------|------|---------|
| `Q-` | Question | What we're trying to answer |
| `I-` | Idea | Original thoughts, hypotheses |
| `R-` | Research | External knowledge (papers, docs, analysis) |
| `S-` | Synthesis | Conclusions, decisions ("so what?") |
| `G-` | Generation | Artifacts we produce (specs, schemas, designs) |

Lineage via `spawned_from`/`spawns`. Status: `seed → developing → mature → superseded`.

Create: `_note_template.md` | Full docs: `/lore-framework` skill

## Lifecycle

```
backlog/ → active/ → blocked/ ↔ active/ → archive/
```

Promotion: `git mv` between directories, update `status` in frontmatter.

## Templates

- `_template.md` — new tasks
- `_note_template.md` — notes in task `notes/` directories
