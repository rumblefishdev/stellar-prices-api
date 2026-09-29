---
id: "0313"
title: "Re-check the 0147 coverage gate on a full post-0286 week — 1d with a weekend, and the 1h partial shares that appeared after rollout"
type: RESEARCH
status: completed
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
  - date: "2026-09-29"
    status: active
    who: akot
    note: >
      Activated on the due date. Measuring both grains over 7 days on prod as
      dev_read.
  - date: "2026-09-29"
    status: completed
    who: akot
    note: >
      Both grains measured as dev_read on 09-22 → 09-29 (1d includes the
      weekend): the share is binary again, 0 of 199,463 buckets strictly
      between 0 and 1, and 0 `pending` with a share above 0. The rollout-day
      partials had resolved. X = 0.5 stays; no change to views.sql, no rollout.
      Every `pending` bucket in the window is sub-1e-12 XLM dust that will
      never publish.
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

## Measurement (2026-09-29)

Run read-only on prod as `dev_read` (CH 26.3.10.60), against the live
`price_usd_series_coverage{,_1h}` views, around 09:40–10:00 UTC. The window is
Mon 2026-09-22 00:00 → Mon 2026-09-29 00:00, 7 full days including the weekend
(09-26, 09-27). No script is committed; the queries stayed in the session
scratchpad.

### `priced_volume_share` histogram

| Grain | Buckets | share = 0 | 0 < share < 1 | share = 1 |
| --- | --- | --- | --- | --- |
| 1d | 32,284 | 5,294 (255 `pending`, 5,039 `unpriceable`) | 0 | 26,990 `priced` |
| 1h | 167,179 | 31,996 (345 `pending`, 31,651 `unpriceable`) | 0 | 135,183 `priced` |

The share is binary again at both grains. No bin in (0, 1) holds a bucket, so
there is no trough to put X in.

### Pending with share > 0, and published with 0 < share < 1

- 0 and 0 at both grains, on every day from 09-22 to 09-29 (today included).
- At 1h, no bucket is partial in any of the last 36 hours, including the hour
  in progress (09-29 09:00).
- The 53 published partial buckets and the 15 `pending` ones with a share
  above 0 seen at rollout (09-24, 1h, 48 h ending 09-24 12:00) are gone: that
  window lies inside this one, and every bucket in it now reads share 0 or 1.
  The views are retroactive, so the partial state was transient: rows priced
  later, as in [[0209]]. The query does not say which write priced them
  (enrichment or a rebuild). It did not recur this week.

### What is left in `pending`: sub-floor XLM dust, not enrichment lag

Every `pending` bucket in the window (1h 345/345, 1d 255/255) has a
native-XLM-quoted, price-forming candle with `close < 1e-11` XLM. That puts
its `close_usd` below the 1e-12 precision floor of the priced predicate: it is
non-zero on 344/345 (1h) and 254/255 (1d), and the remaining one has `close`
itself below the floor. The other candles in those buckets are quoted in assets
with no `usd_rate` row (WGUARDIAN, yXLM, GUARDIAN, SHX, …), so they are not
eligible and are outside the share. These buckets never publish, so `pending`
here does not mean "pending enrichment". It is the same class as the share-0
sub-1e-12 buckets from phase 2 in [[0147]]. They are spread evenly over the
week (1h: 21–103 a day), with no backlog building up.

## Decision (2026-09-29)

**X = 0.5 stays. No change to `views.sql`, no rollout.** The distribution has
no mass between the peaks, so there is no trough to move X to, and every X in
(0, 1] publishes the same set today. 0.5 stays as the guard for the transient
partial state seen on rollout day.

## Acceptance Criteria

- [x] Both grains measured on 7 days; numbers recorded here.
      → "Measurement (2026-09-29)" above.
- [x] Decision on X recorded: keep it, or change `views.sql` plus a rollout.
      → X = 0.5 stays, see "Decision (2026-09-29)".
