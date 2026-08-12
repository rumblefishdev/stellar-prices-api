---
id: "0181"
title: "Rollup leading indicators — pending-mutation age, part counts and view_refreshes exceptions"
type: FEATURE
status: backlog
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
---

# Rollup leading indicators

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
- [ ] Mutation-age and `view_refreshes` indicators ship, **or** the two-line BE
      grant request in §Context is raised and its outcome recorded here.
- [ ] No duplication with [[0109]] — ownership of the `system.mutations` watch is
      settled and written down in both tasks.
- [ ] Thresholds recorded with rationale, consistent with [[0137]]'s bounds.
