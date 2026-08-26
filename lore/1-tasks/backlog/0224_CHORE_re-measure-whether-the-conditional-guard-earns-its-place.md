---
id: "0224"
title: "Re-measure whether the conditional liveness guard earns its place — it protects 1 asset"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0135", "0111", "0215", "0217"]
tags: ["effort-small", "priority-low", "clickhouse", "vwap", "measurement"]
milestone: 3
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
history:
  - date: 2026-08-25
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0135]] future work. 0135 shipped the guard while stating
      plainly that it prevents nothing measurable today; this task is the
      promised re-measurement, made explicit rather than left as a comment.
---

# Does the conditional liveness guard still earn its place?

## Summary

`current.sql`'s `per_source_kept` drops a non-quoting venue when the asset still
has a quoting one, so a dead venue cannot outvote a live one in the **unweighted**
§5.5 median. [[0135]] shipped it while measuring that it currently prevents
**zero** evictions. This task re-measures and decides: keep, or delete.

## Context

0135 kept the guard on a stated argument, not on evidence of live benefit:

> its value is ANTI-correlated with pipeline health

Measured twice, on the population where the defect can occur at all (mixed
live/dead **and** >= 3 sources, since the mask is a no-op below three):

| | pipeline down (2026-08-21) | healthy (2026-08-25) |
|---|---|---|
| mixed | 35 | 15 |
| ...and >= 3 sources | **7** | **1** |

An earlier revision predicted the opposite — that recovery would grow the mixed
population — and was retracted when measured. Assets move wholesale from all-dead
to all-live rather than through a mixed state.

Two things have since changed the pipeline again, which is why this is worth
re-running rather than assuming:

- [[0111]] is archived, so the full-table-scan-per-batch behaviour is gone.
- BE's partition-limited scan (their follow-up to [[0215]]) alters enrichment
  throughput a second time.

## Implementation

- Re-run the population probe: count assets that are mixed live/dead, and of those
  the ones with >= 3 kept sources.
- For each at-risk asset, compare the live-only median against the all-source
  median and check whether either crosses `OUTLIER_PCT` (0.20). In 0135 all 7
  sat within **1%**, so the guard changed nothing even where it armed.
- Decide, and record the decision either way.

## Acceptance Criteria

- [ ] At-risk population re-measured on prod after 0111 and BE's partition work,
      with the date and the asset count recorded
- [ ] For each at-risk asset, live-only vs all-source median compared against
      the 20% threshold — the question is whether the guard would change any
      published number, not whether it arms
- [ ] Decision recorded: keep with the measurement behind it, or delete
- [ ] If deleted: it is a two-line revert (`src_is_live` + the `per_source_kept`
      filter) and must leave every 0135 metric unchanged, since those come from
      the C2 carry rather than from the guard — verify, do not assume
- [ ] If kept: this task's own numbers replace 0135's in the `current.sql`
      comment, so the rationale there is never older than the last measurement

## Notes

- Deleting is a legitimate outcome. 0135 recorded it as a judgement call, not a
  measurement, precisely so it could be revisited without re-litigating.
- Do **not** conflate this with [[0217]], which decides whether `price_usd` itself
  goes through the keep-mask. This task is only about the liveness guard feeding it.
