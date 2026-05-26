---
title: "Soroban Block Explorer — actual infra state and boundaries"
type: research
status: mature
tags: [infra, block-explorer, cross-service]
links:
  - "../../../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md"
  - "../../../../../../soroban-block-explorer/docs/architecture/technical-design-general-overview.md"
  - "../../../../../../soroban-block-explorer/lore/2-adrs/0001_OIDC-cicd-and-public-repo-secret-separation.md"
  - "../../../../../../soroban-block-explorer/lore/2-adrs/0006_no-s3-lifecycle-on-ledger-data.md"
  - "../../../../../../soroban-block-explorer/lore/2-adrs/0007_simplified-2-lambda-architecture.md"
  - "../../../../../../soroban-block-explorer/lore/2-adrs/0010_local-backfill-over-fargate.md"
  - "../../../../../../soroban-block-explorer/lore/2-adrs/0029_abandon-parsed-artifacts-read-time-xdr-fetch.md"
history:
  - date: 2026-05-11
    status: mature
    who: okarcz
    note: "Distilled from BE infrastructure-overview, technical-design-general-overview, ADRs 0001/0006/0007/0010/0029."
---

# Soroban Block Explorer — actual infra state and boundaries

## Source-of-truth precedence

`technical-design-general-overview.md` is authoritative; `infrastructure-overview.md` is the
infra-focused refinement. Where ADRs conflict with prose, ADRs win (newer decisions).

## What BE owns and runs

Per BE `technical-design-general-overview.md` §3.3 and `infrastructure-overview.md` §5:

| Component | Service | Notes |
|---|---|---|
| Galexie | ECS Fargate (1 continuous task) | Live ledger stream → S3 |
| S3 bucket `stellar-ledger-data` | S3 | LedgerCloseMeta XDR; **no lifecycle rule** (ADR 0006) |
| Lambda — Ledger Processor | Lambda (S3 event) | 14-step `persist_ledger`, single-tx writes (ADR 0027) |
| Lambda — Rust/axum API | Lambda (per API GW route) | Read-only; some endpoints fetch public-archive XDR (ADR 0029) |
| RDS PostgreSQL | `db.r6g.large`, Single-AZ | Block-explorer schema (ADR 0027) |
| API Gateway | REST + throttling + caching | |
| AWS WAF | Public-ingress protection | |
| CloudFront | Static React SPA | |
| Route 53, EventBridge, Secrets Manager, CloudWatch + X-Ray | Standard set | |
| CI/CD | GitHub Actions + CDK (TypeScript) | OIDC; per-env IAM roles (ADR 0001) |

**Architecture is 2-Lambda only** (ADR 0007): Ledger Processor + API. No Event Interpreter.

## What BE does NOT operate (contradicts Prices API assumptions)

### 1. No production Fargate backfill task — local CLI only (ADR 0010)

> "Use local backfill via `backfill-bench` CLI tool on a workstation instead of AWS Fargate."

- BE rejected the Fargate backfill plan (its task 0030, superseded).
- Production backfill = `crates/backfill-runner` running on a developer workstation,
  streaming from `s3://aws-public-blockchain/v1.1/stellar/ledgers/pubnet/` over HTTPS,
  writing directly to RDS via SSH/VPN.
- **Implication:** there is **no shared ECS cluster** for "backfill tasks" to land in.
  Prices API §2.3 / §5.6 / §11 assume one exists.

### 2. Galexie is a single ECS task — not "a cluster"

`infrastructure-overview.md` §5.1 + `technical-design-general-overview.md` §3.1 describe
Galexie as "1 continuous Fargate task." A Fargate cluster object exists implicitly to host
that task, but BE does not document it as a multi-tenant "cluster" intended to host other
workloads. Adding Prices API tasks to the same cluster object is technically possible but
needs an explicit decision and capacity/quota review.

### 3. NAT Gateway is implied, not explicitly catalogued

BE infrastructure-overview §6.1 describes a private-subnet runtime and S3 access "through a
VPC endpoint" for Galexie. Egress to non-S3 destinations (Stellar peers, Soroban RPC, etc.)
implies a NAT Gateway, but the doc does not enumerate it as a named cost/component. The
$35/mo "NAT Gateway saving" claimed by Prices API §11.1 needs verification against the
actual BE deployment.

## Critical schema-coupling reality check (ADRs 0029 + 0033/0034)

Prices API §5.6 plans to read BE's `soroban_events` table to extract decoded JSONB topics
and data for AMM swap history. **This needs verification** — BE's read path for events has
been pivoted:

- ADR 0029: BE does not store raw XDR or parsed-event blobs.
- BE schema (per `technical-design-general-overview.md` §3.1 inset) names the table
  **`soroban_events_appearances`** (note suffix), and per ADR 0033 "full event detail
  fetched at read time."
- The "decoded JSONB topics/data" assumed by Prices API may not be stored in BE's
  appearance table — it may need to be re-derived from public-archive XDR at query time.

This is the **#1 risk** for the cross-service `soroban_events` shortcut. Spawned as a
research follow-up — see `S-shared-infra-recommendation.md` open questions.

## CI/CD and secrets model (ADR 0001)

- GitHub Actions assumes AWS roles via OIDC (no long-lived keys).
- Separate IAM roles for staging vs. production with GitHub Environment protections.
- Non-secret config in `infra/aws-cdk/config/*` (BE convention); secret values in Secrets
  Manager / SSM SecureString only.
- Public-repo discipline: nothing in git contains production secrets.

## Environments

| Env | DB | Notes |
|---|---|---|
| Development | Local Postgres | Local + CI |
| Staging | RDS, testnet | Edge-password-protected SPA |
| Production | RDS, mainnet | Public; Multi-AZ deferred until SLA > 99.9% (ADR-staged) |

## Repo layout (BE)

- `crates/*` — Rust workspace (xdr-parser, indexer, api, backfill-runner, backfill-bench)
- `web/` — React SPA
- `infra/aws-cdk/` — CDK app (TypeScript) per ADR 0001 §3
- `lore/` — same lore-framework convention used here

## Takeaways for sharing decisions

1. **S3 + Galexie + VPC sharing is real** and well-documented. Prices API can integrate.
2. **ECS-cluster-for-backfill** assumption needs to be replaced — adopt BE's local-CLI
   pattern OR explicitly stand up a Fargate cluster for the SDEX backfill (whose own
   doc says it runs continuously for ~13+ weeks, which is not BE's pattern).
3. **`soroban_events` JSONB shortcut** needs verification before §5.6 Stream 1 plan is
   credible. If BE only stores appearances, the Soroban AMM backfill must read public
   archive XDR for that stream too.
4. **CDK ownership** must follow BE's OIDC + per-env-role pattern. Cross-repo or
   single-repo CDK is an open design decision.
