---
id: "0158"
title: "Key registry — ClickHouse table mapping Discord user ID to API Gateway key ID and usage plan"
type: FEATURE
status: backlog
related_adr: ["0007", "0008"]
related_tasks: ["0156", "0157", "0159", "0160"]
tags: [layer-infra, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, storage, clickhouse, discord, api-keys, mtls]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../packages/prices-clickhouse"
  - "../../../infra/src/lib/mtls.ts"
  - "../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      Epic "Storage implication". The registry is the only piece of state the
      epic introduces, and every backend endpoint in [[0160]] reads or writes
      it — so it lands before them.
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Rewritten after the 2026-08-07 product meeting. Store is **ClickHouse**,
      not DynamoDB, and the endpoints live in the existing `prices-api` Lambda
      rather than a new one. Two consequences of that are now accepted risks
      rather than solved problems — see "Accepted consequences".
---

# Key registry — Discord user ID → API Gateway key

## Summary

The dashboard has to answer "which key's usage do I show this person?", and the
issuance flow has to answer "does this person already have a key?". Both need a
mapping from Discord user ID to the API Gateway key id and usage plan. The epic
states plainly that this does not exist in the current schema and has to be
added.

Small in content, load-bearing in position: [[0159]] writes the identity into
it, [[0160]] reads it on every call, and the once-per-quota-period rework cap is
enforced from a timestamp stored here.

## Context

The AWS side already holds the key itself — its value is retrievable via
`GetApiKey(includeValue=true)`, which the epic uses to avoid storing raw keys
ourselves. What AWS cannot answer is the reverse question: given a Discord
identity, which key is theirs. Nothing in API Gateway is keyed by our notion of
a user.

**Store decision (2026-08-07, product meeting): ClickHouse.** The registry lives
next to the existing schema in `packages/prices-clickhouse` and is read and
written by the existing `prices-api` Lambda, which already builds an mTLS
ClickHouse client at cold start (`CH_ENABLED: 'true'`). No new datastore, no new
Lambda, no new build. The two costs of that choice are recorded below rather
than left to be discovered later.

## Implementation

**From the epic**

- One record per Discord user: Discord user ID → API Gateway key id + usage
  plan id.
- Sized for one active key per account, per the epic's account model — confirm
  through [[0156]] before building, since a different answer changes the shape.

**Table**

```sql
CREATE TABLE prices.api_key_registry
(
    discord_user_id  String,
    api_key_id       String,                        -- '' while issuance is in flight
    usage_plan_id    String,
    created_at       DateTime64(3, 'UTC'),
    last_rotated_at  Nullable(DateTime64(3, 'UTC')),
    updated_at       DateTime64(3, 'UTC')           -- version column
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY discord_user_id;
```

- Matches the 12 `ReplacingMergeTree` tables already in the schema; no new
  engine, no `PARTITION BY` (the table is a few hundred rows).
- **Every read uses `FINAL`.** Merges are asynchronous, so without it a read
  straight after a write can return a superseded row.
- `last_rotated_at` is the only source of the rework cap in [[0160]] — without
  it the rule has nothing to read.
- **Do not store the raw API key.** The epic's key-reveal design exists
  specifically so we never hold it. Storing it here would quietly reintroduce a
  secret we chose not to own. State this in the DDL comment, not only here.
- No `discord_username` column. It is the one field with no basis in the epic,
  in a document that deliberately declines to hold an email address, and there
  is no deletion path for it — see "Open".

**Issue flow — API Gateway is the arbiter, not the table**

1. Read `FINAL`; a row with non-empty `api_key_id` returns immediately (hot path).
2. `GetApiKeys(name_query = "discord-<userId>")` — AWS, not ClickHouse, is the
   source of truth for whether a key already exists.
3. No key found → `CreateApiKey(name = "discord-<userId>")` + `CreateUsagePlanKey`.
4. `INSERT` the row.
5. Read back `FINAL`; if more than one key exists for this user, keep the one
   with the earliest `createdDate`, `DeleteApiKey` the rest, `INSERT` a
   corrective row.

Step 5 is the reconciler and it is deterministic: both sides of a race read the
same API Gateway list and compute the same winner. **Naming the key after the
Discord user id is what makes every step above possible** — [[0160]] requires
that naming for attribution, and it doubles as the idempotency handle here.

The same reconciler covers two further cases at no extra cost: a row whose
`api_key_id` is empty because a previous attempt died mid-flight, and a row
pointing at a key deleted by hand in the console ([[0160]] Open #4).

**Rework flow**

1. Read `FINAL`; refuse with `409` + `next_eligible_at` unless `last_rotated_at`
   is null or falls before the current quota period start (1st of the month,
   00:00 UTC).
2. Create the new key and attach it to the plan.
3. `INSERT` with the new `api_key_id`, `last_rotated_at = now`, `updated_at = now`.
4. `DeleteApiKey` the old one — **after** the insert, never before. Reversed, a
   crash leaves the user with no working key and no way to recover alone.

## Accepted consequences

**One key per account is no longer enforced by the store.** ClickHouse has no
conditional insert, and `ReplacingMergeTree` deduplicates asynchronously.
`KeeperMap` — the one engine that would give linearizable key-value semantics —
needs ClickHouse Keeper, and the schema has no `Replicated*` engines and no
`ON CLUSTER`; the instance is shared (ADR 0007), so enabling it is not ours to
do. The guarantee therefore moves into the application: deterministic key
naming, the reconciler above, and a standing check for users holding more than
one key. The race window is narrow (two first sign-ins for one account inside
the same insert), the outcome is detectable, and it self-heals. This is an
accepted risk, not a solved problem, and it should be reviewed if the portal
ever sees real concurrency.

**The API Lambda is a reader and now has to write.** `infra/src/lib/mtls.ts`
maps role `api` → CH user `prices_reader` and role `ingestion` →
`prices_writer`; `prices-api` runs with the reader bundle. Three options, in
order of preference:

1. **New role `portal` → CH user `prices_portal`**, granted on this table only.
   One line in the `MtlsRole` union, plus a user and a certificate created by
   whoever owns the shared instance — that handshake is external and needs
   scheduling, so raise it early.
2. Grant `prices_reader` `INSERT` on this table — weakens the reader role for
   every route, not just the portal ones.
3. Hand `prices-api` the `ingestion` bundle — gives the partner-facing Lambda
   write access to the whole ingestion estate. Do not.

## Acceptance Criteria

- [ ] Table exists in `packages/prices-clickhouse`, keyed by Discord user ID,
      holding key id, usage plan id, `created_at`, `last_rotated_at`
- [ ] Raw key values are absent from the table by design, and that is stated in
      the DDL, not just here
- [ ] Every read path uses `FINAL`
- [ ] The reconciler converges two concurrent first sign-ins onto one key and
      deletes the loser, deterministically
- [ ] A standing query reports any user holding more than one key
- [ ] `prices-api` writes this table under a credential scoped to it, not under
      `prices_writer`
- [ ] Rework updates `last_rotated_at` in the same insert that records the new
      key id, and the old key is deleted only afterwards
- [ ] One-key-per-account assumption from [[0156]] reflected in the schema

## Open

1. **No deletion path for a user's record.** We hold a Discord user ID
   indefinitely with no operation that removes it — neither for a user asking to
   be forgotten nor for us cutting someone off. To settle with the team.
2. **Backup posture.** ClickHouse on shared Hetzner (ADR 0007) — establish what
   backup this table inherits. Losing it orphans every issued key: the keys keep
   working, but nobody can tell whose they are.

## Notes

- The rework rule deletes the old API Gateway key and creates a new one, so
  `api_key_id` changes over time while the Discord ID does not. Rows are
  superseded via the version column, not appended as history — the epic does not
  ask for one.
- If [[0156]] comes back with "more than one active key", this becomes a
  collection per user and the rework cap needs rethinking; that is the one
  answer that reshapes this task.
- A DynamoDB variant was designed and rejected at the 2026-08-07 meeting in
  favour of not adding a datastore. Its one material advantage was the atomic
  conditional write — which is precisely what "Accepted consequences" gives up.
