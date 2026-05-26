---
id: '0041'
title: 'Lore Board on GitHub Pages — mirror soroban-block-explorer setup'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0042', '0043']
tags: ['layer-tooling', 'phase-now', 'effort-small', 'priority-medium']
links:
  - 'https://rumblefishdev.github.io/stellar-prices-api/'
  - 'https://github.com/rumblefishdev/stellar-prices-api/pull/16'
  - 'https://github.com/rumblefishdev/stellar-prices-api/pull/17'
history:
  - date: 2026-05-18
    status: backlog
    who: claude
    note: 'Task created. lore/board.html and workflows/deploy-board.yml already copied verbatim from soroban-block-explorer; remaining work is to wire the generator script, npm script, workflow location, and Pages enablement.'
  - date: 2026-05-18
    status: active
    who: oski
    note: 'Activated to start implementation.'
  - date: 2026-05-18
    status: completed
    who: claude
    note: >
      Board live at rumblefishdev.github.io/stellar-prices-api/.
      Shipped via PR #16 (implementation) + PR #17 (CI hotfix: dropped
      npm ci to sidestep pre-existing lockfile mismatch). 39 tasks
      indexed on first deploy. Spawned 0042 (layer-tagging) and
      0043 (lockfile fix).
---

# Lore Board on GitHub Pages — mirror soroban-block-explorer setup

## Summary

Stood up the static HTML backlog board (`lore/board.html` + generated `lore/board.json`) on GitHub Pages for `stellar-prices-api`, mirroring `soroban-block-explorer`. The board renders all lore tasks across `backlog/`, `active/`, `blocked/`, and `archive/` as a filterable Kanban-style view, deployed automatically on every push to `develop`.

Live at: https://rumblefishdev.github.io/stellar-prices-api/

## Context

`soroban-block-explorer` already runs the same board pattern. Two pieces were ported verbatim before this task started:

- `lore/board.html` — static viewer (already re-branded to "Stellar Prices API" by the user before activation)
- `workflows/deploy-board.yml` — GitHub Actions workflow (initially at repo-root `workflows/`)

This task wired up the remaining pieces (generator, npm script, workflow location, Node pin, Pages enablement) so the board renders real data and auto-deploys.

## Implementation Notes

Shipped across two PRs against `develop`:

| PR  | Commit    | What                                                                                                                                                                                                        |
| --- | --------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| #16 | `9c09604` | `tools/scripts/generate-lore-board.mjs` (245 LOC, verbatim from explorer), `npm run board`, `.github/workflows/deploy-board.yml` (moved from repo-root `workflows/`), `.nvmrc` (22.22.0), `lore/board.html` |
| #17 | `5770495` | Dropped `npm ci` + `cache: npm` from the workflow (CI hotfix — see Issues Encountered)                                                                                                                      |

Activation also shipped a direct status-only commit to `develop` (`9a70add`) via `/promote-task`.

Generator outputs `lore/board.json` (270 KB, 39 tasks on first run). Already in `.gitignore` as a build artifact.

## Acceptance Criteria

- [x] `tools/scripts/generate-lore-board.mjs` exists and produces a valid `lore/board.json` when run locally
- [x] `npm run board` works
- [x] `lore/board.html` renders correctly (already re-branded by user before task activation)
- [x] `lore/board.json` is `.gitignore`d (was already in `.gitignore` line 57 — no change needed)
- [x] `.github/workflows/deploy-board.yml` is in place; `workflows/` at repo root is removed
- [x] `.nvmrc` exists at repo root (pinned to 22.22.0 to match `soroban-block-explorer`)
- [x] GitHub Pages enabled with "GitHub Actions" as the source (user did this manually)
- [x] A push to `develop` triggers `Deploy Board to GitHub Pages` and produces a live page

## Design Decisions

### From Plan

1. **Verbatim port from `soroban-block-explorer`.** The generator script, board.html viewer, and workflow are copied without modification. Keeps both repos drift-compatible; any improvements made in one can be pulled into the other.

2. **`.nvmrc` pinned to 22.22.0** to match `soroban-block-explorer`. No reason to diverge — the script uses only Node builtins, so any modern LTS works, but pinning matches CI behavior across the two repos.

### Emerged

3. **Did not backfill `layer-*` tags onto 33 legacy tasks during this task.** The board's primary visual grouping (layer color + filter) is currently degraded — all legacy tasks render under `layer-other` (no color, no filter row). Chose to ship the board working-but-degraded and spawn a separate task (0042) to do the classification carefully, rather than rushing a per-task taxonomy call inside this task. The board still renders all 39 tasks and status/type/priority filters all work; only the layer dimension is sparse.

4. **Dropped `npm ci` from the workflow rather than re-locking `package-lock.json`.** First CI run after PR #16 failed on `npm ci` due to a pre-existing lockfile mismatch (missing `@emnapi/{core,runtime}@1.10.0`). The generator script has zero deps (only `node:fs`/`node:path`/`node:url`), so `npm install` is unnecessary entirely. Dropping the step was a smaller, lower-risk change than regenerating the lockfile (which would have pulled in unrelated dep updates and is outside this task's scope). Spawned 0043 to fix the underlying lockfile issue separately.

5. **No git hook to auto-regenerate `board.json` locally.** Original task notes mentioned this as a possible future enhancement ("defer unless we hit friction"). Did not hit friction during implementation. Not spawned as a backlog task — re-evaluate if the deployed board's freshness ever becomes a problem.

## Issues Encountered

- **First CI run failed on `npm ci`** (run 26035530230, ~21s). Root cause: pre-existing mismatch between `package-lock.json` and `package.json` — lockfile missing `@emnapi/{core,runtime}@1.10.0`. Not introduced by this task (didn't touch deps). Fixed in PR #17 by dropping `npm ci` entirely since the generator is zero-dep. Underlying lockfile issue tracked separately as 0043.

- **Generator's layer taxonomy doesn't match this repo's existing tags.** The script ships with `LAYER_LABELS` matching the explorer's taxonomy (`layer-research`, `layer-domain`, `layer-database`, …). Only one task here (0041 itself) uses a `layer-*` tag; the other 33 use domain tags (`lambda`, `infra`, `stream-1`, etc.) directly. The script falls back to `layer-other`, so the board still renders — but the layer color/filter is effectively dead until tasks get classified. Tracked as 0042.

## Future Work

- **0042** — Classify legacy tasks with `layer-*` tags so the board's layer dimension becomes useful.
- **0043** — Fix `package-lock.json` mismatch (missing `@emnapi/{core,runtime}@1.10.0`); will likely affect any other workflow that runs `npm ci`.
