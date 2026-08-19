# AWS CDK Infrastructure

CDK stacks for stellar-prices-api. Aligned to ADR 0007: Lambdas run
outside any VPC and reach BE's Hetzner ClickHouse over HTTPS-mTLS,
with the client cert + key material held in AWS Secrets Manager.
No RDS, no VPC, no NAT.

This directory mirrors `soroban-block-explorer/infra/` conventions
(TypeScript CDK, per-env JSON config, one stack per file, separate
`cicd` entrypoint) so the two infra surfaces feel familiar to anyone
who has worked on either.

## Account topology

Both `soroban-block-explorer` (BE) and `stellar-prices-api` deploy
into **the same AWS account**. There is no cross-account boundary
between the two services.

Consequences:

- **S3 bucket access** — the ledger processor Lambda reads from
  BE's `stellar-ledger-data` bucket via a standard IAM policy on
  the Lambda execution role. No bucket policy amendment from BE
  is required; same-account IAM evaluation grants access.
- **SNS subscription** — the Lambda subscribes to BE's SNS topic
  with a standard `sns:Subscribe` + Lambda resource policy. No
  cross-account SNS topic policy needed.
- **SSM Parameter Store** — both `/platform/{env}/*` (BE-owned)
  and `/prices/{env}/*` (prices-api-owned) live in the same
  account. Standard IAM scoping enforces the single-writer
  contract per namespace.
- **Secrets Manager** — mTLS secrets and the Lambda roles that
  read them share the same account. No cross-account `kms:Decrypt`
  grants needed.
- **CicdStack isolation** — each service has its own OIDC deploy
  role (BE's is prefixed `soroban-explorer-*`, prices-api's is
  `stellar-prices-api-*`). The GitHub Environment condition on
  each role ensures one service's CI cannot assume the other's
  deploy role.
- **CloudFormation stack naming** — prices-api stacks are prefixed
  `Prices-*`, BE stacks are prefixed differently. No collision.

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

**Currently implemented:** all six stacks called for by task
0011's spec:

- `CicdStack` — GitHub OIDC + per-env deploy roles.
- `SecretsStack` — mTLS material slots + SSM outputs.
- `ComputeStack` — IAM roles + log groups for the two anchor Lambdas;
  helpers (`createPricesLambdaRole`, etc.) reusable by 0039 / 0055.
- `ApiGatewayStack` — REST API with `/health` mock + UsagePlan/ApiKey;
  real `/v1/prices/...` routes land in 0040.
- `EventBridgeStack` — 4 rule shells for the periodic workers;
  Lambda targets land in 0039.
- `ObservabilityStack` — empty dashboard scaffold; widgets + alarms
  land in 0056.

Each subsequent task slots its real resources into the
already-deployable container these stacks provide.

## Prerequisites

- AWS CLI with a configured profile pointing at the shared AWS account
- Node.js (see `.nvmrc` at repo root)
- `export AWS_PROFILE=<shared-account-profile>`

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

CDK prints one CfnOutput:

- `Prices-Cicd.ProductionDeployRoleArn`

### 3. Wire role ARN into the GitHub Environment

In `https://github.com/rumblefishdev/stellar-prices-api/settings/environments`:

- Create environment `production`. Add secret `AWS_DEPLOY_ROLE_ARN` =
  the production output ARN. Add required reviewers if the team
  wants gated production deploys.

Staging is intentionally absent — the eu-central-1 environment is
initially deployed under the `production` name with conservative
test-sized parameters (mirrors BE task 0239); production sizing is
swapped in via `envs/production.json` once the service is
exercised in anger.

The CI workflow (task 0008) consumes this secret to assume the
deploy role via OIDC.

## SSM Key Contract

The infra is the integration boundary with `soroban-block-explorer`.
Both services deploy into the same AWS account (see "Account
topology" above), so the namespace split is enforced by IAM policy,
not by account boundaries. Two SSM namespaces, single-writer per
namespace:

### Inputs — `/platform/{env}/...` (BE publishes, prices-api reads)

| Key                                              | Value                                                                  | Published by |
| ------------------------------------------------ | ---------------------------------------------------------------------- | ------------ |
| `/platform/{env}/ch-endpoint`                    | `https://ch.{env}.sorobanscan.rumblefish.dev:443` (mTLS Caddy address) | BE task 0050 |
| `/platform/{env}/ch-database`                    | `prices`                                                               | BE task 0050 |
| `/platform/{env}/ch-user`                        | CH username scoped to `prices` DB                                      | BE task 0050 |
| `/platform/{env}/stellar-ledger-data-sns-arn`    | SNS topic ARN for S3 PutObject fan-out                                 | BE task 0050 |
| `/platform/{env}/stellar-ledger-data-bucket-arn` | BE-owned bucket ARN (read-only IAM scope)                              | BE task 0050 |

### Outputs — `/prices/{env}/...` (prices-api publishes, downstream reads)

| Key                                         | Value                                                     | Consumer                 |
| ------------------------------------------- | --------------------------------------------------------- | ------------------------ |
| `/prices/{env}/mtls-cert-secret-arn`        | Secrets Manager ARN holding mTLS client cert PEM          | task 0052 Lambdas        |
| `/prices/{env}/mtls-key-secret-arn`         | Secrets Manager ARN holding mTLS client key PEM           | task 0052 Lambdas        |
| `/prices/{env}/ledger-processor-lambda-arn` | Live ingest Lambda ARN (for BE SNS subscription)          | BE task 0050 / task 0038 |
| `/prices/{env}/api-gateway-id`              | REST API ID (for downstream domain wiring)                | task 0040                |
| `/prices/{env}/pricing-api-free-plan-id`    | Usage plan ID for key issuance + `GetUsage`               | task 0160 / 0187         |
| `/prices/{env}/portal-distribution-domain`  | CloudFront domain serving the portal                      | task 0184 / 0186 / 0195  |
| `/prices/{env}/portal-oauth-secret-name`    | Secrets Manager NAME of the portal's Discord OAuth bundle | task 0186                |

**Boundary rule:** the deploy role's IAM scope enforces this — it
can `Get` both namespaces but `Put`/`Delete` only under `/prices/*`.
Cross-team mistakes that would silently overwrite BE values are
caught at the IAM layer, not at code review.

## Commands

From repo root:

```bash
# One-time
npm run infra:bootstrap         # CDK bootstrap (per AWS account + region)
npm run infra:deploy:cicd       # CicdStack — OIDC + deploy role

# Production (the only AWS environment)
npm run infra:synth:production  # Synth env template
npm run infra:diff:production   # Preview changes
npm run infra:deploy:production # Deploy all env stacks
```

From `infra/`:

```bash
make build
make synth-production
make diff-production
make deploy-production
make deploy-production-secrets  # single-stack scoped deploy
```

Per-stack `deploy-production-{stack}` variants exist for every
stack in the app — see `infra/Makefile`.

## Uploading the real mTLS PEMs

`SecretsStack` provisions the two Secrets Manager slots with random
CDK-generated placeholder values. The real cert + key come from BE
task 0050's per-AWS-service issuance script. To upload them after a
deploy:

```bash
aws secretsmanager put-secret-value \
    --secret-id prices/production/clickhouse-mtls-cert \
    --secret-string "$(cat path/to/production-client.cert.pem)"

aws secretsmanager put-secret-value \
    --secret-id prices/production/clickhouse-mtls-key \
    --secret-string "$(cat path/to/production-client.key.pem)"
```

Subsequent `cdk deploy` invocations will NOT overwrite the uploaded
PEMs — CDK manages the resource (and `generateSecretString`
parameters), not the secret value itself, once it has been replaced
out-of-band.

## The portal's Discord OAuth secret

Same rule, third secret (task 0186). `prices/{env}/portal-discord-oauth`
holds `{client_id, client_secret, redirect_uri, session_signing_key}`
for the onboarding portal's sign-in. CDK computes the **name**
(`portalOauthSecretName` in `src/lib/mtls.ts`), grants the api-handler
role read on exactly that ARN, and sets `PORTAL_OAUTH_SECRET_NAME` —
never the value, which the operator creates and updates by hand.

Here the out-of-band ownership is load-bearing for a second reason: the
`redirect_uri` field is re-pointed at the custom-domain cutover, and a
CloudFormation-managed value would be restored to the committed one by
the next deploy, breaking sign-in silently some time afterwards.

Registration, provisioning and the cutover ordering are in
[`../docs/runbooks/portal-oauth-deploy-prep.md`](../docs/runbooks/portal-oauth-deploy-prep.md).

## Where each downstream task plugs in

Per BE's pattern, one downstream task ≈ one chunk of real content
attached to the skeleton:

| Task                             | Where it plugs in                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| -------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `0008` (CI workflow)             | Adds `.github/workflows/deploy.yml` that assumes the per-env CicdStack deploy role via OIDC and runs `make deploy-{env}`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `0038` (Ledger Processor Lambda) | Adds a `RustFunction` to ComputeStack, references `ledgerProcessorRole` + `ledgerProcessorLogGroup`, attaches SNS subscription, adds `s3:GetObject` on BE's bucket (same-account — IAM grant only, no bucket policy needed), publishes Lambda ARN to `/prices/{env}/ledger-processor-lambda-arn`.                                                                                                                                                                                                                                                                                                                                                   |
| `0039` (Periodic workers)        | Adds 4 `RustFunction`s in ComputeStack, calls `rule.addTarget(...)` on each `EventBridgeStack` rule, uses `createPricesLambdaRole` for the per-worker IAM roles.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                    |
| `0040` (API handlers)            | Adds a `RustFunction` to ComputeStack, attaches as Lambda proxy integration onto ApiGatewayStack's REST API root, adds `/v1/prices/...` resources, wires custom domain + Route 53 A-record + ACM cert.                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `0055` (Backfill status)         | Adds a `RustFunction` to ComputeStack using `createPricesLambdaRole`, adds `/backfill/status` resource to ApiGatewayStack.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `0186` (Portal sign-in)          | Adds `portalOauthSecretName` to `lib/mtls.ts`, a `secretsmanager:GetSecretValue` grant + `PORTAL_OAUTH_SECRET_NAME` on the api-handler in ComputeStack, and publishes the secret name from SecretsStack. Nothing in the template holds a secret value; the operator provisions it per the deploy-prep runbook.                                                                                                                                                                                                                                                                                                                                      |
| `0187` (Self-service keys)       | Adds four `apigateway:` grants on `/apikeys` + `/apikeys/*` and `PORTAL_FREE_PLAN_PARAM` on the api-handler in ComputeStack, plus a standalone `iam.Policy` in ApiGatewayStack granting `POST` on the free plan's `/keys`. The last one lives there because the plan id does: a policy in ComputeStack referencing it would make Compute import an export of ApiGateway, while ApiGateway already imports the Lambda from Compute — a cycle. `POST` and `GET` on `/apikeys` cannot be scoped further; `DELETE /apikeys/*` can be, with a tag condition, and is not yet — task 0194 owns it. All three limits are written out in `compute-stack.ts`. |
| `0056` (Alarms)                  | Adds `cloudwatch.Alarm` constructs to ObservabilityStack referencing ComputeStack log groups + ApiGatewayStack stage metrics + Lambda function metrics. Attaches widgets to the dashboard.                                                                                                                                                                                                                                                                                                                                                                                                                                                          |

## The portal's usage-plan handshake

Task 0187 needs the `pricing-api-free` usage-plan id inside the api-handler, and
cannot have it as a cross-stack reference. `ApiGatewayStack` depends on
`ComputeStack` (it proxies to the Lambda), so importing the plan the other way
closes a cycle CloudFormation refuses — the same shape as the `apiBaseUrl`
problem task 0124 hit.

So it travels as an SSM parameter, and the two ends are two hand-typed strings
in two files:

| end   | where                                                                        |
| ----- | ---------------------------------------------------------------------------- |
| write | `api-gateway-stack.ts`, `PricingApiFreePlanIdParam`                          |
| read  | `compute-stack.ts`, `portalFreePlanParameterName` → `PORTAL_FREE_PLAN_PARAM` |

`npm run openapi:verify-routes` compares them across the two synthesized
templates and fails CI on a mismatch. It cannot check that the parameter is
actually _deployed_, which is a release-ordering precondition recorded in the
deploy-prep runbook's §7: opening the portal before `ApiGatewayStack` has
published it fails Lambda init, and one router serves every route group, so that
is `/v1` down.

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
