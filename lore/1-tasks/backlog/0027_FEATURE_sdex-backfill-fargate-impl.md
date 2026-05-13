---
id: "0027"
title: "SDEX backfill Fargate impl — land CDK + Rust binary + schema migration per task 0012 design"
type: FEATURE
status: backlog
related_adr: ["0002", "0003"]
related_tasks: ["0011", "0012", "0022"]
tags: [priority-high, effort-large, infra, ecs, fargate, backfill, sdex, stream-2, rust, cdk]
links:
  - "../active/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-fargate-design.md"
  - "../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-filter-strategy.md"
  - "../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
history:
  - date: 2026-05-13
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0012 future work. Implements the operational
      design in 0012's G-note: Cargo workspace + Rust binary, CDK
      Fargate stack, IAM roles, CloudWatch alarm, schema migrations
      (incl. ADR 0003 PK change), runbook, and staging smoke test.
      Blocked on task 0011 (CDK bootstrap) per 0012's design §11.
---

# SDEX backfill Fargate impl — land CDK + Rust binary + schema migration

## Summary

Lands the implementation for the SDEX (Stream 2) backfill task
designed in task 0012. Cargo workspace, Rust binary, CDK Fargate
stack, IAM roles, CloudWatch heartbeat alarm, schema migrations
(`backfill_progress` table + ADR 0003 `quote_asset_id` PK change on
`price_ohlcv`), runbook at `docs/runbooks/backfill-sdex.md`, and a
staging smoke test against a 10 k-ledger range.

## Context

Implements task 0012's
[`notes/G-sdex-backfill-fargate-design.md`](../active/0012_FEATURE_design-prices-owned-backfill-fargate/notes/G-sdex-backfill-fargate-design.md)
clause-by-clause. See that document for:

- Architecture (§1), task definition shape (§3), IAM contract (§4),
  including the **forbidden-actions list** in §4.3 that the CDK
  unit test must assert.
- Resumability semantics (§5).
- Heartbeat metric + 20-min alarm (§6).
- Failure-mode handling (§7), logging (§8), runbook (§9).
- Rust module split → 0022 spec mapping (§10).

Blocked on **task 0011** (CDK bootstrap with SSM platform lookups).
Without 0011, there is no `infra/aws-cdk/` to add the cluster + task
definition to. 0027 starts when 0011 lands.

## Implementation (ordered)

1. **Cargo workspace bootstrap**
   - Workspace root `Cargo.toml` at repo root (or `packages/sdex-backfill/`
     depending on Nx integration choice).
   - `sdex-backfill` binary crate with module layout per 0012 G-note §10:
     `archive`, `decode`, `filter`, `tick`, `canonical`, `price`,
     `bucket`, `checkpoint`, `heartbeat`, `obs`, `main`.
   - `stellar-xdr` Cargo dependency (BE-authored crate per ADR 0002 §3).
   - CI job builds the release binary, packages into a distroless image,
     pushes to ECR tagged with git SHA.

2. **Schema migrations** in the prices-api PG migration tool:
   - `price_ohlcv` PK change per ADR 0003: add `quote_asset_id` column
     and migrate the PK to `(timestamp, asset_id, quote_asset_id, granularity)`.
   - `backfill_progress` table per 0012 G-note §5.1.

3. **CDK additions** under `infra/aws-cdk/` (after task 0011 lands):
   - `prices-backfill-{env}` ECS Fargate cluster.
   - `sdex-backfill-{env}` task definition matching G-note §3.
   - `PricesBackfillExecution-{env}` and `PricesBackfillSDEX-{env}`
     IAM roles matching G-note §4.
   - **CDK unit test asserting G-note §4.3's forbidden actions are
     not present** in the synthesized task-role policy.
   - CloudWatch alarm matching G-note §6.2 wired to SNS topic
     `prices-backfill-alerts-{env}`.
   - CDK code comments referencing ADR 0002, ADR 0003, and 0012's G-note.

4. **Rust binary** with module layout per 0012 G-note §10. Each
   module is reviewable against its cited section of task 0022's
   filter-strategy or decode-and-bucket spec.

5. **Runbook at `docs/runbooks/backfill-sdex.md`** per 0012 G-note §9.

6. **Staging smoke test:** `aws ecs run-task` against a 10 k-ledger
   range; assert `price_ohlcv` rows land, `backfill_progress
   .current_ledger` advances monotonically, the heartbeat metric
   appears in CloudWatch, and the run completes cleanly.

7. **Verify `cargo tree`** resolves `stellar-xdr` to the BE workspace
   source (or pinned version per ADR 0002 §3).

## Acceptance Criteria

- [ ] Cargo workspace + `sdex-backfill` binary crate with module layout
      matching 0012 G-note §10.
- [ ] `stellar-xdr` consumed as a Cargo dependency, verified via `cargo tree`.
- [ ] Schema migrations land: `backfill_progress` table + ADR 0003 PK change.
- [ ] CDK adds `prices-backfill-{env}` cluster, `sdex-backfill-{env}` task
      definition, both IAM roles, CloudWatch alarm, and SNS topic in
      `infra/aws-cdk/`.
- [ ] CDK unit test asserts the task role does NOT contain the actions
      listed in 0012 G-note §4.3.
- [ ] Runbook at `docs/runbooks/backfill-sdex.md` per G-note §9.
- [ ] Staging smoke test: 10 k-ledger range processes end-to-end with
      `price_ohlcv` rows landing and `backfill_progress.current_ledger`
      advancing monotonically.
- [ ] CDK code comments reference ADR 0002, ADR 0003, and 0012's G-note.

## Blocked by

- **0011** — Bootstrap CDK app with SSM platform lookups. Cannot add
  Fargate cluster + task definition until `infra/aws-cdk/` exists.
