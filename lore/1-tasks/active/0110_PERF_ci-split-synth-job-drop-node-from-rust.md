---
id: "0110"
title: "CI: split synth into its own job so the rust job stops paying for a Node/TS build"
type: PERF
status: active
related_adr: []
related_tasks: ["0077", "0070"]
tags: [layer-ops, ci, cdk, priority-low, effort-small]
links: []
history:
  - date: 2026-07-21
    status: backlog
    who: okarcz
    note: >
      Spawned from the code review of 0077 / PR #134 (finding #7). The synth
      guard added there is correct but structurally misplaced: it made the
      rust job install Node and run a full TypeScript build. Recorded as
      cost-not-correctness at the time and deliberately deferred so the
      correctness fixes could land; this task carries it.
  - date: 2026-08-04
    status: active
    who: akot
    note: >
      Picked up. Starting from the measurement the task asks for: establish what
      fraction of the ~6m20s rust job is actually the Node install + TS build
      before choosing between the artifact-split and won't-do options.
---

# CI: split synth into its own job

## Summary

`make -C infra synth-production` runs at the end of the `rust` job. That target
depends on the Makefile `build` target, which runs
`npx nx build @rumblefish/stellar-prices-api-aws-cdk` — so every `packages/**`
PR now installs Node, runs `npm ci`, and does a full TypeScript build on the
ARM runner before it can synth. A one-line Rust change pays roughly 2–3 minutes
of TypeScript work, and when both jobs fire that same TS build runs twice.

This is a cost problem, not a correctness one. The guard itself works — verified
green on run 29818521440.

## Context

The synth step landed in the `rust` job for a real reason: synth needs the nine
Lambda bootstraps to exist, and only the rust job builds them. Splitting the job
means moving artifacts across a job boundary, which is the whole design question
here rather than an implementation detail.

Relevant files:
- `.github/workflows/ci.yml` — `rust` job, "Synth production app" step and the
  `actions/setup-node` + `npm ci` steps immediately before it.
- `infra/Makefile:4,33-34` — `synth-production: build`, `build:` → `npx nx build`.

## Implementation

Options, roughly in increasing order of effort:

1. **`actions/upload-artifact` + a third `synth` job.** rust builds and uploads
   `target/lambda/**`; a `synth` job needs both `rust` and `typescript`,
   downloads the bootstraps, and runs `make -C infra synth-production`. Cleanest
   separation; costs an artifact round-trip (~110 MB across 9 bootstraps).
2. **Move synth into the `typescript` job**, gated on the bootstraps being
   present — but they are not, so it would need the artifact anyway. Probably
   collapses into option 1.
3. **Give the synth step a leaner build.** If `npx cdk --app "node dist/bin/production.js" synth`
   can run against a prebuilt `dist/`, cache or share it rather than rebuilding.
4. **Do nothing.** 2–3 min on `packages/**` PRs may simply be acceptable.

Measure before choosing: the current rust job is ~6m20s end to end, so establish
what fraction is actually the TS build before optimizing it.

## Acceptance Criteria

- [ ] Synth still runs on every PR that could break it, and still fails on a
      missing asset (reproduce with a bad `*_ASSET_DIR`, as 0077 did).
- [ ] The `rust` job no longer installs Node or runs a TypeScript build, OR the
      measured saving is documented as too small to justify the split and this
      task is closed as won't-do with that number recorded.
- [ ] No regression in the 0077 guards: all 9 assets still built, verified, and
      synthesized; `verify-lambda-assets.sh` still runs on infra-only PRs.

## Out of scope

- The Lambda asset derivation and verification themselves — those landed in 0077
  (PR #134) and are working. This task only moves where synth executes.
- **Adjacent, deliberately not bundled:** `on: push` in `ci.yml` covers `master`
  only while PRs target `develop`, so a `develop` push runs no CI. Largely
  defused by the paths filters now catching the relevant PRs, but it is a
  separate decision about trigger policy (and about branch protection) rather
  than about job cost. Spawn its own task if branch protection ever depends on
  these checks.
