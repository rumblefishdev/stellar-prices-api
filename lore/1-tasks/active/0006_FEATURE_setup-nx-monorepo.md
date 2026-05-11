---
id: "0006"
title: "Bootstrap Nx monorepo workspace (@rumblefish/stellar-prices-api)"
type: FEATURE
status: active
related_adr: []
related_tasks: []
tags: [priority-high, effort-small, infra, tooling]
links: []
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Task created to initialize Nx workspace before service code lands."
  - date: 2026-05-11
    status: active
    who: okarcz
    note: "Activated to bootstrap the Nx workspace."
---

# Bootstrap Nx monorepo workspace (@rumblefish/stellar-prices-api)

## Summary

Initialize this repository as an Nx integrated monorepo named `@rumblefish/stellar-prices-api`,
using the TypeScript (`ts`) preset and `npm` as the package manager. The repo currently has no
`package.json` and only carries documentation, lore, and tooling — this task lays the workspace
foundation so subsequent feature tasks can add apps and libs.

## Status: Active

**Current state:** Greenfield. No `package.json`, `nx.json`, or `tsconfig.base.json` yet.

## Context

The `stellar-prices-api` repository will host the Stellar prices API service plus supporting
libraries (ingestion, schema, venue attribution — see archived tasks 0001–0005). Before any
service code lands we need a workspace layout that:

- supports multiple apps/libs under one repo,
- gives us a single source of truth for TS config, lint, and formatting,
- integrates cleanly with the existing `lore/`, `docs/`, `tools/` directories at the repo root.

Nx with the `ts` preset gives an integrated TypeScript workspace without framework opinions
(no Express/Nest pre-wiring), which matches the open question of which runtime/framework the
service will eventually use.

## Implementation Plan

### Step 1: Run `create-nx-workspace` in place

Use `npx create-nx-workspace@latest` with:
- name: `@rumblefish/stellar-prices-api`
- preset: `ts`
- package manager: `npm`
- no Nx Cloud (decide later)

Because the repo is non-empty, run the generator in a sibling temp directory and copy the
generated files in, preserving existing `lore/`, `docs/`, `tools/`, `.git/`, `CLAUDE.md`,
`.prettierrc`, `.prettierignore`, `.npmrc`, `.vscode/`.

### Step 2: Reconcile root files

Merge generated `.gitignore` with the existing one (keep `.temp/`, lore-related entries).
Verify `.prettierrc`/`.prettierignore` are kept (not overwritten).

### Step 3: Verify workspace

- `npm install` succeeds
- `npx nx report` runs cleanly
- `npx nx graph --file=.temp/graph.json` produces a graph (empty is fine)

## Acceptance Criteria

- [ ] `package.json` exists with `"name": "@rumblefish/stellar-prices-api"` and Nx deps
- [ ] `nx.json` and `tsconfig.base.json` present at repo root
- [ ] `npm install` completes without errors
- [ ] `npx nx report` runs cleanly
- [ ] Existing `lore/`, `docs/`, `tools/`, root config files are preserved
- [ ] `.gitignore` merged (Nx + lore entries both present)

## Notes

- No app/lib is generated as part of this task — only the workspace shell.
- Framework choice (Express/Fastify/Nest) is deferred to a follow-up task once the API
  surface is scoped.
