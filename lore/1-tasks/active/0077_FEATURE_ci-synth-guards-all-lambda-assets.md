---
id: "0077"
title: "CI should build all 9 Lambda assets + run synth-production so a missing asset fails loudly"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0070", "0056", "0026", "0108"]
tags: [layer-ops, ci, cdk, cargo-lambda, priority-medium, effort-small]
links: []
history:
  - date: 2026-07-03
    status: backlog
    who: oski
    note: >
      Spawned from 0070. During the 0070 deploy prep, `make synth-production`
      failed with CannotFindAsset for target/lambda/backfill-freshness-probe:
      the production CDK app references NINE Lambda assets (ledger-processor,
      api-handler/prices-api, asset-discovery, cleanup, supply, oracle,
      enrichment, backfill-freshness-probe, mtls-notafter-probe) but ci.yml
      builds only SIX (missing the two 0056 probes + prices-api) and never runs
      synth-production, so the drift is invisible until an operator synths by hand.
  - date: 2026-07-20
    status: active
    who: okarcz
    note: >
      Activated from the 0108 grooming sweep, which re-verified the gap and found
      it **worse than described**: CI builds 6 crates but the "Verify Lambda
      artifacts" `expected=()` array lists only **5** — `enrichment-worker` is
      built and never checked, so its bootstrap could silently fail to package
      and still pass CI. Confirmed 9 assets are referenced by the production app
      (7 in eventbridge-stack.ts, 2 in compute-stack.ts).
---

# CI should build all 9 Lambda assets + run synth-production

## Summary

`.github/workflows/ci.yml` builds 6 Lambda bootstraps and verifies those exist,
but the production CDK app references 9. Because CI never runs `synth-production`,
a stack referencing an un-built asset passes CI and only fails when the operator
synths during a deploy (as happened in 0070, 2026-07-03). Close the gap so CI
fails loudly instead.

## Context

- Missing from the CI `cargo lambda build` list: `backfill-freshness-probe`,
  `mtls-notafter-probe` (task 0056), and `prices-api` (the api-handler).
- The "Verify Lambda artifacts" step only checks the 6 it builds.
- Root cause: the build/verify lists are hand-maintained and drifted when 0056 +
  the API handler were wired into the stacks.

## Implementation

- Add the 3 missing crates to the `cargo lambda build … --features lambda` step
  and to the `expected=(…)` verify array in `ci.yml`.
- Add a `make synth-production` (or a CI-env synth) step after the build so a
  future asset/stack mismatch fails CI directly — the single source of truth for
  "did we build everything the app needs" is a successful synth.
- Consider deriving the crate list from one place (Makefile var or a small script)
  so the build list, verify list, and CDK asset dirs can't drift again.

## Acceptance Criteria

- [ ] CI builds all 9 arm64 bootstraps the production app references.
- [ ] CI runs `synth-production` (or equivalent) and fails on any missing asset.
- [ ] Verify array / crate list is single-sourced or otherwise drift-resistant.
