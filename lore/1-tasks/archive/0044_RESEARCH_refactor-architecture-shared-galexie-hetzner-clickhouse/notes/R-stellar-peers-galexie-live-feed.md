---
title: "R: Stellar peers → Captive Core → Galexie → S3 → Lambda — live feed end-to-end"
type: research
status: developing
spawned_from: ../README.md
spawns: []
tags: [stellar, galexie, captive-core, overlay, s3, lambda, step-3]
links:
  - "https://developers.stellar.org/docs/data/indexers/build-your-own/galexie"
  - "https://developers.stellar.org/docs/data/indexers/build-your-own/ingest-sdk/developer_guide/ledgerbackends/captivecore"
  - "https://github.com/stellar/stellar-core/blob/master/docs/integration.md"
  - "https://github.com/stellar/go/blob/master/ingest/ledgerbackend/stellar_core_runner.go"
  - "https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0054.md"
  - "../../../../../soroban-block-explorer/docs/architecture/infrastructure/infrastructure-overview.md"
  - "../../../../../soroban-block-explorer/docs/architecture/indexing-pipeline/indexing-pipeline-overview.md"
  - "../../../../../soroban-block-explorer/infra/src/lib/stacks/ingestion-stack.ts"
  - "../../../../../soroban-block-explorer/infra/src/lib/stacks/compute-stack.ts"
  - "../../../../../soroban-block-explorer/lore/1-tasks/archive/0001_RESEARCH_galexie-captive-core-setup/notes/R-galexie-cli-and-image.md"
  - "../../../../../soroban-block-explorer/lore/2-adrs/0006_no-s3-lifecycle-on-ledger-data.md"
history:
  - date: 2026-05-18
    status: developing
    who: okarcz
    note: >
      Distilled from Stellar developer docs (Captive Core, Galexie),
      SEP-0054 file-naming spec, stellar-core integration docs,
      stellar/go runner source, and BE's CDK + infra docs.
---

# R: Stellar peers → Captive Core → Galexie → S3 → Lambda — live feed end-to-end

## Purpose

Step 3 of task 0044. Document the live ingestion chain from first
principles so the refactor reasoning does not lean on "BE does
something opaque". Five hops; each section names the *primitive*
and the *concrete BE deployment value* where they diverge.

This is research only. The recommendation (whether and how
prices-api becomes a second consumer) lives in the later `S-*` note.

---

## 1. Stellar overlay — how a non-validating watcher joins mainnet

**Primitive.** Stellar's overlay protocol is symmetric: there is
**no separate "watcher subscription" API**. A watcher is just a
non-validating stellar-core node that joins the overlay and
**computes the ledger locally from consensus messages**, the same
way a validator does. There is no remote "send me LedgerCloseMeta"
feed to subscribe to.

Quoted from
[developers.stellar.org/docs/validators](https://developers.stellar.org/docs/validators):

> "There are two varieties of _non-validating_ nodes that can be
> used for those purposes" — namely **Stellar RPC** and **Galexie**,
> both of which "bundle an optimized 'Captive' Core to serve their
> operational needs."

**Peer discovery.** From
[admin-guide/configuring](https://developers.stellar.org/docs/validators/admin-guide/configuring):

> "in most cases, it is sufficient to simply rely on node's
> built-in peer discovery"
> "your validator can be configured to connect to specific peers
> via `KNOWN_PEERS` and `PREFERRED_PEERS` in the config file"

**Implication.** Whoever runs Galexie runs a Captive stellar-core
binary that does the overlay handshake, builds its own ledger
state, and emits the local view as `LedgerCloseMeta`. The "live
feed" exists only as a derived artifact of that local computation;
there is no external feed prices-api could subscribe to directly
in place of Galexie.

---

## 2. Captive Core — the subprocess and the named pipe

**Primitive.** "Captive Core" is the runtime contract by which a
**parent process** (Galexie, Stellar RPC, Horizon ingest, etc.)
**spawns the `stellar-core` binary as a subprocess** and reads
XDR-encoded `LedgerCloseMeta` from it.

From
[stellar-core integration.md](https://github.com/stellar/stellar-core/blob/master/docs/integration.md):

> "In captive-core mode, this entire per-ledger transition dataset
> is emitted by stellar-core over a (optionally named) pipe, as
> an xdr encoded `LedgerCloseMeta` object."

From the
[stellar/go runner source](https://github.com/stellar/go/blob/master/ingest/ledgerbackend/stellar_core_runner.go):

> "stellarCoreRunner uses a named pipe
> ([wikipedia](https://en.wikipedia.org/wiki/Named_pipe)) to
> stream ledgers directly from Stellar Core."

From
[developers.stellar.org / Captive Core](https://developers.stellar.org/docs/data/indexers/build-your-own/ingest-sdk/developer_guide/ledgerbackends/captivecore):

> "Captive Core invokes the `stellar-core` binary as a subprocess
> to stream ledgers from the Stellar network."
> "It can be used to stream a ledger range from the past or to
> stream new ledgers whenever they are confirmed by the network."

Two run modes (relevant to the BE setup and to any future
prices-api consumer):

- **`runFrom(from uint32)`** — live tailing from a known ledger.
- **`catchup(from, to uint32)`** — bounded range, used for
  backfill.

**Transport.** XDR-encoded `LedgerCloseMeta` frames over an OS
**named pipe** (FIFO). **No file, no S3, no socket inside Captive
Core itself.** Galexie reads the pipe and is the layer that
serializes to durable storage.

**Mainnet cadence.** ~5–6 seconds per ledger close (Stellar
protocol baseline; not freshly quoted in the URLs above but
consistent across Stellar docs and quoted in BE's
indexing-pipeline-overview §line 145–146: *"The design expectation
is roughly one file every 5 to 6 seconds, aligned with ledger-
close cadence."*).

**Captive-core storage.** A scratch directory under the
`CAPTIVE_CORE_STORAGE_PATH` env var (not in TOML) holds the local
BucketListDB state. BucketListDB is the official primary backend
since August 2024.

---

## 3. Galexie — config, file naming, output

### 3.1 Config (TOML)

From the official
[GCS example](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/examples/gcs-export):

```toml
[datastore_config]
type = "GCS"

[datastore_config.params]
destination_bucket_path = "galexie-data/ledgers/testnet"

[datastore_config.schema]
ledgers_per_file = 1
files_per_partition = 10

[stellar_core_config]
network = "testnet"
```

CLI invocation: `galexie append --start <ledger> --config-file <path>`.

**Backends:**
[prereqs](https://developers.stellar.org/docs/data/indexers/build-your-own/galexie/admin_guide/prerequisites):

> "Galexie exports Stellar ledger metadata to Google Cloud
> Storage (GCS) or Amazon Simple Storage Service (S3)."

**Networks:** `testnet` or `pubnet` shorthand under
`[stellar_core_config]` (auto-configures passphrase + history
archive URLs).

### 3.2 File naming — SEP-0054

From
[SEP-0054](https://github.com/stellar/stellar-protocol/blob/master/ecosystem/sep-0054.md):

- **Partition path** —
  `fmt.Sprintf("%08X--%d-%d/", math.MaxUint32-partitionStartLedgerSequence, partitionStartLedgerSequence, partitionEndLedgerSequence)`
  → e.g. `FFFFFFFF--0-63999/`.
- **Batch (single ledger)** —
  `%08X--%d.xdr.zst` → e.g. `FFFFFF37--200.xdr.zst`.
- **Batch (range)** —
  `%08X--%d-%d.xdr.zst` → e.g. `FFFFFFFD--2-3.xdr.zst`.

Two non-obvious properties:

1. **`MaxUint32 − seq` prefix** is intentional: lexicographic
   listing yields **newest-first**. Useful for `LIST` operations.
2. **Compression is zstd, hardcoded.** Extension is **`.xdr.zst`**;
   Galexie v23.0+ rejects older `.xdr.gz` / `.xdr.zstd` shapes.

### 3.3 Idempotency / dedup

SEP-0054 has **no atomic-write / dedup spec**. Idempotency is
structural: re-emitting ledger N overwrites the same key. The
`append --start N` flag puts responsibility on the operator to
pick a non-overlapping start; Galexie's "checkpoint" on restart
is the **first missing ledger in S3** — that's the resume rule.

---

## 4. BE's concrete pubnet deployment

From
`infra/src/lib/stacks/ingestion-stack.ts` (lines 182–196):

```toml
[datastore_config]
type = "S3"

[datastore_config.params]
destination_bucket_path = "{bucket_name}/"
region = "us-east-1"

[datastore_config.schema]
ledgers_per_file = 1
files_per_partition = 64000

[stellar_core_config]
network = "pubnet"
```

**Concrete values:**

| Setting | Value | Source |
|---|---|---|
| Backend | S3 (`type = "S3"`) | `ingestion-stack.ts:182` |
| Region | `us-east-1` | `ingestion-stack.ts:185` |
| Bucket | `{envName}-stellar-ledger-data` (e.g. `production-stellar-ledger-data`) | `ledger-bucket-stack.ts:29-40` |
| Ledgers per file | **1** (one ledger per file) | `ingestion-stack.ts:188` |
| Files per partition | 64000 | `ingestion-stack.ts:189` |
| Network | `pubnet` (mainnet) | `ingestion-stack.ts:195` |
| Mainnet passphrase | `"Public Global Stellar Network ; September 2015"` | well-known + BE research notes |
| Compression | `zstd` (file extension `.xdr.zst`) | SEP-0054 + BE compute-stack `suffix: '.xdr.zst'` |
| Lifecycle policy | **None** — files retained indefinitely | [BE ADR 0006](../../../../../soroban-block-explorer/lore/2-adrs/0006_no-s3-lifecycle-on-ledger-data.md) |

**Cadence.** One file every ~5–6 seconds (matches mainnet ledger
cadence). At one ledger per file × ~5.5s, that's **~10.9 files
per minute, ~15,700 files per day**.

**Retention.** BE ADR 0006 keeps every file forever. Rationale
quoted in the ADR: replay/debugging, full reprocessing if Lambda
logic changes, storage cost (~$20/mo threshold) low relative to
RDS + NAT Gateway.

**Implication for prices-api.** Live ingest can lean on this
indefinite retention — a prices-api Lambda that lags or errors
during a deploy can replay any historical window by re-firing
S3 events without coordinating with BE. That's a real operational
asset, not a coincidence.

---

## 5. S3 → Lambda event notification (BE's setup)

From
`infra/src/lib/stacks/compute-stack.ts` (lines 236–242):

```typescript
if (config.indexerLambdaConcurrency > 0) {
  ledgerBucket.addEventNotification(
    s3.EventType.OBJECT_CREATED,
    new s3n.LambdaDestination(processorFunction),
    { suffix: '.xdr.zst' }
  );
}
```

**Concrete contract:**

| Aspect | Value |
|---|---|
| Event type | `OBJECT_CREATED` (S3 PutObject) |
| Filter | Key suffix `.xdr.zst` (excludes metadata/logs) |
| Destination | Ledger Processor Lambda (`soroban-explorer-indexer`) |
| Reserved concurrency | `config.indexerLambdaConcurrency` (BE-sized ~20) |
| Retry semantics | Async invoke: 2 retries, then DLQ (SQS) |
| Env vars passed to Lambda | `BUCKET_NAME`, `STELLAR_NETWORK_PASSPHRASE`, `ENRICHMENT_QUEUE_URL` |

**Concurrency sizing.** BE caps at ~20 — sufficient for Galexie's
~12 files/min (`compute-stack.ts:206`). The cap exists to protect
RDS `max_connections`; under the Hetzner-CH refactor this
concern goes away on the BE side, but the Lambda's per-invocation
work has not changed.

---

## 6. Second-consumer registration — what changes when prices-api joins

### 6.1 Today's BE code is single-target

The CDK call in `compute-stack.ts:237` is a **single
`ledgerBucket.addEventNotification(...)`**, not a loop. There is
**no documented mechanism** for multiple concurrent S3 event
targets in the BE infrastructure code today. Prices-api joining
is a **deliberate CDK change**, not a config knob.

### 6.2 The three plausible registration shapes

**Shape A — second `addEventNotification` call.**

Add a parallel notification target alongside BE's. CDK supports
this; both Lambdas receive every `.xdr.zst` PutObject. Cleanest
in code, BUT requires the second Lambda definition to live in
BE's CDK app (or for cross-stack CDK exports to be added so
prices-api's separate CDK app can reference the bucket).

**Shape B — SNS fan-out.**

Insert an SNS topic between the bucket and the consumers. BE
subscribes its Lambda; prices-api subscribes its Lambda. Each
side owns its subscription. Decouples the two consumers; survives
either side adding/removing Lambdas without touching the other.

**Shape C — EventBridge / SQS fan-out.**

S3 → EventBridge → multiple Lambda rules, or S3 → SQS → multiple
consumer SQS subscriptions. Strongest decoupling, more moving
parts, slightly higher per-event latency.

**Recommendation seed.** Shape B (SNS fan-out). It's the
smallest BE-side change that decouples the consumers cleanly:

- BE rewires bucket → SNS, BE-Lambda subscribes. **One-time**
  change on BE side.
- Prices-api subscribes its own Lambda to the SNS topic.
  No further BE involvement when prices-api iterates.

Shape A is acceptable as a fast first step if SNS introduction
is undesirable; Shape C is over-engineered for a two-consumer
fan-out.

### 6.3 Open coordination question

This is a **BE-owned bucket and BE-owned CDK app**. Any
registration shape requires BE to make the change. Forward as
open question to the cross-team conversation alongside
schema-ownership (step 5).

---

## 7. End-to-end picture (for the refactor)

```text
┌──────────────────────────────────────────────────────────┐
│ Stellar mainnet — Public Global Stellar Network ; Sept 2015│
│                                                            │
│  validator-A      validator-B      validator-C   ...       │
│        \              |               /                    │
│         \             |              /                     │
│          ─── overlay (SCP, gossip) ──                      │
└──────────────────────────────┬───────────────────────────┘
                               │ (BE's non-validating
                               │  Captive Core joins
                               │  via KNOWN_PEERS /
                               │  built-in discovery)
                               ▼
┌──────────────────────────────────────────────────────────┐
│ BE ECS Fargate task (us-east-1a, BE-owned)                 │
│                                                            │
│  ┌──────────────────────┐   named pipe   ┌──────────────┐  │
│  │ stellar-core         │ ─────────────▶ │ Galexie       │  │
│  │ (Captive, watcher)   │  XDR-encoded   │ append mode   │  │
│  │ BucketListDB scratch │  LedgerCloseMeta              │  │
│  └──────────────────────┘                └──────┬───────┘  │
└────────────────────────────────────────────────┼──────────┘
                                                 │ PutObject
                                                 │ per ledger (~5–6 s)
                                                 │ `.xdr.zst`, SEP-0054 naming
                                                 ▼
┌──────────────────────────────────────────────────────────┐
│ S3 bucket: {env}-stellar-ledger-data (BE-owned)            │
│ Indefinite retention (BE ADR 0006); no lifecycle rules.    │
└──────────────────────────────┬───────────────────────────┘
                               │ EventBridge / Notification (today: single Lambda)
                               │
              ┌────────────────┼──────────────────────┐
              ▼                                       ▼
   ┌────────────────────┐                ┌──────────────────────┐
   │ BE Ledger Processor│                │ prices-api Ledger    │
   │ Lambda             │                │ Processor Lambda     │
   │ (writes Hetzner CH)│                │ (writes Hetzner CH —  │
   │                    │                │  same instance, own DB)│
   └────────────────────┘                └──────────────────────┘
```

**Where Galexie is shared.** BE owns the ECS task, the bucket,
the lifecycle, the run mode. Prices-api consumes the same files
**read-only** by registering a Lambda against the same bucket.
No prices-api Galexie task is needed; spinning one up would
re-pay BE's cost without benefit.

**Where BE must change.** Bucket notification registration —
either Shape A/B/C from §6.

**Where the prices-api refactor differs from today's RDS plan.**
The bucket → Lambda hop is unchanged; only the Lambda's *write
side* changes (RDS → Hetzner CH over mTLS). The S3 input contract
is identical. **This is the cheapest possible refactor by
construction** — the live-feed-to-Lambda half is already
right, only the storage half flips.

---

## 8. Implications and risks for the refactor

1. **The live feed is already correct.** No second Galexie, no
   second Captive Core, no second peer discovery. Prices-api
   gets the live feed by attaching one more Lambda to one
   existing bucket. That's a real win — it directly extends the
   shared-infra cost story from 0009.

2. **BE owns the choke point.** BE's CDK decides who can attach.
   Prices-api joining requires a BE-side CDK PR. Operationally
   trivial, organizationally a coordination cost.

3. **Lifecycle assumption is load-bearing.** Indefinite retention
   (BE ADR 0006) means prices-api can replay arbitrary windows.
   If BE ever introduces a lifecycle rule for cost reasons,
   prices-api's replay story collapses. Capture as open
   question for the cross-team conversation.

4. **Concurrency cap interaction.** BE caps the BE-Lambda at
   ~20 concurrent invocations to protect RDS. The prices-api
   Lambda would have its **own** concurrency setting; BE's cap
   does not apply. But — both Lambdas share the same Galexie
   throughput baseline (~12 files/min). At ~5s per file, queue
   pressure is irrelevant in steady state.

5. **`.xdr.zst` filter is already strict.** No risk of
   prices-api receiving non-ledger events. Galexie writes one
   class of object.

6. **`stellar-core` binary is the trust root.** Both BE and
   prices-api are downstream of whatever validators Captive
   Core peers with. There is no separate trust boundary the
   prices-api Lambda introduces — same as today.

---

## 9. Open questions surfaced by step 3 (forwarded to README)

11. **Bucket-notification fan-out shape.** Direct dual-Lambda
    registration (Shape A) vs. SNS (Shape B) vs. EventBridge
    (Shape C). Touches BE's CDK; needs BE buy-in.
12. **Retention dependency.** Prices-api's replay story depends
    on BE not adding a lifecycle rule. Document the assumption
    in the eventual ADR; flag in the cross-team conversation.
13. **Concurrency budget.** Prices-api Lambda will have its own
    reserved concurrency. Sizing is independent of BE's ~20, but
    both share the Hetzner CH ingress (mTLS via Caddy:443) —
    the **CH-side capacity** is the real shared constraint, not
    Lambda concurrency. Step 4 (auth + network) picks this up.
14. **Compression library.** The Lambda must zstd-decompress.
    Rust crate `zstd` is the obvious pick; same crate BE's
    indexer uses. Trivial but worth noting (no `gzip` fallback
    needed; SEP-0054 mandates zstd).

## 10. What step 3 does NOT cover

- mTLS connection from Lambda to Hetzner CH — step 4.
- Schema ownership of the `prices` database inside the CH
  instance — step 5.
- Capacity sizing — step 6.
- Final recommendation — step 7.
