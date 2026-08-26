---
id: "0222"
title: "The coarse-sweep no-invocations alarm did not fire after three genuinely empty hours — the instrument that detects a dead schedule is slower than its config claims"
type: BUG
status: completed
related_adr: []
related_tasks: ["0218", "0204", "0220"]
tags: [layer-infra, priority-high, effort-small, observability, cloudwatch, alarms, ops]
milestone: 2
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
history:
  - date: 2026-08-25
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0218]]'s second failed induction of AC 2 state 3. The rule
      was disabled 08:44 and re-enabled ~12:20 UTC; `Invocations` had no
      datapoints for 09:00, 10:00 or 11:00 and `CoarseSweepRuns` was silent
      across the same span, so the precondition was met exactly. The alarm stayed
      OK throughout and 15 minutes past the third bucket closing, with no state
      transition recorded on 2026-08-25 at all. Configuration read from
      `describe-alarms` during the window and confirmed correct, so this is not
      [[0204]]'s blind-alarm failure and the definition needs no fix.
  - date: 2026-08-25
    status: active
    who: okarcz
    note: >
      Activated the same day it was spawned, ahead of the rest of the backlog.
      Reason: it is not a nice-to-have observability improvement, it is the
      safety net for a schedule that has already gone silently dead once — and
      that net has now demonstrably failed a live test. Until the lag is known,
      nobody can say how long a dead coarse sweep would go unnoticed, which is
      [[0218]]'s original defect with the fix in place.
      First step is deliberately the cheap one: measure the lag against an
      already-idle Lambda, touching no production schedule. Two attended ~3.5 h
      windows have been spent on inducing this the expensive way and neither
      produced evidence.
  - date: 2026-08-25
    status: active
    who: okarcz
    note: >
      Experiment run and cleaned up the same hour, no production schedule
      touched. Two temporary alarms on the coarse sweep's own idle window
      (it runs at :30, so it is idle 59 minutes of every hour) breached in
      **2m42s at Period=60** and **6m51s at Period=300**. That kills the worst
      hypothesis — missing-data evaluation is NOT broken for stopped metrics —
      and also kills strict proportionality to Period (5x period gave ~2.5x lag).
      🔴 It does not explain the production failure: on those numbers the 3600 s
      alarm should have breached ~12:05 and did not. A sub-proportional-lag story
      fits every observation but is constructed after the fact and is recorded as
      an untested hypothesis, not a conclusion.
      Decision: stop reverse-engineering CloudWatch's missing-data semantics and
      make silence produce a datapoint instead. ⚠️ Note the correction to the
      original plan — publishing `CoarseSweepRuns = 0` from the worker cannot
      work alone, because if the Lambda is never invoked nothing in it can
      publish, and that is precisely the condition being detected. Metric math
      with FILL(metric, 0) is the cheapest candidate and may reduce this to a
      one-line CDK change; evaluate it before building anything.
  - date: 2026-08-25
    status: active
    who: okarcz
    note: >
      AC 5 RESTATED rather than ticked, and the reasoning matters more than the
      tick: an alarm's time-to-fire and its time-to-test are the same quantity,
      so no configuration change makes a three-hour condition testable in less
      than three hours. The criterion asked for something unachievable. It is
      replaced by proving the FILL mechanism breaches a real alarm on a trailing
      gap; whether to spend a third attended window on 0218's own AC is left to
      0218. Genuinely shortening detection was considered and declined — a
      product question, and it cuts against the ledger-processor flap mitigation.
      AC 6 ticked on the finding: the sibling alarms have a DIFFERENT failure
      mode (NOT_BREACHING → OK on no data), which FILL does not address and whose
      remedy has different semantics; spawned separately. One AC added: watch the
      2/2 ledger-processor alarm for ~2 h post-deploy, since its flap risk was
      mitigated on judgement rather than measurement.
  - date: 2026-08-25
    status: active
    who: okarcz
    note: >
      Deployed to production and AC 4 induced the same hour. All six
      no-invocations alarms now evaluate `FILL(invocations, 0)`; the temp alarm
      `tmp-0222-fill-p300` went OK to ALARM at 17:46:28 UTC on a trailing gap and
      was deleted before the next :30 run. The induction ran at `Period=300`
      rather than the 900 written into AC 4 - at 900 the third empty bucket
      completes exactly at :30, racing the next sweep run, so 900 is a less
      reliable test rather than a stricter one. AC 4 restated accordingly.
      Two findings came out of the `stateReasonData` that are larger than the
      tick: evaluation windows are query-anchored and slide rather than aligning
      to the clock, which falsifies the bucket reasoning used for the earlier
      control run; and on the corrected baseline FILL detects silence roughly an
      order of magnitude faster than the raw form. AC 7 still open - the
      ledger-processor flap watch is a single `describe-alarm-history` read on
      2026-08-26, since history persists.
  - date: 2026-08-26
    status: completed
    who: okarcz
    note: >
      AC 7 closed on the overnight read and the task completed. 7 of 7 criteria
      resolved — 5 ticked as written, AC 4's Period and AC 5 restated rather than
      ticked, with the reasoning recorded in both cases.
      The `2/2` ledger-processor flap did NOT materialise: zero state transitions
      in the 13.5 h since the 17:32 deploy, `StateUpdatedTimestamp` still
      2026-07-09, and the only retained history item is the ConfigurationUpdate
      at 17:31:40. The OK was checked for substance rather than taken at face
      value — 56 of 56 expected 900 s buckets, 155-161 invocations each, FILL
      output byte-identical to raw with no filled buckets.
      🔑 Finding 4 added: a hand-run `get-metric-data` FILL query whose window
      ends in the FUTURE fabricates a trailing gap — 19 consecutive zero buckets
      that read exactly like a 4h45m ingestion halt. Bound the window at `now`.
      It also strengthens the 2/2 judgement call: if future buckets fill as
      zeros, an incomplete current bucket does too.
      Shipped: PR #247 (the FILL fix, deployed 2026-08-25 17:32 UTC), PR #250
      (deploy + induction record). Six no-invocations alarms converted to metric
      math, zero left on the legacy single-metric form. Follow-up [[0223]]
      already spawned for the NOT_BREACHING siblings.
---

# The no-invocations alarm did not fire after three empty hours

## Summary

`prices-production-coarse-sweep-no-invocations` is configured to alarm on **3
consecutive 1-hour periods** with no `Invocations`, `treatMissingData: breaching`.
On 2026-08-25 it was given exactly that and **stayed `OK`**.

The alarm exists so that a coarse sweep which stops running gets noticed. If it
needs substantially longer than its `3 × 1 h` configuration implies, the
schedule can be dead for hours longer than anyone expects — which is [[0218]]'s
original defect, wearing the uniform of its own fix.

## Evidence

Rule `prices-production-coarse-sweep` disabled **08:44 UTC**, re-enabled
**~12:20 UTC**.

`AWS/Lambda` `Invocations`, `FunctionName=prices-production-coarse-sweep`:

```
06:00  1.0        09:00  — no datapoint
07:00  1.0        10:00  — no datapoint
08:00  1.0        11:00  — no datapoint
```

`Prices/Enrichment` `CoarseSweepRuns` — no datapoints 09:00-12:00.

`describe-alarm-history --history-item-type StateUpdate` — **no transition on
2026-08-25**. Newest entries are 2026-08-24's `INSUFFICIENT_DATA → ALARM`
(14:15) and `ALARM → OK` (14:19).

## The configuration is correct — do not re-check it

```
Metric              AWS/Lambda / Invocations
Dimensions          FunctionName=prices-production-coarse-sweep
Period              3600     EvaluationPeriods 3   DatapointsToAlarm 3
Threshold           < 1.0    ComparisonOperator LessThanThreshold
TreatMissingData    breaching
```

Read live from `describe-alarms`. The dimension names the right function, so this
is **not** [[0204]]'s "alarm pointed at a function that does not exist" failure.

⚠️ `StateReason` is stale by construction — CloudWatch rewrites it only on a
state *change*. During the window it still described the previous day's
transition. Do not read it as evidence about the current evaluation.

## Hypothesis — NOT established

`AWS/Lambda` `Invocations` publishes **nothing** for an idle function; it does
not publish a zero. An alarm on a metric that published and then *stopped* may
evaluate missing data much more slowly than `EvaluationPeriods × Period`
suggests.

Contrast worth noting: on 2026-08-24 the same alarm went `INSUFFICIENT_DATA →
ALARM` within minutes of creation — when the metric had **never** published. A
metric that never existed and a metric that stopped are not the same case.

This is a hypothesis. The task is to establish the behaviour, not to assume it.

## Experiment design — refined 2026-08-25, no idle Lambda needed

The original sketch said "find a Lambda that is already idle". Unnecessary, and
it would have tested the wrong thing: a function that has *never* published is
the `INSUFFICIENT_DATA` case, which we already know breaches promptly. The case
that failed is a metric that **published and then stopped**.

🔑 **`prices-production-coarse-sweep` is already idle for 59 minutes of every
hour.** It runs at `:30` and nothing else invokes it. So the exact failing
condition reproduces once an hour, for free, without touching the schedule.

Point *test* alarms at the same metric and dimension with a **short period** and
time the breach. Two of them, because that separates the hypotheses:

| alarm | Period | 3 datapoints span | interpretation |
|---|---|---|---|
| `tmp-0222-p60` | 60 | 3 min | if lag is **constant**, both breach at about the same clock time |
| `tmp-0222-p300` | 300 | 15 min | if lag is **proportional to period**, p300 breaches ~5× later |

If **neither** breaches within the idle hour, the missing-data path is broken for
stopped metrics generally — a larger finding than a tuning problem, and it would
condemn option 2 below before it is tried.

⚠️ Both are created with **no `--alarm-actions`**, so nobody is paged. Names are
prefixed `tmp-0222-` so they are obviously disposable, and they are deleted at
the end of the run.

⚠️ Start just after a `:30` run — that is the moment the metric stops publishing,
which is what makes the window clean.

⚠️ **Caveat to record with the result:** a lag measured at `Period=60` may not
predict `Period=3600`. The two-alarm design is what makes that inference
possible; a single short-period alarm would not support it.

## RESULT — experiment run 2026-08-25 12:37-13:06 UTC

Two temporary alarms, identical to production except in `Period`, on the same
metric and dimension. No alarm actions, so nobody was paged. Both deleted before
the 13:30 run; `describe-alarms` returned empty afterwards.

The sweep last ran at **12:30**, so the metric stopped publishing then.

| alarm | condition met | breached | lag |
|---|---|---|---|
| `tmp-0222-p60` | at creation 12:37:05 (7 empty 1-min buckets already behind it) | **12:39:47** | **2m42s** from creation |
| `tmp-0222-p300` | 12:50:00 (12:35-40, 12:40-45, 12:45-50 complete and empty) | **12:56:51** | **6m51s** |

`p300` correctly read `OK` until then: at 12:37 its three newest 5-minute buckets
included 12:30-35, which holds the 12:30 run — one non-breaching datapoint of
three, `DatapointsToAlarm: 3` → `OK`. A useful check that both test alarms were
evaluating properly rather than sitting inert.

### Two hypotheses are dead

- ❌ **"Missing-data evaluation is broken for stopped metrics."** It is not. Both
  fired. This was the worst case and it is ruled out.
- ❌ **"Lag is strictly proportional to `Period`."** A 5× period increase produced
  a ~2.5× lag increase — 6m51s, not the ~13 minutes proportionality predicts.

⚠️ **Comparability caveat, recorded rather than glossed:** `p60`'s lag is measured
from *alarm creation* and `p300`'s from *condition-met*. Different baselines, so
the 2.5× ratio is indicative, not clean. Do not quote it as a measured constant.

### 🔴 What the experiment did NOT explain

If the lag is a few minutes regardless of period, the production alarm should
have breached around **12:05** — its three empty hours completed at 12:00. It did
not, and was still `OK` at 12:41 carrying a `StateUpdatedTimestamp` from the
previous day.

**The production failure remains unexplained.**

One story fits every observation: the lag is *sub*-proportional but still large
at `Period=3600` — roughly 1.4 periods ≈ 84 minutes, putting a breach near 13:24
— and re-enabling the rule at 12:20 meant the 12:30 run landed in the 12:00-13:00
bucket, breaking the streak before the lagged evaluation resolved.

⚠️ **That is a hypothesis constructed after the fact to fit the data.** It has not
been tested. Testing it costs another multi-hour window, which is the reason for
the decision below.

## Decision — sidestep the semantics rather than reverse-engineer them

Two attended ~3.5 h windows and one experiment have gone into establishing how
CloudWatch evaluates a metric that stops publishing. Proving the remaining
hypothesis costs another multi-hour wait, and even a proof leaves the alarm
depending on behaviour AWS does not document precisely.

🔑 **Stop depending on missing-data semantics. Make silence produce a datapoint.**

A zero is data. The measurement is what makes this the *clear* choice rather than
the merely convenient one: the mechanism demonstrably works when datapoints
exist, so the defect is specifically that `AWS/Lambda` `Invocations` publishes
**nothing** for an idle function.

## ✅ REMEDY ESTABLISHED — `FILL(m, 0)` works, including on a trailing gap

Tested read-only with `get-metric-data` against this morning's own window, which
has real data at 06/07/08, nothing at 09/10/11, and data again at 12.

**Test 1 — gap bracketed by data on both sides** (`06:00 → 13:00`):

| series | n | values |
|---|---|---|
| `m1` raw | 4 | 06, 07, 08, 12 all `1.0` |
| `FILL(m1, 0)` | **7** | `1, 1, 1, 0, 0, 0, 1` |

**Test 2 — the one that actually decides it** (`06:00 → 12:00`, so the gap is
**trailing** with nothing after it):

| series | n | values |
|---|---|---|
| `m1` raw | 3 | 06, 07, 08 |
| `FILL(m1, 0)` | **6** | `1, 1, 1, 0, 0, 0` |

🔑 **`FILL` extends past the last datapoint.** It does not merely interpolate
between known points.

⚠️ **Why test 2 was necessary and test 1 was not enough.** An alarm always
evaluates a window ending at ~now, so the gap it must detect is always trailing —
a halted schedule has no datapoint on the right-hand side. Had `FILL` only
interpolated, it would have filled exactly the case we do not need and not the
one we do, passing test 1 while failing in production. An alarm that only fires
once the service comes *back* is worse than no alarm, because it looks like
coverage. Same class as [[0218]]'s original defect.

### What this means for the alarm

`FILL(Invocations, 0) < 1` sees three **real zero datapoints** for this morning's
09/10/11 and breaches normally. `treatMissingData` stops being load-bearing —
the whole missing-data path that failed is removed rather than tuned.

The unexplained production behaviour (see RESULT) is therefore no longer on the
critical path. It remains unexplained, and deliberately so: the fix does not
depend on understanding it.

## Applied 2026-08-25 — `addWorkerHealthAlarms`, and what it does NOT cover

One change in the shared helper (`observability-stack.ts`), so it lands on every
scheduled worker at once:

```ts
metric: new cloudwatch.MathExpression({
  expression: 'FILL(invocations, 0)',
  usingMetrics: { invocations: metric('Invocations', 'Sum', cadence) },
  period: cadence,
  label: 'InvocationsFilled',
}),
```

`treatMissingData: BREACHING` is **kept** but demoted to a backstop: `FILL` yields
zeros only where the metric has published at some point, so a function that has
**never** been invoked still produces no series — and BREACHING is right there
too, since a worker that has never run is also a halt.

Synth verified against `Prices-production-Observability`. Five alarms convert:

| alarm | cadence |
|---|---|
| `coarse-sweep-no-invocations` | 3600 |
| `enrichment-no-invocations` | 3600 |
| `backfill-freshness-probe-no-invocations` | 900 |
| `rollup-freshness-probe-no-invocations` | 900 |
| `mtls-notafter-probe-no-invocations` | 86400 |

The rendered form — expression member with no `Period`, period carried on the
`MetricStat` — matches the already-deployed `EnrichmentBacklogAlarm`, so the
shape is proven in production rather than novel.

### `ledger-processor-no-invocations` — converted too, on an explicit judgement

It is hand-rolled rather than going through the helper, so it needed its own
change. Same `MathExpression` wrapper; threshold, periods and `1/1` evaluation
shape all unchanged.

⚠️ **This alarm was never observed failing.** The measured failure was at
`Period=3600`; temporary alarms at 60 s and 300 s both breached correctly, and
this one's **900 s was never tested**. It sits between the two.

Converted anyway, for two reasons:

1. `FILL` makes the question irrelevant rather than betting that 900 s happens to
   be unaffected. The remedy removes the dependency; it does not tune around it.
2. It is the **only** alarm watching a total ingestion halt from the consumer
   side. The lag, errors and DLQ alarms all key on the *presence* of messages, so
   a producer-side stop drains the queue and leaves all three `OK` while live
   candles freeze. Leaving the highest-consequence alarm on a mechanism we have
   proven can fail silently is the worse asymmetry.

The alternative — a 45-minute test at `Period=900` in the sweep's idle window —
was costed and declined: it would establish whether this alarm is currently
broken, but would not change what ships.

Synth after the change: **6 of 6** no-invocations alarms on metric math, **0**
remaining on the legacy single-metric form.

### Not addressed: the `NOT_BREACHING` siblings

`-errors` and `-duration-near-timeout` use `TreatMissingData.NOT_BREACHING`, so
they read **OK on no data** — the failure mode already recorded for
`EnrichmentBacklogAlarm`. `FILL` does not help there; a green reading still means
"nothing was published", not "nothing was wrong". Separate concern, named here so
it is not mistaken for covered.

## Review response — PR #247, 2026-08-25

Four findings raised. Two were settled by measurement rather than argument, one
produced a real change, one was a straightforward correction.

### 🔑 The finding that mattered, and it was RIGHT to raise

> `FILL` can only emit zeros if the retrieved series contains at least one real
> datapoint. Both validations used windows with real data on the left and a
> trailing gap — that is not the shape of the alarm's own window at the moment it
> must breach.

Correct about the gap in the evidence. Both earlier tests included healthy
periods; the alarm's evaluation range during a sustained halt contains **no raw
data whatsoever**. Had `FILL` needed an anchor, this change would have been a
no-op in the only case it exists for, while still passing both tests.

**Settled by measurement.** `get-metric-data` over `09:00-12:00` — exactly the
three empty buckets, not one raw datapoint anywhere in the window:

| series | n | values |
|---|---|---|
| `m1` raw | **0** | `[]` |
| `FILL(m1, 0)` | **3** | `0, 0, 0` |

`FILL` needs no anchor; it synthesises across the whole query window. The finding
falls, and so does its corollary that the `1/1` ledger-processor alarm gains
nothing.

⚠️ This evidence is now in the code comment, not only here — it is the assumption
the entire change rests on and it was very nearly shipped untested.

### The finding that produced a change — `ledger-processor` to 2/2

> `FILL` extends zeros to the query window's end; an alarm's window ends inside
> an incomplete period, and `Invocations` has publication latency. With `1/1`
> that is immediately a full breach — flapping a top-severity page.

**Not reproduced**, but not dismissed. A live query over the last hour returned
`raw` and `filled` byte-identical, with no phantom trailing zero — however the
window ended on a *complete* bucket that *had* data, so the scenario was never
exercised. Absence of evidence.

🔑 **What tipped the decision was realising the exposure is not new.** `1/1` with
`BREACHING` already meant one late datapoint was a breach. What plausibly kept it
quiet is the slow missing-data evaluation this task exists to remove. **Removing
the lag can convert a dormant flap into a live one** — on the alarm that pages
"live ingestion is stalled at the source", with both alarm and OK actions on the
ops topic.

Applied: `evaluationPeriods: 2` / `datapointsToAlarm: 2`, alarm description and
prose updated to match.

- **Cost:** a genuine halt is detected in 30 min rather than 15.
- **Accepted because:** a missed 15 minutes on a halt is recoverable; a flapping
  top-severity page teaches people to ignore the channel — the failure already
  sitting on `prices-production-enrichment-errors` ([[0214]], in ALARM since
  2026-07-27). The `3/3` alarms already had this insulation; the hand-rolled one
  did not.

⚠️ **A judgement under uncertainty, not a proof.** Recorded as one.

### Correction — contradictory comments removed

The old "`treatMissingData: BREACHING` is load-bearing" sentence was left standing
next to the paragraph refuting it, and the same stale claim survived in the
coarse-sweep worker entry. Both rewritten rather than layered — given how much of
this task's cost came from misreading what the mechanism actually was, a
contradiction in the comments is not a cosmetic issue.

Synth after the response: **6 of 6** on metric math, ledger-processor at `2/2`,
the rest unchanged at `3/3`.

## Implementation

- ⚠️ **Evaluate option 2 first — it may make this a one-line CDK change.**
  1. **Metric math with `FILL(metric, 0)`** — substitutes zero for missing
     datapoints inside the alarm rather than at publish time. No new
     infrastructure, no worker change, nothing to deploy but the alarm.
  2. **A heartbeat that publishes the sweep's expected-run count**, so silence
     becomes a *low number* rather than *no data*. More moving parts; a real
     answer if option 1 does not hold.
  3. **Rule-level monitoring** — `Invocations` / `FailedInvocations` on the
     EventBridge rule rather than the function.
- ⚠️ Publishing `CoarseSweepRuns = 0` from the worker is **not sufficient on its
  own**: if the Lambda is never invoked, nothing inside the Lambda can publish.
  That is the failure being detected. The zero has to come from somewhere that
  runs when the sweep does not.
- Re-check the sibling alarms — `-errors` and `-duration-near-timeout` sit on the
  same `AWS/Lambda` metrics with the same publish-nothing-when-idle property, and
  [[0220]]'s duration soak depends on one of them.
- Verify by inducing, on the standard [[0218]] set for itself — and the induction
  must now be cheap enough to repeat, which is part of what the fix buys.

## Deploy + induction record — 2026-08-25

### Deployed

`Prices-production-Observability` **alone**, 17:32 UTC, 11.68 s. Only
`observability-stack.ts` changed, so [[0218]]'s "EventBridge before
Observability" ordering does not apply — every function the alarms name already
exists.

`cdk diff` was exactly the change and nothing else: six alarms swapping
single-metric → metric-math, plus `ledger-processor` `1/1 → 2/2` and its
description. No portal / Discord OAuth / Secrets / Compute / ApiGateway
resources, no replacements, and no hidden-changes footer, so no `--strict`
re-read was needed.

⚠️ The `S3?SNS?SQS` in the diff's **removed** description is the known non-ASCII
mangling of the currently-deployed text being read back. The `[+]` side renders
`S3→SNS→SQS` correctly. Text only, not a change.

`prices-production-cleanup` read `DISABLED` **before and after** the deploy.
`prices-production-coarse-sweep` read `ENABLED cron(30 * * * ? *)` after.

Post-deploy, all six converted and all `OK`:

| alarm | eval | expression | period |
|---|---|---|---|
| `enrichment-no-invocations` | 3/3 | `FILL(invocations, 0)` | 3600 |
| `coarse-sweep-no-invocations` | 3/3 | `FILL(invocations, 0)` | 3600 |
| `backfill-freshness-probe-no-invocations` | 3/3 | `FILL(invocations, 0)` | 900 |
| `rollup-freshness-probe-no-invocations` | 3/3 | `FILL(invocations, 0)` | 900 |
| `mtls-notafter-probe-no-invocations` | 3/3 | `FILL(invocations, 0)` | 86400 |
| `ledger-processor-no-invocations` | **2/2** | `FILL(invocations, 0)` | 900 |

### AC 4 — INDUCED 2026-08-25, and this is the first *alarm* evidence

`tmp-0222-fill-p300` — identical to the deployed coarse-sweep alarm except in
`Period` — created 17:36 UTC just after the `:30` run, **no `--alarm-actions`**,
so nobody was paged. Deleted 17:47, before the 18:30 run; `describe-alarms
--alarm-name-prefix tmp-0222` returned empty afterwards.

```
17:36:10  INSUFFICIENT_DATA   (birth state)
17:37:28  INSUFFICIENT_DATA → OK
17:46:28  OK → ALARM
```

The `OK` at 17:37 is a real evaluation, not inertia: its window still held the
17:30 run, so one of three datapoints was non-breaching against
`DatapointsToAlarm: 3`. Everything before today was `get-metric-data`, which
proves FILL's *behaviour*; this proves a CloudWatch **alarm** built on it
transitions.

### 🔑 Finding 1 — evaluation windows are query-anchored and slide

From `stateReasonData` on the two transitions:

| query | window start | buckets evaluated |
|---|---|---|
| 17:37:28 | 17:22:00 | 17:22 `0.0`, **17:27 `1.0`**, 17:32 `0.0` |
| 17:46:28 | 17:31:00 | 17:31 `0.0`, 17:36 `0.0`, 17:41 `0.0` |

The 17:30 run lands in a bucket **starting 17:27**, and the next evaluation's
window starts at **17:31**. Buckets re-anchor to each query rather than to
`:00/:05/:10`.

⚠️ **This falsifies the reasoning used for the earlier control run** in RESULT
above, which computed condition-met from clock-aligned buckets (`12:35-40,
12:40-45, 12:45-50`). The correct baseline is *when the last run ages out of the
sliding window*. The 6m51s figure is left in place as recorded, but should not be
read as measured against a correct baseline.

### 🔑 Finding 2 — FILL detects silence far faster, on the corrected baseline

| form | last run | window clear | breached | lag |
|---|---|---|---|---|
| raw single-metric | 12:30 | 12:45 | 12:56:51 | **~11m51s** |
| `FILL(invocations, 0)` | 17:30 | 17:45 | 17:46:28 | **~1m28s** |

Same function, same dimension, same `Period=300`, same `3/3`. The only difference
is FILL.

⚠️ **Indicative, not a measured constant.** The earlier run's `stateReasonData`
was never captured, so its window alignment is inferred rather than read. Do not
quote the ratio as an established figure.

### ⚠️ Finding 3 — the review's flap concern has its first supporting evidence

The newest bucket in the breaching evaluation (17:41–17:46) closed **28 seconds**
before the query that read it as `0.0`. Nothing was genuinely invoked, so this is
not a reproduction — but an alarm reading a bucket that fresh is exactly the
window in which a late-publishing `Invocations` datapoint would be counted as a
zero.

That is evidence *for* the `2/2` change on `ledger-processor`, which was made on
judgement alone and explicitly recorded as such. It does not settle it; AC 7 does.

## Acceptance Criteria

- [x] The lag between a function going idle and the alarm breaching is
      **measured**, with the method recorded — not inferred from configuration.
      → 2m42s at `Period=60`, 6m51s at `Period=300`. See RESULT.
- [x] Whether the delay is bounded is established.
      → **Bounded and small at 60 s and 300 s.** NOT established at 3600 s, and
      deliberately not pursued — see Decision.
- [x] The remedy removes the dependency on CloudWatch's missing-data semantics
      rather than tuning around them.
      → `FILL(invocations, 0)` in `addWorkerHealthAlarms`; synth verified, five
      alarms convert (six with the hand-rolled ledger-processor).
      **Deployed to production 2026-08-25 17:32 UTC and induced the same hour** —
      see the deploy record above.
- [x] If unacceptable, a remedy ships and is verified by inducing the silence,
      not by reading the config.
      → Shipped 17:32 UTC, induced 17:36-17:47 UTC. `tmp-0222-fill-p300` went
      **`OK → ALARM` at 17:46:28** on a trailing gap, no schedule touched and no
      actions attached. ⚠️ **Run at `Period=300`, not the `900` written above —
      restated, see below.**

  ⚠️ **AC 4's `Period=900` restated.** As written the proxy alarm was to run at
  `Period=900`, `3/3`. That needs 45 minutes of silence, which puts the third
  empty bucket at exactly `:30` — the moment the next sweep run lands. It races
  the very thing it measures, so 900 is a *less reliable* test rather than a
  stricter one. `Period=300` tests the same unproven proposition — that a
  metric-math alarm built on FILL transitions — inside a 15-minute gap the
  schedule reliably provides. Restated and agreed before the run, not after it.
  Same restate-don't-tick pattern as this task's AC 5 and [[0218]]'s AC 4.
- [x] ~~[[0218]] AC 2 state 3 becomes inducible within a window a person can
      actually sit through~~ — **RESTATED, not ticked as written.** See
      "AC 5 restated" below.
      → Replaced by: the FILL mechanism is proven to breach a real alarm on a
      trailing gap, and 0218's own induction is left as 0218's call.
- [x] The other coarse-sweep alarms are checked for the same failure mode.
      → **Checked, and they have a DIFFERENT one.** `-errors` and
      `-duration-near-timeout` use `TreatMissingData.NOT_BREACHING`, so they read
      **OK on no data** — green means "nothing published", not "nothing wrong".
      Same class as `EnrichmentBacklogAlarm`. FILL does not help; the remedy has
      different semantics (a duration alarm must not fire because nothing ran)
      and is spawned separately rather than folded in here.
- [x] The `2/2` ledger-processor change is watched for ~2 h after deploy.
      Its flap risk was mitigated on a judgement, not a measurement — post-deploy
      is when a flap would show, and finding out from `describe-alarm-history`
      beats finding out from a 3am page.
      → **NO FLAP.** Read 2026-08-26 07:0x UTC, covering the whole 13.5 h
      overnight window since the 17:32 deploy. **Zero `OK → ALARM → OK` cycles**
      — in fact zero state transitions of any kind. See "AC 7 — the overnight
      read" below.

```bash
aws cloudwatch describe-alarm-history \
  --alarm-name prices-production-ledger-processor-no-invocations \
  --history-item-type StateUpdate --max-records 20 --region eu-central-1 \
  --query 'AlarmHistoryItems[].[Timestamp,HistorySummary]' --output text
```

## AC 7 — the overnight read, 2026-08-26 07:0x UTC

The flap did not happen. `describe-alarm-history --history-item-type StateUpdate`
returned **empty**, and the discriminator that empty is a *result* rather than a
typo'd alarm name is `describe-alarms`:

```
prices-production-ledger-processor-no-invocations   OK   2/2   threshold 1.0   breaching
StateUpdatedTimestamp  2026-07-09T08:35:18Z
ConfigurationUpdate    2026-08-25T17:31:40Z    <- the deploy
```

The only retained history item is the config update at **17:31:40**, matching the
deploy record. `StateUpdatedTimestamp` is **2026-07-09** — six weeks before the
change — so the alarm held `OK` straight through the conversion and did not even
produce the settling `INSUFFICIENT_DATA → OK` this AC predicted. Updating an
alarm's metric did not reset its state.

### The `OK` is substantive, not vacuous

A green reading is worth nothing on its own — the standing lesson from
`usd-peg-applied` and from `EnrichmentBacklogAlarm`. So the datapoints were read
directly, `17:00 → 07:00`, `Period=900`:

| series | n | range |
|---|---|---|
| `raw` | **56 of 56** expected buckets | **155 – 161** |
| `FILL(raw, 0)` | **56**, byte-identical | zero filled buckets |

Live ingestion ran continuously at ~1 invocation every 5.6 s. The alarm is
reading real non-breaching datapoints, so `OK` means "healthy", not "blind".

### ⚠️ Finding 4 — a FILL query whose window ends in the *future* fabricates a halt

The first version of the check above ran to an end-time of `12:00Z` while the
clock read `07:02Z`. `FILL` synthesised zeros across the ~5 hours of **future**
window, returning **19 consecutive empty 15-minute buckets on a trailing gap** —
output indistinguishable from a genuine 4h45m ingestion halt, and very nearly
reported as one.

That is the documented "FILL extends past the last datapoint" behaviour working
exactly as specified. The operational rule it implies:

🔑 **Bound a hand-run `get-metric-data` FILL window at `now`.** A future end-time
manufactures the exact signal you are testing for. Always `date -u` first.

The alarms themselves are unaffected — an alarm's window always ends at ~now —
but this sharpens the case for the `2/2` change rather than weakening it: if
*future* buckets fill as zeros, an **incomplete** current bucket does too, which
is the flap mechanism Finding 3 caught at 28 seconds. `1/1` on a FILL alarm would
be a live page waiting to happen.

## AC 5 restated — the criterion asked for something unachievable

As written: *"0218 AC 2 state 3 becomes inducible within a window a person can
actually sit through."*

🔑 **An alarm's time-to-fire and its time-to-test are the same quantity.** If it
breaches after three hours of silence, producing a breach requires three hours of
silence. Rewriting `3 × 1 h` as `12 × 15 min` spans the same three hours — there
is no configuration that decouples them. The only way to shorten the wait is to
make the alarm *fire* sooner, which is a different decision with its own cost.

So this fix does **not** shorten 0218's induction window, and no fix of this
shape could. What it changes is that the alarm should now actually fire.

**Restated as:** the FILL mechanism is verified to breach a real CloudWatch alarm
on a trailing gap (AC 4's proxy induction). Whether to spend a third attended
3.5 h window on 0218's own AC 2 state 3 is 0218's decision, now made with
confidence the alarm will respond rather than hope.

### The alternative, considered and declined

Genuinely shortening detection — two missed runs instead of three — would make
the induction shorter *honestly*, because the alarm would be faster. Declined
here because it is a product question ("how long should a dead coarse sweep go
unnoticed?") smuggled in as a testing convenience, and because it cuts against
the change just made: the existing three-period design "absorbs a deploy window
or a delayed datapoint", and under FILL a late datapoint becomes a filled zero —
the exact flap risk mitigated on ledger-processor by moving the other way.

Worth revisiting on its own merits. Not as a side effect of wanting a shorter
test.

⚠️ Same shape as [[0218]]'s own AC 4, restated rather than ticked when the split
dissolved its premise. Restating a criterion that turns out to demand the
impossible is normal; quietly ticking it would not be.

## Future Work

- **[[0223]] — the `-errors` and `-duration-near-timeout` alarms read OK on no
  data.** Spawned 2026-08-25 from AC 6. Different failure mode from this task's:
  `NOT_BREACHING` renders an unpublished period as healthy. `FILL` does not help
  and the remedy has different semantics, so it is not folded into PR #247.
  ⚠️ [[0220]]'s soak depends on one of these two alarms staying OK.

## Out of scope

- The coarse sweep itself — it works; [[0218]] verified ACs 1, 3 and 4 on prod.
- [[0220]]'s duration soak, except for the shared-failure-mode check above.

## Notes

⚠️ **Two attended ~3.5 h windows have already been spent** on inducing this
(2026-08-24 reverted mid-flight, 2026-08-25 ran to completion and failed). Do not
schedule a third without measuring the lag first.
