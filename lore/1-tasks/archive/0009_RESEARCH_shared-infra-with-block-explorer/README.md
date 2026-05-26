---
id: '0009'
title: 'Research shared infrastructure architecture with Soroban Block Explorer'
type: RESEARCH
status: completed
related_adr: []
related_tasks: ['0007', '0008', '0010', '0011', '0012', '0013']
tags:
  [
    layer-research,
    priority-high,
    effort-medium,
    infra,
    architecture,
    aws,
    shared-infra,
    block-explorer,
  ]
links:
  - '../../../../docs/prices-api-general-overview.md'
  - '../../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md'
  - '../../../../../soroban-block-explorer/docs/architecture/technical-design-general-overview.md'
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: 'Task drafted to research shared-infra architecture with soroban-block-explorer.'
  - date: 2026-05-11
    status: active
    who: okarcz
    note: 'Promoted from backlog to active'
  - date: 2026-05-11
    status: completed
    who: claude
    note: >
      Research complete. 5 notes produced (3 R-, 1 I-, 1 S-).
      Component matrix covers 18 rows; 2 hard mismatches found vs BE
      reality (Fargate backfill cluster does not exist per BE ADR 0010;
      soroban_events table is appearance-only per BE ADRs 0029/0033).
      Recommendation: Option A2+B1+C1+D3->D1 — separate CDK app +
      SSM platform lookups + Prices-owned Fargate backfill +
      verify-then-archive-read for AMM stream. Spawned 4 follow-up
      backlog tasks (0010-0013).
---

# Research shared infrastructure architecture with Soroban Block Explorer

## Summary

Analyze the Prices API's infrastructure requirements and produce a concrete plan for sharing
AWS infrastructure with the already-funded Soroban Block Explorer (deployed in the same
dedicated AWS sub-account). Output is a reasoned synthesis with one or more recommended
solutions for how the two services co-deploy, share resources, and remain operationally
decoupled.

## Status: Completed

**Outcome:** Recommendation delivered as `notes/S-shared-infra-recommendation.md`. Four
follow-up backlog tasks (0010–0013) carry the implementation work. Two material
inaccuracies in `docs/prices-api-general-overview.md` flagged for revision (task 0013).

### Headline findings

1. The cost-saving sharing story for Galexie + S3 + VPC + (probably) NAT Gateway holds. ✅
2. The "shared ECS cluster for backfill" assumption in §2.3 / §5.6 / §11.1 is incorrect:
   BE rejected Fargate backfill in favor of a local CLI per
   [BE ADR 0010](../../../../../soroban-block-explorer/lore/2-adrs/0010_local-backfill-over-fargate.md).
   Prices API must own its backfill execution layer. ❌ → reshape needed.
3. The "read BE `soroban_events` table for decoded JSONB topics/data" plan in §5.6 may
   not work — BE's table is `soroban_events_appearances` and full event detail is fetched
   read-time from public archive per
   [BE ADR 0033](../../../../../soroban-block-explorer/lore/2-adrs/0033_soroban-events-appearances-read-time-detail.md).
   Spike needed before committing to the two-stream backfill plan. ⚠️
4. Recommended integration shape: separate CDK app + SSM-based platform lookups + own
   GitHub Actions OIDC + Prices-owned Fargate for SDEX backfill (Option \*\*A2 + B1 + C1
   - D3→D1\*\* from `notes/I-integration-options.md`).

## Context

The Prices API design (post-2nd review) commits to deploying into the **same AWS sub-account
as the Soroban Block Explorer** and reusing several core components — Galexie ECS Fargate,
S3 ledger bucket, VPC, NAT Gateway, ECS cluster, and read-only access to the Block Explorer's
`soroban_events` table (see §2.3 and §11 of `prices-api-general-overview.md`).

The Block Explorer team has already documented its target infrastructure in
`../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md`.
That doc defines: account model (dedicated sub-account, `us-east-1a`), VPC topology,
managed-component choices (ECS Fargate Galexie, Lambda Ledger Processor, RDS,
API Gateway, CloudFront, WAF, Secrets Manager), CI/CD via GitHub Actions → CDK, and the
public/private boundary.

We need a clear research output that maps Prices API requirements onto that existing model
and proposes how to integrate without duplicating cost or breaking the Block Explorer's
operational assumptions.

## Research Plan

### Step 1: Catalogue inputs

- Read `docs/prices-api-general-overview.md` end-to-end (especially §0, §1, §2.3, §5, §11).
- Read `../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md`
  end-to-end.
- Skim `../soroban-block-explorer/docs/architecture/technical-design-general-overview.md` and
  any related ADRs referenced from the infrastructure doc (e.g. ADR 0010, 0027, 0029).

### Step 2: Map shared vs. owned components

Build a side-by-side matrix of every infra component the Prices API needs, marking each as:

- **Shared** (Block Explorer owns/funds; Prices API consumes)
- **Owned** (Prices API funds and operates separately)
- **Coupled** (cross-service dependency such as the read-only `soroban_events` access)

Cross-check against §2.3 / §11.1 / §11.3 of the Prices API doc for double-billing risk.

### Step 3: Identify integration points and risks

For each shared/coupled component, document:

- How the integration works at the AWS level (e.g. second S3 event-notification target on
  `stellar-ledger-data/`, second consumer in the same VPC private subnet, additional ECS
  task definition in the existing cluster)
- What boundary Block Explorer's design assumes that Prices API must respect (IAM scoping,
  no writes to BE schema, VPC subnet allocation, security-group rules)
- Failure-mode coupling (what breaks for whom if the shared component degrades)

### Step 4: Propose solutions

Draft 1–3 candidate integration shapes. For each: describe the deployment topology, CDK
ownership model (single shared CDK app vs. two apps with cross-stack references vs.
SSM-parameter handoff), CI/CD model, environment parity (dev/staging/prod), and trade-offs.

### Step 5: Synthesis README

Write the final summary as `notes/S-shared-infra-recommendation.md` with:

- TL;DR recommendation
- Component-by-component matrix (shared/owned/coupled)
- Recommended CDK + CI/CD shape
- Open questions / decisions still owed to humans
- Suggested follow-up tasks (spawn as backlog entries)

## Acceptance Criteria

- [x] All four input documents read and summarised in `notes/R-*.md`
      (`R-prices-api-infra-requirements.md`, `R-block-explorer-infra-state.md`,
      `R-shared-vs-owned-matrix.md`)
- [x] Component matrix written (shared / owned / coupled) covering every component in §2 of
      the Prices API design — see `notes/R-shared-vs-owned-matrix.md`
- [x] At least one concrete CDK + CI/CD integration approach proposed with trade-offs
      (three dimensions × multiple options in `notes/I-integration-options.md`)
- [x] Cross-service coupling risks (esp. `soroban_events` read-only dependency) explicitly
      documented with mitigations (see findings #2 and #3 above; full analysis in the
      matrix and synthesis notes)
- [x] Final `notes/S-shared-infra-recommendation.md` written with a clear recommendation
- [x] Follow-up backlog tasks spawned: 0010 (verify BE event schema), 0011 (CDK bootstrap),
      0012 (SDEX/AMM backfill design), 0013 (update design doc to match BE reality)

## Implementation Notes

- Inputs catalogued: `docs/prices-api-general-overview.md` (Prices §0–§11);
  BE `infrastructure-overview.md`, `technical-design-general-overview.md`; BE ADRs
  0001 (OIDC + secrets), 0006 (no S3 lifecycle), 0007 (2-Lambda architecture),
  0010 (local backfill, NOT Fargate), 0029 (read-time XDR fetch).
- Notes layout: 3× R- (research), 1× I- (idea/options), 1× S- (synthesis).
- Two design-doc assumptions found to be wrong against BE reality (rows 7 and 8 of the
  matrix); both materially change the backfill execution plan.

## Design Decisions

### From Plan

1. **Notes layout follows lore Q/I/R/S/G convention.** R- for distilled inputs, I- for
   options sketches, S- for the final synthesis. Matches the `_note_template.md` guidance.

### Emerged

2. **Spawned 4 follow-ups, not 5.** The synthesis note initially listed 5 candidates
   (including a standalone "ADR for CDK ownership" task). Collapsed the ADR into task
   0011's acceptance criteria — the ADR is a deliverable of the bootstrap work, not a
   separate parallel task. Keeps the backlog smaller and avoids ADR-without-implementation.

3. **Picked Option A2+B1+C1+D3→D1 as a recommendation rather than presenting all options
   neutrally.** The matrix made one combination clearly best (separate CDK app + own
   OIDC + own Fargate + verify-then-archive-read). Hedging would have left the next
   reader without a starting point. The trade-off table for each dimension is preserved
   in `I-integration-options.md` so the decision can be revisited.

4. **Did not write any ADRs in this repo.** Research only — the cross-team decisions
   (NAT cost share, CDK config sharing mechanism) require human input from the BE team
   before an ADR can be authoritative. Listed as open questions in the synthesis note.

## Future Work

Spawned as backlog tasks (see `notes/S-shared-infra-recommendation.md` open questions):

- **0010** — Verify BE `soroban_events_appearances` schema for Prices AMM backfill
- **0011** — Bootstrap Prices-owned CDK app with SSM-based platform lookups
- **0012** — Design SDEX + AMM backfill on Prices-owned Fargate cluster
- **0013** — Update `prices-api-general-overview.md` §2.3/§5.6/§11 to match BE reality

## Notes

- Today's date: 2026-05-11. Backfill cost estimates and tranche dates in the Prices API doc
  may shift; capture as research, not as commitments.
- Existing related backlog: `0007` (runtime framework) and `0008` (CI workflow) — coordinate
  recommendations so the CI/CD integration story aligns with `0008`.
- Research artefacts go under `notes/` using the Q/I/R/S/G prefix convention. Keep this
  README short; promote synthesis content to `notes/S-*.md`.
