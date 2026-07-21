---
id: "0112"
title: "Progress alarms are blind to a worker that dies — enrichment failed 4 days with no alert"
type: BUG
status: active
related_adr: []
related_tasks: ["0026", "0056", "0111"]
tags: [layer-ops, observability, cloudwatch, alarms, priority-high, effort-small, incident]
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
history:
  - date: 2026-07-21
    status: backlog
    who: okarcz
    note: >
      Spawned from the enrichment timeout investigation ([[0111]]). Enrichment
      failed 72/72 invocations per day for four consecutive days (2026-07-14 →
      07-17) and the stall alarm never fired, because the alarm reads metrics
      the worker only publishes if it survives to the end of a pass.
  - date: 2026-07-21
    status: active
    who: okarcz
    note: >
      Implemented. Reading createWorkerLambda corrected this task's own
      diagnosis: prices-production-enrichment-errors ALREADY existed as an
      AWS/Lambda Errors alarm, but errorAlarmActions was passed at only 2 of 7
      call sites, so five cron workers had alarms that transition to ALARM and
      notify nobody. That, not only the custom-metric blind spot, is why the
      4-day outage was silent. Implementing this task as written would have
      collided on alarmName and left the real defect in place.
      Wired the action for all five, added a reusable
      addWorkerHealthAlarms() (duration-near-timeout + no-invocations on
      platform metrics) for the three workers whose only other alarm reads a
      self-published metric, and single-sourced the function name so a rename
      cannot silently disarm an alarm. Audit found three custom-metric alarms
      share the defect, not one; mtls is the worst (expired cert breaks all
      ingestion). Caught a 259200s alarm period that synth accepts and
      CloudFormation would reject. Fire-test remains operator-run.
---

# Progress alarms are blind to a worker that never reports

## Summary

`enrichmentBacklogAlarm` is built from `EnrichmentRowsEnriched` and
`EnrichmentRowsRemainingRecent` — **custom metrics our own worker publishes at
the end of a pass** (`packages/enrichment-worker/src/main.rs:104`). Its
expression is `(enriched < 1) * (backlog > 0)`, with
`treatMissingData: NOT_BREACHING` (`infra/src/lib/stacks/observability-stack.ts:186`).

A worker killed by the 300 s Lambda timeout never reaches `publish()`. So it
emits **nothing** — and "nothing" is configured as healthy.

The alarm can therefore detect *"enrichment ran and made no progress"* but is
structurally incapable of detecting *"enrichment did not run"*. The second is
the more serious failure and the one that actually happened.

## Evidence

2026-07-14 → 07-17: `AWS/Lambda Errors` = **72/day** on
`prices-production-enrichment` (24 hourly invocations × 3 async retries, i.e.
100% failure), `Duration.Maximum` pinned at ~300,400 ms. Degraded either side:
36 errors 07-10, 29 on 07-13, 9 on 07-18.

Over that window the custom `Prices/Enrichment` metrics have **no datapoints at
all** — which is precisely why the alarm stayed green. The gap in the custom
metric *is* the outage, not an absence of one.

Nobody was paged. It was found only by manually querying CloudWatch on 07-21,
six days after recovery.

## The general defect

This is not only about enrichment. **Any alarm whose signal is emitted by the
process it is monitoring cannot detect that process dying.** Worth auditing the
other `NOT_BREACHING` alarms in `observability-stack.ts` (lines 186, 292, 317,
372, 399, 430) against the same question: *if the emitter dies silently, does
this alarm notice?*

Note line 468 already uses `treatMissingData: BREACHING` deliberately, with a
comment calling it load-bearing — so the distinction is understood in the
codebase; it just was not applied here.

## Implementation

- Add a **platform-metric** alarm for the enrichment Lambda that does not depend
  on the worker reporting on itself: `AWS/Lambda Errors > 0` sustained, and/or
  `Duration.Maximum` approaching the configured timeout.
- Consider an invocation-liveness alarm (`Invocations` missing for N hours with
  `treatMissingData: BREACHING`) so a rule that stops firing is also caught.
- Re-examine whether `NOT_BREACHING` is right for the existing progress alarm
  once a platform-metric alarm covers the death case.
- Audit the other five `NOT_BREACHING` alarms for the same blind spot.

## Acceptance Criteria

- [x] An alarm fires within ~1 h of the enrichment Lambda failing repeatedly,
      without depending on any metric the worker publishes. *Two routes now:
      the pre-existing `-errors` alarm has an action wired (~2 h), and
      `-duration-near-timeout` warns before failures start.*
- [ ] Verified by a real fire-test (the 0056 precedent: breach + recovery), not
      only by synth — an alarm nobody has seen fire is an untested alarm, which
      is exactly what this incident was. **Operator-run; requires deploy.**
- [x] Duration-approaching-timeout warns *before* the timeout, so [[0111]]-class
      regressions surface as a warning rather than an outage. *80% of timeout,
      2 consecutive periods.*
- [x] The other `NOT_BREACHING` alarms are audited and the finding recorded,
      even where no change is made. *Three share the defect, not one; table in
      §Implementation. All three now have a platform-metric backstop.*
- [x] Routed to the same Slack channel as the 0056 alarms. *All alarms use the
      shared `opsAlarmsTopic`; verified `actions=1` on all 13 + 7 in the
      synthesised templates.*
- [x] **The five workers with inert `-errors` alarms are wired.** *Emerged
      during implementation and is the primary defect — see §Implementation.*

## Out of scope

- Fixing the timeout itself — that is [[0111]]. This task ensures the next one
  is noticed in an hour rather than a week.

## Implementation (2026-07-21)

### ⚠️ The original diagnosis in this task was incomplete

This task was written believing the silence had ONE cause: the progress alarm
reading a self-published metric. Reading `createWorkerLambda` showed a second,
simpler, and more direct cause that the task missed entirely.

**`prices-production-enrichment-errors` already existed.** `createWorkerLambda`
creates an `AWS/Lambda Errors ≥ 1` alarm for every worker
(`infra/src/lib/lambda-baseline.ts:309`). With 72 errors/day it would have gone
to ALARM on 07-14 and stayed there.

**It had no action wired.** `errorAlarmActions` was passed at exactly two of
seven call sites — the two probes. The five cron workers (asset-discovery,
cleanup, supply, oracle, **enrichment**) all had error alarms that could never
notify anyone. The prop's own doc warned about this:

> *"Optional: the alarm is created either way, but with no action it is inert
> (transitions to ALARM but notifies no one)."* — `lambda-baseline.ts:189`

So the correct alarm existed, fired, and told nobody. That is a smaller and more
embarrassing defect than "the alarm design is structurally blind", and it is the
primary fix.

Had this task been implemented as written — adding a new errors alarm in
ObservabilityStack — it would have **collided on `alarmName`** with the existing
one and failed to deploy, while leaving the inert-action defect untouched.

### What was actually done

1. **Wired `errorAlarmActions: [opsAlarmAction]` to all five unwired workers.**
   Required hoisting `opsAlarmAction` to the top of the constructor (it was
   defined at line 420, after every worker). All 7 `-errors` alarms now show
   `actions=1` in the synthesised template; previously 2 of 7 did.
2. **Added `addWorkerHealthAlarms()`** — a reusable factory adding the two
   alarms nothing else covers, on `AWS/Lambda` metrics the platform emits
   regardless of whether our code survives:
   - `-duration-near-timeout` (Maximum ≥ 80% of the timeout, 2 periods). **The
     one that would have prevented the outage rather than reported it** —
     enrichment's batch cost climbed for days before crossing the wall.
   - `-no-invocations` (`treatMissingData: BREACHING`, 3 periods). Catches a
     disabled/deleted rule, which the errors alarm cannot see: no invocations
     means no error datapoints either.
   Applied to the three workers whose only other alarm reads a self-published
   metric: enrichment, backfill-freshness-probe, mtls-notafter-probe.
3. **`workerFunctionName()` in `lambda-baseline.ts`** — single source of truth,
   used by both `createWorkerLambda` and the alarm dimensions. An alarm pointed
   at a non-existent function name does not error, it just never fires, so a
   rename that updated only one side would silently disarm the alarms.

### Audit of `treatMissingData` (AC)

| alarm | metric source | blind to a dead emitter? |
|---|---|---|
| `enrichment-backlog` | `Prices/Enrichment` **custom** | ⚠️ yes — the incident |
| `sdex-push-freshness` | `Prices/Backfill` **custom** | ⚠️ yes — same defect |
| `mtls-notafter` | `Prices/Mtls` **custom** | ⚠️ yes — **worst**: an expired cert breaks ALL ingestion |
| `ledger-processor-lag` | `AWS/SQS` platform | no |
| `ledger-processor-errors` | `AWS/Lambda` platform | no |
| `ledger-processor-dlq` | `AWS/SQS` platform | no (documented) |
| `ledger-processor-no-invocations` | `AWS/Lambda`, **BREACHING** | ✅ already the correct pattern |

Three custom-metric alarms share the defect, not one. All three now have a
platform-metric backstop. The correct pattern already existed in-repo for the
ledger-processor; it had simply never been applied to the scheduled workers.

## Design Decisions

### From Plan

1. **Platform metrics, not custom ones.** `AWS/Lambda` is emitted by the
   platform, so it survives the worker dying — the whole point.
2. **`treatMissingData: BREACHING` on no-invocations.** Load-bearing: Lambda
   publishes no `Invocations` datapoint for a period with zero invocations, so
   a `LESS_THAN` threshold alone would never evaluate.

### Emerged

3. **Did NOT add an errors alarm**, contrary to this task's own implementation
   notes — one already exists per worker. Wired its action instead. See above.
4. **Fixed all five unwired workers, not just enrichment.** The defect is a
   class; fixing only the instance that happened to bite leaves four identical
   traps. Costs nothing extra.
5. **Three periods × one cadence, not one period × three cadences.** The
   equivalent-looking form gives the daily mtls probe a 259200 s period, over
   CloudWatch's 86400 s maximum. That is a **deploy-time** validation failure —
   `synth` accepts it happily — so it would have passed every local check and
   failed in CloudFormation. Caught by inspecting the synthesised template
   rather than trusting `synth OK`.
6. **80% duration threshold** rather than a fixed margin, so it scales with each
   worker's timeout (240 s of 300 s for enrichment; 48 s of 60 s for the probes).
7. **Did not thread the Lambda functions into ObservabilityStack as props.**
   Timeout and cadence are duplicated from eventbridge-stack.ts /
   production.json. That drift is real but benign — a stale timeout only
   mis-tunes the threshold, a stale cadence only widens the window; neither can
   silently disarm an alarm. Function *names*, which can, are single-sourced.

## Verification

- `lint`, `build`, `typecheck` green.
- `make -C infra synth-production` succeeds.
- **Synthesised template inspected, not just synth exit code**: all 7 `-errors`
  alarms have `actions=1` (was 2 of 7); 6 new alarms present; no `alarmName`
  collisions; no `Period` exceeds 86400 s.
- Detection windows: enrichment ~2 h (errors, duration) / 3 h (no-invocations);
  freshness probe 30 / 45 min; mtls probe 2 / 3 days. Against **four days**.

### Not verifiable without a deploy (operator)

Standing prepare-not-deploy rule — this branch synths only.

- **Fire-test** (breach + recovery), per the 0056 precedent. An alarm nobody has
  watched fire is an untested alarm, which is exactly what this incident was.
- ~~**Confirm the inert-alarm theory empirically.**~~ **CONFIRMED 2026-07-21** —
  see §Confirmation below. Command retained for reference:
  ```bash
  aws cloudwatch describe-alarm-history --region eu-central-1 \
    --profile soroban-explorer --alarm-name prices-production-enrichment-errors \
    --history-item-type StateUpdate --start-date 2026-07-13T00:00:00Z \
    --end-date 2026-07-19T00:00:00Z \
    --query 'sort_by(AlarmHistoryItems,&Timestamp)[].[Timestamp,HistorySummary]' \
    --output table
  ```
  A transition to ALARM on 07-14 with no notification confirms it. If it never
  transitioned, the diagnosis above is wrong and this needs re-opening.


## Confirmation — the alarm fired for 4.5 days into nothing (2026-07-21)

`describe-alarm-history` on `prices-production-enrichment-errors`:

```
2026-07-13T04:23:59  OK → ALARM
2026-07-13T05:31:59  ALARM → OK
2026-07-13T11:19:59  OK → ALARM
2026-07-13T12:24:59  ALARM → OK
2026-07-13T14:23:59  OK → ALARM
2026-07-13T15:24:59  ALARM → OK
2026-07-13T17:18:59  OK → ALARM      <- and stayed there
2026-07-18T03:31:02  ALARM → OK
```

**Continuously in ALARM for 4 days, 10 hours, 12 minutes**, preceded by three
flaps as the degradation set in.

This settles the diagnosis with no inference left. The alarm was **correct and
timely**: it caught the onset on 07-13 at 17:18, **7.5 days before** a human
noticed on 07-21, and held ALARM for the whole outage. Detection was never the
problem.

It was **mute**. `errorAlarmActions` was empty, so a perfectly functioning alarm
transitioned, held, and recovered without notifying anyone. The entire fix for
the primary defect is one line per worker.

Worth keeping in mind when reading the rest of this task: the custom-metric
blind spot documented above is real, and the platform-metric backstops are worth
having. But they were the *second* problem. The first was that we had already
built the right alarm and never plugged it in — and no amount of additional
alarm design would have helped, because every new alarm would have been wired
the same way.
