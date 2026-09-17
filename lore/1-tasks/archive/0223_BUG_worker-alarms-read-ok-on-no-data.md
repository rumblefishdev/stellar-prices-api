---
id: "0223"
title: "The -errors and -duration-near-timeout worker alarms read OK on no data — a green light that means nothing was published, not that nothing was wrong"
type: BUG
status: completed
related_adr: []
related_tasks: ["0222", "0218", "0214", "0220", "0204", "0226", "0112", "0256", "0284", "0200", "0084", "0288"]
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
  - date: 2026-09-15
    status: backlog
    who: stkrolikiewicz
    note: >
      ⚠️ Two couplings with [[0214]]'s stuck-alarm digest, deployed today, that
      change how the options below must be costed. (1) The digest does NOT cover
      this defect and must not be treated as covering it: it re-reads alarms that
      are OFF OK, while this task is about alarms that read OK while blind — the
      two are orthogonal. (2) ⛔ Option 2, composite alarms, would be INVISIBLE to
      the digest: `alarm_digest::describe` filters `AlarmType::MetricAlarm` and
      its docs justify that with "this stack defines no composite alarms". Ship
      composites without changing that filter and the new single green light is
      exactly the kind of alarm nothing re-surfaces. Also note the daily probe,
      already flagged below as the awkward case, now carries the digest as a
      second job, so its 1/1 -errors alarm became more load-bearing than when
      this task was written.
  - date: 2026-09-15
    status: active
    who: stkrolikiewicz
    note: >
      Activated, the other half of Oskar's item 10, straight after [[0214]].
      Framing decided before any code, from a production count rather than the
      code: 16 alarms, not 15 (6 duration — `oracle` has one too — plus 10
      -errors). ⚠️ The binary question this task asks has a split answer. Seven
      -errors alarms and all six duration alarms are conditional-by-design AND
      honest, because each of those workers has a `-no-invocations` alarm with
      `treatMissingData: breaching` answering "did it run at all". Two do not:
      asset-discovery and supply have no liveness alarm, and the code comment
      exempting them (`observability-stack.ts:1515`) justifies that with "their
      -errors alarm is the coverage today" — i.e. with the very alarm this task
      exists because it is blind to a dead worker. Circular. Decided with
      stkrolikiewicz: supply gets health alarms (scope widened, see below);
      asset-discovery is deferred to [[0256]] on purpose, because that task says
      its scan is currently a no-op and may be removed, and alarming the liveness
      of dead code is not coverage. `treatMissingData` changes nowhere; composite
      alarms rejected — they would be invisible to 0214's digest.
  - date: 2026-09-15
    status: completed
    who: stkrolikiewicz
    note: >
      All seven criteria closed the same day. PRs #317 + #318, deployed 14:21 and
      14:22 UTC; 16 alarm descriptions now say what their OK means, supply has a
      liveness alarm (born 14:22:31Z, OK at 14:24:18Z on real history, routed to
      Slack — screenshot recorded), induced with a Period=300 action-less clone
      that went OK 14:25:34Z → ALARM 14:33:34Z on the worker's natural inter-run
      silence and was deleted at 14:34:13Z; zero SNS publishes in its window.
      ⚠️ The finding to keep: #317 as merged would have given supply a duration
      alarm that latched forever, because its run length is a 240 s budget by
      design (0084) against a 300 s timeout — caught by reading the metric
      before the first deploy, fixed in #318 before anything reached production.
      asset-discovery's liveness is parked in [[0256]] explicitly. Spawned
      [[0288]] for the synth guard that does not cover EventBridge's alarms.
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
     ⛔ If this ships, `alarm_digest::describe` ([[0214]]) must stop filtering
     `AlarmType::MetricAlarm`, or the new composites are invisible to the daily
     re-read and can latch unnoticed — the defect 0214 was built to end.
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

- [x] The framing is decided and written down: conditional-by-design, or a defect
      to fix. **Both, per worker** — see "Framing — decided 2026-09-15".
- [x] A green reading on these alarms is unambiguous to an operator who did not
      write them — from the description or the dashboard, not from the code.
      All 16 descriptions say what OK means and name the liveness sibling or the
      reason there is none; for the seven conditional workers the strip already
      shows that sibling next to the green tile. Rendered in Slack — screenshot.
- [x] The daily-cadence probe (`mtls-notafter`, 86400 s) is checked explicitly
      under whichever option ships; it is the one most likely to break. No
      evaluation change shipped, so nothing to break — "AC 3" below.
- [x] [[0220]]'s soak evidence is re-read against the outcome and its AC states
      whether OK meant "observed healthy" or "no data". Observed healthy —
      `Invocations` 1/hour all week; note appended to 0220.
- [x] All three builders are covered, or the ones deliberately left alone are
      named with a reason. A fix that lands in one helper and silently misses the
      other two repeats [[0222]]'s hand-rolled-alarm trap. Covered: 9 via
      `createWorkerLambda`, 1 hand-rolled, 6 via `addWorkerHealthAlarms` —
      verified on production, 16/16 carry the sentence.
- [x] The `1/1` error alarms are costed separately from the `2/2` duration
      alarms; they are not the same change. Neither evaluation shape changed;
      the duration family got the short sentence for the 1024 cap.
- [x] Verified by inducing, on [[0218]]'s standard — not by reading the config.
      The new supply liveness alarm, induced 2026-09-15 — record below.

## Out of scope

- The no-invocations alarms — [[0222]], PR #247.
- [[0214]]'s latched enrichment-errors alarm, except for the cross-check above.

## Notes

Discovered while answering [[0222]]'s AC 6 rather than by an incident. Recorded
because "we checked and they have a different problem" is a finding, not a
non-finding.

## Framing — decided 2026-09-15, from a production count

Enumerated with `describe-alarms` against production rather than from the code,
because the code's own list was already stale once (2026-08-26) and the task
tells the next person to count before changing anything.

### The count: 16, not 15

| family | alarms | eval | `treatMissingData` |
|---|---|---|---|
| `-errors` | **10** — 9 via `createWorkerLambda` + `ledger-processor-errors` hand-rolled | 1/1 | `notBreaching` |
| `-duration-near-timeout` | **6** — `addWorkerHealthAlarms` | 2/2 | `notBreaching` |

⚠️ The table above in "Where these alarms actually live" says **5** duration
alarms. It is 6: `oracle` is in `workerHealth` too. The correction of 2026-08-26
fixed the `-errors` half and left the duration count wrong.

### The measurement that settles the framing

Whether a green `-errors` is *honest* depends on one thing: does something else
answer "did the worker run at all?". Cross-referenced against the
`-no-invocations` family ([[0222]], `treatMissingData: breaching`):

| worker | `-errors` | duration | liveness (`-no-invocations`) |
|---|---|---|---|
| oracle, enrichment, coarse-sweep | ✓ | ✓ | ✓ |
| backfill-freshness-probe, rollup-freshness-probe, mtls-notafter-probe | ✓ | ✓ | ✓ |
| ledger-processor | ✓ | — | ✓ |
| **asset-discovery** | ✓ | — | **none** |
| **supply** | ✓ | — | **none** |
| cleanup | ✓ | — | none — rule `DISABLED` since [[0200]] |

So the binary question this task asks has a **split** answer:

- **Seven `-errors` alarms and all six duration alarms are conditional by design
  and honest.** Liveness is answered by a sibling alarm on the same dashboard
  strip, which goes red when the worker dies while `-errors` stays green. The
  only defect is that nothing *says* so.
- **Two are a real hole.** `asset-discovery` and `supply` have no liveness alarm.
  🔑 And the comment exempting them, `observability-stack.ts:1515`, reads:
  *"their `-errors` alarm (createWorkerLambda) is the coverage today."* That is
  **circular**: the exemption from a liveness alarm is justified by the alarm
  this task exists because it is blind to a dead worker. Neither has data-level
  cover either — `asset_supply` freshness is still backlog [[0284]], and [[0243]]
  found `assets` unsuitable for a freshness alarm.
- **One is moot.** `cleanup`'s rule is disabled on purpose; OK on no data is
  correct and only needs saying.

Both uncovered workers were checked to be alive before deciding anything, so a
new liveness alarm would not be born latched: `prices-production-asset-discovery`
and `prices-production-asset-supply` both `ENABLED`, `rate(1 hour)`, **24
invocations every day 2026-09-08 → 09-14**.

### Decisions

1. **`treatMissingData` changes nowhere.** AC 6 ("1/1 and 2/2 costed
   separately") is answered: neither evaluation shape changes. `NOT_BREACHING`
   is the right answer for a conditional alarm, and the fix for the two
   unconditional cases is not to make `-errors` do liveness's job.
2. **Composite alarms (option 2) rejected.** They would be invisible to
   [[0214]]'s digest, which filters `AlarmType::MetricAlarm` — a new single green
   light beyond the daily re-read is the defect 0214 was built to end.
3. **`supply` gets health alarms — scope widened, deliberately.** This task
   listed the no-invocations family as out of scope (that referred to [[0222]]'s
   fix of the *existing* ones). Adding one for a worker that has none is the
   direct consequence of the framing, and `addWorkerHealthAlarms` builds the pair,
   so supply gets duration too (harmless on a bounded 5-min body). Agreed with
   stkrolikiewicz 2026-09-15.
4. **`asset-discovery` is deferred to [[0256]], on purpose.** That task says the
   worker's scan is currently a no-op ("re-seeds hourly and scans nothing") and
   may be removed. Alarming the liveness of dead code is not coverage. The
   exemption comment already says "revisit in 0256"; this makes the liveness
   question an explicit item there rather than something 0256 can close around.
5. **Documenting has the same three-builder trap as fixing.** The appended
   sentence must land in all three places — `createWorkerLambda` (9),
   the hand-rolled `ledger-processor-errors` (1), `addWorkerHealthAlarms` (6) —
   or this repeats [[0222]]'s miss in documentation form. The two exempt workers
   get a *different* sentence at their call sites (cleanup: rule disabled;
   asset-discovery: no liveness alarm, see 0256), so the helper must know which
   workers are exempt: `workersWithoutHealthAlarms` moves next to
   `SCHEDULED_WORKERS` in `lambda-baseline.ts`, which also removes a
   cross-stack duplicate.
6. **Induction, AC 7:** for the new supply alarm, exactly [[0222]]'s method — a
   temporary action-less clone at `Period=300` created right after the hourly
   run, which transitions to ALARM within ~10 min of natural silence and is
   deleted before the next run. No rule is disabled, nobody is paged. The
   documentation half has nothing to induce; the underlying property (AWS/Lambda
   publishes nothing on zero invocations) is 0222's measured finding.
7. **AC 4 ([[0220]]):** answered from its own AC — `Invocations` stayed at 1/hour
   for the whole soak, so every period had a datapoint and OK meant *observed
   healthy*, not *no data*. Note appended there.

## Deployed 2026-09-15 — and a defect caught in pre-flight

PRs [#317](https://github.com/rumblefishdev/stellar-prices-api/pull/317)
(merge `aead679`) and [#318](https://github.com/rumblefishdev/stellar-prices-api/pull/318)
(merge `fe50cf9`), deployed together.

| stack | time (UTC) | what |
|---|---|---|
| EventBridge | 14:21:51 | 9 `-errors` descriptions, nothing else — no IAM, no code, no env |
| Observability | 14:22:26 | 7 descriptions, **1** new alarm, dashboard strip 52 → 53 |

Verified on production after the deploy: all 16 `-errors` / `-duration-near-timeout`
alarms carry the `task 0223` sentence (longest 945 / 1024);
`prices-production-supply-no-invocations` exists — `3/3`, `breaching`, one ALARM
and one OK action — born `INSUFFICIENT_DATA` at 14:22:31Z; **no**
`supply-duration-near-timeout` exists; 53 alarms match the prefix.

### Emerged — the second alarm for supply was a self-inflicted latch

#317 as merged gave supply the standard health **pair**. Reading supply's
`Duration.Maximum` to fix the induction window showed **~240.5 s on every run**,
against a 300 s timeout: exactly the 80 % threshold the duration alarm uses.
Cause is in the source, not the metric — `DEFAULT_TIME_BUDGET_SECS = 240`
(`supply-worker/src/lib.rs:27`, [[0084]]): the Horizon walk stops at 240 s by
design. Deployed as merged, `supply-duration-near-timeout` would have latched on
its second evaluation and been re-surfaced by [[0214]]'s digest every day — the
failure this task exists to remove, produced by this task.

Fixed in #318 **before** anything reached production: `addWorkerHealthAlarms`
gains `noDurationAlarm?: string`, set to the reason, which skips the duration
alarm for a worker whose run length is a budget rather than a symptom. Same
pattern as `WORKERS_WITHOUT_HEALTH_ALARMS` — the exception is data with a
reason, not a comment.

⚠️ Two lessons worth more than the fix. First: **"green on the diff" is not
"safe to deploy"** — the diff was exactly as intended both times; what it could
not show is what the metric would do against the threshold. Reading the metric
before the first deploy is the check that caught it. Second, on my own method:
the first negative test of the new synth assertion (post-approval commit on
#317) was **inconclusive** — synth failed, but on the pre-existing assertion,
not the new one. "It failed" is not "it failed for my reason"; the error text has
to be read. Reordered so the specific check runs first, and re-proved.

### AC 3, the daily probe, explicitly

`mtls-notafter-probe` (86400 s) is unchanged in behaviour: its `-errors` and
duration alarms received a description each and nothing else. Its liveness
alarm already exists from [[0222]]. Under this task's framing there was nothing
to change for it — the awkward case is awkward only for options that alter
evaluation, and none shipped.

## Induced 2026-09-15 — the supply liveness alarm, with nothing stopped

[[0222]]'s method, exactly: a clone of the deployed
`prices-production-supply-no-invocations` — same `FILL(invocations, 0)`, `< 1`,
`3/3`, `breaching` — differing only in `Period` (300 instead of 3600), **no
actions**, created 9 minutes after the hourly run at 14:17Z. The worker's
natural 55-minute silence between runs, seen in 5-minute buckets, is the
induction; no rule was disabled and no data stopped.

```
14:22:31Z  real alarm born            INSUFFICIENT_DATA
14:24:18Z  real alarm → OK            3/3 hourly buckets held the 11:17/12:17/13:17 runs
                                      (query-anchored windows, 0222 finding 1) — Slack OK, screenshot
14:25:14Z  clone created              INSUFFICIENT_DATA
14:25:34Z  clone → OK                 1 of 3: the 14:17Z run in the bucket starting 14:15
14:33:34Z  clone → ALARM              3 of 3 zero: 14:18, 14:23, 14:28
14:34:13Z  clone deleted              `--alarm-name-prefix tmp-0223` empty
```

SNS `NumberOfMessagesPublished` on the ops topic over 14:25–14:35Z: none. The
real alarm never left OK. Nine minutes of one extra alarm object was the whole
footprint.

⚠️ For the next induction: the real alarm went OK **two minutes** after birth,
not at the next run — CloudWatch evaluates existing metric history immediately.
Plan the clone right after a run, not around the next one.

## Future Work

- [[0288]] — the 1024-character description guard walks only
  ObservabilityStack; EventBridgeStack's ten `-errors` alarms are unchecked.
- asset-discovery's liveness — parked in [[0256]] with the decision written
  there, not here.
