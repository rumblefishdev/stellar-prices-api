---
id: "0043"
title: "Fix package-lock.json mismatch — missing @emnapi/{core,runtime}@1.10.0"
type: BUG
status: active
related_adr: []
related_tasks: ["0041"]
tags: ["layer-tooling", "phase-now", "effort-small", "priority-medium", "ci", "npm"]
links:
  - "https://github.com/rumblefishdev/stellar-prices-api/actions/runs/26035530230"
history:
  - date: 2026-05-18
    status: backlog
    who: claude
    note: "Spawned from 0041 — first deploy-board run failed on npm ci due to this pre-existing mismatch. 0041 sidestepped it by dropping npm ci (zero-dep script); future workflows that need real deps will hit it."
  - date: 2026-05-18
    status: active
    who: oski
    note: "Activated to regenerate package-lock.json."
---

# Fix package-lock.json mismatch — missing @emnapi/{core,runtime}@1.10.0

## Summary

`npm ci` fails on CI with `Missing: @emnapi/core@1.10.0` and `Missing: @emnapi/runtime@1.10.0`. The lockfile is out of sync with `package.json`. The board workflow (0041) sidesteps this by not running `npm ci` at all — but any future workflow that needs to install deps will fail until the lockfile is regenerated.

## Context

- First run of `deploy-board.yml` after PR #16 merged: failed on `npm ci` in 21s.
- Error: `npm ci can only install packages when your package.json and package-lock.json or npm-shrinkwrap.json are in sync`.
- Specific missing entries: `@emnapi/core@1.10.0`, `@emnapi/runtime@1.10.0` (these are sub-deps pulled in by `@swc-node/register` or `@swc/core`).
- Local `node_modules` is built with `npm install` (forgiving), so this never surfaced locally — only on CI's strict `npm ci`.
- Workaround in 0041: dropped `npm ci` from `deploy-board.yml`. Worked because the board generator uses only Node builtins.

## Implementation

1. Locally: `rm -rf node_modules package-lock.json && npm install` to regenerate the lockfile.
2. Inspect the diff carefully — re-locking may pull in patch/minor bumps on transitive deps. Decide whether to commit the full update or pin specific versions if the diff is unexpectedly large.
3. Confirm `npm ci` succeeds locally on the new lockfile (`rm -rf node_modules && npm ci`).
4. Commit as `fix(lore-0043): regenerate package-lock.json` against develop or via PR depending on diff size.
5. (Optional) Add a smoke `npm ci` check to CI so this can't drift silently again — could be a tiny workflow that runs on PRs touching `package.json` or `package-lock.json`.

## Acceptance Criteria

- [ ] `npm ci` succeeds in a clean checkout
- [ ] Diff is reviewed (especially any major-version bumps in transitive deps)
- [ ] Optional: PR-time CI guard added that runs `npm ci`

## Notes

This is a pre-existing issue, not caused by 0041 — but 0041 surfaced it. Bumping priority if/when another workflow needs `npm ci`.
