# AWS CDK Infrastructure

CDK stacks for stellar-prices-api. Aligned to ADR 0007: Lambdas run
outside any VPC and reach BE's Hetzner ClickHouse over HTTPS-mTLS,
with the client cert + key material held in AWS Secrets Manager.
No RDS, no VPC, no NAT.

This directory mirrors `soroban-block-explorer/infra/` conventions
(TypeScript CDK, per-env JSON config, one stack per file, separate
`cicd` entrypoint) so the two infra surfaces feel familiar to anyone
who has worked on either.

## Stack architecture (target)

```
CicdStack            (one-time, per AWS account — GitHub OIDC + deploy roles)

SecretsStack         (mTLS cert + key Secrets Manager slots, /prices/{env}/* SSM outputs)
    |
ComputeStack         (no-VPC Lambdas + IAM roles)
    |
    +-- ApiGatewayStack    (REST API + usage plan)
    +-- EventBridgeStack   (scheduler rules for periodic workers)

ObservabilityStack   (CloudWatch dashboard scaffold; alarms land in task 0056)
```

**Currently implemented:** `CicdStack`, `SecretsStack`,
`ComputeStack` (roles + log groups only — Lambda functions land
with 0038/0039/0040). Remaining stacks land as separate slices
(API Gateway, EventBridge, Observability — see task 0011 spec
§Implementation Plan).

## Prerequisites

- AWS CLI with a configured profile pointing at the prices-api account
- Node.js (see `.nvmrc` at repo root)
- `export AWS_PROFILE=stellar-prices-api`

## First-time setup

### 1. Bootstrap the CDK toolkit

Once per AWS account + region:

```bash
npm run infra:bootstrap
```

This provisions the `cdk-hnb659fds-*` roles that the GitHub Actions
deploy role assumes for CloudFormation operations.

### 2. Deploy the CicdStack manually

The CicdStack is deployed by a human operator (you), once per AWS
account, **before** any GitHub Actions workflow can deploy anything:

```bash
npm run infra:deploy:cicd
```

CDK prints two CfnOutputs:

- `Prices-Cicd.StagingDeployRoleArn`
- `Prices-Cicd.ProductionDeployRoleArn`

### 3. Wire role ARNs into GitHub Environments

In `https://github.com/rumblefishdev/stellar-prices-api/settings/environments`:

- Create environment `staging`. Add secret `AWS_DEPLOY_ROLE_ARN` =
  the staging output ARN.
- Create environment `production`. Add secret `AWS_DEPLOY_ROLE_ARN` =
  the production output ARN. Add required reviewers if the team
  wants gated production deploys.

The CI workflow (task 0008) consumes these secrets to assume the
deploy roles via OIDC.

## SSM Key Contract

The infra is the integration boundary with `soroban-block-explorer`.
Two SSM namespaces, single-writer per namespace:

### Inputs — `/platform/{env}/...` (BE publishes, prices-api reads)

| Key | Value | Published by |
|---|---|---|
| `/platform/{env}/ch-endpoint` | `https://ch.{env}.sorobanscan.rumblefish.dev:443` (mTLS Caddy address) | BE task 0050 |
| `/platform/{env}/ch-database` | `prices` | BE task 0050 |
| `/platform/{env}/ch-user` | CH username scoped to `prices` DB | BE task 0050 |
| `/platform/{env}/stellar-ledger-data-sns-arn` | SNS topic ARN for S3 PutObject fan-out | BE task 0050 |
| `/platform/{env}/stellar-ledger-data-bucket-arn` | BE-owned bucket ARN (read-only IAM scope) | BE task 0050 |

### Outputs — `/prices/{env}/...` (prices-api publishes, downstream reads)

| Key | Value | Consumer |
|---|---|---|
| `/prices/{env}/mtls-cert-secret-arn` | Secrets Manager ARN holding mTLS client cert PEM | task 0052 Lambdas |
| `/prices/{env}/mtls-key-secret-arn` | Secrets Manager ARN holding mTLS client key PEM | task 0052 Lambdas |
| `/prices/{env}/ledger-processor-lambda-arn` | Live ingest Lambda ARN (for BE SNS subscription) | BE task 0050 / task 0038 |
| `/prices/{env}/api-gateway-id` | REST API ID (for downstream domain wiring) | task 0040 |

**Boundary rule:** the deploy role's IAM scope enforces this — it
can `Get` both namespaces but `Put`/`Delete` only under `/prices/*`.
Cross-team mistakes that would silently overwrite BE values are
caught at the IAM layer, not at code review.

## Commands

From repo root:

```bash
# One-time
npm run infra:bootstrap         # CDK bootstrap (per AWS account + region)
npm run infra:deploy:cicd       # CicdStack — OIDC + deploy roles

# Per-env (staging / production)
npm run infra:synth:staging     # Synth env template
npm run infra:diff:staging      # Preview changes
npm run infra:deploy:staging    # Deploy all env stacks
```

Same shape for production: `infra:synth:production`,
`infra:diff:production`, `infra:deploy:production`.

From `infra/`:

```bash
make build
make synth-staging
make diff-staging
make deploy-staging
make deploy-staging-secrets     # single-stack scoped deploy
```

Equivalent `production` and `*-{stack}` variants exist for every
target — see `infra/Makefile`.

## Uploading the real mTLS PEMs

`SecretsStack` provisions the two Secrets Manager slots with random
CDK-generated placeholder values. The real cert + key come from BE
task 0050's per-AWS-service issuance script. To upload them after a
deploy:

```bash
aws secretsmanager put-secret-value \
    --secret-id prices/staging/clickhouse-mtls-cert \
    --secret-string "$(cat path/to/staging-client.cert.pem)"

aws secretsmanager put-secret-value \
    --secret-id prices/staging/clickhouse-mtls-key \
    --secret-string "$(cat path/to/staging-client.key.pem)"
```

Subsequent `cdk deploy` invocations will NOT overwrite the uploaded
PEMs — CDK manages the resource (and `generateSecretString`
parameters), not the secret value itself, once it has been replaced
out-of-band.

## Future stacks

These are scaffolded by task 0011's spec but not yet implemented in
this repo. Each lands as a separate FEATURE task per BE's pattern
(one stack ≈ one lore task):

| Stack | Owning task | Purpose |
|---|---|---|
| `ApiGatewayStack` | 0040 | REST API shell + usage plan, hooked to `ComputeStack.apiHandlerRole` |
| `EventBridgeStack` | 0039 | Scheduler rules for periodic workers (no Rollup — see ADR 0007 §3.4) |
| `ObservabilityStack` | 0056 | CloudWatch alarms (push-freshness, mTLS NotAfter, error rate) |

## Lambda conventions

Every prices-api Lambda follows a shared shape, captured in
`infra/src/lib/lambda-baseline.ts`:

- **Architecture:** `arm64` (Graviton). ~10-20% cheaper than x86 at
  the same memory.
- **Runtime:** `provided.al2023` (custom runtime targeting
  cargo-lambda bootstrap binaries, per ADR 0006).
- **No VPC.** Per ADR 0007 §3.6, Lambdas run on AWS-managed shared
  subnets and reach Caddy over the public internet.
- **Baseline IAM:** Every role gets `secretsmanager:GetSecretValue`
  on the two mTLS material ARNs + `ssm:GetParameter` on both the
  `/platform/{env}/*` and `/prices/{env}/*` namespaces. Stack-
  specific permissions (S3 read for the processor, etc.) are added
  via `role.addToPolicy(...)` in downstream tasks.
- **Log group:** `/aws/lambda/prices-{env}-{lambdaName}`, retention
  one month, removal policy DESTROY.

Downstream tasks consume these conventions via:

```ts
import {
  createPricesLambdaRole,
  lambdaLogGroupName,
  pricesLambdaDefaults,
  PRICES_LAMBDA_LOG_RETENTION,
} from '@rumblefish/stellar-prices-api-aws-cdk';
```

`ComputeStack` pre-creates the role + log group for the two anchor
Lambdas (`LedgerProcessor` → task 0038, `ApiHandler` → task 0040)
and exposes them as readonly properties. Tasks 0039 (periodic
workers) and 0055 (backfill status) call `createPricesLambdaRole`
to construct their own.

## Why no VPC

Per ADR 0007 §3.6: prices-api Lambdas run on AWS-managed shared
subnets and reach the Hetzner Caddy address over the public
internet. mTLS at Caddy is the access gate, not IP-based controls.

- No Prices-api VPC, no NAT Gateway, no Security Groups for Lambda.
- Cold-start TLS handshake is ~80-130 ms RTT; amortised across
  invocations via the warm-connection pattern in task 0052's
  `clickhouse-client` crate.
- Outbound traffic is unmetered through the Lambda free-tier and
  cheaper than NAT Gateway egress at any volume we plausibly hit.

A synth-time guard in the eventual CI workflow (`cdk synth | grep`
for `AWS::EC2::VPC`, `AWS::RDS::`, `AWS::EC2::NatGateway`) ensures
this property is preserved as stacks are added.
