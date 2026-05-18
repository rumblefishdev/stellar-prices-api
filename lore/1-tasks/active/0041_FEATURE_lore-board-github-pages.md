---
id: "0041"
title: "Lore Board on GitHub Pages — mirror soroban-block-explorer setup"
type: FEATURE
status: active
related_adr: []
related_tasks: []
tags: ["layer-tooling", "phase-now", "effort-small", "priority-medium"]
links:
  - "../../board.html"
  - "../../../workflows/deploy-board.yml"
  - "https://github.com/rumblefish-rnd/soroban-block-explorer/blob/develop/tools/scripts/generate-lore-board.mjs"
  - "https://github.com/rumblefish-rnd/soroban-block-explorer/blob/develop/.github/workflows/deploy-board.yml"
history:
  - date: 2026-05-18
    status: backlog
    who: claude
    note: "Task created. lore/board.html and workflows/deploy-board.yml already copied verbatim from soroban-block-explorer; remaining work is to wire the generator script, npm script, workflow location, and Pages enablement."
  - date: 2026-05-18
    status: active
    who: oski
    note: "Activated to start implementation."
---

# Lore Board on GitHub Pages — mirror soroban-block-explorer setup

## Summary

Stand up the static HTML backlog board (`lore/board.html` + generated `lore/board.json`) on GitHub Pages for `stellar-prices-api`, mirroring the working setup in `soroban-block-explorer`. The board renders all lore tasks across `backlog/`, `active/`, `blocked/`, and `archive/` as a filterable Kanban-style view, deployed automatically on every push to `develop`.

## Context

`soroban-block-explorer` already runs the same board pattern. Two pieces have been ported verbatim into this repo:

- `lore/board.html` — static viewer (currently still branded "Soroban Block Explorer")
- `workflows/deploy-board.yml` — GitHub Actions workflow (currently at repo-root `workflows/`, **not** under `.github/workflows/`)

Missing pieces compared to the explorer setup:

| Piece | Explorer location | Status here |
|-------|-------------------|-------------|
| Generator script | `tools/scripts/generate-lore-board.mjs` | Missing — no `tools/scripts/` dir |
| npm script | `package.json` → `"board": "node tools/scripts/generate-lore-board.mjs"` | Missing — `scripts: {}` is empty |
| Workflow path | `.github/workflows/deploy-board.yml` | Lives at `workflows/deploy-board.yml` instead |
| Node version pin | `.nvmrc` | Missing (workflow uses `node-version-file: .nvmrc`) |
| Board title/branding | `lore/board.html` `<title>` and `<h1>` | Still says "Soroban Block Explorer" |
| Pages settings | repo Settings → Pages source = GitHub Actions | Not yet enabled (manual repo-admin step) |

The generator script is the only real engineering work — it parses YAML frontmatter from every task file and emits `board.json` for the HTML to consume. It is a self-contained `.mjs` file with no external deps. Layer/tag taxonomy (`layer-research`, `layer-domain`, …) must match what the HTML expects.

## Implementation Plan

### Step 1: Port the generator script

- Copy `tools/scripts/generate-lore-board.mjs` from `soroban-block-explorer` into this repo at the same path.
- Verify the `LAYER_LABELS`/`LAYER_ORDER` taxonomy in the script matches the tags used in this repo's tasks (audit existing task frontmatter). Adjust if `stellar-prices-api` uses different layer tags.
- Output goes to `lore/board.json`. Add `lore/board.json` to `.gitignore` (it's a build artifact; check explorer's `.gitignore` to confirm convention).

### Step 2: Wire npm script

- Add to `package.json`:
  ```json
  "scripts": {
    "board": "node tools/scripts/generate-lore-board.mjs"
  }
  ```
- Confirm `npm run board` produces a valid `lore/board.json` locally and that opening `lore/board.html` in a browser renders the board with real data.

### Step 3: Re-brand `lore/board.html`

- Replace `<title>Soroban Block Explorer — Backlog Board</title>` → `Stellar Prices API — Backlog Board`.
- Update the `<h1>` header text accordingly.
- Grep for any other "Soroban Block Explorer" / "soroban-block-explorer" strings in `board.html` and replace.

### Step 4: Relocate the workflow

- `git mv workflows/deploy-board.yml .github/workflows/deploy-board.yml` (create `.github/workflows/` since it doesn't exist yet).
- Verify the workflow still references `lore/board.html`, `lore/board.json`, and `npm run board` — no path changes needed beyond the move.
- Remove the now-empty `workflows/` directory at repo root.

### Step 5: Add `.nvmrc`

- Add `.nvmrc` pinning Node version matching this repo's dev environment (check the explorer's `.nvmrc`; align if no specific reason to diverge).
- Workflow already references `node-version-file: .nvmrc`, so no workflow edit needed.

### Step 6: Enable GitHub Pages

- In repo Settings → Pages, set source to **GitHub Actions** (this is a one-time manual step requiring repo-admin permission).
- After first successful workflow run on `develop`, confirm the board is reachable at the Pages URL.

## Acceptance Criteria

- [ ] `tools/scripts/generate-lore-board.mjs` exists and produces a valid `lore/board.json` when run locally
- [ ] `npm run board` works
- [ ] `lore/board.html` renders correctly when opened locally (no "Soroban Block Explorer" branding remains)
- [ ] `lore/board.json` is `.gitignore`d
- [ ] `.github/workflows/deploy-board.yml` is in place; `workflows/` at repo root is removed
- [ ] `.nvmrc` exists at repo root
- [ ] GitHub Pages is enabled with "GitHub Actions" as the source
- [ ] A push to `develop` triggers `Deploy Board to GitHub Pages` and produces a live page

## Notes

- The board.html viewer assumes the layer-tag taxonomy in the generator. If `stellar-prices-api` adopts different layer tags, both files must stay in sync. Worth a quick audit during Step 1 — see explorer ADR/history for the original taxonomy rationale.
- Future enhancement: regenerate `lore/board.json` locally via a git hook so the board reflects current state even when the workflow hasn't run. Defer unless we hit friction.
- No ADR needed — this is tooling, not architecture. If the layer taxonomy diverges from explorer for a substantive reason, that's worth an ADR.
