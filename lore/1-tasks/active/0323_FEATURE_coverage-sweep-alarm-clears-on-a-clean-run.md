---
id: "0323"
title: "coverage-sweep-unclassified stays in ALARM for 7 days after the residual is triaged — latch it until a clean run instead"
type: FEATURE
status: active
assignee: stkrolikiewicz
related_adr: []
related_tasks: ["0100", "0214", "0294", "0296"]
tags: [layer-infra, observability, coverage, priority-high, effort-small]
links:
  - "../../../packages/coverage-sweep-probe/src/lib.rs"
  - "../../../packages/coverage-sweep-probe/src/main.rs"
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../docs/runbooks/0100-coverage-sweep-triage.md"
history:
  - date: "2026-09-30"
    status: active
    who: stkrolikiewicz
    note: >
      Created after the 2026-09-28 sda finding: allow-listed and deployed the
      same morning, yet the alarm cannot leave ALARM before 2026-10-05, and
      nothing an operator does clears it sooner.
---

# coverage-sweep-unclassified stays in ALARM for 7 days after triage

## Summary

Make `prices-<env>-coverage-sweep-unclassified` reflect the **latest** sweep:
ALARM from a run that finds a residual until a run that finds none. After
triage, one manual invoke clears it. Today a single finding holds it for a week
whatever anyone does.

## Context

[[0100]] designed the alarm as a 7-day hold: the probe publishes
`UnclassifiedSwapEvents` only when non-zero, and the alarm is `Sum >= 1`,
1-day period, 1 of 7, `NOT_BREACHING` (`observability-stack.ts:1404`, runbook
§2). The hold is the longest CloudWatch allows (7 × 86,400 s).

What that did on 2026-09-28:

- 05:17 UTC run: one unclassified emitter, `CDYPJTUT…` (a newer `sda`
  router build, 1 event). ALARM at 08:14 CEST.
- #358 allow-listed it (merged 11:02 CEST). The EventBridge deploy at 10:55:57
  CEST carried it: the deployed `bootstrap` embeds develop's
  `allowlist.toml` byte-for-byte (checked 2026-09-30).
- The alarm still cannot leave ALARM before ~2026-10-05 06:00 UTC, when the
  2026-09-27 06:00 bucket leaves the window. `set-alarm-state OK` reverts on
  the next evaluation, and a re-run publishes nothing, so the old datapoint
  stays in the window.

For M3 that is a week of "not all alarms OK" (AC 8, [[0294]]) after every
triaged finding. The alarm also reads OK 7 days after a probe that stopped
running.

## Design

1. **Publish every run.** `unclassified_metrics` returns both
   `UnclassifiedSwapContracts` and `UnclassifiedSwapEvents` on every
   successful run, `0` when clean. `UnresolvedSwapEmitters` is unchanged
   (no alarm on it).
2. **Alarm on the latest run.** `Maximum >= 1`, 300 s period, 1 of 1,
   `treatMissingData: IGNORE`. A run's datapoint sets the state, and between
   runs the state holds.

Consequences, all written into the runbook:

- Clearing after triage: deploy the allow-list, then invoke the probe once.
  The invoke must land in a later 5-minute bucket than the breaching run,
  because `Maximum` over one bucket holding both stays `>= 1`.
- A failed run publishes nothing, so the state holds (ALARM stays ALARM).
  The `-errors` alarm is still the backstop for "did not run".
- The §4.5 synthetic proof needs a clean invoke afterwards to reset.
- An alarm update keeps the current state (`PutMetricAlarm`), so rollout is:
  EventBridge, then Observability, then one invoke. No IAM change.
- Where the rule never ran, the alarm stays `INSUFFICIENT_DATA`, not OK.
  Only production enables it (`infra/envs/production.json:16`).

Out of scope: the `-errors` alarm has the same 1-of-7 shape.

## Acceptance Criteria

- [ ] A clean run yields `0` for both unclassified metrics (unit test)
- [ ] Synth: the alarm is `Maximum`, 300 s, 1 of 1, `ignore`
- [ ] Code comments, alarm description and runbook §2, §3 and §4.5 describe
      the latch-until-clean-run behaviour; no text still promises the 7-day
      return to OK
- [ ] Production after deploy + one invoke: `coverage sweep complete` with
      `unclassified=0`, a `0` datapoint in `Prices/Coverage`, alarm OK
- [ ] Reviewed by the [[0100]] owner (akot)
