---
title: "SDEX backfill on Prices-owned Fargate — operational design"
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [sdex, backfill, fargate, ecs, iam, runbook, stream-2, design]
links:
  - "../../../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-filter-strategy.md"
  - "../../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../../../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md"
history:
  - date: 2026-05-13
    status: mature
    who: okarcz
    note: >
      Operational design for the dedicated, BE-independent SDEX
      backfill Fargate task. Consumes ADR 0002 (architecture),
      ADR 0003 (PK shape) and task 0022's filter + decode-and-
      bucket specs. Output is the implementation contract for
      the follow-up CDK + Rust landing task.
---

# SDEX backfill on Prices-owned Fargate — operational design

## 0. Scope and non-scope

This note is the **operational and infrastructural design** for the
SDEX (Stream 2) historical backfill task. It is consumed by the
follow-up implementation task (see [§11](#11-handoff--implementation-checklist))
which lands CDK code, Rust binary, schema migrations, and the staging
runbook.

**In scope here:**

- ECS Fargate task definition shape (image, sizing, env, networking).
- IAM role contract — exactly which AWS principals the task may touch,
  and which it must not.
- Processing direction (tip-backward) decision and reasoning.
- Resumability contract against `backfill_progress`.
- CloudWatch heartbeat metric, alarm threshold, SNS topic shape.
- Failure-mode taxonomy and the task's response to each.
- Runbook structure (start / stop / resume / alarm response).
- Rust module split — 1:1 mapping onto task 0022's spec sections.

**Out of scope (deferred to impl task):**

- Concrete CDK TypeScript (depends on task 0011 bootstrap).
- Rust crate layout, Cargo workspace integration with `stellar-xdr`.
- DDL for `backfill_progress` table and the ADR 0003 `quote_asset_id`
  PK migration on `price_ohlcv`.
- Final `aws ecs run-task` invocation, populated `cluster` /
  `task-definition` ARNs.
- Staging deploy + 10k-ledger smoke test.

## 1. Architecture overview

```text
┌──────────────────────────────────────────────────────────────────────┐
│ AWS account: prices-api (staging / production)                       │
│                                                                      │
│  ┌─────────────────────────────────┐    ┌───────────────────────┐   │
│  │ ECS Fargate cluster             │    │ Stellar public        │   │
│  │ `prices-backfill-{env}`         │───▶│ history archive S3    │   │
│  │                                 │    │ (read-only, anonymous │   │
│  │  ┌───────────────────────────┐  │    │  in production; IAM   │   │
│  │  │ Task: sdex-backfill       │  │    │  role still scoped to │   │
│  │  │ image: ECR/sdex-backfill  │  │    │  GET only)            │   │
│  │  │ 2 vCPU / 4 GiB            │  │    └───────────────────────┘   │
│  │  │ env: LEDGER_RANGE_*, ...  │  │                                │
│  │  └─────────────┬─────────────┘  │    ┌───────────────────────┐   │
│  │                │                 │───▶│ Prices PG (RDS)       │   │
│  │                │                 │    │   • price_ohlcv       │   │
│  │                │                 │    │   • backfill_progress │   │
│  │                │                 │    │   • assets            │   │
│  │                │                 │    └───────────────────────┘   │
│  │                ▼                 │                                │
│  │       CloudWatch Logs +          │    ┌───────────────────────┐   │
│  │       PutMetricData              │───▶│ SNS                    │   │
│  │       (Prices/Backfill)          │    │ prices-backfill-alerts│   │
│  └─────────────────────────────────┘    └───────────────────────┘   │
│                                                                      │
│  No path connects this task to any Block Explorer account, RDS,      │
│  ClickHouse, or S3 bucket. The BE-authored `stellar-xdr` parser is   │
│  embedded in the container image as a Cargo library dependency.      │
└──────────────────────────────────────────────────────────────────────┘
```

**Cluster:** one Fargate cluster per environment, named
`prices-backfill-{env}`. The cluster is dedicated to long-running
backfill tasks (Stream 2 today; AMM Stream 1 is local-CH per ADR 0001
and does not run here).

**Task definition:** `sdex-backfill-{env}`, single-container.
Stateless container — all checkpoints live in PG.

**Trigger:** initially manual via `aws ecs run-task` from the
operator's workstation (or CI on demand). No scheduled EventBridge
rule in the v1 design — operators start a run, observe progress, and
the task self-completes when `current_ledger` reaches the configured
floor. A scheduled "restart-on-stop" wrapper is an obvious v2
addition but not required for Tranche 1.

**Networking:** task launches in the prices-api VPC private subnets
(joined to the BE-managed VPC per task 0011's SSM contract) and uses
NAT egress for S3 and SNS, plus the in-VPC RDS endpoint for PG.
Public-archive S3 is reached over the NAT path — no direct internet
gateway attachment on the task.

## 2. Processing direction: tip-backward (newest → oldest)

ADR 0002 §6 explicitly delegates the direction choice to this design.
Three candidates were on the table:

| Direction      | Tranche 1 UX gate              | Checkpoint shape       | Operational cost |
| -------------- | ------------------------------ | ---------------------- | ---------------- |
| Oldest → newest | Fails — Tranche 1 needs the *recent* 6 months exposed, not ledger 1 | Monotonic incr | Simple |
| **Newest → oldest** | **Passes — recent 6 months ready in ~3-4 days** | Monotonic decr | Simple |
| Chunked / sharded | Same as newest-first if newest chunks run first | Per-chunk independent state | Operational overhead, multi-task fan-out |

**Decision: tip-backward, single-task, monotonic-decreasing
`current_ledger`.**

Reasoning:

1. **Tranche 1 acceptance criterion (§5.6 / ADR 0002 §5) is
   recency-biased.** "Approximately 6 months of recent history by
   end of Tranche 1" maps directly to tip-backward. With task 0022's
   measured 311 ledgers/s decode and ~99.35 % trade-bearing density,
   6 months ≈ 1 026 000 ledgers ≈ 55 minutes of pure decode. Real
   wall-clock is archive-transport-bound (12-16 days for the full
   ~57M ledgers per task 0022 §4) but the first 6 months is reached
   in well under a week even at the conservative end.

2. **UPSERT semantics make direction safe.** Task 0022's
   decode-and-bucket spec §5.4 commits to **whole-row replacement**
   for backfill UPSERTs. That means a later-arriving tick for the
   same `(asset_id, quote_asset_id, timestamp, granularity)` row
   always wins regardless of which side of the minute boundary it
   crossed. Direction has no correctness implication — only UX.

3. **`earliest_data_available` retreat is acceptable.** With
   tip-backward, the `GET /backfill/status` endpoint reports a
   shrinking `earliest_data_available` over weeks. This is the
   intended UX per §5.6.

4. **Disjoint-range parallelisation is a v2 escape hatch, not a v1
   requirement.** If 12-16 days proves too long, the same binary
   shards by env-vars `LEDGER_RANGE_START` / `LEDGER_RANGE_END` and
   runs multiple Fargate tasks against disjoint ranges. Each task
   keeps its own row in `backfill_progress` keyed by `stream` —
   reserve names `sdex_backfill_chunk_0`, `sdex_backfill_chunk_1`,
   etc. for that future split. v1 ships with a single task and a
   single `stream='sdex_backfill'` row.

## 3. Task definition shape

```yaml
# Logical shape. CDK code lands this in the impl task.
family: sdex-backfill
networkMode: awsvpc
requiresCompatibilities: [FARGATE]

cpu: "2048"          # 2 vCPU — matches 0020 G-note baseline
memory: "4096"       # 4 GiB — leaves headroom over the streaming decoder

executionRoleArn:    arn:aws:iam::ACCOUNT:role/PricesBackfillExecution-{env}
taskRoleArn:         arn:aws:iam::ACCOUNT:role/PricesBackfillSDEX-{env}

containerDefinitions:
  - name: sdex-backfill
    image: ACCOUNT.dkr.ecr.REGION.amazonaws.com/prices/sdex-backfill:GIT_SHA
    essential: true

    environment:
      - { name: STREAM,             value: sdex_backfill }
      - { name: DIRECTION,          value: tip_backward }    # v2: also `oldest_first`, `chunked`
      - { name: LEDGER_RANGE_START, value: "" }              # empty → resolve from archive HEAD
      - { name: LEDGER_RANGE_END,   value: "1" }             # exclusive floor; 1 means "until genesis"
      - { name: HEARTBEAT_EVERY_N_LEDGERS, value: "1000" }
      - { name: AWS_REGION,         value: REGION }
      - { name: METRIC_NAMESPACE,   value: Prices/Backfill }
      - { name: LOG_LEVEL,          value: info }

    secrets:
      - { name: PG_DSN, valueFrom: arn:aws:secretsmanager:...:secret:prices/rds/backfill-writer }

    logConfiguration:
      logDriver: awslogs
      options:
        awslogs-group: /prices/backfill/sdex
        awslogs-region: REGION
        awslogs-stream-prefix: task
        awslogs-create-group: "true"   # CDK creates the group with 30 d retention
```

**Sizing rationale.** Task 0022's profile measured 311 ledgers/s
single-threaded decode on a developer laptop (`stellar-xdr` v26,
release build). Allocating 2 vCPU on Fargate gives the same effective
budget; archive S3 throughput will saturate before CPU does. Memory
peak in the profile harness is well under 1 GiB (single-ledger
streaming); 4 GiB is conservative headroom for tokio buffers and
the PG client pool.

**Image.** ECR-hosted Rust binary, distroless or `gcr.io/distroless/cc`
base. Built in CI on every push to `develop`; tagged with git SHA.
Production deploy uses immutable tags (no `latest`).

## 4. IAM role contract

Two roles. The **execution role** is what ECS Agent uses to pull the
image and write logs. The **task role** is what the running binary
uses to call AWS APIs.

### 4.1 Execution role: `PricesBackfillExecution-{env}`

Standard ECS execution permissions, scoped:

```jsonc
{
  "Statement": [
    // Pull task's image only — no other ECR repos.
    { "Effect": "Allow",
      "Action": ["ecr:GetAuthorizationToken"],
      "Resource": "*" },
    { "Effect": "Allow",
      "Action": ["ecr:BatchGetImage", "ecr:GetDownloadUrlForLayer"],
      "Resource": "arn:aws:ecr:REGION:ACCOUNT:repository/prices/sdex-backfill" },

    // Write logs to the task's own group.
    { "Effect": "Allow",
      "Action": ["logs:CreateLogStream", "logs:PutLogEvents"],
      "Resource": "arn:aws:logs:REGION:ACCOUNT:log-group:/prices/backfill/sdex:*" },

    // Read the RDS-writer secret at container start.
    { "Effect": "Allow",
      "Action": ["secretsmanager:GetSecretValue"],
      "Resource": "arn:aws:secretsmanager:REGION:ACCOUNT:secret:prices/rds/backfill-writer-*" }
  ]
}
```

### 4.2 Task role: `PricesBackfillSDEX-{env}`

This is the binary's blast radius. Every statement is least-privilege.

```jsonc
{
  "Statement": [
    // Read Stellar public history archive bucket(s). Read-only.
    // Bucket name is configurable per environment; staging may use a
    // mirror, production points at the canonical public archive.
    { "Effect": "Allow",
      "Action": ["s3:GetObject"],
      "Resource": "arn:aws:s3:::STELLAR_ARCHIVE_BUCKET/*" },
    { "Effect": "Allow",
      "Action": ["s3:ListBucket"],
      "Resource": "arn:aws:s3:::STELLAR_ARCHIVE_BUCKET",
      "Condition": { "StringLike": { "s3:prefix": ["ledger/*"] } } },

    // Emit heartbeat + progress metrics.
    { "Effect": "Allow",
      "Action": ["cloudwatch:PutMetricData"],
      "Resource": "*",
      "Condition": { "StringEquals": { "cloudwatch:namespace": "Prices/Backfill" } } },

    // Publish to the alerts topic on terminal-error self-report
    // (CloudWatch alarm handles the heartbeat-stale case).
    { "Effect": "Allow",
      "Action": ["sns:Publish"],
      "Resource": "arn:aws:sns:REGION:ACCOUNT:prices-backfill-alerts" }
  ]
}
```

### 4.3 Explicitly absent — and the test for that

The task role **must not** grant any of the following. The impl task
includes a CDK unit test that asserts these are not in the policy
document:

- `rds:*` on any Block Explorer RDS instance.
- `s3:*` on any Block Explorer S3 bucket (the `stellar-xdr-meta`,
  CH-snapshot, or `soroban-rpc-events` buckets — any account-shared
  BE-owned bucket).
- `ec2:*` on any BE-managed security group or VPC endpoint that
  would imply network reachability to a BE service.
- Cross-account `sts:AssumeRole` into a BE account.

PG access is via the network path only — there is **no IAM-RDS
authentication** here; the role does not appear in any RDS DB
parameter group. PG auth is via username/password from Secrets
Manager. This keeps the IAM contract evaluable from policy alone.

The BE-authored `stellar-xdr` parser crate is embedded **in the
container image** as a Cargo library dependency (per ADR 0002 §3 and
the docs §8 "shared workspace crate" pattern). No BE runtime endpoint
is consulted by the binary at any point.

## 5. Resumability contract

### 5.1 `backfill_progress` table shape

Single-row-per-stream. DDL lands in the impl task; the contract is:

```sql
CREATE TABLE backfill_progress (
    stream          TEXT PRIMARY KEY,            -- 'sdex_backfill' for v1
    current_ledger  BIGINT       NOT NULL,        -- last fully-processed ledger
    direction       TEXT         NOT NULL,        -- 'tip_backward' | 'oldest_first'
    range_start     BIGINT,                       -- nullable; set on first start
    range_end       BIGINT       NOT NULL,        -- exclusive floor (1 for genesis)
    started_at      TIMESTAMPTZ  NOT NULL,
    updated_at      TIMESTAMPTZ  NOT NULL,
    last_heartbeat  TIMESTAMPTZ  NOT NULL
);
```

### 5.2 Per-ledger atomicity

Each ledger is processed in a **single PG transaction**:

```text
BEGIN;
  -- One INSERT … ON CONFLICT DO UPDATE per TradeTick → price_ohlcv row
  -- per task 0022 decode-and-bucket §5.4 (whole-row replacement).
  -- See ADR 0003 for the PK shape.
  INSERT INTO price_ohlcv (timestamp, asset_id, quote_asset_id, granularity, ...) ...
    ON CONFLICT (timestamp, asset_id, quote_asset_id, granularity) DO UPDATE SET ...;
  ... (one per tick) ...

  -- Same tx commits the checkpoint advance.
  UPDATE backfill_progress
     SET current_ledger = $1, updated_at = NOW(), last_heartbeat = NOW()
   WHERE stream = 'sdex_backfill';
COMMIT;
```

Either everything in the ledger lands AND the checkpoint advances,
or nothing does. Crash mid-ledger leaves `current_ledger` pointing
at the **last fully processed** ledger; on restart the binary reads
that value, computes `next = current_ledger - 1` (for tip-backward),
and re-fetches that ledger. The re-fetch produces the same set of
ticks (archive is immutable) and the same UPSERT yields the same
row (whole-row replacement is idempotent for identical input).

### 5.3 First-start initialisation

On a fresh start (no row for the stream), the binary:

1. Reads `LEDGER_RANGE_START` env var. If empty, queries archive HEAD
   (the highest published ledger sequence — typically the network
   tip from a few minutes ago).
2. Inserts the initial `backfill_progress` row with `current_ledger
   = range_start + 1` (so the first decrement lands on `range_start`).
3. Begins the walk.

On restart, the binary reads the existing row and resumes — env vars
are advisory and warn-but-do-not-override the persisted state, so an
accidental different `LEDGER_RANGE_START` on restart does not silently
shift the walk.

### 5.4 Termination

When `current_ledger == range_end`, the binary:

1. Emits a final heartbeat with a terminal flag.
2. Writes a structured `event=backfill_complete` log line.
3. Exits 0. ECS marks the task `STOPPED` with reason `Essential
   container in task exited`.

No row deletion — the completed row stays in `backfill_progress` as
the historical record of the run.

## 6. Heartbeat metric and alarm

### 6.1 Metric

Namespace `Prices/Backfill`. Two metrics per stream:

| Metric              | Unit       | Dimensions          | Emitted when                                     |
| ------------------- | ---------- | ------------------- | ------------------------------------------------ |
| `LedgersProcessed`  | `Count`    | `Stream=sdex`       | Every `HEARTBEAT_EVERY_N_LEDGERS` (default 1000) |
| `Heartbeat`         | `Count`    | `Stream=sdex`       | Same emit point — value always 1                 |

`PutMetricData` is batched: the binary buffers and flushes every 60
seconds OR every 5 000 ledgers (whichever first) to stay well under
the API throttle.

### 6.2 Alarm

```yaml
AlarmName:                  prices-backfill-sdex-heartbeat-stale-{env}
MetricName:                 Heartbeat
Namespace:                  Prices/Backfill
Dimensions:                 { Stream: sdex }
Statistic:                  Sum
Period:                     300                 # 5 min granularity
EvaluationPeriods:          4                   # 4 × 5 min = 20 min
DatapointsToAlarm:          4
Threshold:                  1
ComparisonOperator:         LessThanThreshold
TreatMissingData:           breaching
AlarmActions:               [ arn:aws:sns:...:prices-backfill-alerts ]
```

**Rationale for 20 min.** Task 0022's profile shows ~3 ms decode per
ledger; even at the slow end of archive transport (~150 k ledgers/h)
a healthy task emits a heartbeat every ~24 s at the default
`HEARTBEAT_EVERY_N_LEDGERS=1000`. A 20-minute silence covers transient
S3 5xx retries (the binary's own backoff exhausts in ~5 min, see
§7.1) plus margin, so the alarm fires only on real stuck states.

### 6.3 SNS topic

`prices-backfill-alerts-{env}` — single topic per environment.
Subscribers (email, PagerDuty) are an operations decision and land
in the impl task; the design only requires the topic exist and be
referenced by the alarm.

## 7. Failure modes

### 7.1 Transient archive 5xx

Per-object S3 GET wraps in exponential-backoff with jitter:

| Attempt | Delay  |
| ------- | ------ |
| 1       | —      |
| 2       | 250 ms |
| 3       | 1 s    |
| 4       | 4 s    |
| 5       | 16 s   |
| 6       | 60 s   |

Six attempts ≈ 80 s total. After the sixth, the binary exits non-zero
with `event=archive_fetch_exhausted`; ECS does NOT auto-restart (task
exits, operator triages). The 20-min alarm fires shortly after.

### 7.2 RDS write failure

Treated as fatal. The current ledger's transaction rolls back,
checkpoint is not advanced, binary exits non-zero. On restart the
same ledger is retried. If the failure is persistent (auth, schema
mismatch, disk full), the alarm fires and the runbook directs the
operator to PG-side diagnosis. The binary does not back off and
retry RDS internally — restart is the unit of retry for DB issues.

### 7.3 Parser panic (`stellar-xdr` decode error)

Fatal in v1: exit non-zero, log the offending `ledger_seq` and the
panic message, alarm fires. The recovery path is a parser-crate
bug fix, not a binary-side workaround. v2 may add an opt-in
`SKIP_MALFORMED_LEDGERS=true` env var that logs and advances past
the offending ledger — but only after a real malformed-ledger
incident motivates it; pre-emptive skip-and-log hides upstream bugs.

### 7.4 OOM

Not expected at 2 vCPU / 4 GiB given streaming decode. Mitigation if
encountered: bump to 4 vCPU / 8 GiB in CDK, redeploy. The binary
itself does not need code changes for this.

### 7.5 Clock drift / stuck `last_heartbeat`

The PG `UPDATE … SET last_heartbeat = NOW()` is server-time, so it
follows RDS's clock. CloudWatch alarms use AWS's clock. A multi-minute
clock disagreement between RDS and CW would skew the alarm trigger
but the binary's `event=heartbeat_emitted` log carries the
CW-PutMetricData timestamp so post-hoc reconstruction is possible.
No code-side defence — this is operationally rare and easier to
diagnose than to mask.

## 8. Logging

Log group: `/prices/backfill/sdex` — 30 d retention. Structured JSON
on stdout, one line per event:

```json
{"ts":"2026-05-13T08:21:14.512Z","level":"info","stream":"sdex_backfill",
 "ledger_seq":62442947,"event":"ledger_processed",
 "trade_ticks":24,"upsert_rows":12,"dur_ms":3.41}
```

Stable event names — these are operator-facing:

- `task_started` — once per task start.
- `range_resolved` — after `LEDGER_RANGE_START` / archive-HEAD resolve.
- `ledger_fetched` — emitted at debug only; not standard runtime noise.
- `ledger_processed` — every ledger; primary throughput line.
- `heartbeat_emitted` — every N ledgers; carries metric values.
- `checkpoint_advanced` — every successful ledger commit.
- `archive_fetch_retry` — every retry attempt.
- `archive_fetch_exhausted` — terminal S3 failure.
- `pg_write_failed` — terminal DB failure.
- `parser_panic` — terminal XDR decode failure.
- `backfill_complete` — terminal success.

## 9. Runbook outline

Final runbook lives at `docs/runbooks/backfill-sdex.md` and is
written by the impl task. Structure:

### 9.1 Start

```bash
aws ecs run-task \
  --cluster prices-backfill-{env} \
  --task-definition sdex-backfill-{env} \
  --launch-type FARGATE \
  --network-configuration "awsvpcConfiguration={subnets=[$SUBNET],securityGroups=[$SG],assignPublicIp=DISABLED}" \
  --overrides '{"containerOverrides":[{"name":"sdex-backfill","environment":[{"name":"LEDGER_RANGE_END","value":"1"}]}]}'
```

### 9.2 Observe progress

```bash
# Last fully-processed ledger + heartbeat freshness
psql -c "SELECT stream, current_ledger, direction,
                started_at, updated_at, last_heartbeat
           FROM backfill_progress
          WHERE stream='sdex_backfill'"

# Public-facing aggregate
curl https://prices-api.{env}.../backfill/status | jq
```

### 9.3 Stop (graceful)

```bash
aws ecs stop-task --cluster prices-backfill-{env} --task $TASK_ARN
```

Per-ledger atomic commit means any stop point leaves
`backfill_progress.current_ledger` pointing at a clean last-processed
ledger. The next `run-task` resumes from there.

### 9.4 Resume

Identical to start — the binary detects the existing
`backfill_progress` row and continues.

### 9.5 Alarm response (heartbeat stale > 20 min)

1. Check task status: `aws ecs describe-tasks --cluster ... --tasks ...`.
2. If task is `STOPPED`, fetch the last 100 log lines:
   `aws logs tail /prices/backfill/sdex --since 30m --follow=false`
3. Match the last `event=` field against §8 to pick the failure mode.
4. Follow the failure-mode-specific procedure
   (`archive_fetch_exhausted` → wait for S3 to recover and re-run;
   `pg_write_failed` → PG diagnostics; `parser_panic` → file a
   `stellar-xdr` issue with the offending ledger seq).

## 10. Rust module split — mapping to task 0022's spec

The binary's crate layout maps 1:1 onto task 0022's spec sections so
the AC "spec from task 0022 is folded into the Rust implementation
module" is verifiable clause-by-clause:

| Rust module       | Spec source                                          | Responsibility                                                                  |
| ----------------- | ---------------------------------------------------- | ------------------------------------------------------------------------------- |
| `archive`         | this design §1, §7.1                                 | S3 streaming `GET`; per-object backoff; produces `Bytes` per ledger.            |
| `decode`          | 0022 filter-strategy §1.1-1.3; 0020 G-note §3        | `stellar-xdr::LedgerCloseMeta::from_xdr_bytes`. Single atomic decode per ledger. |
| `filter`          | 0022 filter-strategy §1.4-1.6, §2                    | Post-decode `OperationResultTr` walk; `ClaimAtom` extraction; `txSUCCESS` gate.  |
| `tick`            | 0022 decode-and-bucket §2-§3; 0020 G-note §"Output unit" | `ClaimAtom` → `TradeTick`. Per-variant V0 / ORDER_BOOK / LIQUIDITY_POOL decode. |
| `canonical`       | 0022 decode-and-bucket §1; 0020 G-note §"Pair canonicalisation rule" | Asset canonicalisation, base/quote orientation, asset-id surrogation.           |
| `price`           | 0022 decode-and-bucket §4                            | `amount_bought / amount_sold` → NUMERIC(28,14) with the spec's precision policy. |
| `bucket`          | 0022 decode-and-bucket §5                            | `TradeTick` → `price_ohlcv` 1m row; whole-row replacement UPSERT batch.         |
| `checkpoint`      | this design §5                                       | Read/write `backfill_progress`; transactional commit per ledger.                |
| `heartbeat`       | this design §6                                       | Batched `PutMetricData` to `Prices/Backfill`.                                   |
| `obs`             | this design §8                                       | Structured-JSON `tracing` subscriber on stdout.                                 |
| `main`            | this design §3, §5.3, §5.4                           | Orchestration: range resolve, walk loop, graceful shutdown.                     |

`stellar-xdr` is imported as a Cargo workspace dependency — no
in-repo XDR types. The crate is the BE-authored one per ADR 0002 §3;
the impl task verifies via `cargo tree` that the dep resolves to the
expected source.

## 11. Handoff — implementation checklist

The follow-up impl task (spawned in [§12](#12-spawned-follow-up-tasks))
delivers, in order:

1. **Cargo workspace bootstrap** — Rust workspace root, `sdex-backfill`
   binary crate, `stellar-xdr` dep, CI build job that produces the
   ECR image.

2. **Schema migrations** in the prices-api PG migration tool:
   - `price_ohlcv` PK change per ADR 0003 (add `quote_asset_id` to
     the PK; documented in 0023's S-note).
   - `backfill_progress` table per §5.1 above.

3. **CDK additions under `infra/aws-cdk/`** (depends on task 0011
   landing first):
   - `prices-backfill-{env}` ECS Fargate cluster.
   - `sdex-backfill-{env}` task definition matching §3.
   - `PricesBackfillExecution-{env}` and `PricesBackfillSDEX-{env}`
     IAM roles matching §4. **Unit test asserting §4.3's forbidden
     actions are absent.**
   - CloudWatch alarm matching §6.2 wired to SNS topic from §6.3.
   - CDK code comments referencing ADR 0002, ADR 0003, and this
     G-note.

4. **Rust binary** with module layout per §10. Each module is
   reviewable against the cited spec section.

5. **Runbook at `docs/runbooks/backfill-sdex.md`** per §9.

6. **Staging smoke test:** run the task against a 10 k-ledger range
   in staging; assert `price_ohlcv` rows land and `backfill_progress
   .current_ledger` advances monotonically.

7. **Confirm `cargo tree` resolves `stellar-xdr` to the BE workspace
   source** (or pinned crate version per ADR 0002 §3 — final shape
   set when task 0011 lands).

## 12. Spawned follow-up tasks

This design produces one new backlog task:

- **0027 — SDEX Fargate backfill implementation.** Lands the
  Cargo workspace, Rust binary, schema migrations, CDK stack,
  runbook, and staging smoke test per §11. Blocked on task 0011
  (CDK bootstrap).

No other follow-ups — the 0023 PK ADR (ADR 0003), 0024 enrichment
design, and 0025 live-merge contract (ADR 0004) are already
resolved; 0026 is the 0024 impl follow-up and runs parallel to 0027.
