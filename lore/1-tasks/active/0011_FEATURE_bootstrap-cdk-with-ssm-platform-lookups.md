---
id: "0011"
title: "Bootstrap Prices-owned CDK app with SSM-based platform lookups"
type: FEATURE
status: active
related_adr: ["0007", "0006"]
related_tasks: ["0009", "0008", "0045", "0047", "0050", "0052", "0056", "0038", "0039", "0040"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M1, infra, cdk, aws, shared-infra, clickhouse, hetzner, mtls, secrets-manager]
milestone: 1
links:
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "../backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md"
  - "../backlog/0050_FEATURE_be-side-prep-sns-mtls-prices-db-provisioning.md"
  - "../backlog/0052_FEATURE_clickhouse-mtls-client-shared-crate.md"
  - "../backlog/0056_FEATURE_cloudwatch-alarms-push-freshness-mtls-notafter.md"
  - "../../../../soroban-block-explorer/infra-hetzner/README.md"
  - "../../../../soroban-block-explorer/infra/README.md"
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
  - date: 2026-05-21
    status: active
    who: okarcz
    note: >
      Promoted backlog → active. BE 0227 has landed
      (soroban-block-explorer/infra-hetzner/ now contains the
      Ansible playbook, Caddyfile, and CA tooling; CH-prod-01
      is reachable at the documented Caddy address with mTLS
      gating). Task 0047 (throughput verification) is still
      open as a follow-up but no longer treated as a hard gate
      — the rewrite proceeds against the shared-box shape from
      ADR 0007 §Decision; a RED outcome later only shifts the
      Caddy endpoint host, not the CDK code shape. Spec
      rewrite (RDS/VPC out, Secrets-Manager mTLS in, mirror
      BE's TS-CDK layout) follows in a separate commit on the
      task branch.
  - date: 2026-05-21
    status: active
    who: okarcz
    note: >
      Skeleton complete. All six stacks from §Implementation
      Plan land in `infra/` as buildable + synthable TypeScript
      CDK: CicdStack (GH OIDC + per-env deploy roles),
      SecretsStack (two mTLS Secret slots + SSM ARN outputs),
      ComputeStack (LedgerProcessor + ApiHandler IAM roles +
      LogGroups, plus `createPricesLambdaRole` /
      `lambdaLogGroupName` / `pricesLambdaDefaults` helpers in
      `lib/lambda-baseline.ts`), ApiGatewayStack (REST API
      with /health mock + UsagePlan + ApiKey, SSM publishes
      api-gateway-id), EventBridgeStack (4 rule shells —
      Rollup eliminated per ADR 0007 §3.4), ObservabilityStack
      (empty Dashboard scaffold). cdk synth runs clean for both
      staging and production (verified inside `node:22.22.0`
      docker — matches `.nvmrc`), and the no-VPC/RDS/NAT/SG
      synth-time guard returns nothing on every template. The
      SSM key contract from §SSM Key Contract is wired both
      ways: CicdStack IAM scope reads /platform/* + /prices/*
      and writes /prices/* only; SecretsStack publishes its
      two ARNs to /prices/{env}/mtls-{cert,key}-secret-arn;
      ApiGatewayStack publishes /prices/{env}/api-gateway-id.
      Spec rewrite + skeleton landed across 6 commits on
      `feat/0011_bootstrap-cdk-with-ssm-platform-lookups`,
      PR #28 against develop.
      
      Remaining acceptance items are all deployment-or-CI-
      blocked, not code-blocked: (a) manual CicdStack deploy
      and GitHub-Environment OIDC role-ARN wiring is operator
      work, deferred until the user is ready to touch AWS;
      (b) `make diff-{env}` verification against a fresh AWS
      sub-account requires the operator step from (a) first;
      (c) the CI job running `cdk diff` on infra/ PRs lives in
      task 0008's CI workflow scope. Task stays `active`
      until these clear. Downstream M1 tasks (0038/0039/0040/
      0055/0056) can already start against the skeleton's
      hooks since the cross-stack contract is stable —
      `infra/README.md` §"Where each downstream task plugs in"
      documents the concrete attachment points.
---

# Bootstrap Prices-owned CDK app with SSM-based platform lookups

## Summary

Stand up `infra/` in this repo as a TypeScript CDK app mirroring the Block-Explorer
layout (`soroban-block-explorer/infra/`), but reshaped for the post-ADR-0007 world:
**no RDS, no VPC, no NAT**. Lambdas run outside any VPC and reach BE's Hetzner
ClickHouse over HTTPS-mTLS, with the client cert + key material held in AWS Secrets
Manager. Cross-team values from BE (Hetzner Caddy endpoint, SNS fan-out ARN, CH user
identity) are consumed via AWS SSM Parameter Store under `/platform/{env}/...`;
prices-api-owned identifiers (Secrets Manager ARNs, Lambda ARNs) are published under
`/prices/{env}/...` for downstream consumers (task 0052's `clickhouse-client` crate,
task 0056's alarms).

## Context

Originally drafted as a "join BE's VPC + provision Prices-owned RDS" bootstrap (spawned
from research task 0009 → `S-shared-infra-recommendation.md`'s Option A2). ADR 0007
(accepted 2026-05-20 via task 0045) deletes the RDS line of that plan: the live data
sink is BE's Hetzner ClickHouse, written to over public-internet mTLS. The
implementation impact on this task is recorded in ADR 0007's "Implementation impact"
table — *"Major rewrite. No RDS, no VPC; Secrets Manager mTLS material."*

Two upstream events have now landed which unblock the rewrite:

- **BE 0227** — `soroban-block-explorer/infra-hetzner/` ships the Ansible playbook,
  Caddyfile, and CA tooling that stand up the production Hetzner CH box. The Caddy
  mTLS endpoint is reachable at the FQDN published in BE's `chDomainName` env config.
- **BE CDK shape** — `soroban-block-explorer/infra/` is the reference layout. It uses
  TypeScript CDK with a single `EnvironmentConfig` interface in `src/lib/types.ts`,
  per-env config files under `envs/{staging,production,cicd}.json`, a `createApp`
  factory in `src/lib/app.ts`, one stack per file under `src/lib/stacks/`, and a
  separate `cicd` app for the GitHub OIDC provider + per-env deploy roles. This task
  adopts that layout verbatim — same tooling, same conventions, same field-name
  patterns where overlapping.

Task 0047 (cross-tenant throughput verification) remains open but is no longer treated
as a hard gate: a RED outcome shifts ADR 0007 to Alternative 3 (sidecar CH on the same
Hetzner box, same FQDN+mTLS shape), which changes the SSM-published Caddy address but
not the CDK code shape. The bootstrap proceeds against the shared-box target.

## SSM Key Contract

Two namespaces. **Prices-api never writes under `/platform/`; BE never writes under
`/prices/`.**

### Inputs — consumed from `/platform/{env}/...` (BE publishes, prices-api reads)

| SSM key | Type | Value | Published by |
|---|---|---|---|
| `/platform/{env}/ch-endpoint` | String | `https://ch.{env}.sorobanscan.rumblefish.dev:443` (mTLS Caddy address) | BE task 0050 |
| `/platform/{env}/ch-database` | String | `prices` (per ADR 0007 §3.1) | BE task 0050 |
| `/platform/{env}/ch-user` | String | CH username scoped to the `prices` DB | BE task 0050 |
| `/platform/{env}/stellar-ledger-data-sns-arn` | String | SNS topic ARN BE fans out S3 PutObject to | BE task 0050 |
| `/platform/{env}/stellar-ledger-data-bucket-arn` | String | BE-owned bucket ARN (read-only IAM scope) | BE task 0050 |

BE's CA certificate (PEM) is embedded into the prices-api Rust workspace as a
compile-time asset under `packages/clickhouse-client/ca/be-ca.pem` per task 0052's
spec — **not** an SSM input. SSM is for moving identifiers; static trust material
lives in the repo.

### Outputs — published under `/prices/{env}/...` (prices-api writes, downstream reads)

| SSM key | Type | Value | Consumer |
|---|---|---|---|
| `/prices/{env}/mtls-cert-secret-arn` | String | Secrets Manager ARN holding the mTLS client cert PEM | task 0052 Lambdas |
| `/prices/{env}/mtls-key-secret-arn` | String | Secrets Manager ARN holding the mTLS client key PEM | task 0052 Lambdas |
| `/prices/{env}/ledger-processor-lambda-arn` | String | Live ingest Lambda ARN (for BE to subscribe to SNS) | BE task 0050 / task 0038 |
| `/prices/{env}/api-gateway-id` | String | REST API ID (for downstream domain wiring) | task 0040 |

Every Lambda built by tasks 0038/0039/0040/0055 receives the four `/platform/` strings
+ the two `/prices/` Secret ARNs as environment variables, named per task 0052's
`from_env` contract (`CH_ENDPOINT`, `CH_DATABASE`, `CH_USER`, `CH_CERT_SECRET_ID`,
`CH_KEY_SECRET_ID`). Resolution happens at synth time via
`ssm.StringParameter.valueForStringParameter` — no runtime SSM round-trips.

## Implementation Plan

### Step 1: CDK app skeleton — mirror BE's `infra/` layout

Create `infra/` at repo root with:

```
infra/
├── README.md                ← deploy commands, SSM contract, GH OIDC setup
├── Makefile                 ← bootstrap, diff-staging, deploy-staging, …
├── cdk.json
├── package.json             ← typescript, aws-cdk-lib, constructs, vitest
├── tsconfig.json
├── tsconfig.lib.json
├── eslint.config.mjs
├── envs/
│   ├── staging.json
│   ├── production.json
│   └── cicd.json
└── src/
    ├── index.ts             ← re-exports stacks + types
    ├── bin/
    │   ├── staging.ts       ← node entrypoints (createApp({ config }))
    │   ├── production.ts
    │   └── cicd.ts
    └── lib/
        ├── app.ts           ← createApp factory wiring all stacks
        ├── types.ts         ← EnvironmentConfig + validateConfig + CicdConfig
        ├── ports.ts         ← shared constants (e.g. CH endpoint port)
        └── stacks/
            ├── cicd-stack.ts
            ├── secrets-stack.ts
            ├── compute-stack.ts
            ├── api-gateway-stack.ts
            ├── eventbridge-stack.ts
            └── observability-stack.ts
```

`EnvironmentConfig` in `src/lib/types.ts` carries only the fields each stack actually
reads — no placeholder fields for stacks that don't exist yet. Mirror BE's JSDoc
convention. `validateConfig` rejects `CHANGE_ME` / `PLACEHOLDER` values at synth time.

### Step 2: `CicdStack` — GitHub OIDC + per-env deploy roles

Direct copy of BE's `cicd-stack.ts` shape, retargeted:

- Singleton `iam.OpenIdConnectProvider` for `token.actions.githubusercontent.com`.
- One `iam.Role` per env (`staging`, `production`), trust scoped to GitHub Environment
  `repo:rumblefishdev/stellar-prices-api:environment:{env}`.
- Permissions: `sts:AssumeRole` on `cdk-hnb659fds-*` bootstrap roles + `ssm:GetParameter`
  on `/platform/{env}/*` and `/prices/{env}/*`. **No** ECR / S3-SPA / CloudFront perms
  — prices-api has no SPA and no Galexie image.
- Deployed via a separate `bin/cicd.ts` entrypoint, once per AWS account.
- Outputs the role ARN as a CfnOutput; ARN is added as `AWS_DEPLOY_ROLE_ARN` to the
  GitHub Environment (paired with task 0008's CI workflow).

### Step 3: `SecretsStack` — mTLS material as the two-secrets pattern

Per ADR 0007 §3.5 and task 0052's `from_env` contract:

- Two `secretsmanager.Secret` resources per env: `prices/{env}/clickhouse-mtls-cert`
  and `prices/{env}/clickhouse-mtls-key`. Initial value is `PLACEHOLDER` — the actual
  PEM material is uploaded post-deploy via the BE-supplied issuance bundle from task
  0050 (Ansible-side issuance script handoff via 1Password).
- Secret values are **not** managed by CDK after the initial create — KMS-encrypted,
  no `RemovalPolicy.DESTROY`, and the CloudFormation template does not contain the
  PEMs (avoid leaking them via stack drift / template snapshots).
- Outputs: secret ARNs published to SSM at `/prices/{env}/mtls-cert-secret-arn` and
  `/prices/{env}/mtls-key-secret-arn`.

### Step 4: `ComputeStack` — Lambda set (no VPC)

Define the empty Lambda set that 0038/0039/0040/0055 will fill in. For this bootstrap:

- Create the per-Lambda `iam.Role`s with `secretsmanager:GetSecretValue` scoped to the
  two `prices/{env}/clickhouse-mtls-*` ARNs + `ssm:GetParameter` on
  `/platform/{env}/*` + `/prices/{env}/*`.
- **No `vpc` / `securityGroups` / `vpcSubnets` props.** Lambdas run on the AWS-managed
  shared subnets; outbound traffic to the Hetzner Caddy address goes over the public
  internet.
- Architecture: `arm64` (matches BE's runtime choice and saves ~10–20% on Lambda cost).
- Runtime: `provided.al2023` (custom Rust runtime per ADR 0006). The actual Lambda
  function definitions land in 0038/0039/0040 — this stack only stands up the role +
  log-group naming convention.

### Step 5: `ApiGatewayStack` — REST API shell

Skeleton REST API matching ADR 0006 §axum + task 0040's contract:

- `apigateway.RestApi` with custom domain disabled at this stage (domain wiring is
  task 0040).
- Usage plan + API key pattern wired but no integrations yet — 0040 attaches the
  `/v1/prices/...` routes.
- Outputs: REST API ID to `/prices/{env}/api-gateway-id`.

### Step 6: `EventBridgeStack` — Scheduler rules for the periodic workers

Empty `events.Rule` shells (no targets yet) for task 0039's worker set: price-updater,
oracle-watcher, asset-discovery, cleanup. Note that ADR 0007 §3.4 **eliminates the
Rollup worker** (rollups are CH materialised views); the rule set is 4 workers, not 5
as the original task 0039 spec said. Per-env schedule expressions live in
`EnvironmentConfig`.

### Step 7: `ObservabilityStack` — CloudWatch shell

CloudWatch log group naming convention + a single dashboard scaffold. The actual
alarms (push-freshness, mTLS NotAfter, error rate) land in task 0056.

### Step 8: `app.ts` factory + per-env `bin/` entrypoints

`createApp({ config })` wires the six stacks in order:

```
Secrets ── Compute ─┬─ ApiGateway
                    └─ EventBridge
Observability  (independent)
```

`bin/staging.ts` / `bin/production.ts` read the corresponding `envs/*.json`, validate
it, and call `createApp`. `bin/cicd.ts` is independent (only the `CicdStack`).

### Step 9: README + Makefile

`infra/README.md` documents:

1. The SSM key contract (the two tables above, verbatim).
2. First-time setup: GH OIDC bootstrap → deploy CicdStack manually → add deploy role
   ARN as `AWS_DEPLOY_ROLE_ARN` GitHub Environment secret.
3. Routine deploys: `make deploy-staging`, `make deploy-production`, scoped per-stack
   variants.
4. Post-deploy verification: synthetic SNS subscription test (consume the SNS ARN,
   fire an S3 PutObject in staging, see Lambda invocation) — defers full path
   verification to 0038's acceptance.

`Makefile` mirrors BE's: `bootstrap`, `diff-staging`, `deploy-staging`,
`deploy-staging-{stack}`, plus a `deploy-cicd` target.

### Step 10: CI synth check

Add a GitHub Actions job (or wire into the CI from task 0008) that runs
`make diff-staging --no-deploy` on every PR touching `infra/`. Catches malformed
configs and unresolved SSM lookups at PR time, not deploy time.

## Acceptance Criteria

- [ ] `infra/` directory exists with the file layout in Step 1, mirroring BE's
      `soroban-block-explorer/infra/`.
- [ ] `EnvironmentConfig` in `src/lib/types.ts` defines every field consumed by the
      six stacks, with JSDoc per field. `validateConfig` rejects placeholder values
      on production.
- [ ] `envs/staging.json` and `envs/production.json` populated; no `PLACEHOLDER` /
      `CHANGE_ME` strings on production.
- [ ] `CicdStack` synths and (manually) deploys to the AWS account; deploy role ARN
      output and confirmed assumable via GitHub OIDC from a test workflow.
- [ ] `SecretsStack` synths; secret ARNs published to
      `/prices/{env}/mtls-{cert,key}-secret-arn`.
- [ ] `ComputeStack`, `ApiGatewayStack`, `EventBridgeStack`, `ObservabilityStack`
      synth (no Lambdas attached yet — empty shells with the IAM + log-group
      conventions in place).
- [ ] `make diff-staging` runs clean against a fresh AWS sub-account, given the
      BE-side SSM keys (`/platform/{env}/*`) have been populated by task 0050.
- [ ] `infra/README.md` documents the full SSM key contract (both tables above), the
      GH OIDC bootstrap procedure, the Makefile targets, and the post-deploy
      verification steps.
- [ ] No VPC / RDS / NAT / Security Group resources anywhere in the synthesised
      template. (Synth-time `cdk synth | grep -E 'AWS::EC2::VPC|AWS::RDS::|AWS::EC2::NatGateway'`
      returns nothing.)
- [ ] CI job runs `cdk diff` (or equivalent) on every PR touching `infra/`.

## Out of scope

- **Lambda function bodies** (binary builds, S3 event source, schedule expressions
  attaching to rules) — handled by 0038 / 0039 / 0040 / 0055.
- **mTLS cert issuance** — BE task 0050 issues; this task only provisions the empty
  Secrets Manager slots and the IAM scope to read them.
- **`prices.*` CH schema + MV chain** — task 0051.
- **Custom domain wiring + Route 53 records** for the API Gateway — task 0040, once
  the route shape is settled.
- **CloudWatch alarms** — task 0056. This task lands the dashboard scaffold +
  log-group naming, not the alarm definitions.
- **GitHub Actions deploy workflow** — task 0008 (this task supplies the OIDC role
  ARN; 0008 wires the workflow).
- **An ADR for the SSM key contract** — the contract is documented in
  `infra/README.md` and this task's frontmatter; promoting it to an ADR is deferred
  unless cross-team review demands it.

## Notes

- The original task spec called for an ADR on the SSM-based cross-stack identifier
  mechanism. Post-ADR-0007 that ADR is redundant — ADR 0007 already establishes the
  cross-team integration shape, and the SSM key tables in this task's
  `## SSM Key Contract` section + `infra/README.md` are the operational contract. No
  new ADR planned unless implementation surfaces a non-obvious choice.
- BE's `infra-hetzner/` README explicitly lists "AWS CDK changes in `infra/src/` to
  remove the NAT Gateway, move Lambdas out of the VPC, and move Galexie to a public
  subnet" as future work on BE's side. Prices-api's CDK has no such legacy to evolve
  — the no-VPC shape is the starting point. Coordinate timing with BE to share
  lessons from the no-VPC Lambda pattern (cold-start TLS RTT to Caddy, Secrets
  Manager fetch during global init) once both sides are running it.
- The `arm64` Lambda architecture choice depends on `cargo-lambda` cross-compilation
  working for the Rust binaries — verify in CI before committing the env config
  field.
