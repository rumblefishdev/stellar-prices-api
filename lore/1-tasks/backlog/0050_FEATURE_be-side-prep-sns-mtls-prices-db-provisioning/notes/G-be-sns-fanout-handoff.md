---
id: "G-be-sns-fanout-handoff"
title: "BE handoff — SNS fan-out implementation runbook (step-by-step)"
type: G
task: "0050"
status: mature
spawned_from: ["G-be-sns-fanout-ask"]
spawns: []
related_notes: ["G-be-sns-fanout-ask"]
links:
  - "../../active/0038_FEATURE_prices-ledger-processor-lambda/notes/G-local-prototype-spec.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
---

# BE handoff — SNS fan-out on `stellar-ledger-data`

> **Audience:** BE team (soroban-block-explorer infra).
> **Status:** agreed at the 2026-06-10 cross-team meeting. This is a
> self-contained, copy-pasteable runbook — hand it straight to whoever
> owns `soroban-block-explorer/infra`.
> **Grounded in:** the *current* `infra/src/lib/stacks/compute-stack.ts`
> on `develop` (verified 2026-06-18). Line numbers below are real.
> **Why now:** prices-api's side is already authored (PR #34) — its CDK
> imports the topic ARN, subscribes its own queue, reads the SSM keys
> below, and is prepare-only until you land this. This item is **not**
> gated on BE 0227 or throughput verification (it's pure S3+SNS+SSM), so
> it can ship now.

---

## TL;DR

Today the `stellar-ledger-data` bucket fires `S3 → SQS` straight at the
indexer's `ingestQueue`. We need a **second tenant** (prices-api) to
receive the same `ObjectCreated` doorbells on its own queue. S3 allows
only **one** destination per overlapping `event + suffix`, so the clean
fan-out is to insert an **SNS topic**:

```
ledger PutObject → S3 ObjectCreated (.xdr.zst)
                 → SNS topic  {env}-ledger-events          (BE-owned, NEW)
                 ├─ SQS  {env}-ledger-ingest  (BE)   rawMessageDelivery=true   ← unchanged behaviour
                 └─ SQS  prices-ingest-{env}  (prices-api, already in PR #34)
```

The whole change is **one file** (`compute-stack.ts`) plus publishing a
few SSM keys. Same AWS account as prices-api, so **you do not need any
prices ARN, and no cross-account policy.**

---

## Scope of the change

- **One file:** `infra/src/lib/stacks/compute-stack.ts`.
- **Net-new SSM keys** under a `/platform/{env}/*` namespace (none exist
  today — these are created, not "confirmed").
- **No change** to: the `dlq`, the `ingestQueue` config (visibility,
  `maxReceiveCount`), the indexer's SQS event-source-mapping, or the
  indexer Rust code. Only the *source* of the doorbell moves, and raw
  delivery keeps the SQS body byte-identical.

---

## Step-by-step

### Step 1 — add the three imports

`compute-stack.ts` currently imports `cdk, s3, s3n, sqs` (L1–9). Add:

```ts
import * as sns from 'aws-cdk-lib/aws-sns';
import * as subs from 'aws-cdk-lib/aws-sns-subscriptions';
import * as ssm from 'aws-cdk-lib/aws-ssm';
```

(`ssm` is already a workspace dep — `hetzner-dns-stack.ts` uses it.)

### Step 2 — create the topic

Put this just **above** the bucket-notification block (currently L386).
`config.envName` is the same field used for the queue names
(`${config.envName}-ledger-ingest`):

```ts
// NEW — fan-out topic the bucket publishes to. One per env.
const ledgerEventsTopic = new sns.Topic(this, 'LedgerEventsTopic', {
  topicName: `${config.envName}-ledger-events`,
});
```

### Step 3 — repoint the bucket notification (S3 → SNS)

Replace the existing block at **L386–389**:

```ts
//  ── BEFORE (current L386-389) ──
ledgerBucket.addEventNotification(
  s3.EventType.OBJECT_CREATED,
  new s3n.SqsDestination(ingestQueue),
  { suffix: '.xdr.zst' }
);
```

```ts
//  ── AFTER ──
// S3 → SNS (was SqsDestination(ingestQueue)). S3 allows one destination
// per overlapping event+suffix, so this REPLACES the direct wiring; the
// indexer now receives doorbells via its SNS subscription (Step 4).
ledgerBucket.addEventNotification(
  s3.EventType.OBJECT_CREATED,
  new s3n.SnsDestination(ledgerEventsTopic),
  { suffix: '.xdr.zst' }
);
```

`SnsDestination` auto-adds the topic policy letting S3 publish — CDK
handles that for you.

### Step 4 — re-subscribe the indexer queue (⚠️ `rawMessageDelivery: true`)

This is the one detail that keeps the indexer untouched. With raw
delivery the SQS body is byte-for-byte identical to today's direct
`S3 → SQS` event; without it, SNS wraps the event in an envelope and the
indexer's S3-event parser breaks on **every** ledger.

```ts
// NEW — the indexer's own queue subscribes to the topic. rawMessageDelivery
// keeps the SQS message body identical to the old direct S3→SQS shape, so the
// indexer's event-source-mapping and parser are UNCHANGED.
ledgerEventsTopic.addSubscription(
  new subs.SqsSubscription(ingestQueue, { rawMessageDelivery: true }),
);
```

Leave the existing `processorFunction.addEventSource(new SqsEventSource(
ingestQueue, …))` (L399) and `ingestQueue.grantConsumeMessages(…)` (L411)
exactly as they are — they still drain `ingestQueue`.

### Step 5 — publish the topic ARN to SSM

prices-api's CDK reads this **at deploy time** (never at Lambda runtime):

```ts
// NEW — hand the topic ARN to prices-api's CDK via SSM.
new ssm.StringParameter(this, 'LedgerEventsTopicArnParam', {
  parameterName: `/platform/${config.envName}/ledger-events-topic-arn`,
  stringValue: ledgerEventsTopic.topicArn,
});
```

### Step 6 — publish the remaining `/platform/{env}/*` keys (net-new)

These do **not** exist in your infra today (you only publish
`EcrRepoUriParam` and read the Hetzner CH IP). prices-api's CDK consumes
all of them at deploy. The bucket name/arn are already in hand inside
`ComputeStack` (props `ledgerBucketName` / `ledgerBucketArn`, L20–21), so
publishing them is a one-liner each; `ch-domain` and the network
passphrase come from wherever you keep them today.

| SSM key (String) | Value | Source in your code |
|---|---|---|
| `/platform/{env}/ledger-events-topic-arn` | new SNS topic ARN | Step 5 |
| `/platform/{env}/stellar-ledger-data-bucket-name` | bucket name | `props.ledgerBucketName` |
| `/platform/{env}/stellar-ledger-data-bucket-arn` | bucket ARN | `props.ledgerBucketArn` |
| `/platform/{env}/ch-domain` | Caddy/ClickHouse host | your Hetzner CH domain |
| `/platform/{env}/stellar-network-passphrase` | mainnet/testnet passphrase | indexer env config |

> If any of these already live under a different key name, just tell us
> the names and we'll point prices-api's CDK at them instead — the table
> above is the canonical set our stack references.

### Step 7 — confirm the topic policy isn't subscriber-restricted

Same AWS account, so prices-api subscribes `prices-ingest-{env}` to the
topic via its **own** deploy-role `sns:Subscribe` IAM — no cross-account
policy needed from you. Only confirm your topic policy doesn't explicitly
restrict subscribers to BE principals. A default CDK `sns.Topic` (as
above) does **not**, so there's normally nothing to do here.

---

## What BE does NOT need to do

- ❌ No prices queue ARN (same account; prices owns the subscribe side).
- ❌ No cross-account topic policy.
- ❌ No DLQ / consumer / event-source-mapping changes.
- ❌ No indexer Rust changes (raw delivery → body unchanged).

---

## Cutover (the one path that must never drop ledgers)

Because S3 permits one destination per overlapping `event + suffix`,
Step 3 is a **replace** of the live notification, not an add:

1. **Deploy to a non-prod env first.** Confirm the indexer keeps draining
   `ingestQueue`. If it suddenly fails to parse messages, raw delivery
   wasn't applied (Step 4) — fix before touching prod.
2. **On prod, deploy in a low-write window if possible.** The
   `PutBucketNotificationConfiguration` swap is near-atomic, but it's the
   single path that must not silently drop a ledger.
3. **prices-api subscribes after** the topic + SSM key exist.

---

## Joint verification

- A new `.xdr.zst` PutObject delivers to **both** queues independently.
- BE indexer continues processing post-cutover with **no parser errors**.
- prices-api subscribes a throwaway queue to the topic ARN and observes a
  delivery (captures the envelope as a test fixture).

---

## Alternative considered (on the record)

EventBridge was weighed — additive bucket toggle, leaves your `S3 → SQS`
untouched, lower BE effort/risk. The meeting chose **SNS** for lowest
latency and because the cross-team contract is built around a topic.
Recorded so the trade-off is on file.

---

*Deeper rationale + meeting provenance: [[G-be-sns-fanout-ask]] and the
0038 spec `notes/G-local-prototype-spec.md` §C.1. Topic ownership is
tracked by this task (0050); prices-api's subscriber-side CDK is in
PR #34 (task 0038).*
