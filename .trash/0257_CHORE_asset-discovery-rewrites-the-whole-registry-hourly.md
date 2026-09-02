---
id: "0257"
title: "asset-discovery rewrites all ~207k asset rows every hour, generating 5M rows/day of merge pressure to find a handful of new assets"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0210", "0054", "0256"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M2, ingest, performance]
milestone: 2
links:
  - "../../../packages/prices-ingest-core/src/writer.rs"
history:
  - date: 2026-09-02
    status: backlog
    who: stkrolikiewicz
    note: >
      Measured while deploying [[0210]]. Explains the "registry doubled
      overnight" scare during that deploy — 415,495 raw rows against 207,754
      distinct identities was un-merged parts from the hourly rewrite, not
      data corruption.
---

# The hourly seed rewrites the entire registry

## Summary

Every `asset-discovery` run logs `wrote asset rows: 207754` — the **whole**
registry, re-inserted into a `ReplacingMergeTree`, once an hour. That is roughly
**5 million rows a day** of write and merge pressure to discover, on a typical
run, a couple of dozen new assets.

## Measured on prod, 2026-09-02

`existing_assets` read at the start of three consecutive runs shows the raw row
count oscillating with merge progress:

| run | rows read |
|---|---|
| 07:17 | 623,154 (3×) |
| 08:17 | 207,741 (1×) |
| 09:17 | 415,495 (2×) |

Distinct identities held steady at ~207,754 throughout. Nothing was wrong with
the data — the table simply never stays merged, because a fresh full copy lands
every hour.

This cost a real detour during [[0210]]'s deploy: a `count()` taken mid-cycle
read as "the registry doubled in five days" and had to be disproven with
`uniqExact` before the deploy could continue.

## Implementation

- Write only rows that are new or changed. The seed is idempotent by design, so
  the natural fix is to diff against the loaded registry before inserting rather
  than re-emitting it.
- Alternatively, decouple: seed on cold start / on demand, not every run.
- Worth checking the same shape in the other workers before assuming it is
  unique to this one.

## Acceptance Criteria

- [ ] A steady-state run writes rows proportional to what actually changed, not
      to registry size
- [ ] `SELECT count()` on `prices.assets` stops oscillating between 1× and 3×
- [ ] The measurement is repeated after the change and recorded here
