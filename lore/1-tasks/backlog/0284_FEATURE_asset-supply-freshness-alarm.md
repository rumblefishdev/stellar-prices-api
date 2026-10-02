---
id: "0284"
title: "No alarm watches asset_supply freshness — a dead supply worker leaves market_cap_usd frozen, and the worker has no liveness alarm either"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0243", "0223", "0112"]
tags: [layer-infra, priority-low, effort-small, observability, clickhouse, alarms]
links:
  - "../active/0243_FEATURE_current-prices-freshness-alarm.md"
history:
  - date: 2026-09-14
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0243]]'s note asking whether prices.assets and
      prices.asset_supply share current_prices' blind spot. asset_supply does:
      the supply worker is its only writer, hourly, versioned by fetched_at, and
      the worker has only an -errors alarm — it is exempt from the worker-health
      pair, so a worker that stops being invoked reads OK ([[0223]]).
      prices.assets does not qualify: it has many writers, and asset-discovery
      re-emits ~204k rows an hour, so its max(updated_at) measures activity,
      not freshness.
---

# `asset_supply` can freeze and nothing notices

## Summary

`market_cap_usd` in `current_prices` multiplies the price by `token_supply` from
`prices.asset_supply`. That table's only writer is the supply worker, on
`rate(1 hour)`. If the worker stops, supply stops updating and nothing reports
it: the worker is exempt from the no-invocations and duration alarms, and its
`-errors` alarm reads OK on no data.

## Implementation sketch

- Reuse [[0243]]'s pattern in `rollup-freshness-probe`: a separate check reading
  `count()` and the age of `max(fetched_at)` from `asset_supply`, published as
  `RollupLagSeconds` with `Table=asset_supply`, where an empty table publishes
  the sentinel.
- Derive the bound from `rate(1 hour)` plus the worker's run time, with headroom.
- ⚠️ First measure whether every supply run writes at least one row. If a run can
  legitimately write nothing, `max(fetched_at)` is not a liveness signal and the
  design changes.

## Acceptance Criteria

- [ ] Measured: every supply run writes at least one row, or the design changed.
- [ ] An alarm fires when `asset_supply` stops advancing, with the bound derived
      and written down.
- [ ] Routed to the ops Slack channel.
