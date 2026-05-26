---
id: "0008"
title: "Set up CI workflow for the Nx workspace"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0006"]
tags: [layer-tooling, phase-future, priority-medium, effort-small, infra, ci]
links: []
history:
  - date: 2026-05-11
    status: backlog
    who: claude
    note: "Spawned from 0006 future work."
  - date: 2026-05-26
    status: active
    who: okarcz
    note: "Activated for CI workflow implementation."
---

# Set up CI workflow for the Nx workspace

## Summary

Decide on and commit a CI workflow for the Nx workspace. Options: adopt the Nx-bundled
`.github/` (skills + `ci.yml`) we omitted in 0006, write a custom workflow, or stay with
no CI until the first app exists.

## Context

Task 0006 intentionally skipped the Nx generator's `.github/` directory because it
introduces a bundle of skill files and a `ci.yml` with opinions about the build pipeline.
Once 0007 lands and there's a real app to lint/test/build, we need affected-aware CI.

The repo already runs `deploy-board.yml` from the lore framework — coordinate so the new
workflow doesn't collide with it.

## Implementation

- Decide: adopt vs. write fresh vs. defer.
- If adopting Nx's bundle, inspect every skill file before committing — many are
  Claude-Code-adjacent and may overlap with the existing `.claude/` setup.
- Cover at minimum: `nx affected -t lint test build` against `develop` base, on PR and on
  pushes to `develop` / `master`.
- Consider Nx Cloud (currently `skip` from 0006) or self-hosted distributed cache.

## Acceptance Criteria

- [ ] `.github/workflows/ci.yml` (or equivalent) exists and runs on PR + push
- [ ] `nx affected` runs lint / test / build for changed projects
- [ ] No collision with `deploy-board.yml`
- [ ] Decision on Nx Cloud documented (adr or task note)
