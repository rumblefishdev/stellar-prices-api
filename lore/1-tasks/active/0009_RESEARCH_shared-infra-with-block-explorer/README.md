---
id: "0009"
title: "Research shared infrastructure architecture with Soroban Block Explorer"
type: RESEARCH
status: active
related_adr: []
related_tasks: ["0007", "0008"]
tags: [priority-high, effort-medium, infra, architecture, aws, shared-infra, block-explorer]
links:
  - "../../../../docs/prices-api-general-overview.md"
  - "../../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md"
  - "../../../../../soroban-block-explorer/docs/architecture/technical-design-general-overview.md"
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Task drafted to research shared-infra architecture with soroban-block-explorer."
  - date: 2026-05-11
    status: active
    who: okarcz
    note: "Promoted from backlog to active"
---

# Research shared infrastructure architecture with Soroban Block Explorer

## Summary

Analyze the Prices API's infrastructure requirements and produce a concrete plan for sharing
AWS infrastructure with the already-funded Soroban Block Explorer (deployed in the same
dedicated AWS sub-account). Output is a reasoned synthesis with one or more recommended
solutions for how the two services co-deploy, share resources, and remain operationally
decoupled.

## Status: Active

**Current state:** Drafted from the second-round-reviewed design doc
(`docs/prices-api-general-overview.md`) which explicitly catalogues shared infra in §0 and
§11. No analysis has been written yet.

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

- [ ] All four input documents read and summarised in `notes/R-*.md`
- [ ] Component matrix written (shared / owned / coupled) covering every component in §2 of
      the Prices API design
- [ ] At least one concrete CDK + CI/CD integration approach proposed with trade-offs
- [ ] Cross-service coupling risks (esp. `soroban_events` read-only dependency) explicitly
      documented with mitigations
- [ ] Final `notes/S-shared-infra-recommendation.md` written with a clear recommendation
- [ ] Follow-up backlog tasks spawned (e.g. ADR for CDK ownership, infra-bootstrap task)

## Notes

- Today's date: 2026-05-11. Backfill cost estimates and tranche dates in the Prices API doc
  may shift; capture as research, not as commitments.
- Existing related backlog: `0007` (runtime framework) and `0008` (CI workflow) — coordinate
  recommendations so the CI/CD integration story aligns with `0008`.
- Research artefacts go under `notes/` using the Q/I/R/S/G prefix convention. Keep this
  README short; promote synthesis content to `notes/S-*.md`.
