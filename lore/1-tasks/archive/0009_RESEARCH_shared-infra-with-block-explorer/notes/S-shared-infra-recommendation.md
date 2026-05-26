---
title: 'Recommendation — shared infra architecture with Block Explorer'
type: synthesis
status: mature
spawned_from: notes/R-shared-vs-owned-matrix.md
spawns: []
tags: [recommendation, infra, architecture, decision]
links:
  - '../R-prices-api-infra-requirements.md'
  - '../R-block-explorer-infra-state.md'
  - '../R-shared-vs-owned-matrix.md'
  - '../I-integration-options.md'
history:
  - date: 2026-05-11
    status: mature
    who: okarcz
    note: 'Initial recommendation drafted from research notes.'
---

# Recommendation — shared infra architecture with Block Explorer

## TL;DR

Adopt a **separate-CDK-app + selective-resource-sharing** model:

1. **Share** Galexie + S3 bucket + VPC + (verified) NAT Gateway — exactly as Prices API
   §0/§2.3/§11 catalogues. ✅
2. **Do not share** an ECS cluster for backfill. Stand up Prices's own Fargate setup; the
   "shared cluster" line in §11.1 is a fiction in BE's actual topology.
3. **Drop the BE-DB shortcut** for Soroban AMM history; verify schema first, then plan to
   re-derive AMM swaps from public-archive XDR alongside SDEX.
4. **Mirror** BE's CDK + GitHub Actions OIDC patterns in this repo, but keep CDK app and
   pipeline independent (Option A2 + B1 + C1 + D3→D1 from `I-integration-options.md`).

The infrastructure-sharing story still works. The dollar savings still hold (~$36-71/mo).
What changes is the _execution shape_ of two specific design points (backfill cluster,
Soroban AMM source).

---

## Recommended architecture

### Shared with BE (truly multi-tenant resources)

| Resource                  | Sharing mechanism                                                                          |
| ------------------------- | ------------------------------------------------------------------------------------------ |
| AWS sub-account           | Same account; separate IAM boundaries per service                                          |
| VPC + private subnets     | Prices CDK references VPC ID via SSM (`/be/prod/vpc-id`)                                   |
| NAT Gateway               | Same EIP / route table; verify cost model                                                  |
| S3 `stellar-ledger-data/` | Add `s3:ObjectCreated:*` notification → Prices Ledger Processor Lambda. Bucket ARN via SSM |
| Secrets Manager namespace | Independent secrets, common KMS key acceptable                                             |

### Owned by Prices API (independent)

| Resource                                       | Notes                                                                  |
| ---------------------------------------------- | ---------------------------------------------------------------------- |
| RDS PostgreSQL                                 | `db.t4g.micro` baseline, scale to `db.m6g.large` during backfill       |
| All Lambdas (API + workers + Ledger Processor) | Independent IAM roles; same VPC                                        |
| API Gateway + WAF + CloudFront docs            | Independent                                                            |
| EventBridge rules                              | Independent                                                            |
| **ECS Fargate cluster + SDEX backfill task**   | **New** — design assumed BE's, but BE has none for backfill (ADR 0010) |
| CloudWatch dashboards                          | Reuse BE alarm patterns; new namespace                                 |

### Cross-service couplings (kept narrow)

| Coupling                                                    | Mitigation                                                                                                    |
| ----------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| Read public-archive XDR for SDEX backfill                   | Same source BE uses; no BE dependency                                                                         |
| Read public-archive XDR for Soroban AMM backfill (if D3→D1) | Replaces the BE-DB shortcut; eliminates schema-coupling risk                                                  |
| Shared `stellar-xdr` parsing logic                          | Vendor as a workspace dependency or copy patterns; cross-repo Cargo workspace deferred until both teams agree |
| Live S3 event triggers both Lambdas                         | BE's Ledger Processor + Prices Ledger Processor both subscribed; idempotent processing                        |

---

## CDK + CI/CD shape

- This repo gains `infra/aws-cdk/` mirroring BE layout (separate app, same conventions).
- GitHub Actions workflows in this repo, OIDC trust for Prices-specific IAM roles only
  (`stellar-prices-api-deploy-staging`, `stellar-prices-api-deploy-prod`).
- BE publishes platform-level identifiers (VPC ID, S3 bucket name, NAT Gateway EIP, KMS
  key ARN) to SSM Parameter Store. Prices CDK reads them at synth time.
- One-time bootstrap: a small "platform consumer IAM" stack that gives the Prices CDK
  role permission to look up BE's SSM keys and join BE's VPC.

---

## Component matrix (final, recommended)

(See `R-shared-vs-owned-matrix.md` for the underlying analysis.)

| Component                       | Status                                    |
| ------------------------------- | ----------------------------------------- |
| AWS sub-account                 | shared                                    |
| VPC + private subnets           | shared                                    |
| NAT Gateway                     | shared (verify cost model)                |
| Galexie ECS Fargate task        | shared (BE-owned)                         |
| S3 `stellar-ledger-data/`       | shared (BE-owned)                         |
| ECS cluster for Prices backfill | **owned by Prices** (not BE-shared)       |
| SDEX backfill Fargate task      | owned                                     |
| Soroban AMM backfill task       | owned (re-derive from archive, not BE DB) |
| RDS Postgres (Prices schema)    | owned                                     |
| All Lambdas                     | owned                                     |
| API Gateway + WAF               | owned                                     |
| CloudWatch + X-Ray              | owned (patterns reused)                   |
| Secrets Manager entries         | owned                                     |
| `stellar-xdr` parsing           | shared (workspace pattern; mechanism TBD) |
| CDK app                         | owned (Option A2)                         |
| GitHub Actions CI/CD            | owned (Option B1)                         |

---

## Open questions (unresolved by research; need decisions)

1. **NAT Gateway cost model.** Confirm with BE team whether NAT is BE-funded and whether
   another VPC tenant is acceptable from a cost-allocation standpoint.
2. **BE's `soroban_events_appearances` schema.** Spike: read the actual schema in BE's
   repo (and in any production snapshot if available). Determine whether decoded topics/
   data exist anywhere in BE's RDS.
3. **CDK config sharing mechanism.** SSM Parameter Store vs. CFN cross-stack exports
   vs. a private npm package of constructs. SSM is the recommendation; confirm with BE
   team.
4. **Cross-repo Rust workspace** for `stellar-xdr` patterns: vendor a tagged crate, use a
   git submodule, or copy. Affects 0007 (runtime framework) decision.
5. **Stellar-fork redeployability.** Document the Prices-only redeploy path (without BE)
   for §0 compliance.

## Suggested follow-up tasks (to spawn after review)

| ID (next) | Type     | Title                                                                     | Why                                                                              |
| --------- | -------- | ------------------------------------------------------------------------- | -------------------------------------------------------------------------------- |
| 0010      | ADR      | "CDK ownership and cross-stack identifier mechanism (SSM vs. exports)"    | Captures the A2/B1 decision authoritatively                                      |
| 0011      | RESEARCH | "Verify BE `soroban_events_appearances` schema for Prices AMM backfill"   | Resolves the row-8 hard mismatch                                                 |
| 0012      | FEATURE  | "Bootstrap Prices-owned CDK app with SSM-based platform lookups"          | Implements the recommendation                                                    |
| 0013      | FEATURE  | "Design SDEX + AMM backfill on Prices-owned Fargate cluster"              | Replaces the assumed BE cluster                                                  |
| 0014      | DOCS     | "Update prices-api-general-overview.md §2.3/§5.6/§11 to match BE reality" | Removes the Fargate-cluster and `soroban_events` assumptions from the design doc |

These should be spawned as backlog entries before this research task is closed.
