---
id: "0243"
title: "No alarm watches current_prices freshness — a dead mv_current_prices serves a frozen price behind a healthy HTTP 200"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0178", "0137", "0204", "0218"]
tags:
  [
    "priority-high",
    "effort-small",
    "observability",
    "clickhouse",
    "refreshable-mv",
    "read-surface",
    "milestone-M2",
  ]
milestone: 2
links:
  - "../../../packages/rollup-freshness-probe/src/main.rs"
  - "../../../packages/prices-clickhouse/schema/current.sql"
history:
  - date: 2026-08-31
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0178]]'s deploy-runbook work. Writing the rollback plan for
      that task's DROP + recreate required knowing what would page if the
      recreate failed, and the answer measured on 2026-08-31 is nothing:
      rollup-freshness-probe covers the price_ohlcv_* tiers only, and no
      Observability construct references current_prices. Kept out of 0178
      because that task is a data-correctness fix and this is an ops gap that
      predates it and outlives it.
---

# `current_prices` can freeze and nothing notices

## Summary

`prices.current_prices` is written by exactly one writer, the refreshable MV
`mv_current_prices`, on a 1-minute schedule. **Nothing monitors whether that
writer is still running.** If it stops, the table keeps its last-written rows
and `GET /price` keeps returning HTTP 200 with a plausible price that has
silently stopped moving.

## Why this is worse than an outage

Every consumer-side health signal stays green. The endpoint responds, the
status code is 200, the payload validates, the price is a normal number. Only
`updated_at` betrays it — and `updated_at` is the MV's refresh time, so it stops
advancing exactly when the writer dies.

There is prior art for the failure mode in this repo: [[0215]]'s enrichment pass
failed on **every invocation for 26 days** while ClickHouse logged `QueryFinish`
and every data signal read normal. The lesson recorded there — that a healthy
exit status is not evidence of work done — applies unchanged here.

## Measured 2026-08-31

- `rollup-freshness-probe` watches the `price_ohlcv_*` tiers. `grep` for
  `current_prices` in its source: no match.
- `grep` for `current_prices` / `CurrentPrices` across `infra/`: no match.
- So the gap is total, not partial.

## Implementation sketch

The probe already has the right shape — reuse it rather than writing a second
one, per the working agreement on reusing tested code.

- Metric: `now() - max(updated_at)` on `prices.current_prices FINAL`, in seconds.
- Bound: the refresh interval (1 min) plus refresh duration, with headroom.
  Follow [[0137]]'s rule — the bound is bucket width **plus** the feeding
  refresh, not the width alone.
- ⚠️ The probe reads as `prices_writer`, for which `system.*` is denied and
  cannot be granted. Do not reach for `system.view_refreshes` without checking
  that wall first — see [[0204]]'s and [[0182]]'s experience.
- ⚠️ Never run the alarm at `1/1`. Cf. the FILL/sliding-window notes from 0218.

## Acceptance Criteria

- [ ] An alarm exists that fires when `current_prices` stops advancing.
- [ ] Its bound is derived from the refresh interval, not guessed, and the
      derivation is written down.
- [ ] Verified by INDUCING the condition, not by reading the definition —
      the standard this repo has held since [[0204]].
- [ ] Routed to the same Slack channel as the existing ops alarms.
- [ ] Does not depend on any `system.*` table.

## Notes

- [[0178]] is the task that uncovered this. Its runbook compensates for the gap
  manually (verify `updated_at` advances across two refresh cycles before
  declaring the deploy good); once this alarm exists that manual step can be
  dropped from future MV recreates.
- Worth checking at the same time whether `prices.assets` and
  `prices.asset_supply` have the same blind spot — both are single-writer
  tables feeding the same read surface.
