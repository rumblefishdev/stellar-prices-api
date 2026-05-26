---
title: "Shared / owned / coupled component matrix"
type: research
status: mature
tags: [matrix, infra, sharing]
links: []
history:
  - date: 2026-05-11
    status: mature
    who: okarcz
    note: "Cross-checked Prices API design against BE actual state."
---

# Shared / owned / coupled component matrix

Cross-references `R-prices-api-infra-requirements.md` (what Prices API needs) against
`R-block-explorer-infra-state.md` (what BE actually runs).

## Legend

| Marker | Meaning |
|---|---|
| ✅ | Plan matches BE reality; integration straightforward |
| ⚠️ | Plan needs revision or verification before integration |
| ❌ | Plan contradicts BE reality; redesign required |

## Matrix

| # | Component | Prices API design intent | BE actual state | Verdict |
|---|---|---|---|---|
| 1  | AWS sub-account                    | Same as BE                                                       | Dedicated Rumble Fish sub-account, `us-east-1a`                       | ✅ |
| 2  | VPC + private subnets              | Deploy Prices RDS + Lambdas in BE VPC                            | VPC documented, private subnets for Lambda/RDS/Galexie                | ✅ |
| 3  | Galexie ECS Fargate task           | Reuse; add second S3 event target                                | 1 continuous Fargate task; S3 → Lambda already wired                  | ✅ |
| 4  | S3 `stellar-ledger-data/`          | Add Prices Lambda as second `s3:ObjectCreated:*` notification   | Bucket exists; **no lifecycle rule** (ADR 0006) so files persist     | ✅ |
| 5  | NAT Gateway                        | Reuse BE's; cost saving claimed $35/mo                            | Implied by VPC topology but **not explicitly enumerated** in BE doc   | ⚠️ |
| 6  | ECS Fargate cluster (multi-tenant) | Run SDEX + AMM backfill tasks in BE cluster                       | Cluster exists implicitly for Galexie; **not documented as shared**   | ⚠️ |
| 7  | SDEX backfill (continuous Fargate) | New 2 vCPU/4 GB task, ~13+ weeks continuous                       | BE chose **local CLI over Fargate** (ADR 0010); pattern not in BE prod | ❌ |
| 8  | Soroban AMM backfill (one-time)    | Read BE `soroban_events` table for decoded JSONB                  | Table is `soroban_events_appearances`; full event detail fetched read-time (ADR 0033) | ❌ |
| 9  | `stellar-xdr` parser crate         | Shared workspace crate                                            | `crates/xdr-parser` exists; built around `stellar-xdr`                | ✅ (with caveat: cross-repo workspace requires decision) |
| 10 | RDS — Prices schema                | Own instance (`db.t4g.micro` → `db.m6g.large` during backfill)    | BE RDS is `db.r6g.large` Single-AZ; Prices owns separate instance     | ✅ |
| 11 | Cross-RDS read access              | Soroban AMM backfill reads BE RDS read-only over VPC              | Same VPC available; IAM/SG must permit                                | ⚠️ (depends on row 8 outcome) |
| 12 | API Gateway, Lambdas (API + workers)| Own deployment                                                   | Independent of BE                                                     | ✅ |
| 13 | EventBridge schedules              | Own rules                                                         | Independent                                                            | ✅ |
| 14 | Secrets Manager                    | Own entries; reference BE secrets only if needed                  | BE uses Secrets Manager; ADR 0001 model directly applicable           | ✅ |
| 15 | CloudWatch + X-Ray                 | Own dashboards; reuse alarm patterns                              | BE alarm patterns documented (§3.7 BE)                                | ✅ |
| 16 | CDK (TypeScript) IaC               | "Shared CDK app with Block Explorer stacks" (Prices §8)           | BE CDK at `infra/aws-cdk/`; ADR 0001 binds patterns                   | ⚠️ (single-repo vs. multi-repo decision required) |
| 17 | CI/CD GitHub Actions               | "Shared pipeline" (Prices §8)                                     | BE: OIDC + per-env IAM roles (ADR 0001)                                | ✅ (model reusable; "shared" pipeline shape needs decision) |
| 18 | Public repo + secret discipline    | Open source per Prices §0                                         | BE is public; ADR 0001 codifies discipline                            | ✅ |

## Summary of mismatches

**Hard mismatches (❌):**

- **Row 7 — SDEX backfill on Fargate:** Prices API design assumes a multi-week Fargate
  task in BE's "shared cluster." BE explicitly rejected Fargate backfill (ADR 0010).
  Either:
  (a) Prices API stands up its own Fargate cluster + task (no longer "shared," no cluster
      saving),
  (b) Prices API adopts BE's local-CLI pattern (workstation-bound; not appropriate for a
      13-week continuous run that survives developer reboots),
  (c) Prices API runs SDEX backfill on Lambda invoked from a step function or a long-
      running EC2/ECS-on-EC2 alternative — needs design.

- **Row 8 — `soroban_events` table shortcut:** Plan assumes BE's table holds decoded JSONB
  topics/data. BE's actual table is `soroban_events_appearances` with detail fetched
  read-time from public archive (ADR 0033). The Soroban AMM "fast" backfill stream may
  need to become an archive read instead of a DB read — collapsing it into the SDEX stream
  shape.

**Verifications needed (⚠️):**

- **Row 5 — NAT Gateway** existence and cost share with BE.
- **Row 6 — ECS cluster** willingness to host another team's tasks.
- **Row 11 — Cross-VPC RDS read** depends on row 8 verdict.
- **Row 16 — CDK ownership** is a fork in the road for the whole project.

## What is genuinely shared without rework

Rows 1, 2, 3, 4, 9 (caveat), 14, 15, 17, 18. That covers Galexie + S3 + VPC + secret model
+ CI/CD pattern. The infrastructure-sharing story still works; just not the parts that
assumed BE had cluster/Fargate-backfill resources to lend or pre-decoded event JSONB to
read.

## Cost-saving impact

Of Prices API §11.1's claimed `~$71/mo` shared-component saving:

| Row in §11.1 | Saving | Reality |
|---|---|---|
| Galexie ECS Fargate | ~$36/mo | ✅ valid |
| S3 bucket | ~$2/mo | ✅ valid |
| VPC | ~$0 (one-time) | ✅ valid |
| NAT Gateway | ~$35/mo | ⚠️ depends on row 5 verification |
| ECS Fargate cluster (overhead) | ~$0 | ⚠️ valid only if BE is willing to host Prices tasks |
| BE `soroban_events` (read-only) | ~$0 | ❌ saving claim unaffected, but the *function* that uses it doesn't work as designed |

Most of the dollar savings still hold; the design changes needed are in execution shape,
not in the cost story.
