---
id: "0042"
title: "Classify legacy lore tasks with layer-* tags for board grouping"
type: DOCS
status: backlog
related_adr: []
related_tasks: ["0041"]
tags: ["layer-tooling", "phase-future", "effort-small", "priority-low"]
links:
  - "https://rumblefishdev.github.io/stellar-prices-api/"
history:
  - date: 2026-05-18
    status: backlog
    who: claude
    note: "Spawned from 0041 future work."
---

# Classify legacy lore tasks with layer-* tags for board grouping

## Summary

The lore board's primary visual grouping is by `layer-*` tag (color on cards + filter row in the UI). Only 1 of 34 current tasks (0041 itself) carries a `layer-*` tag; the rest render under `layer-other` with no color/filter. Classify the legacy tasks so the layer dimension becomes useful.

## Context

The board generator (`tools/scripts/generate-lore-board.mjs`) and viewer (`lore/board.html`) ship with the same `layer-*` taxonomy as `soroban-block-explorer`:

| Layer | Use for |
|-------|---------|
| `layer-research` | RESEARCH-type tasks investigating something |
| `layer-domain` | Domain models, business rules, type definitions |
| `layer-database` | Schema, migrations, query optimization |
| `layer-backend` | API handlers, services, request/response logic |
| `layer-indexing` | Stream consumers, ingestion lambdas, event processing |
| `layer-frontend` | UI work (none yet in this repo) |
| `layer-infra` | CDK, AWS, deployment, infra-as-code |
| `layer-tooling` | Scripts, devtools, CI |

Legacy tasks today use freeform domain tags (`lambda`, `clickhouse`, `stream-1`, `phoenix`, …) which the board ignores for layering.

## Implementation

- Audit every task file in `lore/1-tasks/{backlog,active,blocked,archive}` and add one `layer-*` tag to its frontmatter `tags:` list.
- Map suggestions (not authoritative — apply judgment per task):
  - RESEARCH tasks → `layer-research`
  - Schema/migration docs → `layer-database`
  - Lambda/consumer tasks → `layer-indexing`
  - API/handler tasks → `layer-backend`
  - CDK/AWS/infra tasks → `layer-infra`
  - DOCS not fitting elsewhere → match the layer the work *describes* (e.g. a schema doc is `layer-database`)
- Keep existing tags; just add the layer tag.
- Run `npm run board` locally to spot-check that the JSON now classifies tasks correctly.
- Commit as a single `docs(lore-0042): classify legacy tasks with layer tags` (or split per layer if the diff is large).

## Acceptance Criteria

- [ ] Every task file in `lore/1-tasks/` has exactly one `layer-*` tag
- [ ] `npm run board` produces a `board.json` where no task has `layer: "layer-other"`
- [ ] Deployed board shows non-empty filter buttons for each layer that has tasks

## Notes

If a task genuinely spans layers, pick the primary one — the board's design assumes one layer per task. If this becomes painful, that's a signal to revisit the taxonomy (would be an ADR).
