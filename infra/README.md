# AWS CDK Infrastructure

CDK stacks for stellar-prices-api. Aligned to ADR 0007: Lambdas run
outside any VPC and reach the platform's Hetzner ClickHouse over
HTTPS-mTLS, with the client cert + key material held in AWS Secrets
Manager. No RDS, no VPC, no NAT.

This directory mirrors `soroban-block-explorer/infra/` conventions
(TypeScript CDK, per-env JSON config, one stack per file, separate
`cicd` entrypoint) so the two infra surfaces feel familiar to anyone
who has worked on either.

## Stacks

| Stack                             | File                            | What it holds                                                                                                                                                                                |
| --------------------------------- | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Prices-Cicd`                     | `stacks/cicd-stack.ts`          | The `stellar-prices-api-production-deploy` role that the deploy workflow assumes; imports the account's GitHub OIDC provider. Separate app (`src/bin/cicd.ts`), see [CI/CD role](#cicd-role) |
| `Prices-production-Secrets`       | `stacks/secrets-stack.ts`       | **No secrets.** Publishes the three secret _names_ to SSM; the operator creates the values                                                                                                   |
| `Prices-production-Compute`       | `stacks/compute-stack.ts`       | ledger-processor Lambda + its SQS queue/DLQ subscribed to the platform's ledger-events topic, S3 read on the ledger bucket, the api-handler Lambda, roles, log groups                        |
| `Prices-production-ApiGateway`    | `stacks/api-gateway-stack.ts`   | REST API, usage plan + key, ACM certificate, custom domain and its records in your hosted zone. Depends on Compute                                                                           |
| `Prices-production-EventBridge`   | `stacks/eventbridge-stack.ts`   | The scheduled workers and probes (enrichment, rollup sweep, oracle, supply, asset discovery, freshness probes) and their rules                                                               |
| `Prices-production-Observability` | `stacks/observability-stack.ts` | ops-alarms SNS topic, optional Slack delivery through AWS Chatbot, alarms, dashboard. Depends on Compute                                                                                     |

`deploy --all` resolves the order itself (Secrets and Compute first).

## Releasing to production

Production changes only through the release flow (task 0343). The code on
`master` is the code running in production.

| Branch              | Meaning              | Moved by                                                  |
| ------------------- | -------------------- | --------------------------------------------------------- |
| `develop`           | work in progress     | feature PRs                                               |
| `deploy/production` | approved for release | a release PR from `develop`, approved, merge commit       |
| `master`            | confirmed live       | the deploy workflow, after it succeeds; fast-forward only |

### A release

1. Open a PR from `develop` to `deploy/production`. CI runs on it, and it
   needs one approval (ruleset `deploy/production`, no bypass).
2. From the PR's merge ref, take the production diff and paste it into the
   PR (`<n>` is the PR number):

   ```bash
   git fetch origin refs/pull/<n>/merge && git checkout --detach FETCH_HEAD
   AWS_PROFILE=soroban-readonly make -C infra diff-production
   ```

   The merge ref is the merge of `develop` into `deploy/production`, which is
   exactly what deploys. `develop` alone is not: a hotfix that was not yet
   merged back into `develop` would show there as being reverted.

   **This diff is the only place a deletion shows before it happens.** The
   deploy runs `--require-approval never`, because CI cannot answer a prompt,
   and a deploy can destroy a live resource without asking: an API key
   destroyed this way takes its value with it (task 0157). The approver reads
   the diff for removals and replacements, not only additions. Lambda
   `S3Key` hashes differ between build machines and mean nothing.

3. Merge. `.github/workflows/deploy-production.yml` starts:
   - `build`: the Lambdas, with CI's toolchain and script;
   - `deploy`: waits for the `production` environment's approval (any team
     member except whoever started the run), checks that the run can promote
     (its commit is still the tip, `master` fast-forwards to it, the promote
     key authenticates), then `cdk deploy --all` from the merged commit and
     the API Gateway cache flush. A stack with no change is
     a no-op, so a docs-only release deploys too;
   - only if all of that succeeded: `master` fast-forwards to the commit and
     the commit is tagged `prod-YYYY-MM-DD` (`-2`, `-3` for another release
     the same day);
   - `release`: creates the tag's GitHub Release.
4. Apply what is not branch state, from the release commit, and note each in
   the tag's GitHub Release (below).

A failed run leaves `master` and the tags as they were. Fix forward with a
new PR to `deploy/production`, or re-run the workflow on its tip
(`gh workflow run deploy-production.yml --ref deploy/production`).

Never use GitHub's "Re-run" button on an older run: it reuses that run's
commit, so it would deploy an old commit over production. The workflow
refuses such a run before the deploy, but `gh workflow run` on the tip is the
way to start one.

### Hotfix

A PR straight to `deploy/production`, released like any other. Then merge
`deploy/production` back into `develop`, so the fix is not lost by the next
release.

### Rollback

A revert PR on `deploy/production`, released like any other. `master` is
never moved back and never force-pushed; ruleset `master (history)` refuses
both, for everyone.

### Deploying by hand

Every `make deploy-production*`, `destroy-production*` and
`sync-portal-explorer` target first runs `tools/scripts/check-release-commit.sh`.
It refuses unless the working tree is clean (untracked files included) and
`HEAD` is the freshly fetched tip of `origin/deploy/production`. There is no
override, by decision: to ship something else, merge it into
`deploy/production` by PR first.

```bash
git fetch origin && git checkout --detach origin/deploy/production
AWS_PROFILE=soroban-admin make -C infra deploy-production-compute
```

Do not deploy by hand while a `deploy-production.yml` run is in progress or
waiting (`gh run list --workflow deploy-production.yml --limit 3`). The guard
checks the tip once, when the target starts: a hand deploy still building when
a release lands would ship the older tip over the release, and `master` would
say otherwise.

A hand deploy does not move `master`. If what it shipped is the release tip
that the workflow had not deployed, a repository admin fast-forwards `master`
to it (below) and tags it.

### Changes that are not branch state

Applied by hand, from the release commit, and recorded in the tag's GitHub
Release, one line each (what, when in UTC, who):

- **The portal bundle:** `make -C infra sync-portal-explorer` when
  `web/portal/` changed (guarded like a deploy).
- **The ClickHouse schema** (`init.sql`, `views.sql`): applied by CH's
  `default` user from a detached checkout of `origin/deploy/production`. No
  script enforces this.
- **Data repairs** and other one-off operations.

```bash
gh release view prod-2026-11-21 --json body --jq .body > notes.md
# edit notes.md: add the line under "Manual steps"
gh release edit prod-2026-11-21 --notes-file notes.md
```

### The promote key and `master`'s rules

`master` has two rulesets. `master` refuses every update, with two bypass
actors: repository admins and deploy keys. `master (history)` refuses
deletion and non-fast-forward pushes, with no bypass at all. So whoever moves
`master` can only fast-forward it.

- **The workflow** moves `master` with a write deploy key whose private half
  is the `production` environment secret `PROMOTE_DEPLOY_KEY`. GitHub never
  shows a secret's value, so only an approved run can use it.
- **Admins** can fast-forward `master` by hand, for emergencies only: when a
  deploy succeeded but promote did not. Every bypass is logged under the
  ruleset's Rule insights.

  ```bash
  git push origin <deployed commit>:refs/heads/master
  ```

- **Rotating the key** (on a schedule, when an admin leaves, or on any doubt):
  add the new key, overwrite the secret, delete the old key. The ruleset does
  not change, because its bypass covers every deploy key of the repository.
  The key pair lives only in RAM. **Never substitute `/tmp` or any other disk
  directory** if a command below fails: the private key would stay on disk.

  On Linux (`/dev/shm` is RAM-backed):

  ```bash
  d=$(mktemp -d /dev/shm/promote.XXXXXX)
  ssh-keygen -q -t ed25519 -N '' -C 'stellar-prices-api promote (deploy-production.yml)' -f "$d/key"
  gh repo deploy-key add "$d/key.pub" --allow-write --title 'promote (deploy-production.yml)'
  gh secret set PROMOTE_DEPLOY_KEY --env production < "$d/key"
  shred -u "$d/key" "$d/key.pub" && rmdir "$d"
  ```

  On macOS (no `/dev/shm`, no `shred`): a 1 MB RAM disk, detached afterwards,
  which frees the memory and the key with it:

  ```bash
  dev=$(hdiutil attach -nomount ram://2048 | awk '{print $1}')
  diskutil erasevolume HFS+ promote-key "$dev" >/dev/null
  d=/Volumes/promote-key
  ssh-keygen -q -t ed25519 -N '' -C 'stellar-prices-api promote (deploy-production.yml)' -f "$d/key"
  gh repo deploy-key add "$d/key.pub" --allow-write --title 'promote (deploy-production.yml)'
  gh secret set PROMOTE_DEPLOY_KEY --env production < "$d/key"
  hdiutil detach "$dev"
  ```

  Then, on either:

  ```bash
  gh repo deploy-key list                  # then delete the old one:
  gh repo deploy-key delete <old key id>
  ```

- **On a leak,** delete the key first (`gh repo deploy-key delete`): it stops
  working at once. Promote then fails and `master` stays put; an admin
  catches it up by hand until a new key is in.

## Fresh-account deployment

This section takes an AWS account that has never seen the project to a serving
API. Commands run from the repo root unless a step says otherwise.

### What "fresh account" means here

stellar-prices-api is a **tenant of the Soroban Block Explorer platform**
([`rumblefishdev/soroban-block-explorer`](https://github.com/rumblefishdev/soroban-block-explorer)).
It does not stand up its own ledger bucket, ledger-events topic or database
server. It reads them from the platform through five SSM parameters, and it
owns one database (`prices`) on the platform's ClickHouse server. So a fresh
account takes three parts, in this order:

1. **The platform:** its AWS stacks plus a Hetzner dedicated server running
   ClickHouse behind a Caddy mTLS proxy.
2. **The `prices` tenant on that server:** client certificates, the
   certificate-to-user map, and the schema.
3. **This app:** secrets, seeds, `cdk deploy`.

**Manual by design** (the platform's runbook has the same list for its side):

- Ordering the Hetzner dedicated server and its Storage Box. It's a hardware
  order, so no code can do it.
- Issuing client certificates from the platform's CA. The CA key lives in a
  password manager and never touches CDK or CI.
- Creating the three Secrets Manager values and seeding three SSM parameters
  (five with optional Slack alarm delivery).
  CDK owns only the names, so a deploy can never overwrite live credentials
  (see [Secrets](#secrets-cdk-owns-the-name-the-operator-owns-the-value)).
- Registering the Discord application that gates API key sign-up.
- A Route 53 hosted zone for your domain.

### 1. Prerequisites

| Tool                 | Version              | Why                                                                               |
| -------------------- | -------------------- | --------------------------------------------------------------------------------- |
| AWS CLI v2           | any                  | a profile with admin rights in the target account: `export AWS_PROFILE=<profile>` |
| Node.js              | `22.22.0` (`.nvmrc`) | `nvm use && npm ci` at the repo root (Nx + `aws-cdk` come from the lockfile)      |
| Rust                 | `1.97.1`             | the pin CI uses; rustc ≥ 1.98 fails every aarch64 link under zig                  |
| cargo-lambda         | `1.9.1`              | `pip3 install cargo-lambda==1.9.1`                                                |
| zig                  | any                  | all but native ARM Linux, so every Mac: the Lambdas cross-compile to Linux arm64  |
| jq, openssl, python3 | any                  | asset verification and the secret bundles below                                   |

**On macOS**, the build and verify scripts behind step 7 need bash ≥ 4
(`mapfile`, `declare -A`) and the GNU `realpath -m`, `stat --format`,
`sha256sum` and `shred`. macOS ships bash 3.2 and BSD tools. zig also links
more object files at once than the default limit of 256 open files allows,
and the build then fails with `ProcessFdQuotaExceeded`. Install once:

```bash
brew install bash coreutils zig
```

Then run this in the shell you deploy from, before step 4:

```bash
export PATH="$(brew --prefix)/opt/coreutils/libexec/gnubin:$(brew --prefix)/bin:$PATH"
ulimit -n 61440
bash --version | head -1    # GNU bash, version 5.x
```

Check which account you are in before every command in this section. The
account comes from your credentials (`CDK_DEFAULT_ACCOUNT`), so nothing pins
the deploy to one account. `aws sts get-caller-identity` is the check.

The region is `eu-central-1` (`envs/production.json`). The Parameters and
Secrets Lambda Extension layer is mapped only for `eu-central-1` and
`us-east-1` (`src/lib/mtls.ts`). Any other region fails at synth until its
layer ARN is added.

### 2. The platform (AWS stacks and the Hetzner ClickHouse server)

Follow the platform's fresh-account runbook:

- **AWS stacks:** `soroban-block-explorer/infra/README.md` and
  `docs/deployment.md`. Prerequisites, then `make bootstrap`, then
  `make deploy-production`.
- **Hetzner ClickHouse:** `soroban-block-explorer/infra-hetzner/README.md`,
  "First-time setup":
  1. Order the dedicated server and a BX21 Storage Box by hand.
  2. Bootstrap the mTLS CA (`infra-hetzner/ca/generate-ca.sh`).
  3. Fill the Ansible environment file and inventory.
  4. Run `ansible-playbook -i inventory.ini site.yml`. This installs
     ClickHouse, Caddy (Let's Encrypt plus mTLS), the firewall and weekly
     Borg backups.
  5. Publish the server's IPv4 to SSM (`/soroban/production/ch-ip`) and
     deploy the platform's DNS stack.

When the platform is up, the account holds the five parameters this app reads.
Its Compute stack publishes them:

```bash
aws ssm get-parameters-by-path --path /platform/production --query 'Parameters[].Name'
# /platform/production/ch-domain
# /platform/production/ledger-events-topic-arn
# /platform/production/stellar-ledger-data-bucket-name
# /platform/production/stellar-ledger-data-bucket-arn
# /platform/production/stellar-network-passphrase
```

The platform also defines this app's ClickHouse users already: `prices_writer`,
`prices_reader`, `prices_admin` and `dev_read`, in its
`crates/db-clickhouse/users.d/services.xml`, with grants confined to
`prices.*` (`docs/architecture/security/clickhouse-rbac.md`). There is nothing
to create on the server except the certificate mapping in step 3.

### 3. The `prices` tenant on ClickHouse

**Certificates.** In the platform checkout, issue one client certificate per
AWS identity. The CN decides the ClickHouse user:

| CN                            | ClickHouse user | Used by                          |
| ----------------------------- | --------------- | -------------------------------- |
| `prices-ingestion-production` | `prices_writer` | ledger-processor and the workers |
| `prices-api-production`       | `prices_reader` | api-handler                      |

```bash
# [platform checkout]
infra-hetzner/ca/issue-client-cert.sh prices-ingestion-production
infra-hetzner/ca/issue-client-cert.sh prices-api-production
```

Append both pairs to `CLICKHOUSE_CN_USER_MAP` in the platform's Ansible
environment file, then reload Caddy:

```bash
# [platform checkout] CLICKHOUSE_CN_USER_MAP gains:
#   prices-ingestion-production:prices_writer,prices-api-production:prices_reader
source ~/.config/soroban-prod.env
cd infra-hetzner/ansible && ansible-playbook -i inventory.ini site.yml --tags caddy_reload
```

**Schema.** DDL is applied by the server's `default` user over its loopback
HTTP port, never through the mTLS proxy: the scoped users cannot `DROP VIEW`,
which `views.sql` needs (see `packages/prices-clickhouse/README.md`). Open a
tunnel and run the applier from this repo:

```bash
ssh -N -L 8123:127.0.0.1:8123 deploy@<clickhouse-server> &   # the server binds 8123 to loopback only
export CLICKHOUSE_URL=http://localhost:8123
export CLICKHOUSE_USER=default
export CLICKHOUSE_PASSWORD=<CLICKHOUSE_PASSWORD from the platform's Ansible env>
cargo run -p prices-clickhouse --bin prices-clickhouse-init              # database, tables, seed, views
cargo run -p prices-clickhouse --bin prices-clickhouse-init -- --rollups # the rollup materialized views
```

`schema/current.sql` (the `current_prices` view) is applied **later**, in
step 8. It must not go on before enrichment has run, or it serves zero USD
prices.

### 4. Secrets

Create the three values CDK only names. Each mTLS secret is one JSON bundle,
`{cert, key, ca}` in PEM, staged in tmpfs so no plaintext key reaches
persistent disk:

```bash
# [platform checkout] once per CN: prices-ingestion-production, prices-api-production
CN=prices-ingestion-production
D=${D:-/dev/shm/prices-cert}    # tmpfs; on macOS set D first, see below
mkdir -p -m 0700 "$D" && cp infra-hetzner/ca/out/$CN/{$CN.crt,$CN.key,ca.crt} "$D"/
python3 - "$CN" "$D" > "$D"/bundle.json <<'PY'
import json, pathlib, sys
cn, d = sys.argv[1], pathlib.Path(sys.argv[2])
print(json.dumps({"cert": (d/f"{cn}.crt").read_text(), "key": (d/f"{cn}.key").read_text(), "ca": (d/"ca.crt").read_text()}))
PY
aws secretsmanager create-secret --name "prices/production/clickhouse-mtls-$CN" \
    --secret-string "file://$D/bundle.json"
shred -u "$D"/*
```

**macOS has no `/dev/shm`.** Stage the files on a RAM disk instead. Create the
disk before the block and detach it afterwards; the files never reach the SSD.

```bash
RD=$(hdiutil attach -nomount ram://8192 | awk '{print $1}')   # 4 MB, in memory
diskutil erasevolume HFS+ prices-cert "$RD" && D=/Volumes/prices-cert && chmod 700 "$D"
# … the block above, once per CN …
hdiutil detach "$RD"
```

Resulting names, which must match what `src/lib/mtls.ts` computes:
`prices/production/clickhouse-mtls-prices-ingestion-production` and
`prices/production/clickhouse-mtls-prices-api-production`.

The third secret, `prices/production/portal-discord-oauth`, holds the key
portal's Discord application. Register the application and create it as in
[`docs/runbooks/portal-oauth-deploy-prep.md`](../docs/runbooks/portal-oauth-deploy-prep.md)
§1–§2.

### 5. SSM seeds

Three parameters the stacks or the running Lambdas read, and which CDK
deliberately never creates:

```bash
# Ledger where live ingestion starts (a ledger sequence). The processor seeds its
# durable cursor from it on the first cold start, then never reads it again.
aws ssm put-parameter --type String --name /prices/production/ledger-processor/initial-cursor \
    --value "$(curl -s https://horizon.stellar.org | jq .history_latest_ledger)"

# API-key sign-up gate: the Discord guild a visitor must belong to, and the
# minimum Discord account age. Values and reasoning: portal-oauth-deploy-prep.md §2a.
aws ssm put-parameter --type String --name /prices/production/discord-guild-id \
    --value "$(sed -n "s/^export const STELLAR_DISCORD_GUILD_ID = '\([0-9]*\)';/\1/p" web/portal/src/landing/links.ts)"
aws ssm put-parameter --type String --name /prices/production/min-account-age-minutes --value 5
```

**The guild id must equal `STELLAR_DISCORD_GUILD_ID` in
`web/portal/src/landing/links.ts`.** The portal's "join the server" button
opens the guild named there, and step 8.2's upload runs
`verify-portal-guild`, which refuses the upload when the two differ. The
command above reuses the committed guild (Stellar Developers). To gate on a
guild of your own, change `STELLAR_DISCORD_GUILD_ID` and
`STELLAR_DISCORD_INVITE` in `links.ts` to that guild first, then seed its id.

**Slack delivery for alarms (optional).** `envs/production.json` →
`opsAlarms.slack` names two parameters, `/prices/production/slack-workspace-id`
and `/prices/production/slack-channel-id`. It also needs the workspace
authorized in AWS Chatbot. Either seed both, or delete the `slack` key to
deploy the alarms topic with no subscriber.

### 6. Environment config

`envs/production.json` carries this deployment's domain. For your own account,
change:

| Key                                                  | Set to                                    |
| ---------------------------------------------------- | ----------------------------------------- |
| `apiBaseUrl`, `apiDomain.domainName`                 | the API hostname, inside your hosted zone |
| `apiDomain.hostedZoneId`, `apiDomain.hostedZoneName` | your Route 53 hosted zone in this account |
| `portalWebOrigin`                                    | where the portal is served (CORS origin)  |

For the CI/CD app only, `envs/cicd.json` → `githubRepo` is the repository the
OIDC role trusts.

### 7. Bootstrap and deploy

```bash
npm run infra:bootstrap           # once per account + region; needs no build
npm run infra:diff:production     # builds the CDK app + all Lambdas, shows the diff
npm run infra:deploy:production   # builds, deploys every stack, flushes the API cache
```

`infra:deploy:production` builds the Lambda assets from the working tree first
(`tools/scripts/build-lambda-assets.sh`), then checks that each is a distinct
aarch64 binary. Deploy through these targets: a raw `npx cdk deploy` ships
whatever sits in `target/lambda/`.

The deploy targets refuse anything but a clean checkout of
`origin/deploy/production`'s tip ([Deploying by hand](#deploying-by-hand)), so
a first deploy runs from that branch too. Later releases go through
[the release flow](#releasing-to-production).

### 8. After the deploy

1. **Portal cold start.** The api-handler reads the usage-plan id that
   ApiGateway publishes, and ApiGateway deploys after Compute. So on a first
   deploy the handler's first cold start finds no id and closes the portal;
   `/v1` is unaffected. Check `curl -s https://<api-host>/api/config`. If it
   says `"enabled":false`, recycle the handler so its next cold start finds the id:
   `aws lambda update-function-configuration --function-name prices-production-api-handler --description "recycle $(date -u +%FT%TZ)"`.
   The next deploy resets the description. The ordering is in
   `portal-oauth-deploy-prep.md` §7.
2. **Portal bundle.** The portal is hosted by the platform's web
   distribution. `make -C infra sync-portal-explorer EXPLORER_PORTAL_BUCKET=<bucket> EXPLORER_DISTRIBUTION_ID=<id>`
   uploads it to `/api/` on that distribution.
3. **`current_prices`.** Once the enrichment worker has run (hourly), apply
   `packages/prices-clickhouse/schema/current.sql`. The file is a `DROP VIEW`
   plus a `CREATE`, which the HTTP port and `prices-clickhouse-init` cannot
   run as one, so feed it to `clickhouse-client` inside the server's
   ClickHouse container (the `default` user, as in step 3):

   ```bash
   ssh deploy@<clickhouse-server> 'docker exec -i app-clickhouse-1 clickhouse-client --multiquery' \
       < packages/prices-clickhouse/schema/current.sql
   ```

4. **Verify.** Take a key from the API Gateway console (usage plan
   `pricing-api-free-production`), or sign up through the portal:

   ```bash
   curl -s -H "x-api-key: $KEY" https://<api-host>/v1/assets/native/price
   aws cloudwatch describe-alarms --state-value ALARM --alarm-name-prefix prices-production
   ```

   Freshness alarms can sit in ALARM until the first candles land.

A fresh deployment serves prices from the ledger in `initial-cursor` onward.
Loading history is a separate, days-long operation. See
[`docs/runbooks/backfill-sdex.md`](../docs/runbooks/backfill-sdex.md) and
[`docs/runbooks/continue-soroban-backfill.md`](../docs/runbooks/continue-soroban-backfill.md).

### Tear-down

```bash
make -C infra destroy-production
make -C infra destroy-cicd        # only if you deployed it
aws secretsmanager delete-secret --force-delete-without-recovery --secret-id <each of the three>
aws ssm delete-parameters --names /prices/production/ledger-processor/initial-cursor \
    /prices/production/discord-guild-id /prices/production/min-account-age-minutes
# only if you seeded Slack delivery:
aws ssm delete-parameters --names /prices/production/slack-workspace-id /prices/production/slack-channel-id
```

Log groups are removed with their stacks. The CDK bootstrap stack
(`CDKToolkit`) is left in place.

## CI/CD role

`Prices-Cicd` holds the deploy role that `deploy-production.yml` assumes. Its
trust is limited to this repository's `production` GitHub Environment. Besides
the CDK bootstrap roles and the SSM reads, it may flush the production stage's
API Gateway cache, and nothing else of the API.

```bash
AWS_PROFILE=soroban-admin npm run infra:deploy:cicd   # prints Prices-Cicd.ProductionDeployRoleArn
gh secret set AWS_DEPLOY_ROLE_ARN --env production --body <that ARN>
```

The stack **imports** the GitHub OIDC provider; it does not create it. An
account holds one provider per URL, and this account's belongs to
soroban-block-explorer's `Explorer-Cicd` stack. Deleting that stack, or
removing the provider from its code, deletes the provider, and both
projects' deploy workflows stop until it exists again (a re-created provider
has the same ARN, so nothing here changes). Moving it to
soroban-infrastructure is task 0351. On a fresh account without that stack,
create the provider first:

```bash
aws iam create-open-id-connect-provider --url https://token.actions.githubusercontent.com \
    --client-id-list sts.amazonaws.com --thumbprint-list 6938fd4d98bab03faadb97b34396831e3780aea1
```

## SSM key contract

Both services deploy into the same AWS account, so the namespace split is
enforced by IAM, not by account boundaries: the deploy role can `Get` both
namespaces but `Put`/`Delete` only under `/prices/*`.

### Inputs — `/platform/{env}/...` (the platform publishes, prices-api reads at deploy)

| Key                                               | Value                                           |
| ------------------------------------------------- | ----------------------------------------------- |
| `/platform/{env}/ch-domain`                       | ClickHouse hostname behind the Caddy mTLS proxy |
| `/platform/{env}/ledger-events-topic-arn`         | SNS topic fed by new ledger files               |
| `/platform/{env}/stellar-ledger-data-bucket-name` | ledger bucket (read-only IAM grant)             |
| `/platform/{env}/stellar-ledger-data-bucket-arn`  | same bucket, ARN form                           |
| `/platform/{env}/stellar-network-passphrase`      | public network passphrase                       |

### Operator seeds — `/prices/{env}/...` (never created by CDK)

| Key                                                    | Read by                                                |
| ------------------------------------------------------ | ------------------------------------------------------ |
| `/prices/{env}/ledger-processor/initial-cursor`        | Compute, at deploy (ledger-processor `INITIAL_CURSOR`) |
| `/prices/{env}/discord-guild-id`                       | api-handler, at runtime                                |
| `/prices/{env}/min-account-age-minutes`                | api-handler, at runtime                                |
| `/prices/{env}/slack-workspace-id`, `slack-channel-id` | Observability, at deploy (optional)                    |

### Outputs — `/prices/{env}/...` (prices-api publishes)

| Key                                        | Value                                                   |
| ------------------------------------------ | ------------------------------------------------------- |
| `/prices/{env}/mtls-ingestion-secret-name` | Secrets Manager name of the writer bundle               |
| `/prices/{env}/mtls-api-secret-name`       | Secrets Manager name of the reader bundle               |
| `/prices/{env}/portal-oauth-secret-name`   | Secrets Manager name of the Discord OAuth bundle        |
| `/prices/{env}/api-gateway-id`             | REST API id                                             |
| `/prices/{env}/pricing-api-free-plan-id`   | usage plan id, read by the api-handler for key issuance |

## Commands

From repo root:

```bash
# One-time
npm run infra:bootstrap         # CDK bootstrap (per AWS account + region)
npm run infra:deploy:cicd       # CicdStack — the deploy role (imports the OIDC provider)

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
stack in the app — see `infra/Makefile`. Each passes `--exclusively`: it
deploys that stack and nothing it depends on. Every deploy, destroy and
portal-sync target refuses to run off the tip of `origin/deploy/production`
([Deploying by hand](#deploying-by-hand)); `diff-production` does not.

`make build` compiles the CDK TypeScript only. The Lambda code is whatever is
in `../target/lambda/<name>/` at synth time, so every target that can ship a
Lambda (`deploy-production`, `-compute`, `-eventbridge`) — and
`diff-production`, so the diff is of what would ship — first runs
`make build-lambdas` — `tools/scripts/build-lambda-assets.sh`, the same build
CI runs, followed by a check that each bootstrap is a distinct aarch64 ELF
(task 0141). Deploy through `make`; a raw `npx cdk deploy` ships whatever is on
disk. And do not read `GET /health` as proof of a deploy: it is a gateway mock
that never reaches a Lambda.

## Secrets: CDK owns the name, the operator owns the value

`SecretsStack` creates no secrets. It publishes three **names**, and
`ComputeStack` grants each Lambda read on exactly the one it needs:

| Secret                                                | Holds                                                             | Created in                       |
| ----------------------------------------------------- | ----------------------------------------------------------------- | -------------------------------- |
| `prices/{env}/clickhouse-mtls-prices-ingestion-{env}` | `{cert,key,ca}` for CN `prices-ingestion-{env}` → `prices_writer` | fresh-account step 4             |
| `prices/{env}/clickhouse-mtls-prices-api-{env}`       | `{cert,key,ca}` for CN `prices-api-{env}` → `prices_reader`       | fresh-account step 4             |
| `prices/{env}/portal-discord-oauth`                   | `{client_id, client_secret, redirect_uri, session_signing_key}`   | `portal-oauth-deploy-prep.md` §2 |

A CloudFormation-managed value would need a placeholder the runtime cannot
parse, would collide with the operator's `create-secret`, and would be
restored to the committed value by the next deploy. That last one bites the
OAuth bundle: its `redirect_uri` is re-pointed by hand whenever the backend's
hostname changes (it did on 2026-08-31, task 0194). Rotating a certificate is
`aws secretsmanager put-secret-value` on the same name. No deploy is needed.

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
