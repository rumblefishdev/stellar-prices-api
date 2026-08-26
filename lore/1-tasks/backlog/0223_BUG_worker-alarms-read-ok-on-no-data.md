---
id: "0223"
title: "The -errors and -duration-near-timeout worker alarms read OK on no data — a green light that means nothing was published, not that nothing was wrong"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0222", "0218", "0214", "0220", "0204", "0226", "0112"]
tags: [layer-infra, priority-medium, effort-small, observability, cloudwatch, alarms, ops]
milestone: 2
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../infra/src/lib/lambda-baseline.ts"
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
  - date: 2026-08-26
    status: backlog
    who: okarcz
    note: >
      🔴 SCOPE CORRECTED before any work started. The task claimed both alarms
      come from `addWorkerHealthAlarms`; only `-duration-near-timeout` does.
      `-errors` is built by `createWorkerLambda` at `lambda-baseline.ts:309` for
      all 9 workers, and `ledger-processor-errors` is hand-rolled a third time at
      `observability-stack.ts:732`. addWorkerHealthAlarms builds no -errors alarm
      at all — its own header comment says so.
      As originally scoped this task would have fixed the duration half and left
      every -errors alarm carrying the defect it exists to remove. Blast radius
      of that half goes from 5 workers to 10 alarms across three builders, and
      the -errors family is 1/1 rather than 2/2, so the two halves need costing
      separately.
      Found while investigating an unrelated oracle OOM page ([[0226]]) — the
      oracle's -errors alarm turned out not to be where this task said it was.
---

# The worker `-errors` and `-duration-near-timeout` alarms read OK on no data

## Summary

Two alarm families use `TreatMissingData.NOT_BREACHING`. [[0222]] fixed the third
family (`-no-invocations`); these two were deferred here.

| alarm | metric | eval | treatMissingData |
|---|---|---|---|
| `-duration-near-timeout` | `AWS/Lambda` `Duration`, Maximum | 2/2 | `NOT_BREACHING` |
| `-errors` | `AWS/Lambda` `Errors`, Sum | **1/1** | `NOT_BREACHING` |

🔴 **CORRECTED 2026-08-26 — they are NOT built in the same place.** This task
originally said both come from `addWorkerHealthAlarms`. Only one does. See
"Where these alarms actually live" below; the correction widens the blast radius
of the `-errors` half from 5 workers to **10 alarms across three builders**.

`AWS/Lambda` publishes **nothing** for a period with no invocations — the
property [[0222]] measured. So when a worker stops running, both alarms have no
datapoints, and `NOT_BREACHING` renders that as **`OK`**.

A green reading therefore means *"nothing was published"*, not *"nothing was
wrong"*. The dashboard looks healthy precisely when the worker is dead.

## Where these alarms actually live — corrected 2026-08-26

Read from the code, not assumed. **Three** builders, not one:

| alarm | built by | count | eval |
|---|---|---|---|
| `-duration-near-timeout` | `addWorkerHealthAlarms` — `observability-stack.ts:130` | 5 | 2/2 |
| `-errors` | `createWorkerLambda` — **`lambda-baseline.ts:309`** | **9** | **1/1** |
| `ledger-processor-errors` | hand-rolled — `observability-stack.ts:732` | 1 | **1/1** |

`addWorkerHealthAlarms` covers 5 workers: `enrichment`, `coarse-sweep`,
`backfill-freshness-probe`, `rollup-freshness-probe`, `mtls-notafter-probe`.

`createWorkerLambda` covers **all 9**: those five plus `asset-discovery`,
`cleanup`, `supply`, `oracle`.

🔑 **`addWorkerHealthAlarms` builds no `-errors` alarm at all.** Its own header
comment says so — *"Deliberately does NOT include an invocation-errors alarm:
`createWorkerLambda` already creates `prices-{env}-{name}-errors` for every
worker"* (`observability-stack.ts:37`). The original scope bullet would therefore
have fixed the duration half and left **every** `-errors` alarm untouched — the
precise defect this task exists to remove.

⚠️ **Same trap [[0222]] hit.** There, `ledger-processor-no-invocations` was
hand-rolled outside the helper and needed its own change. Here it recurs twice
over: a second helper *and* a hand-rolled `ledger-processor-errors`. Two of the
three paths are easy to miss because the first one looks complete.

⚠️ **The `-errors` family is `1/1`, the duration family is `2/2`.** Any option
that changes evaluation semantics must be costed separately for each — and `1/1`
is the flap-prone shape [[0222]] deliberately moved *away* from on
`ledger-processor-no-invocations`.

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
- ⚠️ Apply across **all three builders**, not just `addWorkerHealthAlarms` — see
  the table above. Enumerate before changing anything: 5 duration alarms
  (`observability-stack.ts:130`), 9 error alarms (`lambda-baseline.ts:309`), and
  the hand-rolled `ledger-processor-errors` (`observability-stack.ts:732`).
  Cadences run from 900 s to 86400 s and the daily probe is the awkward case for
  any option.
- ⚠️ `createWorkerLambda` also wires the **OK action** on every `-errors` alarm
  ([[0112]]), so any sensitivity change alters notification volume in both
  directions, not just when firing.

## Acceptance Criteria

- [ ] The framing is decided and written down: conditional-by-design, or a defect
      to fix.
- [ ] A green reading on these alarms is unambiguous to an operator who did not
      write them — from the description or the dashboard, not from the code.
- [ ] The daily-cadence probe (`mtls-notafter`, 86400 s) is checked explicitly
      under whichever option ships; it is the one most likely to break.
- [ ] [[0220]]'s soak evidence is re-read against the outcome and its AC states
      whether OK meant "observed healthy" or "no data".
- [ ] All three builders are covered, or the ones deliberately left alone are
      named with a reason. A fix that lands in one helper and silently misses the
      other two repeats [[0222]]'s hand-rolled-alarm trap.
- [ ] The `1/1` error alarms are costed separately from the `2/2` duration
      alarms; they are not the same change.
- [ ] Verified by inducing, on [[0218]]'s standard — not by reading the config.

## Out of scope

- The no-invocations alarms — [[0222]], PR #247.
- [[0214]]'s latched enrichment-errors alarm, except for the cross-check above.

## Notes

Discovered while answering [[0222]]'s AC 6 rather than by an incident. Recorded
because "we checked and they have a different problem" is a finding, not a
non-finding.
