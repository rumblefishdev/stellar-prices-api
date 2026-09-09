---
id: '0261'
title: 'Asset listing churns 60 % between walks — cursor pagination over a table replaced every minute'
type: BUG
status: backlog
related_adr: []
related_tasks: ['0121', '0120', '0210']
tags: [layer-backend, priority-medium, effort-small, api, pagination, clickhouse]
links: []
history:
  - date: 2026-09-03
    status: backlog
    who: stkrolikiewicz
    note: >
      Noticed while building the load-test asset pool for [[0121]]. Two full
      walks of the listing a day apart returned wildly different sets; the
      pagination, not the data, is the suspect.
---

# Asset listing churns 60 % between walks

## Summary

Two complete walks of `GET /v1/assets` a day apart returned **3543** and
**4306** ids, with **2128 added and 1365 removed** between them. Roughly 38 % of
the catalogue disappearing in 24 hours is too much to be genuine listing
turnover, and the likely cause is that cursor pagination walks a table that is
being replaced underneath it.

## Context

`current_prices` is replaced every minute by a refreshable materialized view —
documented as a GOTCHA in `packages/prices-api/loadtest/seed.sql`, which tells
you to use a clean volume locally so a manual seed is not overwritten.

A full listing walk is 200 rows per page with a pause between pages: **18–22
pages, roughly 25 seconds**. That comfortably spans at least one refresh. If the
cursor is a value from the replaced table — the observed cursors decode to
something like `{"v":"651580.459637104131116","id":4}`, i.e. a sort key plus an
id — then rows that move across the cursor boundary during a refresh are
**skipped**, and rows that move backwards are **returned twice**. That matches
the symptom: every walk produces a plausible-looking but different set.

Two independent observations from the same task are consistent with it:

- The set of assets returning `404` on `/price` drifts day to day — `USDC`/`RON`
  on 2026-09-01, `AUD`/`EQL` on 2026-09-03, five different ids on the wide pool.
- A 200-asset random sample was 100 % servable, yet a full-pool probe found 39
  unservable — the discrepancy is easier to explain by *which rows the walk
  caught* than by the data itself.

**Why it matters beyond the load test.** Any consumer paginating the catalogue
gets a silently incomplete answer, with no error and no way to detect it. The
load test tolerated it because `setup()` drops unservable ids, but an integration
building a local asset table would not.

## Implementation

- Confirm the mechanism before fixing: walk the listing twice back-to-back and
  compare, then walk it again with the refresh paused or against a static
  snapshot. If the churn disappears, the cursor is the cause.
- Check what the cursor encodes and whether it is stable across a refresh.
- Options, cheapest first: paginate over an immutable key rather than a
  refresh-volatile sort value; serve the listing from a snapshot consistent for
  the duration of a walk; or document the guarantee honestly as "best effort,
  may skip or duplicate across pages" if neither is worth the cost.

## Acceptance Criteria

- [ ] Mechanism confirmed or ruled out, with two comparable walks as evidence
- [ ] Whether a walk can **skip** rows is stated definitively — duplicates are
      an annoyance, skips are data loss for the consumer
- [ ] Either the pagination gives a stable full walk, or the limitation is
      documented in the API docs where a consumer will see it
- [ ] Confirmed whether this explains the drifting `404` set seen by [[0120]]
      and [[0121]], and whether [[0210]] is related

## Notes

- Reproduce cheaply: two walks are ~40 requests each, no load implications.
- Not a load-test problem and not blocked on [[0260]] — independent of the
  read-path collapse, and much cheaper to answer.
