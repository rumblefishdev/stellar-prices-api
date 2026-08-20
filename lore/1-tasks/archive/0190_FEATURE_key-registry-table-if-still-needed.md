---
id: "0190"
title: "Key registry table — deferred, and has to justify itself before it is built"
type: FEATURE
status: completed
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
  - date: 2026-08-20
    status: completed
    who: claude
    note: >
      Decided **CANCEL**. Measured on the production account rather than
      argued: control-plane budget 10 rps / burst 40 (AWS docs), real 14-day
      CloudTrail volume 961 calls peaking at 12/s — top consumer
      CloudFormation, not visitors — one portal key in existence, and a cold
      dashboard load costing 4 control-plane calls (~1.14 s), two of which are
      the same GetApiKeys run twice. A registry could replace only those two;
      the in-process cache the usage route already has replaces all four.
      Premise 2 has no customer (0191 states no stored timestamp is needed);
      premise 3 holds technically (ClickHouse is on Hetzner) but puts a
      durability-critical record on the volume that stalled 11.5 h on
      2026-08-13 with no free-space alarm (0204). Decisive: 0158/0190's
      ReplacingMergeTree ORDER BY discord_user_id keeps one row per user, so
      it would overwrite 0192's revocation record and reset the cap. No DDL
      written, no code changed, 0187/0188/0191 untouched. Evidence in
      docs/epics/self-service-onboarding.md; re-open triggers named there.
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

- [x] A decision is recorded, with the measurement behind it, before any DDL is
      written: build, or cancel this task — **CANCEL**. No DDL was written at
      any point; the measurements are below and in the epic doc
- [~] *If built* — **not applicable, nothing was built.** Table in
      `packages/prices-clickhouse`, `FINAL` on every read, written only after
      `CreateApiKey`, no raw key value, no Discord profile field, and
      [[0187]]'s reconciler still the arbiter: none of it exists, and the
      absence is deliberate rather than deferred
- [x] If canceled: the reasoning is written into
      `docs/epics/self-service-onboarding.md` so it is not re-proposed — a
      dedicated section carrying the measurement table and the two named
      re-open triggers, placed directly under the re-slice bullet that set the
      burden of proof

## Notes

- Sequencing: cannot be decided before [[0187]] and [[0191]] are both running.
  Deliberately positioned after them rather than before.

- **The sequencing note above was overtaken, not ignored.** It says the decision
  needs [[0187]] and [[0191]] both *running*. [[0187]] is archived; [[0191]] is
  still backlog — but the fact this task needed from it is already written into
  its spec ("no stored timestamp is needed unless [[0190]] is built"), and the
  fact that actually decided the question came from [[0192]] instead, which
  nobody had read for this purpose. Waiting for [[0191]] to *run* would have
  bought only a throttle observation that [[0194]] owns anyway.

## Decision

**CANCEL.** Recorded 2026-08-20 with the measurement behind it, per the first
acceptance criterion. Full evidence — measurement table, per-premise verdict,
and the two named re-open triggers — lives in
`docs/epics/self-service-onboarding.md`, because that is where it survives this
file being archived.

## Implementation Notes

**Nothing was implemented, and that is the deliverable.** No DDL, no migration,
no crate change, no IAM change, no integration into [[0187]]/[[0188]]/[[0191]].
The work was measurement and analysis:

| What | How | Result |
| --- | --- | --- |
| Control-plane budget | AWS *API Gateway quotas*, "Total operations" row | 10 rps sustained, burst 40, non-adjustable; `GetApiKeys`/`GetApiKey`/`GetUsage` are "Other operations", i.e. on that one bucket. Independently corroborated by [[0191]]'s own text |
| Real spend | CloudTrail `lookup-events` on `apigateway.amazonaws.com`, 2026-08-06 → 08-20, paged to exhaustion (20 pages) | 961 calls; peak 12/s, 42/min, 245/h; top consumers `AWSCloudFormation` 355, `adam.kot` 318, `resource-explorer-2` 277 |
| Existing load | `GetApiKeys` on the production account | 5 keys, one page, exactly **one** portal-issued (`discord-…-key`, 2026-08-18) |
| Cold dashboard load | timed against the real account, CLI startup subtracted | **4 calls**, ≈1.14 s: `GetApiKeys` 286 ms, `GetApiKey` 264 ms, `GetApiKeys` 298 ms, `GetUsage` 288 ms |
| Which calls a registry could replace | read of `keys::lookup` and `usage::fetch` | only the two `GetApiKeys`. `GetApiKey(includeValue)` and `GetUsage` are unreplaceable — AWS holds the credential and the counter |
| Premise 2's customers | full read of [[0191]] and [[0192]] | neither asks for prior-key history |
| Premise 3's substrate | `docs/runbooks/deploy-ledger-processor.md` | ClickHouse is on **Hetzner**, so it would outlive an AWS rebuild — the one premise that holds on its own terms |

`GetApiKey` was measured with `includeValue=false`. The production code uses
`true`; a latency measurement does not need a live bearer credential pulled into
a terminal, and the two differ by nothing that matters here.

## Design Decisions

### From Plan

1. **Decide before writing DDL**, as the first acceptance criterion demanded.
   The three premises were tested against measurements and code, not against
   the plausibility of the original [[0158]] argument.

### Emerged

2. **The registry is *strictly dominated* on its own strongest premise.** Its
   ceiling is 4 → 2 calls per load. The in-process cache the usage route
   already runs takes warm loads to **0**, and de-duplicating the shared
   `GetApiKeys` (the two routes each run their own `list_named` for the same
   user in the same load) takes cold loads to 3 — both without storage, IAM, or
   a second source of truth. A component that loses to changes already in the
   codebase does not get built.
3. **"No measurable load" is itself the measurement.** The portal is closed,
   one key exists, and the budget's peak consumer is `cdk deploy`. Sizing a
   cache for traffic never observed is the speculative hardening the task text
   forbids by name.
4. **The decisive argument turned out to be structural, not budgetary.**
   [[0158]]/[[0190]]'s `ReplacingMergeTree(updated_at) ORDER BY
   discord_user_id` holds **one row per user, replaced on every write**. After
   [[0192]] revokes, the next issue overwrites the revocation row — resetting
   the cap the epic explicitly forbids resetting. Building this table would not
   merely be unnecessary; it would hand [[0192]] a structure that loses
   [[0192]]'s data. This is written into [[0192]]'s notes so the trap is found
   before the shape is copied.
5. **Premise 3 was granted, then declined on its merits.** ClickHouse being on
   Hetzner really does buy survival of an AWS rebuild. It was still refused:
   there is no DR requirement in the epic, attribution already travels in the
   key *name* (`discord-<userId>-key`), and the shared Hetzner volume stalled
   ingestion for 11.5 h on 2026-08-13 and still has no free-space alarm
   ([[0204]], open). Sole custody of a durability-critical record does not go on
   the least-monitored storage in the system.
6. **Two re-open triggers named, rather than "revisit someday".** A cancel with
   no stated trigger gets re-proposed; a cancel with a trigger gets *tested*
   against it. They are [[0192]] starting (needs an append-only record, not this
   shape) and [[0194]] measuring real per-load cost against real traffic.
7. **[[0194]]'s costing criterion was corrected while the numbers were in
   hand.** It described the per-load footprint as `GetApiKey + GetUsage` — one
   `GetApiKeys` short of what the code does, and missing the duplicate listing
   entirely. Left uncorrected, the audit would have costed a load that does not
   exist. Not scope creep: the criterion's own text is "nobody has costed this
   yet", and this task costed it.

## Issues Encountered

- **The task's sequencing rule pointed at the wrong slice.** It said the
  decision needs [[0191]]; [[0191]] had already answered in its spec, and the
  argument that actually decided the outcome was in [[0192]], which the task
  never named as relevant. A dependency written at slicing time survived past
  the point where it was true.
- **SSM parameter inspection was blocked by the sandbox** while checking where
  ClickHouse lives. Not worked around — the answer came from
  `docs/runbooks/deploy-ledger-processor.md` instead, which is the better source
  anyway.
- **CloudTrail `lookup-events` caps at 50 results per page regardless of
  `--max-results`.** The first pass reported 50 events and read as low volume;
  paging to exhaustion gave 961. Worth knowing before anyone quotes a
  control-plane figure from a single call.

## Future Work

No backlog task spawned — the two follow-ups both already have owners, and
inventing tasks for them would duplicate existing scope:

- **De-duplicate the shared `GetApiKeys` and cache the reveal** → [[0194]]'s
  costing criterion, now corrected to name both remedies. They are performance
  work with no measured problem yet, so they are deliberately *not* a task of
  their own.
- **A durable revocation record** → [[0192]], with the overwrite trap written
  into its notes.
