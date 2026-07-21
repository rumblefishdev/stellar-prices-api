---
id: "0112"
title: "Progress alarms are blind to a worker that dies — enrichment failed 4 days with no alert"
type: BUG
status: backlog
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

- [ ] An alarm fires within ~1 h of the enrichment Lambda failing repeatedly,
      without depending on any metric the worker publishes.
- [ ] Verified by a real fire-test (the 0056 precedent: breach + recovery), not
      only by synth — an alarm nobody has seen fire is an untested alarm, which
      is exactly what this incident was.
- [ ] Duration-approaching-timeout warns *before* the timeout, so [[0111]]-class
      regressions surface as a warning rather than an outage.
- [ ] The other `NOT_BREACHING` alarms are audited and the finding recorded,
      even where no change is made.
- [ ] Routed to the same Slack channel as the 0056 alarms.

## Out of scope

- Fixing the timeout itself — that is [[0111]]. This task ensures the next one
  is noticed in an hour rather than a week.
