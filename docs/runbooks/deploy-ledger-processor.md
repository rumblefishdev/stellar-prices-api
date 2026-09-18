# Runbook — Deploy the live `prices-ledger-processor` Lambda

**Audience:** anyone deploying a new build of the live ledger processor to
production. No prior context assumed. Follow the steps top to bottom.

## What this does

The live ledger processor is a Rust Lambda (`prices-production-ledger-processor`)
that consumes BE's S3-ledger doorbell over SQS, decodes each ledger, extracts
SDEX/AMM/oracle trades, and writes candles to the Hetzner ClickHouse over mTLS.
This runbook ships a **new build** of that Lambda.

Two things you must understand up front:

1. **CI never deploys.** `.github/workflows/ci.yml` only _builds and verifies_ the
   Lambda bootstraps. Deploying is a **manual CDK** operation you run from your
   machine.
2. **CDK packages a pre-built binary.** The ComputeStack consumes
   `target/lambda/prices-ledger-processor/bootstrap` via `Code.fromAsset` — it
   does **not** compile Rust at synth time. So whatever bootstrap is sitting in
   that path is exactly what ships. Until task 0141 that made a skipped build a
   **silent redeploy of the old binary** (it happened, 2026-08-03). Now
   `make deploy-production-compute` and `make deploy-production-eventbridge`
   depend on `make build-lambdas`, which builds every Lambda the CDK app
   references and refuses any bootstrap that is not a distinct aarch64 ELF. A raw
   `npx cdk … deploy` has no such guard — deploy through `make`.

### ⚠️ This runbook covers ONE of the two Lambdas that decode ledger XDR

Exactly **two** deployed Lambdas call `decode_object`, and they sit in
**different stacks**:

| Lambda                               | stack                           | deploy target                        |
| ------------------------------------ | ------------------------------- | ------------------------------------ |
| `prices-production-ledger-processor` | `Prices-production-Compute`     | `make deploy-production-compute`     |
| `prices-production-asset-discovery`  | `Prices-production-EventBridge` | `make deploy-production-eventbridge` |

`oracle-worker` depends on `stellar-xdr` but never decodes a ledger, so it is
out of scope for a decode change.

🔴 **A protocol / XDR bump is not shipped until BOTH are deployed.** Task 0277
discovered `asset-discovery` missing from its own scope and would have left it
on the previous `stellar-xdr`. If what you are shipping touches decode, plan
both deploys before you start. The steps below apply unchanged to the
EventBridge stack — substitute the build and the deploy target:

```bash
cd infra && make deploy-production-eventbridge   # builds the Lambdas first
```

> First shipped this way in task 0066 (RustFunction adoption is a later
> follow-up). The proto27 unfreeze (tasks 0091 → 0094) is the motivating case:
> the xdr-27 decode fix (PR #104) only reaches the running Lambda once deployed
> via this runbook.

## Prerequisites

- You are on `develop` (or the branch carrying the build you intend to ship) and
  it is up to date.
- `export AWS_PROFILE=soroban-admin` — the shared account profile.
- `export AWS_REGION=eu-central-1`.
- Node per the repo `.nvmrc`.
- `cargo-lambda` installed (`pip3 install cargo-lambda`).
- mTLS cert/key already present in Secrets Manager (steady-state — no secret
  provisioning needed for a routine code deploy; see
  [`../../infra/README.md`](../../infra/README.md) "Uploading the real mTLS PEMs"
  only if rotating).

---

## Steps

### 0. Preflight

```bash
cd <repo-root>

export AWS_PROFILE=soroban-admin
export AWS_REGION=eu-central-1

git checkout develop && git pull --ff-only
git log --oneline -1                       # confirm the build you intend to ship

aws sts get-caller-identity --query '[Account,Arn]' --output text
command -v cargo-lambda >/dev/null || pip3 install cargo-lambda

npm run xdr:verify-protocol-gap          # are we behind mainnet's protocol?
```

That last line is the protocol-lag guard (task 0098). It is cheap, it needs no
AWS credentials, and it answers a question this runbook cannot otherwise see:
whether the binary you are about to ship can decode the ledgers it will be
handed. See [Protocol-version lag](#protocol-version-lag--the-standing-check)
below for what its three answers mean.

### 1. Build the ARM64 bootstraps **with the new code**

```bash
cd infra && make build-lambdas
```

This is the same script CI runs (`tools/scripts/build-lambda-assets.sh`): one
`cargo lambda build --release --arm64 --features lambda -p …` over every asset
the CDK app references, the list derived from the CDK source. Do not hand-type
the cargo command instead — `--features lambda` is mandatory (each bin is
`required-features = ["lambda"]`, so a build without it silently skips the bin
and produces no bootstrap), and a single-crate build resolves cargo features
differently from the group build, so it can yield a different binary from the one
CI verified.

Step 4 runs this again by itself; doing it here first is what makes the diff in
step 3 a diff of the artifacts that will ship.

Needs `cargo-lambda` and, on an x86 machine, `zig`. 🔴 Use the toolchain CI
pins (`.github/workflows/ci.yml`: rustc 1.97.1, cargo-lambda 1.9.1) — rustc ≥
1.98 fails every aarch64 link under zig with `unsupported linker arg`. That
failure is loud; it cannot ship anything.

### 2. Confirm the artifacts are correct

The build ends by verifying every bootstrap and printing one line per asset:

```
<sha256>  prices-ledger-processor  14134024 bytes
…
verified 11 Lambda bootstrap(s)
```

It exits non-zero, naming the asset, if a bootstrap is missing, is not an ELF
(the 10-byte `#!/bin/sh` stubs found on 2026-08-12), is not aarch64, or is
byte-identical to another asset's. Freshness is not something to eyeball from an
mtime: cargo decided what needed rebuilding.

### 3. Preview the change (read-only, safe)

🔴 **Do NOT use `make diff-production`.** It runs `cdk diff` with **no stack
filter** (`infra/Makefile:42`), so it prints every stack in the app and a routine
code deploy looks alarming. Diff the stack you are actually deploying, **by
name**:

```bash
cd infra
npx cdk --app "node dist/bin/production.js" diff Prices-production-Compute \
  --method=template --strict
```

- `--method=template` compares templates only — **no changeset is created**, so
  this is a pure read against the account.
- 🔴 **`--strict` is mandatory.** Without it CDK hides changes behind
  _"Omitted N changes … likely mangled non-ASCII"_. Those have been harmless
  before (repairs to mangled `—` / `→` / `§` inside rule and alarm
  descriptions), but **you cannot tell cosmetic from functional without
  looking**, and the omission message will not tell you which you have.

⚠️ **`cdk` runs `dist/`, not `src/`.** If you edited a stack, run
`npx nx build infra` first or you will diff the _previous_ version of your own
change. The `make` targets do this for you via `build-production`; the raw
`npx cdk` command above does not.

Expect **only** the ComputeStack Lambda code asset (a new code hash /
`AssetParameters…S3Key`) to change. If the diff proposes IAM, SQS, env-var, or
other-stack edits you did not intend → **stop and investigate** before deploying.

⚠️ **A dirty-looking asset hash is not always a real change.** A Lambda rebuilt
in a group build can hash differently with no code change between, via cargo
feature unification. Verify the _running_ binary's `CodeSha256` (step 5) rather
than trusting the diff alone.

### 4. Deploy (scoped to ComputeStack)

```bash
# still in infra/
make deploy-production-compute
```

`deploy-production-compute` builds the Lambdas, then deploys only the
ComputeStack — every per-stack target passes `--exclusively`, so no dependency
stack rides along (before task 0141 none did, and `deploy-production-apigateway`
would have deployed Compute as a side effect). `make
deploy-production` deploys _all_ stacks — avoid it unless you intend a full-app
deploy. Override the asset path with `LEDGER_PROCESSOR_ASSET_DIR` if building
elsewhere.

### 5. Verify the new code is live

```bash
aws lambda get-function-configuration \
  --function-name prices-production-ledger-processor \
  --query '[LastModified,Runtime,Architectures[0],CodeSha256]' --output text
```

`LastModified` should be seconds ago; `Architectures[0]` = `arm64`.

### 6. Confirm ingestion is healthy (the real success signal)

The candle frontier must advance. Re-run the freshness query and watch
`latest_candle` climb toward now:

```bash
ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
  "docker exec -i app-clickhouse-1 clickhouse-client -q \
   \"SELECT source, max(timestamp) AS latest_candle, now() - max(timestamp) AS behind_sec \
     FROM prices.price_ohlcv_1m GROUP BY source ORDER BY source\""
```

`price_ohlcv_1m` has **no ledger column** — freshness is measured by candle
`timestamp`. Source column is `source` (`sdex` / `aquarius` / `phoenix`).

If the frontier stays flat after a few minutes, inspect the Lambda:

```bash
aws logs tail /aws/lambda/prices-production-ledger-processor --since 10m --follow
```

and check the DLQ depth (`prices-ingest-dlq-production`).

---

## After the deploy — not part of this runbook

Step 6 only covers **forward** re-ingestion via the SQS doorbell. If the
processor was frozen (e.g. the proto27 stall), the historical gap between the
last-written candle and the current tip needs a **replay** for each live source,
plus a DLQ drain if anything dead-lettered. That replay is a separate operation —
see the ingestion operator guide
[`running-ingestion-components.md`](running-ingestion-components.md) and, for the
proto27 case specifically, task 0094.

## Protocol-version lag — the standing check

`stellar-xdr`'s major version tracks the Stellar protocol version: 27 decodes
protocol 27, 28 decodes 28. When mainnet advances past our pin, the processor
hits an XDR **decode wall** — and the failure is silent in every way that
matters.

That is not hypothetical. Protocol 27 "Zipper" froze the live candle frontier
at ledger `~63,384,067` for **six days**. Throughout, the SQS queue drained
normally and both it and the DLQ were empty (nothing to redrive), the Lambda
logged zero parse, XDR, panic or ClickHouse errors, and the doorbell-lag alarm
stayed green because it watches queue age and the queue was healthy. The pass
drained its messages and wrote no candles. It was found by reading
`max(timestamp)` out of `price_ohlcv_1m` by hand.

### The check

```bash
npm run xdr:verify-protocol-gap      # advisory — also a step in CI
npm run xdr:watch-protocol-gap       # strict — what the daily workflow runs
```

It compares our pinned major against Horizon's root document and has three
answers:

| result      | meaning                                               | do                                                                       |
| ----------- | ----------------------------------------------------- | ------------------------------------------------------------------------ |
| **current** | pin matches mainnet                                   | nothing                                                                  |
| **LAGGING** | `core_supported_protocol_version` is ahead of our pin | mainnet has not voted yet. **This is the lead time** — open the bump now |
| **BEHIND**  | `current_protocol_version` is ahead of our pin        | the wall is live or one ledger away. Bump **and deploy**, today          |

It also fails when two `stellar-xdr` majors resolve into the lockfile, which is
the skew described below.

### ⚠️ It reads the REPO, not the deployed binary

This is the check's one real limit, and it is the same trap as
[task 0141](../../lore/1-tasks/archive/) — merging a fix is not shipping one.
Task 0091 merged the proto27 bump on 2026-07-14 and **production stayed frozen**
until 0094 deployed the binary days later. A green protocol check means the
source is correct; only steps 2 and 5 of this runbook establish that the
running Lambda is.

The deployed half is covered from the other side, by the
`prices-production-rollup-freshness-1m` alarm, which measures **data** rather
than exit status and whose own description names "upstream ingestion has
halted". Between the two, both halves are watched — neither alone is enough.

### ⚠️ `xdr-parser` must move with us

`packages/prices-ingest-core/src/decode.rs` takes `LedgerCloseMeta` across the
crate boundary from BE's `xdr-parser`, which we track on `branch="develop"`
rather than a rev pin — deliberately. So a `stellar-xdr` bump is never ours
alone: if the two sides disagree on the major, Cargo resolves **both** into the
graph and the types stop matching. Ask BE to bump `xdr-parser` first, then
re-pin to their merge rev.

### Where the check runs

| where                                     | mode     | catches                                                                                                           |
| ----------------------------------------- | -------- | ----------------------------------------------------------------------------------------------------------------- |
| `ci.yml`, job `XDR protocol lag`          | advisory | a PR opened while already BEHIND. Ungated by path filters — the condition produces no diff on our side            |
| `xdr-protocol-watch.yml`, daily 06:17 UTC | strict   | **mainnet moving while nothing in the repo changes** — the proto27 case, and the reason this is on a clock at all |
| this runbook, step 0                      | advisory | shipping a binary that cannot decode current ledgers                                                              |

### How a failure reaches a person

The daily watch opens **one tracking issue** and keeps it current. No secret,
no Slack app, no AWS — `GITHUB_TOKEN` with `issues: write` is the whole
mechanism.

| event                            | what happens                             |
| -------------------------------- | ---------------------------------------- |
| first detection                  | issue opened → notifies watchers         |
| still the same tier next day     | body refreshed, **no comment, silent**   |
| LAGGING → BEHIND (mainnet voted) | comment → notifies                       |
| check passes                     | comment + **issue closed automatically** |

It is deliberately not a daily comment. The condition persists for as long as
the bump takes, and a daily notification saying nothing new gets a thread
muted — at which point the guard is worse than absent, because it looks
present. The body is rewritten every run regardless, so the issue is never
stale even on the silent days.

GitHub's own scheduled-workflow failure email fires daily and separately. It
goes to _"the user who last modified the cron syntax in the workflow file"_ —
one inbox, subject to that person's notification settings, and silently
reassigned by an unrelated edit to the `cron:` line. The issue exists because
that is too thin on its own.

#### Why not Slack

All three routes are closed, recorded so they are not re-attempted:

- **Incoming webhook** — needs a Slack app, and the workspace is at its
  installed-app limit.
- **Slack GitHub app** — installed in the workspace, but not on the GitHub org;
  installing it needs an organisation owner.
- **SNS → AWS Chatbot** (the path every ops alarm uses, task 0056) — needs AWS
  credentials this workflow does not have and should not be handed for one
  notification.

Routing through Chatbot properly would mean a CloudWatch metric published by
something holding AWS credentials — an OIDC role, or a scheduled Lambda beside
the existing probes — plus a CDK change and a deploy. That is its own task.

## Rollback

Redeploy the previous good build: check out the prior commit (or a tag), rerun
Steps 1–4. Because the asset is content-addressed, redeploying the old bootstrap
reverts the function code. The frozen-gap replay is idempotent per source (candle
writes are keyed by `(timestamp, asset_id, quote_asset_id, source)`).
