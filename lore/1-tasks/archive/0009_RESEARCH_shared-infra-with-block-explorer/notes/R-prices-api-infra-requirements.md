---
title: "Prices API — infra components and how they're sourced"
type: research
status: mature
tags: [infra, requirements, prices-api]
links:
  - '../../../../../docs/prices-api-general-overview.md'
history:
  - date: 2026-05-11
    status: mature
    who: okarcz
    note: 'Distilled from prices-api-general-overview.md §0, §2, §5, §6, §11.'
---

# Prices API — infra components and how they're sourced

Source: `docs/prices-api-general-overview.md` (post-2nd-review).

## Service-level facts

- **Account model:** same dedicated AWS sub-account as the Block Explorer (§0).
- **Region/AZ:** inferred `us-east-1a` (matches BE).
- **Language/runtime:** Rust on Lambda (`provided.al2` via `lambda_runtime`); axum HTTP
  framework; sqlx for DB; ECS Fargate for the SDEX backfill task (§0, §8).
- **IaC + CI/CD:** AWS CDK (TypeScript), GitHub Actions; reuse BE's CDK patterns (§8).
- **Public repo:** open-source mandated; Stellar may fork & redeploy (§0).

## Component inventory

Tagged: **OWN** = Prices API funds and operates · **SHARE** = consume BE-funded resource ·
**COUPLE** = cross-service runtime dependency.

| #   | Component                                               | Tag    | Sourcing                                                         |
| --- | ------------------------------------------------------- | ------ | ---------------------------------------------------------------- |
| 1   | RDS PostgreSQL (Prices schema, partitioned)             | OWN    | `db.t4g.micro` baseline; `db.m6g.large` during backfill (§3, §6) |
| 2   | API Gateway (REST, usage plans, response cache)         | OWN    | §2.1, §6                                                         |
| 3   | Lambda — API handlers (per route group, Rust/axum)      | OWN    | §2.1                                                             |
| 4   | Lambda — Prices Ledger Processor (S3-event-driven)      | OWN    | §2.1, §5.2                                                       |
| 5   | Lambda — Current Price Updater (rate(1m))               | OWN    | §2.1                                                             |
| 6   | Lambda — OHLCV Rollup (rate(15m))                       | OWN    | §2.1                                                             |
| 7   | Lambda — Oracle Fetcher (rate(5m))                      | OWN    | §2.1                                                             |
| 8   | Lambda — Asset Discovery (rate(1h))                     | OWN    | §2.1                                                             |
| 9   | Lambda — Cleanup Worker (cron 02:00 UTC)                | OWN    | §2.1                                                             |
| 10  | EventBridge Scheduler rules                             | OWN    | §5.4                                                             |
| 11  | Secrets Manager entries (DB pwd, API keys, oracle addr) | OWN    | §2.1                                                             |
| 12  | CloudWatch + X-Ray (dashboards, alarms)                 | OWN    | §2.1, §6                                                         |
| 13  | S3 + CloudFront for OpenAPI/Swagger UI hosting          | OWN    | §2.1                                                             |
| 14  | ECS Fargate task — SDEX backfill (Rust, continuous)     | OWN\*  | §5.6 — \*runs in shared ECS cluster per design                   |
| 15  | ECS Fargate task — Soroban AMM backfill (one-time)      | OWN\*  | §5.6 — \*runs in shared ECS cluster per design                   |
| 16  | Galexie ECS Fargate task                                | SHARE  | BE-funded; second S3 event target added (§2.3, §11.1)            |
| 17  | S3 bucket `stellar-ledger-data/`                        | SHARE  | BE-owned; Prices Lambda reads same files (§2.3, §11.1)           |
| 18  | VPC + private subnets                                   | SHARE  | Prices RDS + Lambdas join BE VPC (§2.3)                          |
| 19  | NAT Gateway                                             | SHARE  | BE-funded; Prices Lambda egress through it (§2.3)                |
| 20  | ECS Fargate cluster                                     | SHARE  | Backfill tasks land in BE cluster (§2.3)                         |
| 21  | BE's `soroban_events` table (read-only)                 | COUPLE | Soroban AMM backfill queries BE RDS (§2.3, §5.6, §11.1)          |
| 22  | Shared `stellar-xdr` Rust crate                         | SHARE  | Workspace crate compiled into both Ledger Processors (§8)        |
| 23  | CDK / CI/CD patterns                                    | SHARE  | Reused, not literally co-deployed (§8)                           |

## Key design assumptions worth verifying

1. **§0 / §2.3:** "share core infrastructure — Galexie ECS, S3, VPC, NAT Gateway." BE's
   `infrastructure-overview.md` confirms Galexie + S3 + VPC. **NAT Gateway is implied but
   not explicitly enumerated as a BE-owned component** in the BE doc.
2. **§2.3 (row 5) + §11.1:** "Prices API historical backfill tasks run in the same ECS
   cluster (separate task definition, no cluster fee)." — assumes BE has an ECS cluster
   beyond Galexie's.
3. **§2.3 (row 6) + §5.6 / §11.4:** Soroban AMM backfill reads BE's `soroban_events` table
   to extract decoded JSONB topics/data, avoiding ~8.5M ledger archive reads.
4. **§8:** "shared CDK app with Block Explorer stacks" and "shared pipeline with Block
   Explorer." Implies single-repo or cross-repo CDK orchestration.

These assumptions drive integration decisions and must be cross-referenced against actual BE
state — see `R-block-explorer-infra-state.md` and `R-shared-vs-owned-matrix.md`.
