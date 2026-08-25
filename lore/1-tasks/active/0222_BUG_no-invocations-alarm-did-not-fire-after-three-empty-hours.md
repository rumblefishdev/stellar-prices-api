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

## Implementation

- **Measure the lag out-of-band.** Point an identical alarm at a Lambda that is
  already idle and time how long it takes to breach. ⚠️ Costs nothing and touches
  no production schedule — unlike a third induction.
- Establish whether the delay is bounded and predictable, or unbounded.
- If the lag is real and long, decide the remedy. Options to cost:
  1. **Alarm on the custom metric instead.** `CoarseSweepRuns` is ours and could
     be published as an explicit `0`, which removes the missing-data path
     entirely. Most likely the right answer.
  2. Shorten `Period` and raise `EvaluationPeriods` for the same total window,
     if evaluation frequency rather than total span is what lags.
  3. A heartbeat/canary that publishes on a schedule independent of the sweep.
- Whatever ships must be **verified by inducing**, on the same standard [[0218]]
  set for itself.
- Re-check the other alarms in the same family — `-errors` and
  `-duration-near-timeout` both sit on `AWS/Lambda` metrics with the same
  publish-nothing-when-idle property, and [[0220]]'s duration soak depends on one
  of them.

## Acceptance Criteria

- [ ] The lag between a function going idle and the alarm breaching is
      **measured**, with the method recorded — not inferred from configuration.
- [ ] Whether the delay is bounded is established.
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
