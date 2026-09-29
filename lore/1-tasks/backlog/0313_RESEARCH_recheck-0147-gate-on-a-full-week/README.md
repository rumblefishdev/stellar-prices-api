---
id: "0313"
title: "Re-check the 0147 coverage gate on a full post-0286 week — 1d with a weekend, and the 1h partial shares that appeared after rollout"
type: RESEARCH
status: backlog
related_adr: ["0292"]
related_tasks: ["0147", "0286"]
tags: [layer-database, priority-low, effort-small, data-correctness, clickhouse]
links:
  - "../../archive/0147_FEATURE_price-usd-series-volume-coverage-gate.md"
history:
  - date: "2026-09-24"
    status: backlog
    who: akot
    note: >
      Spawned from 0147 at close. Phase 2 (2026-09-23) found the share binary
      (0 buckets strictly between 0 and 1). The rollout check the next day
      already showed 53 published 1h buckets with a partial share, and 15
      withheld as pending with a share above 0. Due on or after 2026-09-29.
---

# Re-check the 0147 coverage gate on a full post-0286 week

## Summary

0147 set X = 0.5 because the measured share was binary, so X withheld
nothing. The day after rollout, the 1h grain showed partial shares. The 1d
grain has not been measured on a full week that includes a weekend. Measure
both and decide whether X = 0.5 stays.

## Context

See [[0147]]: the "Phase 2" and "Rollout" sections. The gate's coverage views
(`price_usd_series_coverage{,_1h}`) are on prod, so this needs no inlined
bodies any more.

## Implementation

- On or after 2026-09-29, as `dev_read`, with no script committed to develop:
  - the `priced_volume_share` histogram per grain over 7 days (1d including a
    weekend);
  - the counts of `pending` rows with a share above 0, and of published rows
    with a share strictly between 0 and 1;
  - for the partial buckets: which quote assets are still unpriced, and
    whether enrichment catches up later (the [[0209]] shape).
- Pick X at the trough between the peaks if the distribution is no longer
  binary. Otherwise record that X = 0.5 stays.

## Acceptance Criteria

- [ ] Both grains measured on 7 days; numbers recorded here.
- [ ] Decision on X recorded: keep it, or change `views.sql` plus a rollout.
