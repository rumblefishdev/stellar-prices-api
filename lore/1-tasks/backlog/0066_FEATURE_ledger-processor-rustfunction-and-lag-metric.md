---
id: "0066"
title: "Ledger Processor: RustFunction CDK + lag metric + rustls dedup"
type: FEATURE
status: backlog
related_adr: ["0006", "0007"]
related_tasks: ["0038"]
tags: [layer-indexing, priority-low, effort-small, lambda, cdk, observability]
links:
  - "../active/0038_FEATURE_prices-ledger-processor-lambda/notes/G-local-prototype-spec.md"
history:
  - date: 2026-06-24
    status: backlog
    who: oski
    note: "Spawned from 0038 future work (packaging + observability + dep hygiene)."
---

# Ledger Processor: RustFunction CDK + lag metric + rustls dedup

## Summary

Three small production-hardening items for the Prices Ledger Processor that
are out of scope for the deploy-deferred build.

## Context

Spawned from task 0038. The Lambda is code-complete behind the `lambda`
feature; these are deploy/observability/dep-hygiene polish.

## Implementation

- **`cargo-lambda-cdk` `RustFunction`**: drop the `Code.fromAsset` seam in
  `infra/.../compute-stack.ts` for synth-time builds (BE's exact shape) once
  the dep is added.
- **`prices.ledger_processor.lag_seconds`**: emit `now() - ledger.closed_at`
  per invocation to CloudWatch (namespace `prices/lambda`) + a >60s-sustained
  alarm (spec §9.6 / §C.8).
- **rustls dedup**: `aws-sdk-s3` pulls rustls 0.21 (older smithy) alongside
  our 0.23.40 (mTLS). Unify to one version to shrink the `provided.al2023`
  ZIP — investigate aws-smithy-http-client TLS feature selection.

## Acceptance Criteria

- [ ] `RustFunction` synth-time build wired (no pre-built asset seam).
- [ ] `lag_seconds` metric + alarm present in the CDK synth.
- [ ] `cargo tree --features lambda` shows a single rustls version.
