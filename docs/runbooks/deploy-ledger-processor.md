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
   that path is exactly what ships. **If you skip the build step, you silently
   redeploy the old binary.**

> First shipped this way in task 0066 (RustFunction adoption is a later
> follow-up). The proto27 unfreeze (tasks 0091 → 0094) is the motivating case:
> the xdr-27 decode fix (PR #104) only reaches the running Lambda once deployed
> via this runbook.

## Prerequisites

- You are on `develop` (or the branch carrying the build you intend to ship) and
  it is up to date.
- `export AWS_PROFILE=soroban-explorer` — the shared account profile.
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

export AWS_PROFILE=soroban-explorer
export AWS_REGION=eu-central-1

git checkout develop && git pull --ff-only
git log --oneline -1                       # confirm the build you intend to ship

aws sts get-caller-identity --query '[Account,Arn]' --output text
command -v cargo-lambda >/dev/null || pip3 install cargo-lambda
```

### 1. Build the ARM64 bootstrap **with the new code**

`--features lambda` is **mandatory**. The `prices-ledger-processor` bin is
declared `required-features = ["lambda"]`, so a build without it silently skips
the bin and produces no bootstrap.

```bash
cargo lambda build -p prices-ledger-processor --release --arm64 --features lambda
```

→ writes `target/lambda/prices-ledger-processor/bootstrap`.

### 2. Confirm the artifact is fresh and correct

```bash
ls -l target/lambda/prices-ledger-processor/bootstrap    # mtime = seconds ago
file target/lambda/prices-ledger-processor/bootstrap     # ELF 64-bit ... ARM aarch64
```

This check is your guard against Step 1 having failed or been skipped — never
deploy on a stale/missing artifact.

### 3. Preview the change (read-only, safe)

```bash
cd infra && make diff-production
```

Expect **only** the ComputeStack Lambda code asset (a new code hash /
`AssetParameters…S3Key`) to change. If the diff proposes IAM, SQS, env-var, or
other-stack edits you did not intend → **stop and investigate** before deploying.

### 4. Deploy (scoped to ComputeStack)

```bash
# still in infra/
make deploy-production-compute
```

`deploy-production-compute` deploys only the ComputeStack. `make
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

## Rollback

Redeploy the previous good build: check out the prior commit (or a tag), rerun
Steps 1–4. Because the asset is content-addressed, redeploying the old bootstrap
reverts the function code. The frozen-gap replay is idempotent per source (candle
writes are keyed by `(timestamp, asset_id, quote_asset_id, source)`).
