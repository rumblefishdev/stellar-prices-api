---
id: "G-be-sns-fanout-ask"
title: "BE-side ask — SNS fan-out on stellar-ledger-data (ready-to-implement)"
type: G
task: "0050"
status: mature
spawned_from: []
spawns: []
related_notes: []
links:
  - "../../active/0038_FEATURE_prices-ledger-processor-lambda/notes/G-local-prototype-spec.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
---

# BE-side ask — SNS fan-out on `stellar-ledger-data`

> **Audience:** BE team (soroban-block-explorer infra).
> **Status:** agreed at the 2026-06-10 cross-team meeting; this is the
> concrete implementation ask. Scopes **item 1** of task 0050 (the SNS
> fan-out); the mTLS-cert and `prices.*`-DB items are unchanged and
> live in the parent README.
> **Why now:** prices-api task 0038 has authored its side (PR #34) —
> its CDK already imports the topic ARN, subscribes its own queue, and
> reads the platform SSM keys below. It is **prepare-only** until BE
> lands this.

---

## TL;DR

Move the `stellar-ledger-data` bucket notification from
`S3 → SQS` to `S3 → SNS → SQS`, so a second tenant (prices-api) can
subscribe its own queue. One topic per env, BE's existing indexer
queue re-subscribed with **`rawMessageDelivery: true`** (so the
indexer's S3-event parser is byte-for-byte unchanged), and the topic
ARN published to one SSM key. Same AWS account, so prices owns its
own subscription — BE does **not** need the prices queue ARN or a
cross-account policy.

Target end-state:

```
ledger PutObject → S3 ObjectCreated (.xdr.zst)
                 → SNS topic  {env}-ledger-events           (BE-owned, NEW)
                 ├─ SQS  {env}-ledger-ingest   (BE)   rawMessageDelivery=true
                 └─ SQS  prices-ingest-{env}   (prices, already in PR #34)
```

---

## The change in BE's `infra/.../compute-stack.ts`

Today (BE `compute-stack.ts`):

```ts
// ~L278 — direct S3 → SQS
ledgerBucket.addEventNotification(
  s3.EventType.OBJECT_CREATED,
  new s3n.SqsDestination(ingestQueue),
  { suffix: '.xdr.zst' },
);
```

Proposed:

```ts
import * as sns from 'aws-cdk-lib/aws-sns';
import * as subs from 'aws-cdk-lib/aws-sns-subscriptions';
import * as ssm from 'aws-cdk-lib/aws-ssm';

// NEW — topic the bucket fans out to
const ledgerEventsTopic = new sns.Topic(this, 'LedgerEventsTopic', {
  topicName: `${config.envName}-ledger-events`,
});

// CHANGED — S3 → SNS (was SqsDestination(ingestQueue)). One destination
// per overlapping event+suffix, so this REPLACES the direct wiring.
ledgerBucket.addEventNotification(
  s3.EventType.OBJECT_CREATED,
  new s3n.SnsDestination(ledgerEventsTopic),
  { suffix: '.xdr.zst' },
);

// NEW — BE's own indexer queue re-subscribes. rawMessageDelivery keeps
// the SQS body identical to today, so the indexer's S3-event parser is
// UNCHANGED. (Without it, SNS wraps the event in an envelope and the
// parser breaks on every ledger.)
ledgerEventsTopic.addSubscription(
  new subs.SqsSubscription(ingestQueue, { rawMessageDelivery: true }),
);

// NEW — publish the topic ARN for prices-api's CDK to read at deploy.
new ssm.StringParameter(this, 'LedgerEventsTopicArnParam', {
  parameterName: `/platform/${config.envName}/ledger-events-topic-arn`,
  stringValue: ledgerEventsTopic.topicArn,
});
```

`SnsDestination` auto-adds the topic policy letting S3 publish; CDK
handles that. The `dlq`, `ingestQueue` (visibility, `maxReceiveCount`),
and the indexer's SQS event-source-mapping are all **unchanged** — only
the *source* of the doorbell moves.

---

## SSM keys BE publishes under `/platform/{env}/*`

prices-api's CDK (PR #34) reads these **at deploy time** (never at
Lambda runtime). Canonical names — these are what 0038's stack already
references:

| SSM key | Value | Status |
|---|---|---|
| `/platform/{env}/ledger-events-topic-arn` | the new SNS topic ARN | **NEW (this ask)** |
| `/platform/{env}/stellar-ledger-data-bucket-name` | bucket name | confirm published |
| `/platform/{env}/stellar-ledger-data-bucket-arn` | bucket ARN | confirm published |
| `/platform/{env}/ch-domain` | Caddy/CH host | confirm published |
| `/platform/{env}/stellar-network-passphrase` | mainnet/testnet passphrase | confirm published |

> Supersedes the older `/platform/{env}/stellar-ledger-data-sns-arn`
> name floated in the 0050 README Step 1 — use
> `ledger-events-topic-arn` to match 0038's CDK.

---

## What BE does NOT need to do

- **No prices queue ARN.** Same AWS account, and prices owns the
  subscription side — prices-api's CDK subscribes `prices-ingest-{env}`
  to the topic itself (already in PR #34).
- **No cross-account topic policy.** Same account → prices' deploy
  role subscribes via its own `sns:Subscribe` IAM. *Only* confirm BE's
  topic policy doesn't explicitly restrict subscribers to BE
  principals (a default CDK `sns.Topic` does not).
- **No DLQ / consumer changes.** prices owns its own DLQ + Lambda.

---

## Cutover (BE's critical path — handle with care)

Because S3 allows one destination per overlapping `event + suffix`,
this is a **replace** of the live notification, not an add:

1. Deploy to a non-prod env first; confirm BE's indexer keeps draining
   (this is the `rawMessageDelivery` check — if the indexer starts
   failing to parse, raw delivery wasn't applied).
2. On prod, deploy during a low-write window if possible; the
   `PutBucketNotificationConfiguration` swap is near-atomic but it is
   the one path that must never silently drop ledgers.
3. prices-api subscribes after the topic + SSM key exist.

> **Alternative considered:** EventBridge (additive bucket toggle,
> leaves BE's `S3 → SQS` untouched — lower BE effort/risk). The meeting
> chose **SNS** for lowest latency and because the cross-team contract
> is built around a topic. Recorded here so the trade-off is on file.

---

## Verification (joint)

- A new `.xdr.zst` PutObject delivers to **both** queues independently.
- BE indexer continues processing post-cutover (no parser errors).
- prices subscribes a throwaway queue to the topic ARN and observes a
  delivery — capture the envelope as a fixture for 0038 (per the 0050
  README Step 3 note; still wanted even though the prices Lambda ignores
  the body).

## Gating

Topic + SSM publish can land **independently of BE 0227 / task 0047**
(they're S3+SNS+SSM, not Hetzner-CH). The mTLS-cert and `prices.*`-DB
items of 0050 remain gated on BE 0227; this SNS item does not — it can
ship in Week 1 to unblock 0038's deploy prep.
