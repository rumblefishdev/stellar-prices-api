---
id: "0181"
title: "Rollup leading indicators — pending-mutation age, part counts and view_refreshes exceptions"
type: FEATURE
status: completed
assignee: akot
related_adr: []
related_tasks: ["0137", "0136", "0109", "0134"]
tags:
  ["priority-medium", "effort-small", "clickhouse", "observability", "milestone-M2"]
milestone: 2
links: []
history:
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      Renumbered 0179 → 0181 on 2026-08-12: task 0156's PR #187 merged
      concurrently and spawned its own 0179 (Stellar Discord / SDF integration).
      This side renumbered because it was the least-referenced of the two — one
      inbound link, against ~8 for the Discord task (ADR 0010, 0159, 0162, 0163,
      0164, 0156's notes).
      Spawned from [[0137]]. The primary freshness signal (per-tier
      `now() - max(timestamp)`) shipped without these three leading indicators
      because they need `system.mutations` and `system.view_refreshes`, whose
      readability by the scoped mTLS user was unmeasured — and the runtime users
      are XML-managed by BE, so we cannot `GRANT` them ourselves ([[0134]]).
      0137 was deliberately designed to need no `system.*` access at all so it
      could ship first.
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      ACCESS MEASURED on prod, so the scope is now precise rather than an open
      question. `prices_writer` holds `SELECT ON system.parts` and nothing else
      under `system.*`, so the **part-count indicator is unblocked and should be
      built first**; mutation-age and view_refreshes need a two-line BE grant.
      The cluster uses explicit grants, NOT ClickHouse's usual
      open-with-row-filtering — and the `system.parts` grant is itself the proof,
      since it would be redundant under the permissive model. `storage =
      users_xml` on both runtime users confirms [[0134]]: we cannot GRANT them.
      Batch the grant request with the 0136 note already owed to BE.
  - date: "2026-10-07"
    status: completed
    who: akot
    note: >
      Closed on the operator's decision, two of four parts delivered
      elsewhere. view_refreshes exceptions and the empty-1M hole are covered
      by [[0203]] (MvRefreshFailingCount, RollupMismatchBuckets for 1M; all
      four mv-refresh alarms OK, MvRefreshUnreadable 0 on 96 reads a day).
      Part-count and mutation-age indicators NOT built: measured today max 11
      active parts per partition (threshold ~1,000) and 0 pending mutations
      in prices. No follow-up tasks.
---

# Rollup leading indicators

## ✅ Closed 2026-10-07: two parts delivered by 0203, two not built

Verified in code and on production, 2026-10-07:

| indicator | state |
|---|---|
| `view_refreshes` exceptions | **delivered by [[0203]]**: `refresh_waits.rs`, `MvRefreshFailingCount` (non-empty `exception`, or no success for > 2 own periods) plus `MvRefreshWaitingCount`, `MvRefreshDisabledCount`, `MvRefreshUnreadable`. Four `prices-production-mv-refresh-*` alarms, OK since 2026-10-02. `MvRefreshUnreadable` = 0 on 96 reads a day, so `prices_writer` holds `SELECT ON system.view_refreshes` (rollout check in `0142-rollup-mv-reapply.md:329`); the BE ask below is obsolete for this table |
| empty `1M` tier | **covered by [[0203]]**: `RollupMismatchBuckets` with `Table=price_ohlcv_1M` counts closed `1M` buckets missing against `1d`, so an empty `1M` reads > 0 |
| part counts | **not built.** Max active parts per partition in `prices` today: 11 (`price_ohlcv_1m`), against the ~1,000 `parts_to_delay_insert` threshold |
| pending-mutation age | **not built.** 0 pending mutations in `prices` today. Whether `prices_writer` can read `system.mutations` was not verified (`dev_read` cannot `SHOW GRANTS FOR` another user). [[0109]]'s own `system.mutations` preflight criterion is unaffected |

Closed on the operator's decision without the last two: both read far from
any threshold today, and no follow-up task was spawned. If a 0136-style
mutation backlog recurs, the lagging signals (freshness, mismatch,
`mv-refresh-*`) still fire; only the early warning is missing.

## Summary

[[0137]] alarms when a rollup tier's newest bucket ages past its bound — the
signal that would have caught [[0136]]'s nine-day freeze. That is the *lagging*
indicator: by the time it fires, the data is already stale.

0136 also left three **leading** indicators, each of which would have fired days
earlier:

- any row in `system.mutations` with `is_done = 0` older than ~1 h — in 0136
  these sat for **13 days**;
- any `prices` table above ~1,000 active parts (`parts_to_delay_insert`), well
  before the 5,000 throw limit;
- a non-empty `exception` on any row of `system.view_refreshes`.

## Context — access MEASURED on prod 2026-08-12

The blocker is access, not design. **It is now measured, not assumed**
(`SHOW GRANTS FOR prices_writer` on ch-prod-01):

```
GRANT SELECT, INSERT, ALTER DELETE, OPTIMIZE ON prices.* TO prices_writer
GRANT SELECT ON system.parts                            TO prices_writer
```

| table | granted | indicator | status |
|---|---|---|---|
| `system.parts` | ✅ explicitly | part counts | **unblocked — build first** |
| `system.mutations` | ❌ | pending-mutation age | blocked on BE grant |
| `system.view_refreshes` | ❌ | refresh exceptions | blocked on BE grant |

⚠️ **This cluster uses explicit grants, not open-with-row-filtering.** ClickHouse
often exposes `system.*` to every user with rows filtered to what that user can
see, which would have made these readable for free. It does **not** here — and
the proof is the `system.parts` grant itself: if system tables were open, that
line would be redundant, and somebody added it deliberately because it was not.
Do not assume a `system.*` table is readable because ClickHouse usually allows
it; check `SHOW GRANTS` first.

`SELECT name, storage FROM system.users` returns `users_xml` for both
`prices_reader` and `prices_writer`, confirming [[0134]] empirically: these are
XML-managed and **we cannot `GRANT` them ourselves.**

### The BE ask — small and concrete

Two lines in BE's ClickHouse users XML:

```
GRANT SELECT ON system.mutations      TO prices_writer
GRANT SELECT ON system.view_refreshes TO prices_writer
```

📌 **Batch this with the [[0136]] note already owed to BE** (coarse `prices` data
was stale 2026-07-21 → 08-03 and has since moved) rather than pinging them
twice.

## Implementation

- Extend `rollup-freshness-probe` rather than adding a fourth probe — it already
  runs every 15 minutes, already holds a CH client, and already has dead-probe
  cover in the [[0112]] `workerHealth` array.
- Publish as additional metrics under the existing `Prices/Rollup` namespace so
  the IAM grant needs no change: e.g. `PendingMutationAgeSeconds`,
  `MaxActivePartsPerTable` (dimension `Table`), `ViewRefreshExceptions`.
- ⚠️ `RollupLagSeconds` with `Table=current_prices` is taken since [[0243]] (the
  current_prices writer-liveness check, 2026-09-14). Pick names that do not
  collide with it.
- ⚠️ **Overlaps [[0109]]'s guard**, which already has to watch `system.mutations`.
  Settle ownership before building — the 0137 acceptance criterion was written
  as "here or in 0109, without duplicating each other".

## Also carried here: the coarsest-tier empty hole

[[0137]] synthesises a breaching sentinel for a tier that is empty **while a
coarser tier is populated** — which is what stops a tier emptied by retention
mid-freeze from reading as recovered. `price_ohlcv_1M` has no coarser tier, so
an empty `1M` cannot be caught that way and its alarm can never fire.

Low severity (`1M` is the least load-bearing tier and any real freeze shows up in
the finer tiers first), but it needs a different signal — a row-count metric, or
comparing `1M`'s newest bucket against `1w`'s.

## Acceptance Criteria

- [x] Readability of `system.mutations` and `system.view_refreshes` by the scoped
      mTLS user is **measured** and recorded either way. ✅ **Done 2026-08-12** —
      neither is granted; only `system.parts` is. See §Context.
- [ ] Part-count indicator ships (unblocked — build this first).
      Not built; closed without it (max 11 parts/partition on 2026-10-07).
- [ ] Mutation-age and `view_refreshes` indicators ship, **or** the two-line BE
      grant request in §Context is raised and its outcome recorded here.
      `view_refreshes`: shipped by [[0203]], grant in place. Mutation age:
      not built, grant unverified.
- [ ] No duplication with [[0109]] — ownership of the `system.mutations` watch is
      settled and written down in both tasks. Moot: no watch built here.
- [ ] Thresholds recorded with rationale, consistent with [[0137]]'s bounds.
      Moot for the unbuilt two; 0203's are in `observability-stack.ts`.
