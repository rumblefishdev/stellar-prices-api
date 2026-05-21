---
id: "0011"
title: "Bootstrap Prices-owned CDK app with SSM-based platform lookups"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0009", "0008", "0045", "0047"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M1, infra, cdk, aws, shared-infra, clickhouse, hetzner]
links:
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "./0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md"
history:
  - date: 2026-05-11
    status: backlog
    who: okarcz
    note: "Spawned from 0009 future work. Implements Option A2 from the integration-options note."
  - date: 2026-05-18
    status: backlog
    who: okarcz
    note: >
      Redesign pending. Task 0044's research (synthesis §3) and
      ADR 0007 (proposed) call for major rewrite of this task —
      RDS line and Prices-api VPC integration are out; Secrets
      Manager mTLS material + no-VPC Lambdas + IAM scope for
      `secretsmanager:GetSecretValue` come in. Hold rewrite until
      both gating events clear: (1) BE Hetzner CH ships, (2)
      ADR 0007 transitions proposed → accepted (gated on task
      0045). Do not start implementation against this spec.
  - date: 2026-05-20
    status: backlog
    who: okarcz
    note: >
      ADR 0007 accepted via task 0045's closure. Architectural
      uncertainty is gone — the rewrite shape is "no RDS, no
      VPC, Secrets Manager mTLS material, IAM secretsmanager
      scope". Remaining gates are engineering: BE 0227 (so the
      Caddy address + cert issuance script are concrete) and
      task 0047 (throughput verification — a RED outcome shifts
      this task's CDK targets from BE's shared box to a
      Prices-api-owned sidecar box, same code shape). Task
      stays in backlog pending those two events.
---

# Bootstrap Prices-owned CDK app with SSM-based platform lookups

## Summary

Stand up `infra/aws-cdk/` in this repo as a TypeScript CDK app that mirrors the Block
Explorer convention but deploys independently. Cross-stack values from BE (VPC ID, S3
bucket name, NAT EIP, KMS key ARN, etc.) are consumed via AWS SSM Parameter Store, not
direct stack imports.

## Context

Spawned from research task 0009. Per `notes/S-shared-infra-recommendation.md`, the
recommended integration shape is **Option A2 + B1** (separate CDK app + own GitHub Actions
OIDC). This task implements the CDK side; CI workflow setup is task 0008.

## Implementation

- Write an ADR in `lore/2-adrs/` capturing the SSM-based cross-stack identifier mechanism
  and per-environment role/account model (mirrors BE ADR 0001).
- Coordinate with BE team to publish platform identifiers under `/platform/<env>/...`
  SSM paths.
- Scaffold `infra/aws-cdk/` with stacks for: networking-consumer (joins BE VPC), RDS,
  Lambda set, API Gateway, EventBridge.
- Implement OIDC trust + per-env IAM roles (staging/production).
- `cdk synth` succeeds for staging from a clean clone.

## Acceptance Criteria

- [ ] ADR drafted and committed (`lore/2-adrs/`)
- [ ] `infra/aws-cdk/` exists with stacks for VPC consumer, RDS, Lambdas, API Gateway,
      EventBridge
- [ ] SSM lookups parameterised by environment (`/platform/{env}/vpc-id`, etc.)
- [ ] OIDC trust + per-env role definitions present
- [ ] `cdk synth --context env=staging` produces a valid template against a fresh AWS
      sub-account given the SSM keys are present
- [ ] README in `infra/aws-cdk/` documents the SSM key contract with BE
