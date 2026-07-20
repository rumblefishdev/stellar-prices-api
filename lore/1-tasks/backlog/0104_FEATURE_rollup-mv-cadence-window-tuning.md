---
id: "0104"
title: "Tune rollup MV cadence vs window against prod merge load"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0095", "0059"]
tags: [layer-infra, priority-low, effort-small, clickhouse, rollup, materialized-view, performance]
links:
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: 2026-07-17
    status: backlog
    who: okarcz
    note: "Spawned from 0095 future work — cadence/window re-eval deferred to real prod merge metrics."
---

# Tune rollup MV cadence vs window against prod merge load

## Summary

The six APPEND rollup MVs (task 0095) kept their pre-existing refresh intervals
and windows. Under APPEND, each coarse bucket is re-appended `window ÷ interval`
times before it ages out of the window — duplicate rows the `ReplacingMergeTree`
merges collapse, but still write + merge load on the shared `ch-prod-01`:

| MV | interval | window | re-appends/bucket |
|----|----------|--------|-------------------|
| 1m→15m | 1 min | 2 h | 120× |
| 15m→1h | 15 min | 8 h | 32× |
| 1h→4h | 1 h | 1 d | 24× |
| 4h→1d | 4 h | 7 d | 42× |
| 1d→1w | 1 d | 60 d | 60× |
| 1w→1M | 1 d | 400 d | 400× |

This is load, **not** a correctness issue — window alignment + `sum(version)`
hold at any window (0095). 0095 kept the values to minimise behaviour change on
the M1-blocking deploy and deferred tuning to real measurement.

## Context

The windows look sized for the old replace-mode world (where re-append count did
not matter). The `1w→1M` 400-day window especially buys nothing for a live-tail
rollup — deep history is pre-rolled and static; the MV only needs to cover the
live tail plus realistic late-arrival lag (enrichment, task 0026).

## Implementation

- Measure post-0095 merge load on the coarse tables (`system.merges`,
  `system.part_log`, parts count / merge time per `price_ohlcv_*`).
- Tighten windows to just cover realistic source-update lateness, and/or widen
  the fastest intervals, to cut the worst amplification (`15m` 120×, `1M` 400×).
- Keep window alignment and `sum(version)` unchanged.
- Re-run `rollup_append_it.rs` + `rollup_chain_it.rs` after any change; update
  the per-grain cadence rationale comment in `rollups.sql`.

## Acceptance Criteria

- [ ] Merge/amplification load measured on prod before and after.
- [ ] Cadence/window adjusted (or explicitly justified as-is) with the rationale
      documented per grain in `rollups.sql`.
- [ ] Tests still green on CH 26.3.10.60; no partial-bucket / history-loss
      regression.
