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

- [ ] `packages/prices-ledger-processor` binary builds against
      `provided.al2` (cargo lambda or equivalent).
- [ ] Lambda is registered as a second S3 notification target on
      BE's `stellar-ledger-data/` bucket via CDK; no conflict
      with BE's own Ledger Processor registration.
- [ ] Given a recorded `LedgerCloseMeta` containing ≥1 Soroban
      AMM swap and ≥1 SDEX trade, the binary writes the expected
      1-min `price_ohlcv` rows via UPSERT with the ADR 0003 PK
      shape and ADR 0004 multi-source columns.
- [ ] Re-invoking with the same ledger event is idempotent: row
      counts and column values unchanged (incremental-merge
      preserves `open`, refreshes `close`, etc.).
- [ ] `prices.ledger_processor.lag_seconds` metric published to
      CloudWatch; alarm wired up to fire on >60s sustained lag.
- [ ] Integration test covers: S3 event → fetched object →
      decoded XDR → dispatched extract → UPSERTed row against
      a local Postgres mirroring the 0011 schema.
- [ ] Docs: README in `packages/prices-ledger-processor`
      describing the S3 event contract, the BE-coordination
      step for bucket notifications, and the SSM keys consumed.

## Blocked on

- **0011** — RDS + CDK Lambda stack scaffolding + SSM platform
  lookups (VPC, BE bucket ARN). Without 0011, this Lambda has
  no target DB and no CDK stack to live in.
- **0037** — `packages/ledger-processor::dispatch` kernel and
  the `SwapExtractor` trait surface. Without 0037, this task
  has no extraction primitive to call.

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

## Future Work

- Adopt `cargo-lambda-cdk` `RustFunction` (drop the `fromAsset` seam).
- Production-rewrite punch-list — see spec Part E (gated on BE 0227 +
  task 0047): S3-client `ObjectFetcher`, CH-backed cursor (spec D.1),
  mTLS CH `OhlcvSink`, CW lag alarm, end-to-end smoke.
