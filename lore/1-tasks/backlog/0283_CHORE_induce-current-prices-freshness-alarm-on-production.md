---
id: "0283"
title: "Induce two alarms on production in one attended window — stop mv_current_prices for 0243, and prove 0214's digest actually publishes"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0243", "0214", "0218", "0178", "0136"]
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
  - date: 2026-09-15
    status: backlog
    who: stkrolikiewicz
    note: >
      Second induction folded in. [[0214]]'s digest deployed today and every
      link was verified except the one that matters most: with 0 of 52 alarms
      off OK, the publishing branch never ran, so `sns:Publish` from inside the
      Lambda is unexercised on production. Narrowed as far as possible without
      an induction — `simulate-principal-policy` on the real Lambda role returns
      `allowed` for sns:Publish on the ops topic, and DescribeAlarms is proven
      at runtime (the probe logged `matched: 52`) — and a failed publish would
      page rather than go quiet, because the digest error fails the invocation
      and trips the probe's ops-wired error alarm. Folded here rather than
      spawned separately because the two share an attended, announced window and
      the 0214 half is a 60-minute WAIT that runs in the background while the
      0243 half is performed. ⚠️ They do NOT compose automatically: 0283's own
      ALARM lasts ~20 minutes by its hard limit, well under the digest's
      one-hour floor, so the digest would not report it.
---

# Induce two alarms on production in one attended window

## Summary

Two inductions, one window, deliberately sequenced so the second one's waiting
happens during the first one's work.

**Part A — [[0243]].** Stop the real `prices.mv_current_prices` long enough for
`prices-production-current-prices-freshness` to fire, then restart it in the same
step. This closes [[0243]]'s induction criterion for real. It would be the first
`SYSTEM STOP VIEW` ever run on production in this repo.

**Part B — [[0214]].** Prove the stuck-alarm digest actually publishes, by
putting a **throwaway alarm** into ALARM for over an hour and invoking the probe.

## Order of operations

Part B is started FIRST and finished LAST, because its cost is an hour of
elapsed time and nothing else:

```
T-60  Part B step 1-2   create the throwaway alarm, force it to ALARM
T0    Part A            stop the view, wait for ALARM, restart  (~20 min, hard limit)
T+60  Part B step 3-5   invoke the probe, read the digest, delete the alarm
```

## Preconditions

- [[0243]] is deployed and its alarm sits in OK **with datapoints**.
- [[0214]]'s digest is deployed — done 2026-09-15 12:22 UTC — and a manual probe
  invocation returns `"stuck_alarms": 0` with `matched: 52` in the log. If it
  does not, part B is testing the wrong thing.
- The Milestone 2 review has closed, or the team agrees a window regardless.
- The window is announced to the team, at a quiet hour, with a hard deadline
  written down.
- [[0243]]'s local rehearsal IT (stop and restart a real `mv_current_prices`)
  passes on the pinned engine, 26.3.10.60.

## Procedure, part A — [[0243]], attended, as `default` via `CHQ`

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

## Procedure, part B — [[0214]], a throwaway alarm, no production impact

⚠️ **Do NOT `set-alarm-state` a real alarm for this.** An hour of false ALARM on
a live alarm is exactly the "trains people to scroll" failure [[0214]] exists to
end, and it would hide a real incident on that alarm for the whole window. Use a
disposable one instead — the same action-less clone pattern [[0243]] used.

1. Create the throwaway. The name **must** carry the `prices-production-` prefix
   or the digest will not see it (`alarm_digest::run` filters on exactly that),
   and it **must** have no alarm actions, so only the digest speaks:

   ```
   aws cloudwatch put-metric-alarm --profile soroban-admin --region eu-central-1 \
     --alarm-name prices-production-tmp-0214-digest-induction \
     --alarm-description "Throwaway, task 0283 part B. Delete after the window." \
     --namespace Prices/Rollup --metric-name RollupLagSeconds --statistic Maximum \
     --period 900 --evaluation-periods 1 --threshold 0 \
     --comparison-operator GreaterThanThreshold --treat-missing-data notBreaching
   ```

2. Force it and note T_B0:
   `aws cloudwatch set-alarm-state --alarm-name prices-production-tmp-0214-digest-induction --state-value ALARM --state-reason "0283 part B induction"`.
   ⚠️ `set-alarm-state` stamps `StateTransitionedTimestamp` at **now**, so the
   digest's one-hour floor (`MIN_STUCK_SECONDS`) starts here. There is no way to
   backdate it; the hour is the price of the test.
3. At T_B0 + 65 min, invoke the probe by hand rather than waiting for `rate(1 day)`:
   `aws lambda invoke --profile soroban-admin --region eu-central-1 --function-name prices-production-mtls-notafter-probe <out>`.
   The response must read `"stuck_alarms": 1`.
4. Read the channel. Expect ONE message titled `production: alarms stuck off OK`
   listing the throwaway with an age just over `1h`, its name a working link to
   the CloudWatch console. Screenshot it.
5. **Delete it in the same step**, and verify it is gone:
   `aws cloudwatch delete-alarms --alarm-names prices-production-tmp-0214-digest-induction`,
   then `describe-alarms --alarm-name-prefix prices-production-tmp` must be empty.
   ⛔ A throwaway alarm left behind would be reported by every future digest —
   the digest would become the noise it was built to remove.

**Hard limit:** the throwaway exists no longer than T_B0 + 2 h. If the window is
abandoned, still run step 5.

## Acceptance Criteria

### Part A — [[0243]]

- [ ] The alarm reached ALARM from a really stopped writer, recorded with its
      StateReason.
- [ ] It returned to OK after START, recorded.
- [ ] `current_prices` was frozen no longer than the hard limit; the freeze window
      is recorded.
- [ ] [[0243]]'s induction criterion is re-ticked with this evidence.

### Part B — [[0214]]

- [ ] The probe returned `"stuck_alarms": 1` from a real `DescribeAlarms` read.
- [ ] Exactly one digest message reached the channel, listing the throwaway with
      its age; screenshot recorded.
- [ ] The console link in that message was clicked and resolved to the alarm.
- [ ] The throwaway alarm is deleted and `--alarm-name-prefix prices-production-tmp`
      returns empty.
- [ ] [[0214]]'s AC 4 evidence is extended with this, replacing the
      "publish path unexercised" caveat recorded on 2026-09-15.

## Notes

- Standing rules from [[0136]]: touch `prices.*` only, and pair any `SYSTEM STOP`
  with its `START` in the same runbook step.
- The API Gateway cache can stretch what consumers see by its TTL.
