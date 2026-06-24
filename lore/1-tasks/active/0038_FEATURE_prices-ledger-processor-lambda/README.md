---
id: "0038"
title: "Prices Ledger Processor Lambda — live S3-event-driven ingestion into price_ohlcv"
type: FEATURE
status: active
related_adr: ["0001", "0003", "0004", "0005", "0006", "0007"]
related_tasks: ["0011", "0037", "0045", "0047", "0048", "0050"]
tags: [layer-indexing, priority-high, effort-large, milestone-M1, stream-1, lambda, ingestion, rust, aws, clickhouse, hetzner]
milestone: 1
links:
  - "../../../../docs/prices-api-general-overview.md"
  - "../../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md"
  - "../../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "../../archive/0011_FEATURE_bootstrap-cdk-with-ssm-platform-lookups.md"
  - "../../archive/0037_FEATURE_tranche1-ledger-processor-skeleton.md"
  - "../../backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md"
history:
  - date: 2026-05-18
    status: backlog
    who: oski
    note: >
      Drafted to fill the gap between 0011 (CDK + RDS bootstrap) and
      0037 (Rust extractor skeleton). The general-overview §5.2 and
      ADR 0001 §4 (point 4 of the Decision) both name a live
      "Prices Ledger Processor Lambda" but no task represented it
      end-to-end. This task wires the 0037 dispatch kernel into a
      Lambda binary that consumes S3 PutObject events on BE's
      `stellar-ledger-data/` bucket and writes 1-min OHLCV rows
      into the prices RDS provisioned by 0011.
  - date: 2026-05-18
    status: blocked
    who: oski
    by: ["0011", "0037"]
    note: >
      Moved to blocked/ — 0011 provides the RDS + Lambda CDK
      stack scaffolding; 0037 provides the extractor kernel and
      dispatch surface. Both are themselves backlog as of this
      date.
  - date: 2026-05-18
    status: blocked
    who: okarcz
    note: >
      Redesign pending. Task 0044's research (synthesis §3) and
      ADR 0007 (proposed) call for major rewrite of this task —
      sqlx → `clickhouse` crate; PG UPSERT with ADR 0004 merge
      formula → ReplacingMergeTree per-source rows; VPC Lambda
      → no-VPC + mTLS. Hold rewrite until both gating events
      clear: (1) BE Hetzner CH ships, (2) ADR 0007 accepted
      (gated on task 0045). Do not start implementation against
      this spec.
  - date: 2026-05-19
    status: blocked
    who: okarcz
    note: >
      Decoder + Lambda E2E spec landed in task 0048's G-note
      (`lore/1-tasks/active/0048_…/notes/G-soroban-events-pricing-decoder.md`).
      Spec is grounded in a 10k uniform sample of the local
      backfill CH and aligned with ADR 0007. When the
      gating events clear, the rewrite implements 0048 directly
      (not the §Implementation Plan in this file, which
      assumed the RDS/VPC shape).
  - date: 2026-05-20
    status: blocked
    who: okarcz
    note: >
      ADR 0007 accepted (PR closing task 0045 lands today; BE
      agreement record at G-be-agreement-record.md is the
      authoritative cross-team contract). The architectural
      uncertainty is resolved; this task's blockers are now
      strictly engineering: (a) BE 0227 ships the Hetzner mTLS
      endpoint, (b) task 0047 verifies cross-tenant throughput
      GREEN/YELLOW (a RED outcome supersedes ADR 0007 to the
      sidecar-CH variant — same rewrite shape, different host).
      Task stays blocked; rewrite begins once (a) and (b) clear.
  - date: 2026-06-08
    status: active
    who: oski
    note: >
      Activated with scope reduction. Original engineering blockers
      ((a) BE 0227 mTLS endpoint, (b) task 0047 cross-tenant
      throughput verification) remain unresolved, so this activation
      is **local-only**: build a runnable local Lambda binary,
      exercise it against recorded fixtures, and produce a written
      design document for cross-team discussion with the BE team.
      Out-of-scope for this activation: any AWS deploy, S3
      notification registration on BE's bucket, SSM platform-key
      consumption, CDK stack apply, or live RDS/CH writes. The
      goal is to give BE something concrete (binary + spec) to
      react to before the gating events clear — see the
      forthcoming G-note on local-prototype scope under
      `notes/G-local-prototype-spec.md`.
  - date: 2026-06-08
    status: blocked
    who: oski
    note: >
      Local-prototype scope shipped: spec G-note + runnable Phase 1
      scaffolding + Phase 2 real XDR decode against
      BE-sourced fixtures (commits f17353f, 1137464, bd2ea9d,
      10b60a3, fb57196 on branch feat/0038_prices-ledger-processor-lambda;
      PR #34). Task moves back to blocked pending the cross-team
      meeting with the BE team — the Part C asks in
      `notes/G-local-prototype-spec.md` are the agenda
      (SQS notification ownership, env-var injection vs SSM-at-runtime,
      xdr-parser tag-pinning + semver, `db-clickhouse::mtls` reuse,
      Caddyfile `CLICKHOUSE_CN_USER_MAP` for `prices-api-{env}`,
      mTLS cert issuance). Original engineering gates
      (BE 0227 + task 0047) also remain open. Unblocks: after the
      meeting answers Part C, and either gating engineering event
      clears.
  - date: 2026-06-10
    status: active
    who: oski
    note: >
      Cross-team meeting held. **Part C.1 RESOLVED: SNS fan-out**
      (not a second direct S3→SQS notification). BE will refactor
      their bucket-side notification from `S3 → SQS` to
      `S3 → SNS → SQS` (`SnsDestination` + `rawMessageDelivery: true`
      so their indexer's S3-event parser is unchanged); prices-api
      owns its **own** `prices-ingest-{env}` SQS queue + DLQ
      subscribing to the BE SNS topic, plus its own Lambda. Failure
      isolation preserved (a prices-side backlog/DLQ never pressures
      BE's indexer queue). The doorbell-cursor design is unaffected
      by the transport choice — the Lambda ignores the message body
      regardless of SNS-vs-SQS — so no reconcile-loop change; only
      doc/comment narrative and the (gated) CDK wiring change.
      Decision recorded inline in `notes/G-local-prototype-spec.md`
      §C.1. The SNS-topic ownership + cross-account subscription is
      the cross-team artefact tracked by task 0050. Moved back to
      active for continued local-scope work; the production AWS
      wiring (Part E) stays gated on BE 0227 + task 0047.
  - date: 2026-06-24
    status: active
    who: oski
    note: >
      Refactored the Lambda onto the **shared, tested ingestion core**
      and landed the two production data-plane seams 0052/0063 unblocked.
      The prototype's hand-rolled decode/bucket/canonicalisation diverged
      from the tested `sdex-backfill` (String asset ids + lexicographic
      orientation + f64, vs the real `price_ohlcv_1m`'s UInt32 surrogate
      ids + SAC→classic collapse + Decimal/version) — so writing it to the
      **shared** `prices.price_ohlcv_1m` would split liquidity. Extracted
      `packages/prices-ingest-core` (canonical/price/tick/bucket/filter/
      soroban + the transport-agnostic `OhlcvWriter`) out of sdex-backfill
      and repointed both crates at it, so live + backfill now emit
      byte-identical rows. Replaced the prototype `bucket.rs`/`decode.rs`/
      stdout+sql_file sinks (→ `.trash/`) with: a core-backed reconcile
      loop, an `S3Fetcher` (`aws-sdk-s3`, `lambda` feature), and a
      `ClickHouseSink` over `prices-clickhouse::mtls` (`aws-mtls` feature,
      the task-0052 client). Default build stays lean (no rustls/lambda);
      `--features lambda` compiles the full SQS-doorbell + S3 + mTLS path.
      Tests: 13 core + 5 sdex (regression gate green) + 15 lambda-unit + 3
      real-fixture e2e (decode→bucket→cursor, gap-stop, idempotent). fmt +
      clippy clean. **Prepare-only — no deploy, no prod writes** (Part E
      deploy/cert/Caddy still gated on BE 0227 + task 0047). Stays active.
  - date: 2026-06-24
    status: active
    who: claude
    note: >
      Applied the safe-set fixes from the PR #34 review (commit 673f775):
      wired INITIAL_CURSOR (prices SSM) / CURSOR_FILE / MAX_ITERATIONS into
      the Lambda env, optional kms:Decrypt grant, SQS maxConcurrency=2,
      BadResponse redaction moved to the core error source, concurrent
      cold-start init. Added a Deploy prerequisites checklist (bootstrap
      cursor SSM param + source-bucket KMS confirmation). Findings #1/#3/#5
      annotated on follow-ups 0064/0065. Stays active.
---

# Prices Ledger Processor Lambda — live S3-event-driven ingestion into price_ohlcv

## Summary

Build the production Lambda function that drives **live go-forward
ingestion** for prices-api: an S3-PutObject-triggered Rust binary
that consumes one `LedgerCloseMeta` XDR file per invocation,
extracts SDEX trades and Soroban AMM swaps via the
`packages/ledger-processor::dispatch` kernel from task 0037, and
UPSERTs 1-minute OHLCV rows into the cloud RDS `price_ohlcv` table
using the PK shape mandated by ADR 0003. This is the on-tip half
of the data ingestion layer described in §1.2 / §5.2 of the
general-overview doc; the historical half lives in ADR 0001
(Stream 1) and ADR 0005 (Stream 2).

## Context

Per the general-overview doc §2.1 (Components Hosted by Prices API)
and §5.2 (Prices Ledger Processor (Rust)), the live ingestion path
is a Rust Lambda driven by a **content-free SQS doorbell**. Per the
2026-06-10 cross-team decision (history below; spec §C.1), BE's
`stellar-ledger-data/` bucket fans out object-created events via
**SNS** (`S3 → SNS → SQS`); prices-api owns its **own**
`prices-ingest-{env}` SQS queue + DLQ subscribed to that topic, so a
prices-side backlog can never pressure BE's indexer queue. (BE's own
queue subscribes to the same topic with `rawMessageDelivery: true`,
leaving their indexer's S3-event parser unchanged.) Per ADR 0001 §4
(Decision point 4 — "Live go-forward Soroban AMM ingestion does
NOT depend on CH"), this Lambda is the system of record for live
Soroban AMM swaps once Stream 1 has landed its one-shot historical
push; CH is bounded to the historical window.

As of 2026-05-18:

- 0011 (CDK bootstrap) provisions the RDS, the Lambda execution
  role, and the per-env CDK stack scaffolding — but does not yet
  create the Prices Ledger Processor function itself.
- 0037 (Tranche 1 Ledger Processor skeleton) lands the workspace
  layout, the per-venue `SwapExtractor` trait, the Phoenix pool
  registry, and a stub `dispatch()` function — but no Lambda
  packaging, no S3 client, no RDS writer, no XDR decode.

This task fills the gap: it takes the kernel from 0037, wraps it
in `lambda_runtime`, and wires the S3-event → XDR-decode →
extract → bucket → UPSERT loop end-to-end.

## Implementation Plan

### Step 1: Lambda binary crate

Add `packages/prices-ledger-processor` (binary crate) inside the
workspace established by 0037. Depend on:

- `lambda_runtime` — `provided.al2` custom runtime entrypoint
  (per overview §8 Tech Stack and ADR 0006 §Decision).
- `aws_sdk_s3` — to GET the ledger object referenced by the event.
- `xdr-parser` — BE-authored crate consumed as a git Cargo dep
  per ADR 0005 §3; decodes `LedgerCloseMeta` and the
  `SorobanTransactionMeta.events` / `OperationResult` shapes.
- `packages/ledger-processor::dispatch` — the kernel from 0037.
- `sqlx` (Postgres, async, compile-time queries) — per ADR 0006.

`main` should: deserialize an `S3Event`, fetch the object,
zstd-decompress, parse as `LedgerCloseMeta`, hand the parsed
ledger to `dispatch()`, bucket the returned trades into 1-min
OHLCV candles, and UPSERT into `price_ohlcv`.

### Step 2: S3-event handler

For each S3 record in the incoming batch:

1. GET the object from the bucket/key in the event.
2. Decompress (`zstd`); the Galexie output is `*.xdr.zstd`
   (§5.1).
3. Parse via `xdr-parser` into `LedgerCloseMeta`.
4. Pass to `dispatch()` — the kernel does Soroban AMM extraction
   today (0037) and SDEX trade extraction once that extractor
   is wired in (see ADR 0002 / task 0022's spec for SDEX
   trade-shaped op types and `ClaimAtom` → `TradeTick`).

### Step 3: 1-minute OHLCV bucketing + UPSERT

Per overview §5.2 "Write semantics — UPSERT, not INSERT":

- Group extracted trades by `(floor_minute(closed_at), asset_id,
  '1m', source)`.
- For each bucket, emit one `INSERT ... ON CONFLICT (timestamp,
  asset_id, granularity) DO UPDATE` with the **incremental-merge**
  update expression (preserve `open`, overwrite `close`,
  `GREATEST(high)`, `LEAST(low)`, sum `volume_base` /
  `volume_quote_usd` / `trade_count`, recompute `vwap`).
- PK is the ADR 0003 shape (`timestamp, asset_id, granularity`)
  — quote_asset_id participates per that ADR if/when the column
  is added; this task follows whatever PK shape the 0011 schema
  migration lands.
- `source` column is set to `'sdex' | 'soroswap' | 'aquarius' |
  'phoenix'` per overview §3.2 examples and ADR 0004's
  multi-source merge columns.

### Step 4: CDK Lambda stack wiring (depends on 0011)

In `infra/aws-cdk/` (created by 0011):

- New `LedgerProcessorStack` (or extension of an existing Lambda
  stack) defining the `prices-ledger-processor` function:
  runtime `provided.al2`, memory 512–1024 MB (size at impl
  time), timeout 60s, VPC attachment to BE's VPC (per §11.1,
  via the 0011 SSM lookups).
- IAM role: read on BE's `stellar-ledger-data/` bucket, write
  on RDS via Secrets Manager DB credentials.
- **S3 notification registration**: add the Lambda as a second
  notification target on BE's bucket. Per §5.1 this requires
  coordination with the BE team (the bucket is BE-owned).
  Document the SSM key BE must publish (e.g.
  `/platform/{env}/stellar-ledger-data-bucket-arn`) and any
  BE-side stack change required.
- CloudWatch alarms: invocation errors, duration p95, DLQ depth
  (deferred to follow-up if DLQ design is non-trivial).

### Step 5: Tests

- Unit: feed a recorded `LedgerCloseMeta` fixture through
  `dispatch()` + bucketing and assert the emitted UPSERT rows
  (use sqlx test-tx pattern).
- Integration: spin up a local Postgres (Docker) with the 0011
  schema applied, run the binary against a recorded S3 event
  + fixture, assert rows land with the expected PK and
  incremental-merge semantics.

### Step 6: Observability

- Structured logs (JSON) per invocation: ledger sequence,
  decode time, trade count by source, UPSERT count, total
  duration. CloudWatch Logs + X-Ray per §2.1.
- Metric: `prices.ledger_processor.lag_seconds` =
  `now() - ledger.closed_at` at invocation time; alarms if
  >60s sustained (matches the §5.1 Galexie lag alarm shape).

## Acceptance Criteria

> Criteria below are the post-ADR-0007 (CH + mTLS + ReplacingMergeTree)
> shape; the original RDS/UPSERT wording is superseded.

- [x] `packages/prices-ledger-processor` builds; the `lambda`-feature
      binary compiles the full `provided.al2023` path (S3 + mTLS +
      `lambda_runtime`). `cargo lambda` ZIP packaging is the deploy step.
- [x] Decode → extract → bucket → write reuses the **tested**
      `prices-ingest-core` (same code as `sdex-backfill`), so live rows
      match the backfill: ADR 0003 PK (`asset_id, quote_asset_id,
      source, timestamp`), ADR 0004 multi-source columns, UInt32
      surrogate ids with SAC→classic collapse, `Decimal(38,14)`.
- [x] Re-invoking from the same cursor is idempotent — proven by the
      `idempotent_on_re_run_from_same_cursor` e2e test (deterministic
      candle set + `version` → ReplacingMergeTree collapses re-inserts).
- [x] Real-fixture integration test: S3-equivalent object →
      `decode_object` → dispatch/extract → bucketed candles → cursor
      advance, gap-stop, idempotency (`tests/reconcile_e2e.rs`,
      self-skips when fixtures absent).
- [x] mTLS sink goes through `prices-clickhouse::mtls` (task 0052), not
      reinvented; CH error bodies redacted via `safe_log` before logging.
- [x] Docs: `packages/prices-ledger-processor/README.md` — S3/SNS event
      contract, BE-coordination (task 0050), env-var/SSM keys consumed.
- [x] Lambda registered as the prices SNS→SQS doorbell target via CDK
      (`infra/.../compute-stack.ts`, prepare-only — 2026-06-10).
- [ ] `prices.ledger_processor.lag_seconds` CloudWatch metric + >60s
      alarm — **deferred** (CW emit is a deploy concern; spec Part E).
- [ ] Live mTLS write against the Hetzner `prices` DB — **deferred**
      (prepare-not-deploy; transport already proven by task 0052's smoke).

## Blocked on

- **0011** — RDS + CDK Lambda stack scaffolding + SSM platform
  lookups (VPC, BE bucket ARN). Without 0011, this Lambda has
  no target DB and no CDK stack to live in.
- **0037** — `packages/ledger-processor::dispatch` kernel and
  the `SwapExtractor` trait surface. Without 0037, this task
  has no extraction primitive to call.

## Deploy prerequisites (operator)

> Prepare-only items the operator must complete before / at deploy. Synth
> fails fast if the SSM param below is absent, so a half-configured Lambda
> can't ship silently.

- [ ] **Bootstrap cursor** — create SSM param
  `/prices/{env}/ledger-processor/initial-cursor` (type `String`) with the
  ledger live ingestion should resume from. The reconcile loop seeds its
  cursor from this on first start and begins at `value + 1`, so set it to the
  **last ledger already accounted for**: the SDEX backfill's
  `max(sequence) FROM prices.backfill_sdex_ledgers` for a seamless handoff,
  or `currentTip − 1` for a forward-only start. Do **not** use `0` (an
  empty-table sentinel — would walk from genesis and never catch up). Wired
  into the Lambda env from this key in `compute-stack.ts`; one-time
  bootstrap, retired by task 0064. (PR #34 review, findings #2/#3.)
- [ ] **Source-bucket KMS** — confirm with BE whether `stellar-ledger-data`
  is SSE-KMS encrypted. If so, set `ledgerProcessor.bucketKmsKeyArn` in
  `infra/envs/{env}.json` so the role gets `kms:Decrypt`; otherwise every
  `GetObject` 403s and the doorbell DLQs. (PR #34 review, finding #4.)

## Out of scope

- SDEX trade extractor body — 0037's skeleton stubs it; the real
  body is task 0022's spec landed under a separate FEATURE task
  (not yet spawned). This Lambda just calls `dispatch()` and
  uses whatever extractors exist at the time.
- Historical backfill — Stream 1 (ADR 0001) and Stream 2
  (ADR 0005) are separate paths; this task is **live only**.
- Asset registry maintenance — handled by 0039's Asset Discovery
  worker.
- Current-price aggregation across sources — handled by 0039's
  Current Price Updater worker.

## Notes

- This is the first Rust Lambda in the project per ADR 0006
  §Decision. Conventions for `cargo lambda` packaging, CI build
  caching, and `provided.al2` ZIP layout established here will
  be reused by 0039 and 0040.
- Coordinate the S3 notification registration with the BE team
  early — adding a second notification target to a bucket BE
  owns is a cross-team change, not a unilateral one.
- The 1-min UPSERT contract is shared with both backfill
  streams; keep the merge SQL in a shared `packages/ohlcv-writer`
  module (or similar) so live + backfill writers stay in sync.

## Implementation Notes

> The `## Implementation Plan` above predates ADR 0007 and still
> describes the retired RDS/sqlx/VPC shape. The authoritative design
> is `notes/G-local-prototype-spec.md` (CH + mTLS + no-VPC +
> SNS-doorbell). What was actually built:

**Local prototype (Phase 1–2, branch `feat/0038_…`, PR #34).**
`packages/prices-ledger-processor` mirrors BE's indexer structure
with three production swap-seams (`ObjectFetcher`, `Cursor`,
`OhlcvSink`). The doorbell-cursor reconcile loop (`src/reconcile.rs`)
reads the cursor, derives the Galexie S3 key for `cursor+1`, fetches,
decodes, dispatches via the 0037 kernel, buckets to 1-min OHLCV, and
**advances the cursor last** — the ordering barrier. Runs against
local fixtures; `cargo check -p prices-ledger-processor` green.

**SNS decision + CDK ingest wiring (2026-06-10).** Folded the live
ingest wiring into `infra/src/lib/stacks/compute-stack.ts`:
prices-owned `prices-ingest-{env}` SQS + `prices-ingest-dlq-{env}`
DLQ (`maxReceiveCount=10`), an SNS subscription to BE's imported
`ledger-events` topic (`rawMessageDelivery`), the ledger-processor
`lambda.Function` (ARM64 / `provided.al2023`, `reservedConcurrency=1`,
`batchSize=1`, `timeout+60s` visibility), the event-source-mapping,
and IAM (S3 read on BE's bucket, CloudWatch lag metric, X-Ray).
Env-var contract sourced from `/platform/{env}/*` SSM at deploy
(spec §C.2, incl. the new `ledger-events-topic-arn` key). `nx build`
+ `cdk synth Prices-production-Compute` both pass. **Prepare-only —
no deploy** (gated on BE 0227 + task 0047 + BE publishing the SSM
keys/topic).

**Shared-core refactor + data-plane seams (2026-06-24).** The
prototype reimplemented decode/bucket/canonicalisation by hand and it
diverged from the tested `sdex-backfill` — fatal once a real sink
writes to the *shared* `prices.price_ohlcv_1m` (different asset ids +
orientation → split liquidity). Fixed by extracting
`packages/prices-ingest-core` (the tested `canonical`/`price`/`tick`/
`bucket`/`filter`/`soroban` modules + a transport-agnostic
`OhlcvWriter` split out of the backfill `Sink`) and repointing **both**
`sdex-backfill` and this Lambda at it. The Lambda now keeps only its
transport shell:
- `src/reconcile.rs` — doorbell-cursor loop calling `prices_ingest_core`
  (`extract_trades` + `process_ledger` → `CandleAccumulator`), warm
  `AssetRegistry` + `Registries` loaded from `prices.assets` at cold
  start, accumulate across the contiguous run, flush + advance cursor
  **last**.
- `src/object_fetcher/s3.rs` — `S3Fetcher` (`aws-sdk-s3` GetObject;
  `NoSuchKey`→gap), `lambda` feature.
- `src/sink/mod.rs` — `ClickHouseSink` over the shared `OhlcvWriter`;
  `plaintext` (local) and `from_lambda_env` (mTLS via
  `prices-clickhouse::mtls`, `aws-mtls` feature); writes retried via
  `retry.rs`, CH errors redacted via `safe_log`.
- `src/bin/cli.rs` — local fixture runner (`--dry-run` counts; else
  writes to local plaintext CH).
- `src/main.rs` — SQS-doorbell entrypoint (`lambda` feature, eager
  cold-start init).

Retired to `.trash/0038-lambda-prototype/`: `bucket.rs`, `decode.rs`,
`sink/{sql_file,stdout}.rs`. Feature matrix: `default` lean (no
rustls/lambda), `aws-mtls`, `lambda` (= `aws-mtls` + runtime + S3).
Tests: 13 core + 5 sdex (regression gate) + 15 lambda-unit + 3
real-fixture e2e. fmt + clippy clean.

**Broken/modified tests:** `tests/reconcile_e2e.rs` rewritten — the old
synthetic-`LedgerDecoder` fakes are gone (the decode seam was removed in
favour of the shared `decode_object`); it now drives the real pipeline
over the three bundled fixture ledgers (62460540–542) and self-skips
when fixtures are absent. Intentional, not a regression.

## Design Decisions

### Emerged

1. **Ingest wiring lives in `ComputeStack`, not a separate
   `IngestStack`.** First drafted as a standalone stack consuming
   ComputeStack's `ledgerProcessorRole`; this created a
   CloudFormation **dependency cycle** — the event-source-mapping and
   the queue/bucket grants mutate the role's policy with the other
   stack's ARNs, so Compute↔Ingest depend on each other. Co-locating
   role + queue + Function in one stack removes the cycle and matches
   BE's single-`compute-stack.ts` shape. (`ingest-stack.ts` moved to
   `.trash/`.)
2. **`lambda.Function` + `Code.fromAsset`, not `RustFunction`.** The
   prices infra doesn't carry `cargo-lambda-cdk`; rather than add an
   uninstalled dependency, the Function consumes the pre-built
   `cargo lambda build` bootstrap. Adopting `RustFunction` (synth-time
   build, exactly BE's shape) is a follow-up once the dep lands.
3. **`reservedConcurrency` pinned to exactly 1 in `validateConfig`.**
   Not a tunable — serial execution is the ordering guarantee, so the
   config validator rejects any other value rather than letting a
   typo silently break ordering at deploy.
4. **Refactor onto the shared core instead of keeping the prototype's
   own decode/bucket (2026-06-24).** The user-confirmed call: a real
   sink writing the prototype's String-id/f64/lexicographic rows to the
   *shared* `prices.price_ohlcv_1m` would not match the backfill →
   split liquidity. Resolved by extracting `prices-ingest-core` and
   reusing it (partial "reconcile" of the live path onto the tested
   code), not by reconciling ids inside the sink. Realises the task's
   own Notes ask ("keep the merge SQL in a shared module so live +
   backfill writers stay in sync").
5. **`OhlcvWriter` takes a `clickhouse::Client`, not a URL.** Lets the
   one writer serve both the plaintext local client and the task-0052
   mTLS client (both are `clickhouse::Client`) — the audit rule that
   every remote CH access goes through `prices-clickhouse::mtls` holds.
6. **Candles accumulate across the whole contiguous run, flushed once.**
   Matches the backfill's per-chunk accumulation so intra-run minutes
   aggregate. A minute split across two *separate* invocations lands as
   two `version`-keyed rows (RMT keeps the latest) — the same
   characteristic the backfill has across partition boundaries; the fix
   is a periodic re-aggregation (spawned as backlog).

## Future Work

> Each item below is spawned as a backlog task (don't leave as prose).

- **0064** — CH-backed cursor (replace `StubFileCursor`; spec D.1).
- **0065** — periodic OHLCV re-aggregation for cross-invocation /
  cross-chunk intra-minute candles (live + backfill share the gap).
- **0066** — `cargo-lambda-cdk` `RustFunction` + CloudWatch
  `lag_seconds` metric/alarm, and unify the dual rustls (0.21 from
  `aws-sdk-s3` vs 0.23 from mTLS) to shrink the Lambda ZIP.
- Production deploy + live end-to-end smoke — spec Part E, still gated
  on BE 0227 + task 0047 (not a standalone backlog item; unblocks with
  those gates).
