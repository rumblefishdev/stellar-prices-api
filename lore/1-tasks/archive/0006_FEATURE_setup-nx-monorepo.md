---
id: "0006"
title: "Bootstrap Nx monorepo workspace (@rumblefish/stellar-prices-api)"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0007", "0008"]
tags: [layer-tooling, priority-high, effort-small, infra, tooling]
links:
  - "https://github.com/rumblefishdev/stellar-prices-api/pull/6"
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Task created to initialize Nx workspace before service code lands."
  - date: 2026-05-11
    status: active
    who: okarcz
    note: "Activated to bootstrap the Nx workspace."
  - date: 2026-05-11
    status: completed
    who: claude
    note: >
      Bootstrapped Nx 22.7.0 integrated workspace via PR #6 (merged as
      ab05514). 7 files added (+5142 lines, mostly package-lock.json).
      No apps/libs generated. Spawned 0007 (runtime framework choice)
      and 0008 (CI workflow) from emerged future work.
---

# Bootstrap Nx monorepo workspace (@rumblefish/stellar-prices-api)

## Summary

Initialized this repository as an Nx integrated monorepo named `@rumblefish/stellar-prices-api`,
using the TypeScript (`ts`) preset and `npm` as the package manager. Lays the workspace
foundation so subsequent feature tasks can add apps and libs.

## Status: Completed

Merged via PR #6 (`ab05514`) into `develop`.

## Context

The `stellar-prices-api` repository will host the Stellar prices API service plus supporting
libraries (ingestion, schema, venue attribution — see archived tasks 0001–0005). Before any
service code landed we needed a workspace layout that:

- supports multiple apps/libs under one repo,
- gives us a single source of truth for TS config, lint, and formatting,
- integrates cleanly with the existing `lore/`, `docs/`, `tools/` directories at the repo root.

Nx with the `ts` preset gives an integrated TypeScript workspace without framework opinions
(no Express/Nest pre-wiring), which matches the open question of which runtime/framework the
service will eventually use.

## Implementation

Generated the workspace in `/tmp/nx-bootstrap` via:

```
npx create-nx-workspace@latest stellar-prices-api \
  --preset=ts --packageManager=npm \
  --nxCloud=skip --ci=skip --interactive=false --no-formatter
```

Copied the essential workspace files into the repo root, patched the workspace name to
`@rumblefish/stellar-prices-api`, merged `.gitignore`, and ran `npm install`.

### Files added / modified

| File | Origin | Notes |
|------|--------|-------|
| `nx.json` | Nx generator | Verbatim |
| `tsconfig.base.json` | Nx generator | Verbatim |
| `tsconfig.json` | Nx generator | Verbatim |
| `package.json` | Nx generator | `name` patched `@org/source` → `@rumblefish/stellar-prices-api` |
| `package-lock.json` | Nx generator | `name` patched in all 2 occurrences |
| `packages/.gitkeep` | Nx generator | Verbatim |
| `.gitignore` | merged | +3 Nx-specific patterns |

### Toolchain

- Nx 22.7.0 + `@nx/js` 22.7.0
- TypeScript 5.9.3
- npm workspaces under `packages/*` (empty)
- 344 npm packages installed

### Verification

- `npm pkg get name` → `"@rumblefish/stellar-prices-api"` ✓
- `npx nx report` clean ✓
- `npx nx show projects` → `[]` (empty workspace, expected) ✓

## Acceptance Criteria

- [x] `package.json` exists with `"name": "@rumblefish/stellar-prices-api"` and Nx deps
- [x] `nx.json` and `tsconfig.base.json` present at repo root
- [x] `npm install` completes without errors
- [x] `npx nx report` runs cleanly
- [x] Existing `lore/`, `docs/`, `tools/`, root config files are preserved
- [x] `.gitignore` merged (Nx + lore entries both present)

## Design Decisions

### From Plan

1. **`ts` preset over framework presets**: Integrated TypeScript workspace with no framework
   opinion. Framework (Express/Fastify/Nest) is a separate decision once the API surface is
   scoped — see 0007.

2. **npm over pnpm/yarn/bun**: Existing repo `.npmrc` already configures the GitHub Packages
   registry for `@rumblefishdev`. Sticking with npm avoids re-wiring auth.

3. **Generate in temp dir, copy in**: `create-nx-workspace` requires an empty target. Generated
   in `/tmp/nx-bootstrap` and merged in to preserve existing `lore/`, `docs/`, `tools/`,
   `CLAUDE.md`, `.claude/`, `.vscode/`, `.prettier*`, `.npmrc`.

### Emerged

4. **Workspace name patched post-generation**: `create-nx-workspace@22.7.1` rejects scoped
   names like `@rumblefish/stellar-prices-api` (`INVALID_WORKSPACE_NAME`: "Workspace names
   must start with a letter"). Generated with plain `stellar-prices-api`, then patched the
   `name` field in `package.json` and `package-lock.json` after copy. Plan didn't anticipate
   this validation.

5. **Omitted all non-Claude agent configs**: Nx 22 generator now emits `.agents/`, `.codex/`,
   `.cursor/`, `.gemini/`, `.opencode/`, `opencode.json`, plus its own `AGENTS.md` /
   `CLAUDE.md` / `README.md`. All skipped to avoid conflicting with the existing root
   `CLAUDE.md` and the project's Claude Code setup. Plan didn't account for these — they're
   new in recent Nx versions.

6. **Omitted Nx-generated `.github/`**: The generator now bundles skill files
   (`nx-generate`, `nx-import`, `nx-plugins`, `nx-run-tasks`, `nx-workspace`, `monitor-ci`)
   and a `ci.yml` workflow. Left for follow-up (see 0008) — CI is a separate decision and
   the bundled workflow has opinions about the test/build pipeline that don't apply yet.

7. **`.gitignore` merged additively**: Existing repo `.gitignore` was richer (lore session
   files, CDK, Rust, Docker overrides). Added only the 3 missing Nx-specific patterns
   (`.nx/self-healing`, `.cursor/rules/nx-rules.mdc`, `.github/instructions/nx.instructions.md`)
   rather than replacing.

8. **`.prettierrc`/`.prettierignore` kept as-is**: Existing `.prettierrc` had identical content
   to generated (`{"singleQuote": true}`); existing `.prettierignore` was already more tailored
   (includes `AGENTS.md`, `CLAUDE.md`). No reason to overwrite.

## Issues Encountered

- **Scoped workspace name rejected**: First attempt with `@rumblefish/stellar-prices-api`
  failed with `INVALID_WORKSPACE_NAME`. Root cause: Nx 22's `create-nx-workspace` validates
  names against `^[a-zA-Z]` prefix. Fix: generate with a plain slug and patch the `name`
  field in `package.json` and `package-lock.json` afterward. Not a regression — generator
  behaviour.

- **`git mv` from `backlog/` to `active/` failed during promotion**: The freshly created
  task file wasn't tracked yet, so `git mv` aborted with "not under version control". Used
  plain `mv` followed by `git add` on the destination. Cosmetic, not a regression.

- **npm audit findings on a fresh install**: 7 vulnerabilities (6 moderate, 1 high) reported
  by `npm install` for the generator-pinned deps. Not actionable here — they came from
  upstream. See 0009 if/when it becomes worth chasing.

## Future Work

Spawned as backlog tasks (see frontmatter `related_tasks`):

- **0007** — Choose runtime framework and generate first app.
- **0008** — Decide on CI workflow (adopt Nx-bundled `.github/`, write fresh, or none).

Mentioned but not spawned (low-value at this stage):

- npm audit findings (6 moderate, 1 high) on generator-pinned deps. Track via `npm audit`
  when adding real code.
