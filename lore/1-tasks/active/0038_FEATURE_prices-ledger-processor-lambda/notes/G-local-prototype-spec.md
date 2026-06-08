---
id: "G-local-prototype-spec"
title: "Local-prototype scope + BE cross-team contract for the Prices Ledger Processor Lambda"
type: G
task: "0038"
status: developing
spawned_from: []
spawns: []
related_notes: []
links:
  - "../../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
  - "../../../../2-adrs/0006_runtime-framework-rust-axum.md"
  - "../../../../2-adrs/0005_stream2-sdex-local-workstation-backfill.md"
  - "../../../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../../../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md"
  - "../../../archive/0048_RESEARCH_soroban-events-pricing-decoder-spec/notes/G-soroban-events-pricing-decoder.md"
  - "../../../archive/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-agreement-record.md"
  - "../../../archive/0037_FEATURE_tranche1-ledger-processor-skeleton.md"
  - "../../../backlog/0047_RESEARCH_cross-tenant-throughput-verification-on-shared-hetzner-ch.md"
  - "../../../../../docs/prices-api-general-overview.md"
---

# Local-prototype spec + BE cross-team contract

> **Audience:** prices-api implementer (Part A), BE team reviewers (Part C).
> **Status:** draft for cross-team discussion.
> **Why this note exists:** task 0038's 2026-06-08 activation history
> entry promised a "local-only binary + design document" deliverable
> while the original engineering blockers (BE 0227 mTLS endpoint,
> task 0047 cross-tenant throughput) remain open. This document is
> that design.

---

## 0. TL;DR

We are building a **local-only** Rust Lambda binary that exercises
the live-ingestion path end-to-end against recorded fixtures —
S3 event → XDR decode → `dispatch()` → 1-min OHLCV bucketing →
**stub sink** (stdout / file emit). It does NOT deploy to AWS, does
NOT register on BE's S3 bucket, does NOT write to Hetzner ClickHouse.
The prototype's value is twofold:

1. **De-risk the binary shape** — prove the kernel from task 0037
   composes correctly with `lambda_runtime`, `aws_sdk_s3` (mocked at
   the trait boundary), and the `xdr-parser` decode crate.
2. **Ground the BE meeting** — Part C of this note is the concrete
   list of cross-team commitments the production Lambda needs.
   Giving BE a runnable binary + a written contract is cheaper than
   asking for those commitments in the abstract.

When the gating events clear (BE 0227 ships; task 0047 verifies
throughput GREEN/YELLOW), the prototype's interior is reused;
only the sink, the S3 client, and the CDK packaging change.

---

## Part A — Local prototype scope

### A.1 What the binary does

A single Rust binary, `prices-ledger-processor`, that on each
invocation:

1. Accepts an `aws_lambda_events::s3::S3Event` JSON document on
   stdin (when run via `cargo lambda invoke`) or on `--event <path>`
   (when run via `cargo run`).
2. For each record in the event, **fetches the referenced object**
   via an `ObjectFetcher` trait — wired in prototype mode to a
   local-disk implementation that maps `s3://bucket/key` to
   `fixtures/<key>`.
3. **zstd-decompresses** the bytes (Galexie output is `*.xdr.zstd`
   per general-overview §5.1).
4. **Decodes** the bytes as `LedgerCloseMeta` via the BE-authored
   `xdr-parser` crate (ADR 0005 §3, ADR 0006 §Decision).
5. **Normalizes** Soroban contract events into the
   `SorobanEventRow` shape consumed by `dispatch()` (the kernel
   from task 0037), grouped by `(transaction_id, contract_id)`.
6. **Calls** `ledger_processor::dispatch::dispatch(&rows, &venue_registry, &phoenix_registry)`
   and collects the returned `TradeRow` set.
7. **Buckets** trades into 1-minute OHLCV candles in-process per
   the merge formula from ADR 0004 §Decision (preserve `open`,
   overwrite `close`, `GREATEST(high)`, `LEAST(low)`, sum
   `volume_base`/`volume_quote_usd`/`trade_count`, recompute `vwap`).
8. **Writes** to a stub sink (see A.7) — no network egress.

### A.2 Workspace placement

```
packages/
├── extractors-core/              # existing (from 0037)
├── ledger-processor/             # existing (from 0037)
├── phoenix-extractor/            # existing (from 0037)
├── soroswap-extractor/           # existing (from 0037, stub)
├── aquarius-extractor/           # existing (from 0037, stub)
├── sdex-backfill/                # existing
└── prices-ledger-processor/      # NEW — this prototype
    ├── Cargo.toml
    ├── src/
    │   ├── main.rs               # lambda_runtime entrypoint
    │   ├── handler.rs            # S3Event → Vec<OhlcvRow>
    │   ├── decode.rs             # xdr-parser → SorobanEventRow
    │   ├── bucket.rs             # 1-min OHLCV merge (ADR 0004)
    │   ├── sink/                 # writer abstraction
    │   │   ├── mod.rs            # trait `OhlcvSink`
    │   │   ├── stdout.rs         # JSON-lines to stdout
    │   │   └── sql_file.rs       # ALTER-friendly SQL dump
    │   └── object_fetcher/       # input abstraction
    │       ├── mod.rs            # trait `ObjectFetcher`
    │       └── local_disk.rs     # `fixtures/<key>` mapping
    ├── fixtures/                 # gitignored sample LedgerCloseMeta
    └── tests/
        └── e2e_fixture.rs        # one ledger end-to-end test
```

The two trait boundaries (`ObjectFetcher`, `OhlcvSink`) are
deliberate seams: production swaps `local_disk` for `aws_sdk_s3`
and `stdout` for a ClickHouse `clickhouse::Client`. Everything
else stays.

### A.3 Inputs — fixture, not S3

For the prototype, fixtures come from BE's existing
`stellar-ledger-data/` bucket layout — we copy a handful of
`*.xdr.zstd` files locally, plus a matching `S3Event` JSON
mocked from CloudTrail format. Concretely:

```
packages/prices-ledger-processor/fixtures/
├── events/
│   ├── single-soroban-swap.json     # 1 record, 1 Phoenix swap
│   ├── multi-swap-batch.json        # 1 record, 4 swaps mixed venues
│   └── empty-ledger.json            # 1 record, no swaps (negative test)
└── ledgers/
    ├── 62019999.xdr.zstd            # known-Phoenix-swap ledger
    ├── 62020247.xdr.zstd            # known multi-venue ledger
    └── 62079982.xdr.zstd            # known empty ledger
```

Fixture ledgers are picked from the 10k uniform sample analysed
in task 0046 / 0048 — same evidence base as the decoder spec, so
expected outputs are pre-known.

### A.4 Decode boundary — `xdr-parser`

The `xdr-parser` BE-authored crate is consumed as a `git`-source
Cargo dependency per ADR 0005 §3:

```toml
[dependencies]
xdr-parser = { git = "ssh://git@github.com/rumblefishdev/soroban-block-explorer.git", branch = "main", package = "xdr-parser" }
```

**Open question for BE (C.4):** the production form needs a
**pinned tag** (e.g. `xdr-parser-v0.4.2`), not a moving `main`.
BE owns the release cadence. The prototype can ride `main` for
now; the production rewrite cannot.

What we need out of the crate:

- `LedgerCloseMeta::decode(&[u8]) -> Result<LedgerCloseMeta, _>`
- A walk of `SorobanTransactionMeta.events` that yields
  `(transaction_id, contract_id, event_index, topics, data)`
  tuples (already implemented in BE's local Ledger Processor —
  we want the same path exposed as a library function).

If BE has not yet exposed that walk as a library API (it may live
inside their Lambda binary today), C.4 asks them to lift it.

### A.5 Kernel boundary — `dispatch()`

The prototype does NOT re-implement extraction. It calls the
existing kernel surface from task 0037:

```rust
use ledger_processor::dispatch::dispatch;
use extractors_core::{SorobanEventRow, TradeRow, VenueRegistry};
use phoenix_extractor::PhoenixPoolRegistry;

let trades: Vec<TradeRow> = dispatch(&rows, &venue_registry, &phoenix_registry)?;
```

Today the kernel implements Phoenix XYK only. Soroswap and
Aquarius extractors return `VenueNotImplemented`. The prototype
tolerates that error variant — it counts those rows in a
`unimplemented_venue` metric and continues, exactly like the
production Lambda should once those extractors land.

**Implication for the BE meeting:** Soroswap and Aquarius live
ingestion is **gated on extractor work that is NOT part of this
task** (separate FEATURE tasks, not yet spawned). The Lambda
shape is complete without them; the venues just yield empty
output until their extractors arrive.

### A.6 OHLCV bucketing

In-process, no DB round-trip. Pseudocode:

```rust
let bucket_key = |t: &TradeRow| OhlcvKey {
    timestamp: floor_to_minute(t.closed_at),
    asset_id: t.base_asset(),
    granularity: Granularity::OneMinute,
    quote_asset_id: t.quote_asset(),  // ADR 0003
    source: t.venue.into(),           // ADR 0004
};

let mut candles: HashMap<OhlcvKey, OhlcvRow> = HashMap::new();
for trade in trades {
    candles.entry(bucket_key(&trade))
        .and_modify(|c| c.merge(&trade))   // ADR 0004 merge formula
        .or_insert_with(|| OhlcvRow::from_first_trade(&trade));
}
```

The `merge` impl is the canonical place to keep the incremental-
merge SQL: it gets tested in-process and the production rewrite
can either reuse the in-memory merge or translate it to a CH
`AggregatingMergeTree` materialised view (per task 0048's
recommendation §6.3).

### A.7 Sinks — stub only

Two prototype sinks, both pure-local:

1. **`StdoutJsonSink`** — emits one JSON line per `OhlcvRow` to
   stdout. Tail-friendly, grep-able, diff-able across runs.
2. **`SqlFileSink`** — writes one `.sql` file per invocation
   under `out/` containing the `INSERT INTO prices.price_ohlcv ...
   ON CONFLICT ...` statements the production writer would emit.
   This is the artefact we hand to BE in the meeting — they can
   read it and tell us whether the column shape lines up with what
   their `prices.*` database (per ADR 0007) is going to host.

**Explicitly out of prototype scope:**

- No `clickhouse::Client` connection (no Hetzner reachability yet).
- No RDS Postgres connection (ADR 0007 supersedes the RDS path).
- No CloudWatch metric / log emit (stdout structured-JSON is
  enough; CloudWatch is a deployment concern).

### A.8 Operator invocation surface

Two modes the operator on a local machine can use:

```bash
# Mode 1: lambda_runtime via cargo-lambda (closer to production)
cargo lambda invoke prices-ledger-processor \
    --data-file fixtures/events/single-soroban-swap.json

# Mode 2: direct cargo run (faster iteration)
cargo run -p prices-ledger-processor -- \
    --event fixtures/events/multi-swap-batch.json \
    --sink stdout
```

Mode 2 is the inner-loop. Mode 1 proves the `provided.al2`
runtime shape works locally.

### A.9 Prototype acceptance

- [ ] `cargo build -p prices-ledger-processor --release` succeeds.
- [ ] `cargo lambda invoke` against `single-soroban-swap.json`
      emits the expected `OhlcvRow` for the known Phoenix XLM/USDC
      swap in ledger 62019999.
- [ ] `cargo run -- --event multi-swap-batch.json --sink sql_file`
      produces a `.sql` file whose `INSERT ... ON CONFLICT ...`
      statements use the PK shape mandated by ADR 0003
      (`timestamp, asset_id, granularity, quote_asset_id`) and the
      merge columns from ADR 0004.
- [ ] Re-running the same invocation is bit-identical (idempotent;
      proves the merge is deterministic).
- [ ] One `tests/e2e_fixture.rs` test, runnable on a clean clone
      with `nx test prices-ledger-processor`, that covers the
      whole pipeline against one of the three ledger fixtures.
- [ ] This G-note's Part C reviewed by BE; their answers captured
      below the questions inline (or as a follow-up G-note).

No deployment, no AWS calls, no live network.

---

## Part B — Out of prototype scope (explicit non-goals)

Listed so the meeting doesn't accidentally extend scope:

- **CDK stack.** No `infra/aws-cdk/` changes. The original
  Implementation Plan Step 4 in this task's README is deferred to
  the production-rewrite task (see Part E).
- **S3 notification registration on BE's bucket.** The prototype
  never touches the real bucket. Registration is a BE-coordination
  step, not a unilateral one (general-overview §5.1).
- **SSM platform-key consumption.** No `/platform/{env}/*` reads.
  The prototype takes bucket name and key prefix as CLI args /
  env vars only.
- **mTLS to Hetzner ClickHouse.** No certificates issued, no
  `clickhouse-rs` wiring. Sink stays local.
- **VPC, IAM, Lambda execution role.** All AWS-side; deferred.
- **CloudWatch alarms, X-Ray traces, DLQ.** Observability is
  prototype-side stdout JSON only.
- **Soroswap / Aquarius extractor bodies.** The prototype tolerates
  `VenueNotImplemented`; those bodies are separate tasks.
- **SDEX trade extraction.** The 0037 kernel currently dispatches
  Soroban-only; classic SDEX ops travel a different path that the
  Lambda inherits when 0022's extractor lands.

---

## Part C — Cross-team contract (BE meeting agenda)

This is the action-item list for the BE conversation. Each item is
phrased as a concrete decision we need from them, with the
prices-api position pre-staked so the meeting is about confirming
or pushing back, not co-designing from scratch.

### C.1 — S3 notification registration on `stellar-ledger-data/`

**The ask:** add `prices-ledger-processor` as a **second**
event-notification target on the existing bucket, for `s3:ObjectCreated:*`
events under the same key prefix BE's own Ledger Processor consumes.

**Why a contract item:** the bucket is BE-owned. Adding a second
target requires a CDK change in BE's infra repo (or wherever the
bucket lives), not in ours.

**Open sub-questions for the meeting:**

- Do BE's event filters include `*.xdr.zstd` only, or do we need
  client-side filtering? (Prices Lambda will filter regardless,
  but we'd rather not fire on irrelevant objects.)
- Should this be SNS-fan-out (per ADR 0007's "Cluster A:
  announcement-not-approval" norm hints at SNS) or two direct
  Lambda subscriptions? Trade-off: SNS adds 1 hop but decouples
  consumer changes from BE's bucket config.

### C.2 — SSM platform keys

Per the `ssm-key-contract-split` memory: `/platform/{env}/*` is
BE-owned, `/prices/{env}/*` is prices-owned. The Lambda needs
to read **identifier-only** values (never bulk trust material)
from `/platform/{env}/*`. Proposed key set:

| Key | Type | Purpose |
|-----|------|---------|
| `/platform/{env}/stellar-ledger-data-bucket-arn` | String | bucket the Lambda is subscribed to |
| `/platform/{env}/stellar-ledger-data-bucket-name` | String | for S3 client GetObject (avoid ARN parse) |
| `/platform/{env}/stellar-ledger-data-kms-key-arn` | String | KMS key the bucket uses for SSE-KMS (if any) so the Lambda role can be granted `kms:Decrypt` |
| `/platform/{env}/hetzner-ch-endpoint` | String | Caddy address for `prices.*` writes (per ADR 0007) |
| `/platform/{env}/hetzner-ch-ca-cert-arn` | String | ARN of the Secrets Manager secret holding the BE-issued CA cert for mTLS validation |

**The ask:** BE commits to populating these keys (with appropriate
IAM read grants for the prices-api Lambda role) and notifying us
before any rotation. **None of these contain secrets**; the mTLS
key+cert pair lives under `/prices/{env}/*` and is owned by us.

**Open sub-question for the meeting:**

- Naming — do the keys above match BE's existing `/platform/`
  conventions, or should they live under a sub-namespace
  (`/platform/{env}/stellar-ledger-data/...`)?

### C.3 — IAM principal authorisation

The prototype doesn't need this; the production Lambda does.

**The ask:** BE's bucket-policy and KMS-key-policy explicitly trust
the prices-api Lambda execution role ARN. The role ARN will be
exported from the prices-api CDK stack and published under
`/prices/{env}/lambda-ledger-processor-role-arn` for BE to
consume in their own CDK.

This is the standard cross-account / cross-stack handshake; the
contract is just "BE agrees to wire this once it lands."

### C.4 — `xdr-parser` crate publishing

The Lambda depends on BE's `xdr-parser` crate via a `git`-source
Cargo dep (ADR 0005 §3).

**The ask:**

- BE publishes **tagged releases** of `xdr-parser` (e.g.
  `xdr-parser-v0.x.y`). Prices-api pins to a tag, not `main`.
- BE exposes the `LedgerCloseMeta` → `(tx_id, contract_id, events)`
  walk as a public library function (not just an internal helper
  in their Lambda binary). If it already is public, point us at
  it.
- BE commits to **semver discipline** on that public surface:
  payload-shape changes get a MAJOR bump, additions get MINOR,
  bug fixes get PATCH. We don't need an SLA on cadence, just on
  semver.

**Open sub-question for the meeting:**

- Cargo registry vs git tag: would BE prefer to publish to a
  private cargo registry (crates.io is public; there's no
  obvious private registry today)? Git tags are fine for now;
  flagging in case BE has a preference.

### C.5 — Hetzner ClickHouse mTLS write contract

Per ADR 0007 §Decision: prices-api writes into a separate `prices.*`
database on BE's Hetzner CH cluster over mTLS via Caddy.

**The ask (production-only, surfaced now for awareness):**

- A `prices` database (CH-level), not `default`. ADR 0007 §5
  notes the "separate-`prices`-database shape" was the all-yes
  outcome of task 0045's Cluster A.
- A CH user `prices_writer` (or similar) with `INSERT`, `ALTER`,
  `OPTIMIZE`, `SELECT` (for self-readback) on `prices.*` only.
- mTLS cert issuance: BE-operated CA issues per-env certs
  (`prices-api-dev`, `prices-api-prod`) per ADR 0007 §Decision
  Cluster C (per-env mTLS, 1-year manual rotation,
  CA-rotation revocation).
- Caddy endpoint reachable from the Lambda's outbound CIDR
  (Lambdas without VPC use the AWS public egress — confirm
  whether BE wants to whitelist or relies purely on mTLS).

**Gating:** this whole item is blocked behind BE 0227 (Hetzner CH
ships) and task 0047 (cross-tenant throughput verification). It
is in this spec to confirm the **shape** of the eventual contract,
not to schedule it. A RED outcome from task 0047 supersedes
ADR 0007 to the sidecar-CH variant — same shape, different host.

### C.6 — DLQ, retry, lag alarms

Lambda-side concerns where BE's S3 retry semantics intersect with
our DLQ / lag-alarm story:

**The ask:**

- Confirm BE's bucket has `s3:ObjectCreated:*` notifications
  configured with the default at-least-once delivery semantics
  (i.e. we should treat duplicate invocations as normal, not
  exceptional — the prototype's idempotent merge per A.9 is the
  right design).
- Agree on a DLQ pattern: per general-overview §5.2 we plan a
  per-Lambda SQS DLQ for messages that fail decode or write 3x.
  Confirm BE is OK with us re-fetching the same object after
  re-driving from DLQ (i.e. no expiration on the ledger objects
  for at least DLQ retention).
- Agree on a lag alarm: `prices.ledger_processor.lag_seconds`
  = `now() - ledger.closed_at` at invocation time, alarm if
  >60s sustained. Matches the Galexie §5.1 lag-alarm shape;
  flagged here so BE doesn't see our alarm and assume their
  pipeline is broken.

---

## Part D — Open questions for the meeting

Not commitments; just things we want BE's input on that aren't
yet phrased as concrete asks.

1. **OHLCV column shape — `quote_asset_id` and `quote_volume_usd`:**
   ADR 0003 puts `quote_asset_id` in the PK. ADR 0004 adds the
   `volume_quote_usd` merge column. Both are prices-api decisions,
   but if BE expects to read `prices.price_ohlcv` for any reason
   (BE-side analytics, board), the column shape is a soft
   coordination item.
2. **CH retention on `prices.*`:** prices-api's empirical footprint
   from task 0046 is ~0.45 GB/yr. BE's retention policy on the
   shared cluster — does our database inherit BE's TTLs, or do
   we set our own? Lean: we set our own (separate DB → separate
   retention).
3. **Backfill coexistence:** Stream 1 (ADR 0001) and Stream 2
   (ADR 0005) backfill writers will eventually also write to
   `prices.*`. The 1-min UPSERT contract is shared with the live
   Lambda. Sequencing question: do we backfill before the live
   Lambda goes live, or backfill into a side table and `INSERT
   ... SELECT` into the live table once the live tip is healthy?
4. **Empty-ledger optimization:** task 0048's 10k sample showed
   most ledgers contain zero pricing-relevant events. Worth
   asking BE if they're willing to pre-filter at the bucket
   level (e.g. only notify on `*.has-soroban-events.zstd` if
   their pipeline tags such ledgers), or if we just eat the
   no-op invocations on our side.

---

## Part E — When gates clear: production rewrite punch list

Surfaced here so the meeting can react to the **full sequence**, not
just the prototype. These items are NOT in scope for this
activation; they spawn as separate backlog tasks when (a) BE 0227
lands and (b) task 0047 verifies throughput.

1. Replace `LocalDiskFetcher` with `aws_sdk_s3` GetObject. (~1 day)
2. Replace `StdoutJsonSink` / `SqlFileSink` with
   `clickhouse::Client` + mTLS + the ADR 0004 merge SQL. (~3 days)
3. CDK stack — Lambda function, role, S3 notification, SSM reads,
   CloudWatch alarms, DLQ. (~3 days)
4. Cert issuance + rotation playbook (mTLS to Hetzner CH). (~1 day)
5. Cross-stack handshake — publish Lambda role ARN under
   `/prices/{env}/...`, BE consumes it in their CDK. (~0.5 day)
6. xdr-parser pin from `main` to first tagged release. (~0.5 day)
7. Lag-alarm wiring + dashboard. (~1 day)
8. End-to-end smoke from a real ledger-data event in `dev`. (~1 day)

Total once gates clear: roughly 10 engineering days.

---

## Appendix — references

- General overview §5.2 — Prices Ledger Processor (Rust)
- ADR 0001 — Stream 1 historical backfill (CH-sourced)
- ADR 0003 — `price_ohlcv` PK shape with `quote_asset_id`
- ADR 0004 — multi-source merge columns
- ADR 0005 — Stream 2 backfill; xdr-parser as git Cargo dep
- ADR 0006 — runtime framework Rust/axum
- ADR 0007 — live data sink on shared Hetzner ClickHouse
- Task 0037 — Tranche 1 Ledger Processor skeleton (the kernel)
- Task 0048 — Soroban events pricing decoder spec
- Task 0045 — BE agreement record (G-note)
- Task 0047 — cross-tenant throughput verification (gating)
