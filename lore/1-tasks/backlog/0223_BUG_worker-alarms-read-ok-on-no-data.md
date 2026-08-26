---
id: "0223"
title: "The -errors and -duration-near-timeout worker alarms read OK on no data — a green light that means nothing was published, not that nothing was wrong"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0222", "0218", "0214", "0220", "0204"]
tags: [layer-infra, priority-medium, effort-small, observability, cloudwatch, alarms, ops]
milestone: 2
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
history:
  - date: 2026-08-25
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0222]]'s AC 6, which asked whether the sibling alarms shared
      the no-invocations failure mode. They do not — they have a different one.
      Both use `TreatMissingData.NOT_BREACHING`, so a period with no datapoint
      reads OK. `FILL` does not help, and the remedy has different semantics from
      0222's, which is why it is a separate task rather than folded into PR #247.
---

# The worker `-errors` and `-duration-near-timeout` alarms read OK on no data

## Summary

`addWorkerHealthAlarms` builds three alarms per scheduled worker. [[0222]] fixed
the third. The other two use `TreatMissingData.NOT_BREACHING`:

| alarm | metric | treatMissingData |
|---|---|---|
| `-duration-near-timeout` | `AWS/Lambda` `Duration`, Maximum | `NOT_BREACHING` |
| `-errors` | `AWS/Lambda` `Errors` | `NOT_BREACHING` |

`AWS/Lambda` publishes **nothing** for a period with no invocations — the
property [[0222]] measured. So when a worker stops running, both alarms have no
datapoints, and `NOT_BREACHING` renders that as **`OK`**.

A green reading therefore means *"nothing was published"*, not *"nothing was
wrong"*. The dashboard looks healthy precisely when the worker is dead.

## Context

This is the same shape already recorded for `EnrichmentBacklogAlarm` reading OK
on no data, and it is why [[0204]] found "10 of 13 alarms blind" — seven had
settled to OK on no data.

⚠️ **`NOT_BREACHING` is not simply wrong here.** A duration alarm must not fire
because nothing ran — that would page on every idle period of a daily probe. The
remedy is therefore *not* flipping the flag, which is why this is separate from
0222 rather than part of it.

The honest framing: these two alarms are **conditional** — they answer "when the
worker ran, did it error / run long?" — and the "did it run at all?" question
belongs to the no-invocations alarm. That division is defensible. What is not
defensible is that nothing in the naming, description or dashboard says so, so a
green `-errors` reads as an all-clear.

⚠️ **[[0220]]'s week-long soak depends on `-duration-near-timeout` staying OK.**
If OK can mean "no data", the soak's evidence is weaker than it looks and its
AC should say which it observed.

## Implementation

- Decide the framing first: are these alarms **conditional by design** (documented
  as such), or should they distinguish "no data" from "healthy"?
- Options to cost:
  1. **Document and leave.** Amend the alarm descriptions to state that OK means
     "no failing datapoints observed" and that liveness is the no-invocations
     alarm's job. Cheapest; changes no behaviour.
  2. **Composite alarm** — `-errors` OK **and** `-no-invocations` OK — so a
     single green light means both "ran" and "ran cleanly".
  3. **`MISSING`** instead of `NOT_BREACHING`, so an idle period holds the prior
     state rather than asserting health. Subtler than it looks; check against a
     daily-cadence probe before adopting.
- Whatever ships, re-check [[0214]] against it — `prices-production-enrichment-errors`
  has been in ALARM since 2026-07-27, so that alarm's behaviour on the error path
  is already suspect for other reasons.
- ⚠️ Apply to every worker built by `addWorkerHealthAlarms`, not just the coarse
  sweep — cadences run from 900 s to 86400 s and the daily probe is the awkward
  case for any option.

## Acceptance Criteria

- [ ] The framing is decided and written down: conditional-by-design, or a defect
      to fix.
- [ ] A green reading on these alarms is unambiguous to an operator who did not
      write them — from the description or the dashboard, not from the code.
- [ ] The daily-cadence probe (`mtls-notafter`, 86400 s) is checked explicitly
      under whichever option ships; it is the one most likely to break.
- [ ] [[0220]]'s soak evidence is re-read against the outcome and its AC states
      whether OK meant "observed healthy" or "no data".
- [ ] Verified by inducing, on [[0218]]'s standard — not by reading the config.

## Out of scope

- The no-invocations alarms — [[0222]], PR #247.
- [[0214]]'s latched enrichment-errors alarm, except for the cross-check above.

## Notes

Discovered while answering [[0222]]'s AC 6 rather than by an incident. Recorded
because "we checked and they have a different problem" is a finding, not a
non-finding.
