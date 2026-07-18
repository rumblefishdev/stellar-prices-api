---
id: '0008'
title: 'Set up CI workflow for the Nx workspace'
type: FEATURE
status: completed
related_adr: []
related_tasks: ['0006']
tags: [layer-tooling, phase-future, priority-medium, effort-small, infra, ci]
links:
  - https://github.com/rumblefishdev/stellar-prices-api/pull/33
history:
  - date: 2026-05-11
    status: backlog
    who: claude
    note: 'Spawned from 0006 future work.'
  - date: 2026-05-26
    status: active
    who: okarcz
    note: 'Activated for CI workflow implementation.'
  - date: 2026-05-26
    status: completed
    who: okarcz
    note: >
      Unified CI workflow with path-based change detection, ESLint + Vitest +
      Husky + lint-staged infrastructure. 11 new/modified config files, 130 files
      reformatted. PR #33 merged. Nx Cloud removed.
---

# Set up CI workflow for the Nx workspace

## Summary

Custom CI workflow modeled after soroban-block-explorer, with path-based change
detection (dorny/paths-filter) for Rust and TypeScript jobs. Also added full dev
tooling: ESLint, Vitest, Husky git hooks, lint-staged, and Prettier formatting
pass across the entire repo.

## Context

Task 0006 intentionally skipped the Nx generator's `.github/` directory because it
introduces a bundle of skill files and a `ci.yml` with opinions about the build pipeline.
The repo previously had a standalone `rust.yml` covering only Rust crates.

## Acceptance Criteria

- [x] `.github/workflows/ci.yml` exists and runs on PR + push to master
- [x] `nx affected` runs lint / test / build for changed projects (via path-based change detection)
- [x] No collision with `deploy-board.yml` (separate workflow, separate triggers)
- [x] Decision on Nx Cloud documented: removed nxCloudId — not needed, was blocking CI

## Implementation Notes

**CI workflow** (`.github/workflows/ci.yml`):

- `changes` job: dorny/paths-filter detects Rust vs TypeScript changes
- `typescript` job: `npm ci` → `nx format:check --all` → `nx run-many -t lint build typecheck`
- `rust` job: `cargo fmt --check` → `cargo check` → scoped clippy → `cargo test`
- Replaces standalone `rust.yml`

**Dev tooling added:**

| File                                    | Purpose                                                  |
| --------------------------------------- | -------------------------------------------------------- |
| `eslint.config.mjs`                     | Flat config with Nx module boundary rules                |
| `vitest.workspace.ts`                   | Vitest workspace config                                  |
| `.husky/pre-commit`                     | lint-staged + `run-affected-checks.mjs staged`           |
| `.husky/pre-push`                       | scoped cargo clippy + `run-affected-checks.mjs push`     |
| `tools/scripts/run-affected-checks.mjs` | Intelligent affected-target detection for hooks          |
| `.editorconfig`                         | Editor formatting (charset, indent, trailing whitespace) |

**nx.json** updated with `@nx/eslint/plugin` and `@nx/vitest` plugins.

**package.json** updated with scripts (`build`, `lint`, `test`, `typecheck`, `format`,
`format:check`, `prepare`, `verify:staged`, `verify:push`) and devDependencies
(`@nx/eslint`, `@nx/vitest`, `eslint`, `husky`, `lint-staged`, `vitest`, `typescript-eslint`).

**.prettierignore** updated to exclude JSONL evidence files, `board.html`, Rust sources,
and `Cargo.lock`.

Ran `nx format:write --all` to bring all 130 existing files in line with Prettier.

## Issues Encountered

- **Root-owned `node_modules`, `.nx`, `infra/dist`, `infra/cdk.out`**: Previous `npm install`
  was run as root. Nx cache restore failed with EACCES when trying to write to root-owned
  `infra/dist`. Fix: `rm -rf` the root-owned build artifacts (gitignored, safe to delete).

- **Lockfile version drift**: Local Node 24 (npm 11) generated a lockfile that CI's Node 22
  couldn't parse. Fix: clean `rm -rf node_modules package-lock.json && npm install` with
  matching Node version.

- **sdex-backfill clippy warnings**: Pre-push hook initially ran `cargo clippy --all-targets`
  which caught pre-existing warnings in sdex-backfill. Fix: scoped clippy to exclude
  sdex-backfill, matching the CI configuration.

- **Nx Cloud blocking CI**: `nxCloudId` in nx.json caused `nx run-many` to fail with
  "Workspace is unable to be authorized. Exiting run." Fix: removed nxCloudId entirely.

## Design Decisions

### From Plan

1. **Write fresh CI rather than adopt Nx bundle**: The Nx generator's `.github/` includes
   Claude-Code-adjacent skill files that overlap with the existing `.claude/` setup.
   Custom workflow gives full control.

### Emerged

2. **Scoped pre-push clippy to exclude sdex-backfill**: Has pre-existing clippy warnings
   that predate this task. Matches the CI scoping to avoid blocking pushes.

3. **Added `cdk.out` to ESLint ignores**: CDK-generated JavaScript artifacts fail lint
   rules (no-var, prefer-const, etc.). These are build outputs, not source code.

4. **Removed Nx Cloud entirely**: Workspace was never connected within the 3-day window.
   The nxCloudId was actively blocking CI runs rather than just warning.

5. **Formatted all existing files in a single pass**: Rather than formatting incrementally
   as files are touched, ran `nx format:write --all` to establish a clean baseline. This
   avoids noisy formatting diffs in future PRs.
