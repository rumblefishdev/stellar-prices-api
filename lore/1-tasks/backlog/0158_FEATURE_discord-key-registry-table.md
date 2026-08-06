---
id: "0158"
title: "Key registry — table mapping Discord user ID to API Gateway key ID and usage plan"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0156", "0157", "0159", "0160"]
tags: [layer-infra, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, storage, dynamodb, discord, api-keys]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../infra/src/lib/stacks/compute-stack.ts"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      Epic "Storage implication". The registry is the only piece of state the
      epic introduces, and every backend endpoint in [[0160]] reads or writes
      it — so it lands before them.
---

# Key registry — Discord user ID → API Gateway key

## Summary

The dashboard has to answer "which key's usage do I show this person?", and the
issuance flow has to answer "does this person already have a key?". Both need a
mapping from Discord user ID to the API Gateway key id and usage plan. The epic
states plainly that this does not exist in the current schema and has to be
added.

Small in content, load-bearing in position: [[0159]] writes the identity into
it, [[0160]] reads it on every call, and the once-per-month rotation cap is
enforced from a timestamp stored here.

## Context

The AWS side already holds the key itself — its value is retrievable via
`GetApiKey(includeValue=true)`, which the epic uses to avoid storing raw keys
ourselves. What AWS cannot answer is the reverse question: given a Discord
identity, which key is theirs. Nothing in API Gateway is keyed by our notion of
a user.

## Implementation

**From the epic**

- One record per Discord user: Discord user ID → API Gateway key id + usage
  plan id.
- Sized for one active key per account, per the epic's account model — confirm
  through [[0156]] before building, since a different answer changes the shape.

**Follows from the epic, but not stated in it**

- **DynamoDB**, partition key = Discord user ID. See the comparison below.
- Fields beyond the mapping, each with a reason:
  - `createdAt` — support questions ("when did I get this key?").
  - `lastRotatedAt` — the once-per-calendar-month rotation cap in [[0160]] is
    enforced against this; without it the rule has nothing to read.
  - `discordUsername` — so an operator can identify a record without calling
    Discord. Note it goes stale; the ID is the identity, the name is a label.
- **Do not store the raw API key.** The epic's key-reveal design exists
  specifically so we never hold it. Storing it here would quietly reintroduce a
  secret we chose not to own.
- Issue the key with a conditional write (`attribute_not_exists(discordUserId)`)
  so two parallel sign-ins cannot mint two keys for one account — the
  one-key-per-account rule is enforced by the store, not by application timing.
- Table lives in CDK with the rest of the infrastructure, `RETAIN` on delete —
  losing this table orphans every issued key: the keys keep working, but nobody
  can tell whose they are.
- Least-privilege IAM: the onboarding Lambda ([[0160]]) gets item-level
  read/write on this table and nothing wider.

## Why DynamoDB and not ClickHouse

ClickHouse is the obvious "we already have a database" answer, and it is the
wrong engine for this record. The registry is read on every dashboard load by
primary key and written once per user — an OLTP access pattern with no
analytical component at all.

| | **DynamoDB** | **ClickHouse (existing)** |
| --- | --- | --- |
| Read pattern | Point read by partition key, ~5–10 ms | Built for scans; a single-row lookup is possible but not what the engine is for |
| Write semantics | Single-item conditional write is atomic — `attribute_not_exists` settles the double-sign-in race in one call | `ReplacingMergeTree` deduplicates **asynchronously**; a read straight after a write can return the previous row, and there is no conditional insert to lean on |
| Failure domain | Inside AWS, reachable with IAM alone | mTLS through Caddy on Hetzner — a Hetzner incident would take down portal sign-in, not just data freshness |
| Latency from Lambda | Single-digit ms | ~85 ms RTT, the figure ADR 0007 records for this path |
| Cost at this size | Cents per month on-demand | Already paid for — its one genuine advantage |
| Schema ownership | CDK, next to the rest of the infrastructure | `prices-clickhouse`, next to the ingestion pipeline it has nothing to do with |

The deciding pair is the middle two rows: asynchronous merges mean the store
cannot itself guarantee one key per account, and putting Discord sign-in behind
a Hetzner dependency adds a failure mode the portal has no reason to carry.
ADR 0007 chose no VPC and no relational database for the serverless side;
DynamoDB is the option that stays inside that decision rather than reopening it.

## Acceptance Criteria

- [ ] Table exists in CDK, keyed by Discord user ID, holding key id, usage plan
      id, `createdAt`, `lastRotatedAt`
- [ ] Raw key values are absent from the table by design, and that is stated in
      the code, not just here
- [ ] A conditional write prevents two keys being issued to one account under
      concurrent sign-ins
- [ ] Onboarding Lambda has read/write on this table only
- [ ] Removal policy retains data; the consequence of losing the table is
      recorded
- [ ] One-key-per-account assumption from [[0156]] reflected in the schema

## Notes

- The epic's rotation rule deletes the old API Gateway key and creates a new
  one, so `apiKeyId` changes over time while the Discord ID does not. Records
  are updated in place, not appended — there is no key history, and the epic
  does not ask for one.
- If [[0156]] comes back with "more than one active key", this becomes a
  collection per user and the rotation cap needs rethinking; that is the one
  answer that reshapes this task.
