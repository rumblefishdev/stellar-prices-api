---
id: "0221"
title: "Confirm the MONTH quota rollover instant on production, on or after 1 September 2026"
type: RESEARCH
status: backlog
related_adr: ["0010"]
related_tasks: ["0191", "0180", "0157"]
tags: [layer-backend, priority-low, effort-small, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, measurement]
milestone: 3
links:
  - "../../archive/0191_FEATURE_rework-key-once-per-quota-period.md"
  - "../../archive/0180_RESEARCH_settle-undocumented-discord-and-aws-behaviours/README.md"
history:
  - date: "2026-08-25"
    status: backlog
    who: claude
    note: >
      Spawned from [[0191]] future work — the one acceptance criterion 0191
      could not close, because the next real `MONTH` rollover is 1 September
      2026. Dated, not blocked: nothing waits on it.
  - date: "2026-09-25"
    status: backlog
    who: okarcz
    note: >
      Measured the 2026-09-01 rollover on production with GetUsage (free
      plan 71t9im, 2026-08-27 → 09-04). The reset lands in the 09-01 daily
      bucket for every key with August usage; the 08-31 bucket still ends on
      August's balance. The handler log has no quota-reset warning since
      09-01 (0 of 1.46M records). The instant within 09-01 is unobservable
      with current logging. ADR 0010 correction #2 restated. Left open for
      a one-request probe at the 2026-10-01 boundary.
---

# Confirm the MONTH rollover instant on production

## Summary

AWS documents neither the quota reset instant nor its timezone (ADR 0010
correction #2). [[0191]] stopped presenting "00:00 UTC on the 1st" as
AWS-documented and states it as **our** rule, defined once in
`portal/period.rs` — that closed the wording half of the criterion. What is
still unmeasured is what AWS actually does at a `MONTH` boundary.

## Context

[[0180]] item 7's `DAY`-period proxy was abandoned on 2026-08-24 after two runs
died silently, and the scratch stack was torn down — the proxy existed to avoid
waiting for 1 September, and by then 1 September was 8 days away. The
replacement is to read the real rollover off production instead of a scratch
plan.

## Implementation

- On or after **2026-09-01**, look in the api-handler log for the warn
  `summarize_days` emits when `GetUsage` reports a **quota reset inside the
  queried period** (`packages/prices-api/src/portal/keys/gateway.rs`). The
  timestamp of the reset row is the answer.
- If production traffic is too thin to produce the warn, the fallback is the
  archived `item7-quota-rollover.sh` against a `MONTH` scratch plan drained on
  31 August — **its defects are recorded at the top of the file and must be
  fixed first**, chiefly the `teardown` that reports success while leaving the
  usage plan alive (it deletes the plan before the REST API that still
  references its stage, and every delete is `|| true`).
- If it is ever re-run: `UpdateUsagePlan` is throttled to **1 request per 20 s
  per account, non-adjustable**, and the control plane shares a **10 rps /
  burst 40** budget with our deploys. A careless loop slows CI for everyone.

## Acceptance Criteria

- [ ] The `MONTH` reset instant and its timezone are recorded with the date and
      the source (log line or measurement), or recorded as still unobserved
      with the reason. **Partly met, 2026-09-25:** the day is measured (the
      1st), and resets at midnight in any timezone east of UTC are ruled out.
      The time within the day is unobserved (no gateway access log; the
      handler doesn't log the calling key). See the measurement below.
      Closes with the 2026-10-01 probe, or by accepting the day-level result.
- [x] ADR 0010 correction #2 updated: either closed with the measured value, or
      restated with what is now known. Restated 2026-09-25, and the stale
      "`DAY`-period proxy" line (abandoned 2026-08-24) is replaced.
- [x] If AWS's instant differs from ours, the difference is written down as a
      dashboard-label wrinkle — **not** a change to the cap, which is ours by
      definition ([[0191]] decision #2) and stays one definition in
      `portal/period.rs`. No difference is visible at day granularity. The
      most the two could differ is from 00:00 UTC to the first request that
      counted in the new month on 09-01. On a 100,000 free quota that
      gap held a handful of requests; no label change needed.

## Measurement — the 2026-09-01 rollover (run 2026-09-25)

**In short:** AWS resets the `MONTH` quota on the 1st. On every key with
August usage, the balance refills in the **09-01** daily bucket, while the
**08-31** bucket still ends on August's balance. That agrees with our rule
(00:00 UTC on the 1st) at day level. The time within 09-01 can't be seen
with what we log today.

### 1. The warning this task planned to read never fired

Logs Insights on `/aws/lambda/prices-production-api-handler`, 2026-09-01
00:00 UTC → 2026-09-25, `filter @message like /GetUsage reported a quota reset/`:
**0 matches in 1,459,890 records.** On its own this is weak evidence. The
warning fires only when someone opens `/api/usage` and the queried range
(the 1st → today) shows `remaining` rising, and the handler doesn't log
individual requests, so the number of such calls is unknown.

### 2. GetUsage across the boundary (direct)

`aws apigateway get-usage --usage-plan-id 71t9im` (`pricing-api-free-production`,
`MONTH`, limit 100,000) `--start-date 2026-08-27 --end-date 2026-09-04`, profile
`soroban-readonly`, eu-central-1. Each cell is `used / remaining` for that day:

| key | 08-29 | 08-30 | **08-31** | **09-01** | 09-02 |
|---|---|---|---|---|---|
| `t61phbbhhj` | 0 / 98,793 | 0 / 98,793 | 1 / **98,792** | 3 / **99,997** | 4 / 99,993 |
| `31z25p…` | 0 / 100,000 | 0 / 100,000 | 29 / **99,971** | 0 / **100,000** | 0 / 100,000 |
| `6ncoc0…` | 0 / 100,000 | 0 / 100,000 | 1 / **99,999** | 0 / **100,000** | 7 / 99,993 |

- `remaining` is the balance at the end of each day: `t61phb`'s 08-27 → 08-28
  step is 98,934 − 141 = 98,793 exactly.
- `t61phb` shows the reset plainly. 99,997 = 100,000 − 3, so all three of its
  09-01 requests counted against September, and its 08-31 request counted
  against August.
- The other plans (`analyst`, `lite`, `pro`, `basic`) were created with
  [[0311]] on 2026-09-24 and haven't crossed a boundary. The load-test plan
  has no usage in the window.

### What this settles and what it doesn't

- **Settled:** the reset happens on the **1st**. It happens **after the
  08-31 bucket ends**, because every key's 08-31 balance is still August's.
  Assuming GetUsage's daily buckets are UTC days (AWS returns bare dates and
  states no timezone), that rules out a reset at midnight in any timezone
  east of UTC, e.g. 22:00 UTC on 08-31 for CET.
- **Not settled:** where inside 09-01 (UTC) the reset falls. The upper bound
  is `t61phb`'s first 09-01 request, and its time can't be recovered. The
  gateway has no access log (the only API Gateway log group is
  `/aws/apigateway/welcome`), and the handler doesn't log the calling key.
  The handler's first invocation on 09-01 was in the 08:00 UTC hour, so a
  reset at midnight US time (07:00–08:00 UTC) isn't ruled out either.

### To close the rest: a probe at 2026-10-01

A one-request probe pins the instant without new infrastructure:
1. On 2026-09-30, pick a test key on a `MONTH` plan with a balance below the
   limit (any key with September usage).
2. At **2026-10-01 00:05 UTC**, send exactly **one** keyed `/v1` request.
3. On 10-02, read `get-usage` for 09-30 → 10-01. If the 10-01 bucket shows
   `remaining = limit − 1`, the reset happened before 00:05 UTC and our rule
   matches AWS to within 5 minutes. If it shows the September balance − 1,
   the reset is later, and a second request at a later hour brackets it.
