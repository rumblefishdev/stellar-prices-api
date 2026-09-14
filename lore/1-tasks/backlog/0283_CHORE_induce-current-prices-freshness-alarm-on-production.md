---
id: "0283"
title: "Induce the current-prices-freshness alarm on production — stop mv_current_prices in an attended, announced window once the Milestone 2 review has closed"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0243", "0218", "0178", "0136"]
tags: [layer-infra, priority-medium, effort-small, observability, clickhouse, refreshable-mv, alarms, ops]
links:
  - "../active/0243_FEATURE_current-prices-freshness-alarm.md"
  - "../../../docs/runbooks/0072-current-prices-mv-rollout.md"
history:
  - date: 2026-09-14
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0243]]. That task ships the alarm and proves it with an IT
      that stops a real mv_current_prices locally, an action-less clone alarm on
      the real metric and a set-alarm-state routing check, and records its
      induction criterion as "Test-covered, not prod-induced". The production
      induction was deferred on 2026-09-14 because it freezes GET /price for
      ~16 minutes on the shared cluster while the Milestone 2 reviewer key is
      live.
---

# Induce the current-prices-freshness alarm on production

## Summary

Stop the real `prices.mv_current_prices` on production long enough for
`prices-production-current-prices-freshness` to fire, then restart it in the same
step. This closes [[0243]]'s induction criterion for real. It would be the first
`SYSTEM STOP VIEW` ever run on production in this repo.

## Preconditions

- [[0243]] is deployed and its alarm sits in OK **with datapoints**.
- The Milestone 2 review has closed, or the team agrees a window regardless.
- The window is announced to the team, at a quiet hour, with a hard deadline
  written down.
- [[0243]]'s local rehearsal IT (stop and restart a real `mv_current_prices`)
  passes on the pinned engine, 26.3.10.60.

## Procedure — attended, as `default` via `CHQ`

1. Baseline: `SELECT count(), max(updated_at), now() FROM prices.current_prices`
   and `SELECT status, last_success_time, exception FROM system.view_refreshes
   WHERE database = 'prices' AND view = 'mv_current_prices'`.
2. `SYSTEM STOP VIEW prices.mv_current_prices;` and note T0.
3. Poll `toUnixTimestamp(now()) - toUnixTimestamp(max(updated_at))` until it
   passes 905 s, then invoke the probe by hand so the breaching reading does not
   wait for the schedule:
   `aws lambda invoke --profile soroban-admin --region eu-central-1 --function-name prices-production-rollup-freshness-probe <out>`.
4. When ALARM lands in Slack, **in the same step**:
   `SYSTEM START VIEW prices.mv_current_prices; SYSTEM REFRESH VIEW prices.mv_current_prices;`
   then confirm the age is back to ≤ ~5 s and `count()` matches the baseline.
5. **Hard limit:** START no later than T0 + 20 min, whatever the alarm is doing.
   If the view does not come back, re-apply `current.sql` with [[0178]]'s sequence.
6. Record the ALARM and the OK that follows 15–30 min later, with timestamps and
   StateReasons.

## Acceptance Criteria

- [ ] The alarm reached ALARM from a really stopped writer, recorded with its
      StateReason.
- [ ] It returned to OK after START, recorded.
- [ ] `current_prices` was frozen no longer than the hard limit; the freeze window
      is recorded.
- [ ] [[0243]]'s induction criterion is re-ticked with this evidence.

## Notes

- Standing rules from [[0136]]: touch `prices.*` only, and pair any `SYSTEM STOP`
  with its `START` in the same runbook step.
- The API Gateway cache can stretch what consumers see by its TTL.
