---
id: "0222"
title: "The coarse-sweep no-invocations alarm did not fire after three genuinely empty hours — the instrument that detects a dead schedule is slower than its config claims"
type: BUG
status: active
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

### 🔴 `ledger-processor-no-invocations` is NOT fixed by this

It is hand-rolled separately rather than going through the helper, and still
renders the **legacy single-metric form** at `EvaluationPeriods 1 /
DatapointsToAlarm 1`, `period=900`. Same defect, and the tightest configuration
of the set — which makes it the one most likely to be silently trusted.

Deliberately left alone in this change: it is a different worker with different
semantics (`1/1`, not `3/3`), and folding it in would widen a one-line fix into a
behaviour change for the ingest path. Flagged rather than fixed.

### Not addressed: the `NOT_BREACHING` siblings

`-errors` and `-duration-near-timeout` use `TreatMissingData.NOT_BREACHING`, so
they read **OK on no data** — the failure mode already recorded for
`EnrichmentBacklogAlarm`. `FILL` does not help there; a green reading still means
"nothing was published", not "nothing was wrong". Separate concern, named here so
it is not mistaken for covered.

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
      alarms convert. **Not yet deployed or induced.**
- [ ] If unacceptable, a remedy ships and is verified by inducing the silence,
      not by reading the config.
- [ ] [[0218]] AC 2 state 3 becomes inducible within a window a person can
      actually sit through, or the AC is formally restated with the reason.
- [ ] The other coarse-sweep alarms are checked for the same failure mode.

## Out of scope

- The coarse sweep itself — it works; [[0218]] verified ACs 1, 3 and 4 on prod.
- [[0220]]'s duration soak, except for the shared-failure-mode check above.

## Notes

⚠️ **Two attended ~3.5 h windows have already been spent** on inducing this
(2026-08-24 reverted mid-flight, 2026-08-25 ran to completion and failed). Do not
schedule a third without measuring the lag first.
