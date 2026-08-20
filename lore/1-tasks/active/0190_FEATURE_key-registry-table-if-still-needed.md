---
id: "0190"
title: "Key registry table — deferred, and has to justify itself before it is built"
type: FEATURE
status: active
related_adr: ["0007", "0010"]
related_tasks: ["0183", "0158", "0187", "0191", "0192"]
tags: [layer-infra, priority-low, effort-small, milestone-M3, epic-self-service-onboarding, storage, clickhouse, slice-7]
milestone: 3
links:
  - "../archive/0158_FEATURE_discord-key-registry-table.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Carries [[0158]] forward, demoted from "the epic's first brick" to "prove
      you need it". The re-slice found that [[0187]] and [[0191]] can both be
      built without it, using deterministic key naming and the surviving key's
      `createdDate`. If that holds through those two slices, this task is
      canceled rather than built.
  - date: 2026-08-20
    status: active
    who: akot
    note: >
      Activated by Adam on a branch cut from [[0189]]'s. Sequencing caveat
      recorded rather than resolved: the task's own note says the decision
      cannot be made before [[0187]] and [[0191]] are both running, and
      [[0191]] is still backlog. The build-vs-cancel evidence is therefore
      incomplete on activation — [[0187]] is archived, [[0188]] (dashboard
      load) is still active, and the per-load control-plane cost that feeds
      the "hot path" criterion is owned by [[0194]].
---

# Key registry — only if it earns its place

## Summary

**Story:** *as the operator, I want a record of who holds which key and when it
was last reworked — if and only if AWS cannot already tell me.*

[[0158]] specified a `ReplacingMergeTree` in ClickHouse mapping Discord user ID
→ key id → plan id, and put it first in the epic on the grounds that every
backend endpoint reads or writes it. The re-slice tests that claim and it does
not survive.

## The case against building it

Two questions the registry was there to answer, and where the answer actually
lives:

| Question | Registry answer | Answer without it |
| --- | --- | --- |
| Does this user already have a key? | look up the row | `GetApiKeys(nameQuery)` + exact filter on `discord-<userId>-key`. [[0158]]'s own issue flow says **API Gateway, not ClickHouse, is the source of truth** for exactly this |
| When was this key last reworked? | `last_rotated_at` | the surviving key's `createdDate`. A rework deletes the old key and creates a new one, so `createdDate` **is** `coalesce(last_rotated_at, created_at)` |

The second row is the one that changes the plan. [[0158]] argued the
`created_at` fallback was load-bearing precisely because both timestamps
describe the same event — the moment this key came into existence — which is the
one fact API Gateway records for free.

## What would still justify it

Build this only if one of these turns out to matter:

- **A hot path that cannot afford a control-plane call.** `GetApiKeys` is
  throttled per account and shares a budget with our deploys. If [[0187]]'s
  reveal path or [[0188]]'s dashboard load measurably competes with CI, a
  ClickHouse read in front of it is the fix.
- **History.** `createdDate` knows the current key, not the previous three. If
  anyone needs "how often does this user rework", only a table has it.
- **Attribution surviving AWS.** If the account is ever rebuilt, key names are
  the only link back to a Discord id and they go with the keys.

None of these is speculative-hardening-shaped; all three are measurable during
[[0187]] and [[0191]]. Decide then.

## If it is built

The schema from [[0158]] stands unchanged — do not redesign it:

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

With [[0158]]'s constraints, all of which still apply: every read uses `FINAL`
(merges are async, and a read straight after a write returns a superseded row);
the row is inserted only after `CreateApiKey` succeeds, never partially; **no raw
key value**, stated in the DDL comment and not only here; no `discord_username`
and no membership columns — the checks run once at issuance and nothing re-reads
them, so storing `pending`/`joined_at`/`roles` would mean holding Discord profile
data we never use, in a table that already declines to hold an email.

And its accepted consequence: ClickHouse has no conditional insert, so the table
cannot enforce one-key-per-account. The reconciler in [[0187]] remains the
guard whether or not this table exists — which is another way of saying the
table was never the invariant.

## Acceptance Criteria

- [ ] A decision is recorded, with the measurement behind it, before any DDL is
      written: build, or cancel this task
- [ ] If built: table created in `packages/prices-clickhouse` matching the
      schema above, read with `FINAL`, written after `CreateApiKey` only
- [ ] If built: no raw key value and no Discord profile field in any column
- [ ] If built: [[0187]]'s reconciler still runs and still wins ties — the table
      is a cache, not the arbiter
- [ ] If canceled: the reasoning is written into
      `docs/epics/self-service-onboarding.md` so it is not re-proposed

## Notes

- Sequencing: cannot be decided before [[0187]] and [[0191]] are both running.
  Deliberately positioned after them rather than before.
