---
id: "0158"
title: "Key registry — ClickHouse table mapping Discord user ID to API Gateway key ID and usage plan"
type: FEATURE
status: backlog
related_adr: ["0007", "0008", "0010"]
related_tasks: ["0156", "0157", "0159", "0160", "0180"]
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
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      [[0156]] / ADR 0010 confirms one active key per account, so the schema is
      unchanged. Two corrections though: `nameQuery`'s matching semantics are
      undocumented (not "a prefix match"), which makes the exact-match filter
      load-bearing; and the quota-period boundary is our rule, not AWS
      behaviour. Both measured by [[0180]].
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
- Sized for one active key per account. **Confirmed by [[0156]] / ADR 0010 on
  2026-08-10 — the shape below stands unchanged**, and the confirmation is
  stronger than the epic's own argument: AWS charges quota per
  `(usage plan, API key)` and has **no principal that aggregates keys**
  (`customerId` is documented as *"An AWS Marketplace customer identifier"* and
  is only a listing filter). Multi-key would force our own fan-out over N
  `GetUsage` calls plus summation — precisely the work [[0160]]'s rework cap
  exists to avoid.
- **AWS will not enforce one-key-per-account for us.** A key may sit in up to 10
  usage plans, a plan holds arbitrarily many keys, and `name` is optional and
  **not** unique — only key *values* are unique and enforced. The invariant
  lives here, in application code, which is what "Accepted consequences" below
  is about.

**Table**

```sql
CREATE TABLE prices.api_key_registry
(
    discord_user_id  String,
    api_key_id       String,
    usage_plan_id    String,
    created_at       DateTime64(3),
    last_rotated_at  Nullable(DateTime64(3)),
    updated_at       DateTime64(3)                  -- version column
)
ENGINE = ReplacingMergeTree(updated_at)
ORDER BY discord_user_id;
```

- Matches the 12 `ReplacingMergeTree` tables already in the schema; no new
  engine, no `PARTITION BY` (the table is a few hundred rows). `init.sql`'s
  own engine-assignment rule already says "state / registry tables →
  `ReplacingMergeTree(updated_at)`", so this needs no new convention.
- **Timestamps are bare `DateTime64(3)`**, matching every other table in the
  schema — no timezone qualifier is used anywhere. The quota-period boundary is
  therefore computed in UTC by the caller, not inferred from the column type.
- **Every row is written by the app, never partially.** There is no
  "reservation" state: the row is inserted only after `CreateApiKey` succeeds,
  so `api_key_id` is never empty. Without a conditional insert a reservation
  would buy nothing, which is why the DynamoDB design had one and this does not.
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
- **No membership columns either — settled 2026-08-10 by ADR 0010.** The
  membership and account-age checks run **once, at issuance** ([[0159]]) and
  nothing re-reads them, so `pending`, `joined_at` and `roles` are deliberately
  **not** stored. Storing them would mean holding Discord profile data we never
  use, in the same table that declines to hold a username. The schema above is
  therefore unchanged by the membership decision.

**Issue flow — API Gateway is the arbiter, not the table**

Key names are `discord-<userId>-key`. The trailing delimiter matters — see the
prefix hazard below.

1. Read `FINAL`; a row with a populated `api_key_id` returns immediately (hot
   path).
2. `GetApiKeys(nameQuery = "discord-<userId>-key")`, then **filter the result to
   exact name equality in the client**. AWS, not ClickHouse, is the source of
   truth for whether a key already exists.
3. No key found → `CreateApiKey(name = "discord-<userId>-key")` +
   `CreateUsagePlanKey`.
4. `INSERT` the row.
5. Re-list and exact-match again; if more than one key survives the filter, keep
   the one with the earliest `createdDate`, `DeleteApiKey` the rest, `INSERT` a
   corrective row.

**`nameQuery`'s matching semantics are undocumented — corrected 2026-08-10 by
[[0156]].** AWS's entire documentation of the parameter is one sentence, *"The
name of queried API keys."*, on both `GetApiKeys` and the CLI reference. It
states **no** matching rule. Prefix matching is widely-reported community
knowledge, not an AWS contract, and AWS has committed to nothing.

Two consequences, and the second is the important one:

- **The client-side exact-match filter is load-bearing, not defence in depth.**
  Do not let a later reader "simplify" it away on the grounds that the `-key`
  suffix already handles it. It is the only guard that does not depend on
  undocumented behaviour.
- **If matching *is* prefix-based** — the behaviour to assume until [[0180]]
  measures it — then because Discord snowflakes are 17–19 digits, a shorter user
  id is a prefix of a longer one: a lookup for `discord-1234567890123456` would
  return the key belonging to `discord-12345678901234567`, and step 5 would
  delete it — silently, permanently, on an account that was never part of the
  race. The `-key` suffix means no bare id can prefix another complete name,
  which is why the naming keeps it.

Step 5 is the reconciler and it is deterministic: both sides of a race read the
same API Gateway list and compute the same winner. **Deterministic key naming is
what makes every step above possible** — [[0160]] requires that naming for
attribution, and it doubles as the idempotency handle here.

**What this reconciler does *not* cover: a key deleted by hand in the console.**
That case presents as a row with a populated `api_key_id`, so step 1
short-circuits and step 5 never runs — the user would get a dead key id back
forever. Recovery has to hang off the **reveal path** instead: when
`GetApiKey(apiKeyId)` returns 404, re-enter this flow from step 2. See [[0160]]
"Settled 2026-08-07" #4.

**Rework flow**

1. Read `FINAL`; refuse with `409` + `next_eligible_at` unless
   **`coalesce(last_rotated_at, created_at)`** falls before the current quota
   period start (1st of the month, 00:00 UTC).

   **That boundary is *our* rule, not AWS behaviour we inherit — corrected
   2026-08-10 by [[0156]].** AWS never documents the reset instant or its
   timezone. Its only statement anywhere is an example caption, *"creates a
   usage plan that resets at the beginning of the month"*, and `offset` is a
   **request count** (*"The number of requests subtracted from the given limit
   in the initial time period"*), not a time shift. Keep the rule — it is a
   sound product decision and gives one date to render — but state it as ours.
   [[0180]] measures the actual rollover so the two can be reconciled.

   **Gating on `last_rotated_at` alone reopens the loophole the cap exists to
   close.** Issuance never sets that column, so it is null for every fresh key —
   meaning a user could take a key on 1 August, burn the full 100 000, and
   rework on 2 August into a clean counter, because quota is scoped to
   `(usagePlanId, apiKeyId)`. That is 200 000 requests in one period. Falling
   back to `created_at` also matches the epic's own wording: a new key is
   issuable once *the current one's* quota period has rolled over.
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
   One line in the `MtlsRole` union, plus a CN in BE's CA namespace, an entry in
   Caddy's `CLICKHOUSE_CN_USER_MAP`, a CH user and grant, and a secret bundle
   uploaded by hand per the 0063 issuance runbook — CDK does not manage that
   material. Four steps across two teams, so raise it early.
2. Grant `prices_reader` `INSERT ON prices.api_key_registry` — one table, not the
   estate. **Viable fallback**, and worth being honest about the gap between it
   and option 1: both identities would live in the same Lambda process, so a
   compromised `prices-api` reaches either client regardless. What option 1
   actually buys is attribution in the ClickHouse query log, and keeping the
   write grant off a credential that BE may reuse elsewhere — not isolation from
   a compromised handler.
3. Hand `prices-api` the `ingestion` bundle — gives the partner-facing Lambda
   write access to the whole ingestion estate. Do not.

**Ask for option 1, ship on option 2 if the handshake drags.** Migrating later
is one grant and one secret; nothing in the schema or the write path changes.
That keeps this task off the critical path of another team's queue.

## Acceptance Criteria

- [ ] Table exists in `packages/prices-clickhouse`, keyed by Discord user ID,
      holding key id, usage plan id, `created_at`, `last_rotated_at`
- [ ] Raw key values are absent from the table by design, and that is stated in
      the DDL, not just here
- [ ] Every read path uses `FINAL`
- [ ] The reconciler converges two concurrent first sign-ins onto one key and
      deletes the loser, deterministically
- [ ] Key lookup exact-matches the name; a user id that is a prefix of another
      user's id cannot return, adopt or delete that other user's key
- [ ] Rework is refused for a key issued inside the current quota period, not
      only for one already reworked in it
- [ ] A dead `api_key_id` is recovered from the reveal path, not the issue path
- [ ] A standing check reports any user holding more than one key. Note it
      cannot be a query over this table — one row per user is guaranteed by
      `ORDER BY discord_user_id`, so duplicates are only visible on the AWS
      side, by listing keys and grouping by name
- [ ] `prices-api` writes this table under a credential scoped to it, not under
      `prices_writer`
- [ ] Rework updates `last_rotated_at` in the same insert that records the new
      key id, and the old key is deleted only afterwards
- [x] One-key-per-account assumption from [[0156]] reflected in the schema —
      confirmed 2026-08-10 by ADR 0010; `ORDER BY discord_user_id` is correct
- [ ] The client-side exact-match filter is present **and** commented as
      load-bearing, so it survives a later cleanup — `nameQuery` matching is
      undocumented, not merely quirky

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
- [[0156]] came back with **one active key, confirmed** (ADR 0010), so the
  collection-per-user variant that would have reshaped this task is off the
  table. Recorded rather than deleted: the reason multi-key was rejected is that
  AWS quota accounting has no user-shaped principal, and that reason will still
  be true if anyone reopens it.
- A DynamoDB variant was designed and rejected at the 2026-08-07 meeting in
  favour of not adding a datastore. Its one material advantage was the atomic
  conditional write — which is precisely what "Accepted consequences" gives up.
