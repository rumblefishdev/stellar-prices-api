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
> **Status:** draft for cross-team discussion. Revised 2026-06-08
> after reading BE's production indexer crate.
> **Why this note exists:** task 0038's 2026-06-08 activation history
> entry promised a "local-only binary + design document" deliverable
> while the original engineering blockers (BE 0227 mTLS endpoint,
> task 0047 cross-tenant throughput) remain open. This document is
> that design.

---

## 0. TL;DR

We are building a **local-only** Rust Lambda binary that mirrors
the **shape** of BE's production indexer (`crates/indexer/` in the
soroban-block-explorer repo) — same doorbell-cursor pattern, same
S3 → SQS → Lambda trigger model, same mTLS-to-Hetzner-CH sink —
but exercised against local fixtures + a stub cursor instead of
real S3 / real CH. It does NOT deploy to AWS, does NOT consume
real SQS messages, does NOT write to Hetzner ClickHouse.

The prototype's value is twofold:

1. **De-risk the binary shape** — prove the kernel from task 0037
   composes correctly with BE's reusable building blocks
   (`xdr-parser`, `db-clickhouse::mtls`, Galexie key derivation)
   and the doorbell-cursor reconcile loop adapts cleanly to our
   narrower extraction surface.
2. **Ground the BE meeting** — Part C of this note is the concrete
   list of cross-team commitments the production Lambda needs.
   The big questions are dependency distribution (`xdr-parser` is
   currently an internal workspace path-dep at BE, not a published
   crate) and ownership of the new SQS queue between our bucket
   notifier and our Lambda.

When the gating events clear (BE 0227 ships; task 0047 verifies
throughput), the prototype's interior survives: only the cursor
store, the S3 client, and the CDK packaging swap from stub
implementations to production wiring.

---

## 1. Reference: BE production indexer (the model we mirror)

Reading `soroban-block-explorer/crates/indexer/` is **prerequisite
context for the meeting** — the shape we propose IS BE's shape,
modulo a different extraction surface (Soroban swaps for price
discovery, not the 17 RMT tables BE writes) and a different
target database (`prices.*` per ADR 0007, not `default.*`).

### 1.1 Patterns we MUST inherit (load-bearing, not preference)

| Pattern | BE source | Why load-bearing |
|---|---|---|
| `reservedConcurrentExecutions = 1` | `compute-stack.ts:260` | Two concurrent invocations would race the CH cursor. Ordering correctness depends on serial execution. |
| Doorbell-cursor reconcile (ignore SQS body; read `max()` from CH) | `handler/mod.rs:160-251` | Order comes from the cursor + S3 contents, not SQS delivery order. Removes any need for FIFO. |
| Last-row-wins commit ordering per ledger | BE: `ledgers` row written last; us: equivalent "cursor advance" written last | A crash mid-ledger resumes cleanly from the unchanged cursor; partial writes get superseded by `ReplacingMergeTree` on next merge. |
| Lambdas outside the VPC, mTLS only | `compute-stack.ts:32-36` (task 0239) | The shared Caddy/Hetzner-CH path is mTLS-terminated; no SG or VPC peering. Putting our Lambda in a VPC would also need a NAT GW for S3. |
| `safe_error_message` redaction | `handler/mod.rs:416-485` | CH `BadResponse` bodies can echo offending row values; their `Display` would leak data into CW Logs. We need the same redactor. |

### 1.2 Patterns we CHOOSE to inherit (sensible defaults, not absolutes)

- **Retry backoff `[50, 200, 800] ms`** (`handler/mod.rs:113`) — three retries, four wire calls total, only on transient errors (network / timeout / 5xx).
- **Partial-batch-failure SQS response** (`handler/mod.rs:64-75, 160-189`) — fail just the offending message, ack the rest.
- **Eager init at cold start** — surface missing env / unreachable extension as a Lambda Init Errors entry, not a per-event panic (`main.rs:40, 50-67`).
- **Structured JSON tracing-subscriber** with `EnvFilter::from_default_env()` driven by `RUST_LOG`.
- **`maxReceiveCount = 10` on the SQS source** (`compute-stack.ts:147`) — higher than the usual 3 because with `concurrency = 1` the ESM over-polls and gets throttled; the queue absorbs that without false-DLQ'ing a processable record.
- **`visibilityTimeout = lambdaTimeout + 60s`** (`compute-stack.ts:139`).

### 1.3 Patterns we DO NOT inherit

- **`default` CH database.** Per ADR 0007 we live in our own `prices.*` database on the same Hetzner cluster.
- **One cursor table named `ledgers`.** BE persists every ledger they see; we only persist ledgers containing pricing-relevant trades. Cursor design is open (Part D.1).
- **Enrichment SQS fan-out.** BE has a separate `enrichment-worker` Lambda fed from the indexer. We don't need that pattern in scope of 0038 — Soroswap/Aquarius asset-discovery is task 0039's job.
- **17 RMT tables.** Our write surface is just `prices.price_ohlcv` (and possibly a small `prices.processed_ledgers` cursor table — see Part D.1).

---

## Part A — Local prototype scope

### A.1 What the binary does

A single Rust binary, `prices-ledger-processor`, that on each
invocation runs the doorbell-cursor reconcile loop locally:

1. Reads its **cursor** — for the prototype, a `--cursor <N>` CLI
   arg (production: a CH-table read).
2. Computes the deterministic S3 key for ledger `cursor + 1` using
   the **same Galexie key derivation as BE** (one's-complement
   prefixes, `.xdr.zst` extension — see §1.3 below).
3. Resolves that key via an `ObjectFetcher` trait — wired in
   prototype mode to a local-disk impl that maps the derived key
   to `fixtures/ledgers/<key>`. Misses → "no new ledger yet, stop"
   (gap-stop is normal; future doorbell resumes).
4. Hits → `zstd`-decompresses + calls
   `xdr_parser::deserialize_batch()` → iterates the
   `LedgerCloseMeta` batch.
5. Per ledger: extracts Soroban contract events via the
   `xdr-parser` walk, normalises into the `SorobanEventRow` shape
   consumed by `dispatch()` from task 0037, groups by
   `(transaction_id, contract_id)`, calls `dispatch()`, collects
   `TradeRow`s.
6. **Buckets** the trades into 1-min OHLCV candles in-process per
   the ADR 0004 merge formula (preserve `open`, overwrite `close`,
   `GREATEST(high)`, `LEAST(low)`, sum `volume_base` /
   `volume_quote_usd` / `trade_count`, recompute `vwap`).
7. **Writes** to a stub sink (see A.7) — no network egress.
8. **Advances the cursor** (writes new value to the prototype
   stub: `out/cursor.txt`) **last** — the equivalent of BE's
   "ledgers row written last" ordering barrier.
9. Loops back to step 2 until a gap, the in-process time budget,
   or `--max-iterations` is hit.

### A.2 Workspace placement + trait seams

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
    │   ├── main.rs               # lambda_runtime entrypoint + CLI mode
    │   ├── reconcile.rs          # doorbell-cursor loop
    │   ├── decode.rs             # xdr-parser walk → SorobanEventRow
    │   ├── bucket.rs             # 1-min OHLCV merge (ADR 0004)
    │   ├── galexie_key.rs        # ledger# → S3 key (copy of BE's)
    │   ├── retry.rs              # [50,200,800]ms backoff
    │   ├── safe_log.rs           # redaction wrappers (mirrors BE)
    │   ├── object_fetcher/       # input abstraction
    │   │   ├── mod.rs            # trait `ObjectFetcher`
    │   │   └── local_disk.rs     # fixtures/ledgers/<key>
    │   ├── cursor/               # cursor abstraction
    │   │   ├── mod.rs            # trait `Cursor`
    │   │   └── stub_file.rs      # out/cursor.txt
    │   └── sink/                 # writer abstraction
    │       ├── mod.rs            # trait `OhlcvSink`
    │       ├── stdout.rs         # JSON-lines to stdout
    │       └── sql_file.rs       # ALTER-friendly SQL dump
    ├── fixtures/                 # gitignored sample ledger files
    └── tests/
        └── reconcile_e2e.rs      # one full loop through fixtures
```

The three trait seams (`ObjectFetcher`, `Cursor`, `OhlcvSink`) are
the **production swap points**. In the production rewrite:

- `LocalDiskFetcher` → `aws_sdk_s3::Client::get_object`
- `StubFileCursor` → CH-backed cursor (see Part D.1)
- `StdoutJsonSink` / `SqlFileSink` → `clickhouse::Client` over
  mTLS, via `db_clickhouse::mtls::client_from_lambda_env`

Everything else — the reconcile loop, the decode, the
bucketing, the redaction, the retry — survives.

### A.3 Inputs — fixtures, not S3 events

For the prototype, fixtures are real Galexie outputs copied locally,
indexed by their **derived** key (so the same `galexie_key.rs`
function we ship works in both modes):

```
packages/prices-ledger-processor/fixtures/
└── ledgers/
    ├── FC45E5FF--62528000-62591999/
    │   ├── FC45E5C4--62528059.xdr.zst    # known Phoenix swap
    │   ├── FC45E5C3--62528060.xdr.zst    # empty
    │   └── FC45E5C2--62528061.xdr.zst    # multi-venue
    └── ...
```

The operator picks fixtures from the 10k uniform sample analysed
in tasks 0046 / 0048 — same evidence base as the decoder spec, so
expected outputs are pre-known. Filling `fixtures/` is a one-time
manual step (`aws s3 cp` against the dev bucket, after which the
prototype is offline-runnable).

**No `S3Event` JSON fixtures.** The doorbell pattern means the
SQS message body would be ignored anyway — fabricating S3-event
JSONs gains us nothing and falsely suggests the Lambda parses
them.

### A.4 Decode boundary — `xdr-parser`

**Significant cross-team item.** BE's `xdr-parser` is a workspace
**path dep** at `soroban-block-explorer/crates/xdr-parser/`, not a
published crate. The prototype needs decisions on:

**Option 1 — Vendor a snapshot** into
`packages/prices-ledger-processor/vendored/xdr-parser/`. Pros:
zero BE coordination; clean Cargo build. Cons: drifts on every
Stellar protocol upgrade; explicit re-sync ceremony.

**Option 2 — Git submodule** of the BE repo, with a Cargo
`path = "../../soroban-block-explorer/crates/xdr-parser"` dep.
Pros: pinned commit, simple update. Cons: weird workspace layout;
breaks `cargo publish` (irrelevant for us) and `nx`-only mental
models.

**Option 3 — Git Cargo dep** against the BE GitHub repo. Pros:
clean Cargo idiom. Cons: requires BE to keep `xdr-parser` a
**top-level package in their workspace** (it already is) and accept
that prices-api pins against specific commits. Stellar-XDR major
bumps still require coordinated PRs.

**Option 4 — Ask BE to publish to a private cargo registry**
(e.g. CodeArtifact). Most disruptive; only justifies itself if
multiple downstream consumers exist.

**Prototype recommendation: Option 3.** It is the cheapest
"works today" option that doesn't impose on BE — we just pin a
commit sha:

```toml
[dependencies]
xdr-parser = { git = "ssh://git@github.com/rumblefishdev/soroban-block-explorer.git", rev = "<sha>", package = "xdr-parser" }
stellar-xdr = "<workspace-pinned-version>"  # transitively required
```

**Production rewrite item:** lock to a tagged release (e.g.
`xdr-parser-v0.4.0`) and agree on a semver discipline (Part C.4).

What we need from the crate (all already exposed per the indexer's
usage at `handler/mod.rs:313-316, 327`):

- `xdr_parser::decompress_zstd(&[u8]) -> Result<Vec<u8>, ParseError>`
- `xdr_parser::deserialize_batch(&[u8]) -> Result<Batch, ParseError>` where `Batch` has `.ledger_close_metas: Vec<LedgerCloseMeta>`
- A walk of `SorobanTransactionMeta.events` that yields the `(transaction_id, contract_id, event_index, topics, data)` tuples the dispatcher expects (BE's `handler/process::parse_ledger` does this; we may not need the full parse, just the events walk — Part C.4 sub-question).

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
tolerates that variant — counts those rows in an
`unimplemented_venue` metric and continues, exactly like the
production Lambda should once those extractors land.

**Implication for the BE meeting:** Soroswap and Aquarius live
ingestion is **gated on extractor work outside this task**
(separate FEATURE tasks, not yet spawned). The Lambda shape is
complete without them; the venues just yield empty output until
their extractors arrive.

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
merge logic: it gets tested in-process and the production rewrite
can either reuse the in-memory merge or translate it to a CH
`AggregatingMergeTree` materialised view (per task 0048's
recommendation §6.3).

### A.7 Sinks + cursor — stub only

Three prototype-side stubs, all pure-local:

1. **`StdoutJsonSink`** — emits one JSON line per `OhlcvRow` to
   stdout. Tail-friendly, grep-able, diff-able across runs.
2. **`SqlFileSink`** — writes one `.sql` file per invocation
   under `out/` containing the `INSERT INTO prices.price_ohlcv ...`
   statements the production writer would emit. Hand to BE in
   the meeting; they can read it and tell us whether the column
   shape lines up with what `prices.*` will host.
3. **`StubFileCursor`** — reads/writes `out/cursor.txt` (a single
   `u64`). Production replaces with a CH-table read (see Part D.1).

**Explicitly out of prototype scope:**

- No `clickhouse::Client` connection (no Hetzner reachability yet).
- No `aws_sdk_s3` client (no AWS reachability).
- No `aws_sdk_sqs` client (no real queue).
- No CloudWatch metric / log emit (stdout structured-JSON is
  enough; CW is a deployment concern).

### A.8 Operator invocation surface

Two modes the operator on a local machine can use:

```bash
# Mode 1: lambda_runtime via cargo-lambda (closer to production)
cargo lambda invoke prices-ledger-processor \
    --data '{"Records":[{"messageId":"local-doorbell","body":"ignored"}]}'

# Mode 2: direct cargo run (faster iteration)
cargo run -p prices-ledger-processor -- \
    --cursor 62528058 \
    --max-iterations 16 \
    --sink stdout
```

Mode 2 is the inner-loop. Mode 1 proves the `provided.al2`
runtime shape works locally and exercises the full doorbell event
deserialise path (even though the body is ignored — same as BE).

### A.9 Prototype acceptance

- [ ] `cargo build -p prices-ledger-processor --release` succeeds.
- [ ] `cargo lambda invoke` against a stub doorbell event, with a
      fixtures dir containing the known Phoenix-swap ledger
      62528059, emits the expected `OhlcvRow` for that swap and
      advances `out/cursor.txt` to 62528059.
- [ ] `cargo run -- --cursor 62528058 --max-iterations 16 --sink sql_file`
      walks contiguous fixtures, produces a `.sql` file whose
      `INSERT ... ON CONFLICT ...` statements use the PK shape
      from ADR 0003 (`timestamp, asset_id, granularity,
      quote_asset_id`) and the merge columns from ADR 0004.
- [ ] Re-running the same invocation from the same starting
      cursor is bit-identical (idempotent; proves the merge is
      deterministic).
- [ ] Hitting a missing fixture is logged as `"reached gap on S3
      — contiguous run done"` (mirrors BE's wording for the same
      condition) and exits cleanly without advancing past the gap.
- [ ] One `tests/reconcile_e2e.rs` test, runnable on a clean
      clone with `nx test prices-ledger-processor`, that covers
      the whole pipeline against three fixture ledgers (swap,
      empty, gap-stop).
- [ ] This G-note's Part C reviewed by BE; their answers captured
      below the questions inline (or as a follow-up G-note).

No deployment, no AWS calls, no live network.

---

## Part B — Out of prototype scope (explicit non-goals)

Listed so the meeting doesn't accidentally extend scope:

- **CDK stack.** No `infra/aws-cdk/` changes. The original
  Implementation Plan Step 4 in this task's README is deferred to
  the production-rewrite task (see Part E).
- **Real S3 → SQS wiring.** No notification configuration on BE's
  bucket; no SQS queue creation.
- **Lambda execution role / IAM.** All AWS-side; deferred.
- **mTLS cert issuance.** No CA call, no Secrets Manager write,
  no Caddyfile change.
- **CloudWatch alarms, X-Ray traces.** Observability is
  prototype-side stdout JSON only.
- **DLQ.** No `aws_sdk_sqs::Client`, no DLQ behaviour modelled.
- **Soroswap / Aquarius extractor bodies.** The prototype tolerates
  `VenueNotImplemented`; those bodies are separate tasks.
- **SDEX trade extraction.** The 0037 kernel currently dispatches
  Soroban-only; classic SDEX ops travel a different path that the
  Lambda inherits when 0022's extractor lands.
- **xdr-parser republishing.** Prototype consumes via git Cargo
  dep against the BE repo on a pinned commit. Tag-pinning and
  semver discipline are Part C.4 items, not prototype work.

---

## Part C — Cross-team contract (BE meeting agenda)

Each item is phrased as a concrete decision we need from BE, with
the prices-api position pre-staked so the meeting is about
confirming or pushing back, not co-designing from scratch.

### C.1 — SQS queue ownership + S3 → SQS notification

**Background.** Post-task-0241 (BE), the indexer is triggered by
**SQS doorbells**, not direct S3 → Lambda. The flow is:

```
ledger object PutObject  →  S3 ObjectCreated event
                         →  SQS message ("doorbell", body ignored)
                         →  Lambda invocation (batchSize=1, concurrency=1)
```

Our Lambda follows the same shape — a separate SQS queue with its
own doorbells, fed from the same `ObjectCreated` events on the
same bucket.

**The ask:** add a **second** event notification on BE's
`stellar-ledger-data` bucket targeting a **prices-api-owned SQS
queue** (`prices-ingest-queue-{env}`), filtered to `.xdr.zst`
suffix (same filter BE uses — `compute-stack.ts:278`).

Why a prices-api-owned queue, not a shared one: failure isolation.
A backlog or DLQ-spam on the prices side mustn't pressure BE's
indexer queue.

**Open sub-questions for the meeting:**

1. SNS-fan-out vs two direct notifications. BE today wires the
   bucket directly to their SQS queue. Adding our queue as a
   second target on the same bucket is supported by S3, but if BE
   anticipates a third or fourth consumer they may prefer to
   move the bucket-side to SNS and let everyone subscribe.
2. Notification filter precision. `.xdr.zst` is bucket-wide;
   ledgers don't have a separate prefix today. If BE plans to
   add other object types to the bucket (snapshot dumps,
   diagnostic exports), we'd want a prefix filter on our
   subscription so we don't process them.

> **✅ RESOLVED — 2026-06-10 cross-team meeting → SNS fan-out.**
>
> BE and prices-api agreed to move the bucket-side to **SNS** rather
> than wire a second direct `S3 → SQS` notification. Final shape:
>
> ```
> ledger PutObject → S3 ObjectCreated
>                  → SNS topic (BE-owned, on stellar-ledger-data)
>                  ├─ SQS  ledger-ingest-{env}    (BE)  — rawMessageDelivery=true
>                  └─ SQS  prices-ingest-{env}     (prices-api) + its own DLQ
>                                                  → prices Lambda (this task)
> ```
>
> **Ownership split (the user's words):** *"BE will refactor the code
> to use SNS; prices-api does its own SQS with DLQ and Lambda."*
>
> - **BE side:** repoint the existing notification to
>   `SnsDestination(topic)` (was `SqsDestination(ingestQueue)`) and
>   re-subscribe their own queue to the topic with
>   `rawMessageDelivery: true` so the SQS body stays byte-identical
>   to today and their indexer's S3-event parser is unchanged. BE
>   adds a topic resource policy permitting the prices-api account
>   to `sns:Subscribe`.
> - **prices-api side:** own `prices-ingest-{env}` SQS + DLQ
>   (`maxReceiveCount = 10`, `visibilityTimeout = lambdaTimeout + 60s`,
>   per §C.8), subscribe it to the BE topic (cross-account), and a
>   queue policy permitting the topic to deliver. This is the
>   prices-side CDK in the Part E punch-list (gated on BE 0227 +
>   task 0047).
>
> **Why SNS over a second direct notification:** failure isolation
> *and* extensibility — a third/fourth consumer (asset-discovery,
> analytics) just adds a subscription with no further change to BE's
> bucket. EventBridge was considered (lighter for BE — additive bus
> toggle, their `S3 → SQS` untouched) but SNS was chosen for lowest
> latency and because the cross-team contract is being negotiated
> around a topic; topic ownership + subscription is tracked by
> **task 0050**.
>
> **Impact on this Lambda's code: none to the reconcile loop.** The
> doorbell is content-free — the handler ignores the SQS message body
> whether it arrives raw or SNS-wrapped — so the doorbell-cursor
> mechanism (`src/reconcile.rs`) is unaffected. Only doc/comment
> narrative and the (gated) CDK wiring carry the SNS shape.
>
> Sub-question (2) (prefix filter): deferred — `.xdr.zst` suffix
> remains sufficient; BE has no plans for other object types on the
> bucket. Revisit only if that changes.

### C.2 — Env-var injection contract (NOT SSM-at-runtime)

**Correction to the earlier draft.** I previously proposed
`/platform/{env}/*` SSM keys read at Lambda runtime. **BE's actual
pattern (compute-stack.ts:261-267) is CDK-time SSM reads baked
into Lambda env vars** at deploy. We mirror that.

**The ask:** BE publishes the following identifiers under
`/platform/{env}/*` for our CDK to consume at deploy time:

| SSM key | Type | Consumed at deploy → injected as env var |
|---|---|---|
| `/platform/{env}/stellar-ledger-data-bucket-name` | String | `BUCKET_NAME` |
| `/platform/{env}/stellar-ledger-data-bucket-arn` | String | (CDK-side, for IAM grant) |
| `/platform/{env}/ch-domain` | String | `CH_DOMAIN` (Caddy host) |
| `/platform/{env}/stellar-network-passphrase` | String | `STELLAR_NETWORK_PASSPHRASE` (xdr-parser cache init) |
| `/platform/{env}/ledger-events-topic-arn` | String | (CDK-side, SNS topic the prices queue subscribes to — added by the §C.1 SNS decision) |

**Why this changes the contract.** No prices-api Lambda runtime
reads from SSM. The Lambda only sees env vars. SSM is the
deploy-time handshake, not a runtime dependency.

**Open sub-question:**

- Does BE already publish a `STELLAR_NETWORK_PASSPHRASE` SSM key
  (mainnet vs testnet)? BE's indexer reads it from env; if their
  CDK reads it from SSM at deploy, point us at the key.

### C.3 — IAM principal authorisation

Lighter than first draft because Caddy's CN mapping (C.6) does
most of the data-plane auth. The remaining IAM grants:

**The ask:** BE's bucket policy + KMS key policy (if SSE-KMS)
explicitly trusts the prices-api Lambda execution role ARN for:

- `s3:GetObject`, `s3:HeadObject` on the bucket
- `kms:Decrypt` on the bucket's KMS key (if any)

The role ARN will be exported from the prices-api CDK stack and
published under `/prices/{env}/lambda-ledger-processor-role-arn`
for BE to consume in their own CDK.

This is the standard cross-stack handshake — contract is "BE
agrees to wire this once our CDK stack lands."

### C.4 — `xdr-parser` distribution model

**The biggest single item in this meeting.** Today
`xdr-parser` is a workspace path-dep in
`soroban-block-explorer/crates/xdr-parser/`, not a published
crate. The prototype runs against a git-source Cargo dep pinned
to a commit (Option 3 in A.4). The production Lambda needs a
sturdier dependency contract.

**The ask:**

1. BE keeps `xdr-parser` as a **top-level workspace package**
   (already true; just confirming nobody intends to fold it into
   the indexer binary).
2. BE publishes **tagged releases** of `xdr-parser`
   (`xdr-parser-vMAJOR.MINOR.PATCH`). Prices-api pins to a tag
   in production, not `main` or a sha.
3. BE commits to **semver discipline** on the public surface
   (the `decompress_zstd` / `deserialize_batch` / `parse_ledger`
   functions and the public types they return). Payload-shape
   changes get a MAJOR bump; additions get MINOR; bug fixes PATCH.
   We don't need an SLA on cadence, just on semver.
4. BE exposes (if not already) the `SorobanTransactionMeta` events
   walk as a **public library function** distinct from
   `parse_ledger`. We don't need the full BE parse — we only need
   the events stream + `(tx_id, contract_id, event_index, topics,
   data)` tuples. If `parse_ledger` is the only entrypoint
   today, we'd ride that (paying the cost of fields we discard);
   if BE is willing to factor the events walk out, that's
   cleaner.

**Open sub-questions:**

- Cargo registry vs git tag: would BE prefer to publish to
  CodeArtifact (or similar)? Git tags work fine for now; flag
  in case BE has a preference.
- `stellar-xdr` version pin. The prototype must use the **same**
  `stellar-xdr` version as `xdr-parser` (Rust ABI). Today BE's
  workspace pins it in the root `Cargo.toml`. Whose pin wins
  when both repos drift? Proposal: prices-api pins to whatever
  the `xdr-parser` tag we depend on transitively requires; we
  follow BE on `stellar-xdr` updates within `xdr-parser` semver.

### C.5 — Reuse of `db-clickhouse::mtls`

**Background.** BE's `db-clickhouse` crate contains
`mtls::client_from_lambda_env(database: &str) -> Result<clickhouse::Client, MtlsError>`
which fetches `{cert, key, ca}` from Secrets Manager via the
Parameters and Secrets Lambda Extension on `localhost:2773`,
parses the PEM bundle, assembles a `rustls::ClientConfig`, and
returns a ready `clickhouse::Client` (`db-clickhouse/src/mtls.rs`).
This is exactly what our Lambda needs.

**The ask:**

- BE is willing to let prices-api depend on **just the `mtls`
  module** of `db-clickhouse`, exposed as a smaller crate (e.g.
  `db-clickhouse-mtls` or `clickhouse-mtls-aws`) — OR
- BE is willing to let prices-api depend on the **whole
  `db-clickhouse` crate** (path `db-clickhouse = { ..., features = ["aws-mtls"] }`),
  accepting we pull in their schema / persist code as dead
  weight in our binary (Cargo dead-code-strips, so wire-size
  impact ≈ zero) — OR
- BE is fine with prices-api **vendoring `mtls.rs` verbatim**
  with a clear "synced from BE rev X" comment.

**Position:** Option 2 (depend on the whole crate) is the
lowest-friction. Cargo's dead-code-elimination handles the unused
modules; we get the helper "for free" and inherit fixes when BE
ships them. If BE prefers we don't carry the dependency, Option 3
(vendor) is acceptable; Option 1 (factor a smaller crate) is the
most disruptive on BE's side.

**Open sub-question:**

- The `mtls::client_from_lambda_env` reads `MTLS_SECRET_NAME` and
  `CH_DOMAIN` env vars. Are those names canonical, or should
  prices-api use a different prefix to avoid clashing if both
  Lambdas ever share a process (they won't, but the env-var
  name is in the public API of the helper)?

### C.6 — Caddyfile `CLICKHOUSE_CN_USER_MAP` for prices-api

**Background.** Per BE's mTLS design
(`db-clickhouse/src/mtls.rs` module docs and task 0240), Caddy
**strips** any client-supplied `X-ClickHouse-User` and re-applies
the user mapped from the certificate's CN via
`CLICKHOUSE_CN_USER_MAP`. The client never sets a user; Caddy
decides.

**The ask:** BE adds two CN → CH-user mappings to the production
Caddy config:

- `prices-api-dev` → `prices_writer_dev` (CH user)
- `prices-api-prod` → `prices_writer` (CH user)

…and provisions the corresponding CH users with `INSERT`, `ALTER`,
`OPTIMIZE`, `SELECT` grants on the **`prices.*`** database only
(no access to `default.*`). The CN values match the issued cert
CNs (Part C.7).

**Open sub-question:**

- Does BE want prices-api to draft the `CREATE USER` DDL itself
  (per ADR 0007's announcement-not-approval norm), or do they
  prefer to author it? Lean: we draft, they apply, we land the
  SQL in `lore/3-wiki/` for traceability.

### C.7 — mTLS cert issuance for `prices-api-{env}`

**Background.** BE operates the CA and the per-service cert
issuance procedure (`infra-hetzner/ca/README.md`).

**The ask (production-only, surfaced now for awareness):**

- BE-operated CA issues two prices-api certs (`prices-api-dev`,
  `prices-api-prod`) with the CNs from C.6.
- Per ADR 0007 Cluster C: per-env, 1-year manual rotation,
  CA-rotation revocation.
- Bundle uploaded to Secrets Manager under
  `${mtlsSecretNamePrefix}/lambda-prices-ledger-processor-{env}`
  (matches BE's naming convention from `compute-stack.ts:251,
  305`); prices-api Lambda role granted Secrets Manager read.

**Gating:** blocked behind BE 0227 (Hetzner CH ships) and task
0047 (cross-tenant throughput verification). In this spec to
confirm the **shape** of the eventual contract, not to schedule
it.

### C.8 — DLQ + lag-alarm coordination

**The ask:**

- prices-api owns its own DLQ for the prices-ingest queue
  (`prices-ingest-dlq-{env}`). `maxReceiveCount = 10` matches
  BE's value for the same reason: with `concurrency = 1` the
  ESM over-polls and gets throttled, which absorbs without
  false-DLQ'ing a processable doorbell.
- Lag alarm: `prices.ledger_processor.lag_seconds` =
  `now() - ledger.closed_at` at invocation time, alarm if >60s
  sustained. Flagged here so BE doesn't see our alarm and
  assume their pipeline is broken — our alarm fires on **our**
  Lambda being behind, not on Galexie being behind.

---

## Part D — Open questions for the meeting

Not commitments; questions where we want BE's input but haven't
pre-staked a position.

### D.1 — Cursor source

BE's cursor is `max(sequence) FROM default.ledgers` — they
persist every ledger they see. We only persist ledgers
containing pricing-relevant trades, so `max(...) FROM
prices.price_ohlcv` is a UNDER-COUNT, not the cursor we need.

**Three options:**

1. **Own cursor table `prices.processed_ledgers`** — single-row,
   updated last per invocation per ADR 0007's last-row-wins
   convention. Pros: independent of BE. Cons: yet another
   `ReplacingMergeTree` to operate.
2. **Cross-DB read of `default.ledgers.max(sequence)`** as our
   ceiling, processed-up-to stored on our side as a small file
   or table. Pros: no parallel state. Cons: couples our cursor
   to BE's persist pipeline; if BE pauses (`indexerLambdaConcurrency
   = 0`), we'd also stall.
3. **Driven purely from S3** — HEAD-probe forward from the last
   confirmed key, keep no cursor in CH. Pros: stateless. Cons:
   restart cost on cold start (scan to find the floor).

**Lean: Option 1.** Independence > parallel-state savings. Worth
~5 minutes of meeting time to confirm BE is fine with us adding
one tiny RMT table to `prices.*`.

### D.2 — OHLCV column shape

ADR 0003 puts `quote_asset_id` in the PK. ADR 0004 adds the
`volume_quote_usd` merge column. Both are prices-api decisions,
but if BE expects to read `prices.price_ohlcv` for any reason
(BE-side analytics, board, debugging), the column shape is a
soft coordination item.

### D.3 — Retention on `prices.*`

prices-api's empirical footprint from task 0046 is ~0.45 GB/yr.
BE's retention policy on the shared cluster — does our database
inherit BE's TTLs, or do we set our own? Lean: own (separate DB
→ separate retention).

### D.4 — Backfill / live coexistence

Stream 1 (ADR 0001) and Stream 2 (ADR 0005) backfill writers
will eventually also write to `prices.*`. The 1-min UPSERT
contract is shared with the live Lambda. Sequencing question:
backfill before live, or backfill into a side table and
`INSERT ... SELECT` into the live table once live tip is healthy?

### D.5 — Empty-ledger optimisation

Task 0048's 10k sample showed most ledgers contain zero
pricing-relevant events. Worth asking BE if they're willing to
pre-tag at the bucket level (e.g. an additional notification on
`*.has-soroban-events.zst` if their pipeline tags such ledgers),
or if we eat the no-op invocations. Likely answer: eat them —
the Lambda no-op path is cheap.

### D.6 — Batch size

BE uses `batchSize = 1` because their concurrency = 1 makes
larger batches pointless. Should we do the same, or — given
that most prices-relevant ledgers cluster and we expect long
gaps — increase to (say) 5 to amortise cold-start over multiple
doorbells? Probably not worth complexity; mirror BE at 1.

---

## Part E — Production rewrite punch list (when gates clear)

Surfaced here so the meeting can react to the **full sequence**.
These items are NOT in scope for this activation; they spawn as
separate backlog tasks when (a) BE 0227 lands and (b) task 0047
verifies throughput.

| # | Item | Est. days |
|---|---|---|
| 1 | Replace `LocalDiskFetcher` with `aws_sdk_s3` GetObject + HeadObject. | 1 |
| 2 | Replace `StubFileCursor` with the cursor strategy chosen in D.1. | 1 |
| 3 | Replace `StdoutJsonSink` / `SqlFileSink` with `db_clickhouse::mtls`-backed `clickhouse::Client` + ADR 0004 merge SQL. | 2 |
| 4 | CDK stack — Lambda function, role, SQS queue + DLQ, S3 notification on BE's bucket, env vars from `/platform/{env}/*` SSM reads, CW alarms. | 3 |
| 5 | mTLS cert issuance + Caddy `CN_USER_MAP` change with BE + cert upload to Secrets Manager. | 1 |
| 6 | Cross-stack handshake — publish Lambda role ARN under `/prices/{env}/...`, BE consumes in their CDK. | 0.5 |
| 7 | Pin `xdr-parser` from commit-sha to first tagged release. | 0.5 |
| 8 | Lag-alarm wiring + dashboard. | 1 |
| 9 | End-to-end smoke from a real `dev`-bucket doorbell. | 1 |

**Total once gates clear: ~11 engineering days.**

---

## Appendix — references

### Code in BE repo (`soroban-block-explorer/`)
- `crates/indexer/src/main.rs` — cold-start shape, env-var contract
- `crates/indexer/src/handler/mod.rs` — doorbell-cursor reconcile loop
- `crates/indexer/src/handler/process.rs` — `parse_ledger` walk
- `crates/xdr-parser/` — XDR decode crate we'll depend on
- `crates/db-clickhouse/src/mtls.rs` — reusable mTLS client builder
- `infra/src/lib/stacks/compute-stack.ts` — Lambda + SQS + DLQ CDK wiring
- `infra-hetzner/Caddyfile` — `CLICKHOUSE_CN_USER_MAP`
- `infra-hetzner/ca/README.md` — cert issuance procedure

### Local docs
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
