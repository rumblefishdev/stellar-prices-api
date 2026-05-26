---
id: '0043'
title: 'Fix package-lock.json mismatch — missing @emnapi/{core,runtime}@1.10.0'
type: BUG
status: completed
related_adr: []
related_tasks: ['0041']
tags:
  ['layer-tooling', 'phase-now', 'effort-small', 'priority-medium', 'ci', 'npm']
links:
  - 'https://github.com/rumblefishdev/stellar-prices-api/actions/runs/26035530230'
  - 'https://github.com/rumblefishdev/stellar-prices-api/pull/18'
history:
  - date: 2026-05-18
    status: backlog
    who: claude
    note: 'Spawned from 0041 — first deploy-board run failed on npm ci due to this pre-existing mismatch. 0041 sidestepped it by dropping npm ci (zero-dep script); future workflows that need real deps will hit it.'
  - date: 2026-05-18
    status: active
    who: oski
    note: 'Activated to regenerate package-lock.json.'
  - date: 2026-05-18
    status: completed
    who: claude
    note: >
      Regenerated lockfile (+38/-12) via full
      `rm -rf node_modules package-lock.json && npm install`. Adds the
      two missing @emnapi entries plus four dev-only patch bumps.
      Verified with docker node:22.22.0 (matches CI exactly) before
      pushing. Shipped via PR #18. No follow-ups.
---

# Fix package-lock.json mismatch — missing @emnapi/{core,runtime}@1.10.0

## Summary

`npm ci` failed on CI with `Missing: @emnapi/core@1.10.0` and `Missing: @emnapi/runtime@1.10.0`. Regenerated `package-lock.json` from scratch so resolution would actually include those entries (in-place `npm install` left the lockfile unchanged because local resolution didn't need them). Verified with Docker on Node 22.22.0 before pushing. Shipped via PR #18 (commit `5ca8039`, merged into develop as `24a7571`).

## Context

- First run of `deploy-board.yml` after PR #16 merged: failed on `npm ci` in 21s (run 26035530230).
- Error: `npm ci can only install packages when your package.json and package-lock.json or npm-shrinkwrap.json are in sync`.
- Missing entries: `@emnapi/core@1.10.0` and `@emnapi/runtime@1.10.0`, nested under `@oxc-resolver/binding-wasm32-wasi`.
- 0041 sidestepped this by dropping `npm ci` from the board workflow (zero-dep generator). 0043's job was to fix the lockfile itself for any future workflow that needs deps.

## Implementation Notes

PR #18 contains a single commit (`5ca8039`) modifying only `package-lock.json` (+38/-12):

- Added the two previously-missing `@emnapi/{core,runtime}@1.10.0` entries.
- Incidental patch bumps to dev-only browser-data packages: `caniuse-lite` (1.0.30001792 → 1.0.30001793), `electron-to-chromium` (1.5.353 → 1.5.357), `node-releases`, `baseline-browser-mapping` (2.10.29 → 2.10.30). All transitive to nx/typescript/eslint tooling; no direct-dep changes.

Verified end-to-end:

```bash
docker run --rm -v "$PWD:/app" -w /app node:22.22.0 npm ci
```

That matches CI's environment exactly (npm 10.9.4 bundled with Node 22.22.0). The deploy-board run on the merge commit also went green, confirming nothing else broke.

## Acceptance Criteria

- [x] `npm ci` succeeds in a clean checkout (verified locally with npm 11.6.1 and via Docker with npm 10.9.4)
- [x] Diff is reviewed — no major-version bumps; only the two missing entries and dev-only patch bumps
- [ ] PR-time CI guard added that runs `npm ci` — **deferred**. Original task marked this as optional, and out of scope for a one-line lockfile fix. Worth considering if drift recurs.

## Design Decisions

### From Plan

1. **Full regenerate, not in-place `npm install`.** Plan listed both as possible approaches. Tried in-place first; it produced zero diff (npm 11 saw the lockfile as fine for local Linux x64 resolution). Escalated to full regenerate to force the resolver to recompute the whole tree, which surfaced the missing nested entries.

### Emerged

2. **Verified with Docker before pushing.** Plan said "Confirm `npm ci` succeeds locally on the new lockfile." Local npm is 11.6.1; CI is 10.9.4. Different npm versions disagree on lockfile validity — exactly the class of bug we were fixing. Running `docker run --rm -v $PWD:/app -w /app node:22.22.0 npm ci` made the verification match CI exactly. Saved at least one round-trip of "push, watch CI, fix, push again." Worth remembering as a pattern for any future lockfile work.

3. **Docker leaves `node_modules` owned by root.** Cleaned up via `docker run … rm -rf node_modules` (no sudo needed). Worth noting because a careless follow-up `rm -rf node_modules` after a docker session will hit a permissions error.

4. **No CI guard spawned.** Plan listed this as optional. Skipped because (a) the underlying drift came from a 2-week-old lockfile getting stale against npm's resolution rules, not from a regression introduced by a PR; and (b) the only workflow currently running `npm ci` was the one this task fixes. Reconsider if a second workflow lands.

## Issues Encountered

- **In-place `npm install` produced zero diff.** Initial hypothesis was that the lockfile just needed reconciliation. After `npm install` the working tree was clean — npm 11 was happy with the existing 1.4.5 entries because local Linux x64 doesn't actually need the `wasm32-wasi` nested deps. The CI failure was specific to CI's lockfile-consistency check, not to runtime resolution. Full regenerate fixed it.

- **Lockfile last touched in 93295b8 (May 4) — 2 weeks of stale resolution rules.** Not a code problem; just transitive registry drift over time. A `npm install` cadence (or a CI guard) would prevent this kind of latent rot.
