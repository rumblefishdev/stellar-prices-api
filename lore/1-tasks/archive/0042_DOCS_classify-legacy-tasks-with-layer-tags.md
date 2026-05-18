---
id: "0042"
title: "Classify legacy lore tasks with layer-* tags for board grouping"
type: DOCS
status: completed
related_adr: []
related_tasks: ["0041"]
tags: ["layer-tooling", "phase-now", "effort-small", "priority-low"]
links:
  - "https://rumblefishdev.github.io/stellar-prices-api/"
  - "https://github.com/rumblefishdev/stellar-prices-api/pull/19"
history:
  - date: 2026-05-18
    status: backlog
    who: claude
    note: "Spawned from 0041 future work."
  - date: 2026-05-18
    status: active
    who: oski
    note: "Activated to classify legacy tasks."
  - date: 2026-05-18
    status: completed
    who: claude
    note: >
      Added a layer-* tag to 38 legacy task files via mechanical
      script. Shipped via PR #19 (+38/-38, one token per tags: line).
      Resulting distribution: research 18, indexing 10, tooling 6,
      database 3, infra 3, backend 1. Zero tasks remain on
      layer-other. No follow-ups.
---

# Classify legacy lore tasks with layer-* tags for board grouping

## Summary

Added one `layer-*` tag to each of 38 legacy task files so the lore board's layer dimension (card color + filter row) is no longer dominated by `layer-other`. Shipped as a single mechanical PR (#19, commit `76af72d`).

## Implementation Notes

The mapping was applied via a one-shot bash script driving `sed` over a hand-built `id → layer` association table. The script:

1. Resolved each task ID to its file path (handling both `NNNN_TYPE_slug.md` and `NNNN_TYPE_slug/README.md` shapes across all four status dirs).
2. Skipped any file that already had a `layer-` tag (so 0041, 0042, 0043 were no-ops).
3. Inserted the layer tag at the front of the existing `tags: [...]` list (e.g. `tags: [priority-medium, ...]` → `tags: [layer-research, priority-medium, ...]`).

Resulting per-layer counts (board.json after deploy):

| Layer | Count |
|-------|-------|
| `layer-research` | 18 |
| `layer-indexing` | 10 |
| `layer-tooling` | 6 |
| `layer-database` | 3 |
| `layer-infra` | 3 |
| `layer-backend` | 1 |

`layer-domain` and `layer-frontend` are unused — this project has no frontend, and domain types currently live within indexing crates rather than as standalone tasks.

## Acceptance Criteria

- [x] Every task file in `lore/1-tasks/` has exactly one `layer-*` tag (verified by re-running the dump script post-merge)
- [x] `npm run board` produces a `board.json` where no task has `layer: "layer-other"` (verified locally before pushing and again on the deployed board)
- [x] Deployed board shows non-empty filter buttons for each layer that has tasks

## Design Decisions

### From Plan

1. **One layer tag per task**, inserted at the front of `tags:` for visual prominence. Matches the convention 0041/0043 already used.

2. **Single mechanical commit**, not split per-layer. Plan allowed either; the diff was small (+38/-38, one token per file) so splitting would have added review overhead without clarity gain.

### Emerged

3. **DOCS tasks classified by the subject they describe, not by being "docs".** A schema-update doc (0003, 0013, 0030) is `layer-database`; an architecture-propagation doc (0014) is `layer-infra`; a Stream-1 design doc (0029) is `layer-indexing`; a research-completion doc (0033) is `layer-research`. There is no `layer-docs` in the explorer's taxonomy and inventing one for this project would have meant diverging — chose subject-based classification instead.

4. **0023 (OHLCV PK research) → `layer-research`, not `layer-database`.** The task is investigative ("decide base-only PK vs (base, quote) PK") even though the deliverable is a schema decision. Going by `type: RESEARCH` keeps the layer aligned with the work shape rather than the eventual output.

5. **0017 (local ClickHouse setup) → `layer-infra`, not `layer-tooling`.** Setting up a runtime data store is operational/architectural, not devtools. Tooling was reserved for things like Nx, CI, generators, and runtime-framework choice.

6. **0012 ("Design SDEX backfill" — type FEATURE despite being a design task) → `layer-indexing`.** The task title says "design" but it's classified as FEATURE. Followed the subject (backfill = indexing work) rather than the type.

7. **No follow-ups spawned.** The taxonomy is now usable on the board. If a future task feels like it doesn't fit any layer, that's the moment to either pick the closest and move on, or write an ADR proposing a new layer — not a blocker today.

## Issues Encountered

- **Initial mis-count** (37 vs 38 OKs). My head-counting of the LAYERS associative array missed one entry; the sed script correctly handled 38. Caught by cross-referencing `git diff --stat` with the OK output. No corruption — just an off-by-one in commentary.

- **YAML tags line shape was uniform** — all legacy files used inline bracket form `tags: [a, b, c]`. No multi-line `tags:\n  - a\n  - b` cases, so a single `sed` substitution worked for all 38. If a future task ever uses the multi-line form, the script in this task won't match it; would need an awk/python pass instead.
