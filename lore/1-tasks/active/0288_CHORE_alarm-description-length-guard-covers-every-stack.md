---
id: "0288"
title: "The 1024-character alarm-description synth guard only walks ObservabilityStack — the ten -errors alarms in EventBridgeStack can exceed the CloudWatch cap and fail mid-deploy"
type: CHORE
status: active
assignee: okarcz
related_adr: []
related_tasks: ["0223", "0243"]
tags: [layer-infra, priority-low, effort-small, observability, cloudwatch, alarms]
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../infra/src/lib/lambda-baseline.ts"
history:
  - date: "2026-11-25"
    status: active
    who: okarcz
    note: >
      Implemented on chore/0288_… (14a3953d): the guard moves to
      alarm-guards.ts and runs over the whole App in createApp. Negative test
      fails synth with the guard's message; templates unchanged. PR to follow.
  - date: "2026-11-25"
    status: active
    who: okarcz
    note: "Activated for implementation (quick-fix queue; Stan OK'd the handover 2026-11-24)."
  - date: 2026-09-15
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0223]] future work. Found while appending a sentence to all
      16 worker alarms: the guard (`assertAlarmDescriptionsFitCloudWatch`,
      observability-stack.ts ~2370) caught the one duration alarm that crossed
      1024, but it walks only ObservabilityStack's construct tree. The nine
      `-errors` alarms built by `createWorkerLambda` live in EventBridgeStack
      and were checked by hand (longest 467). A later edit there fails in
      CloudFormation, mid-deploy, not at synth — the exact failure the guard
      exists to prevent.
  - date: 2026-09-16
    status: backlog
    who: stkrolikiewicz
    note: >
      Noted an adjacent guard/coverage mismatch in the lore framework itself —
      see "Adjacent observation" below. Deliberately parked here as a note
      rather than given an id, and explicitly OUTSIDE this task's acceptance
      criteria: different system, not a CDK stack.
---

# The alarm-description length guard must cover every stack that builds alarms

## Summary

`assertAlarmDescriptionsFitCloudWatch` runs at synth over ObservabilityStack
only. EventBridgeStack builds ten `-errors` alarms through `createWorkerLambda`
and none of them is checked. [[0223]] measured them by hand (longest 467 of 1024)
and [[0243]] established that the cap is enforced only by CloudFormation, i.e.
after the deploy has started.

## Implementation

- Move the guard into a shared helper (`lambda-baseline.ts` or a small
  `alarm-guards.ts`) and call it from **both** stacks, or run it once over the
  whole `App` from `app.ts`.
- Keep the failure message naming the alarm and its length.
- Prove it with the same negative test [[0223]] used for its assertion: push one
  EventBridge description over the cap locally and confirm synth — not deploy —
  refuses it. Read the error text; "synth failed" is not "failed for this reason".

## Acceptance Criteria

- [x] Every `AWS::CloudWatch::Alarm` in every production stack is length-checked
      at synth.
      ✅ 2026-11-25: the guard runs once over the whole `App` in `createApp`
      (`infra/src/lib/app.ts`), before `app.synth()`. Production synth: 90 alarms
      — EventBridge 10 (longest 629), Observability 80 (longest 1018); no other
      stack builds one. A stack added later is covered without a new call.
- [x] A deliberately over-long `-errors` description fails synth with the
      guard's own message, recorded in this task.
      ✅ Prefixed the asset-discovery worker's description with
      `'NEGATIVE-TEST-0288 '.repeat(60)` (local only, reverted), built, synthed:

      ```
      Error: 1 CloudWatch alarm description(s) exceed the 1024-character API limit and would fail mid-deploy:
        prices-production-asset-discovery-errors — 1491 chars
      Shorten the description(s); the limit is enforced by CloudWatch, not by synth.
      node dist/bin/production.js: Subprocess exited with error 1
      ```

      Control: the same edit on `develop`'s code synthesizes successfully, with
      the 1491-char description in the EventBridge template — the gap was real.

## Implementation Notes

Branch `chore/0288_alarm-description-length-guard-covers-every-stack`, commit `14a3953d`.

- New `infra/src/lib/alarm-guards.ts`: `assertAlarmDescriptionsFitCloudWatch`,
  moved verbatim from `observability-stack.ts` (doc comment updated, same error
  message), now exported.
- `observability-stack.ts`: the private copy and its call removed; a comment
  points to `app.ts`.
- `app.ts`: `assertAlarmDescriptionsFitCloudWatch(app)` before `app.synth()`.
- All five production templates are byte-identical before and after. `nx test`
  for the CDK project and `npm run infra:verify-dashboard` pass.

## Design Decisions

### Emerged

1. **One call over the whole App, not one per stack.** The task offered both.
   Per-stack calls must be remembered in every new stack — the same gap again.
   The App-level walk covers every stack by construction. `cicd-app.ts` builds
   no alarms, so it does not call it.
2. **No per-stack fallback in ObservabilityStack.** Production synth always goes
   through `createApp`; keeping a second call would only duplicate the walk.

## Issues Encountered

- During the control run a stale `infra/dist/lib/alarm-guards.js` was deleted
  with `rm -f` instead of `mv … .trash/` (repo rule). It was a generated build
  output, recreated by the next build; noted for the record.

## Adjacent observation — the lore validator has the same shape of defect

⚠️ **Not in scope for this task. Not an acceptance criterion.** Parked here
because it is the same failure pattern — a guard whose schema does not match the
artifacts it is supposed to check — and because it is small enough that minting
an id for it would cost more than the fix.

Found 2026-09-16 while validating the [[0226]] / [[0241]] / [[0256]] edits:
`lore-framework_validate` reports

```
history.0.date: Expected string, received date
```

on **every** history entry of every task it checks, including entries written
months ago and untouched since. Measured across `lore/1-tasks/`:

| form | task files |
|---|---|
| `- date: 2026-09-16` (unquoted → YAML parses it as a date) | **342** |
| `- date: "2026-09-16"` (quoted → string) | 39 |

And `lore/1-tasks/_template.md:11` specifies the **unquoted** form, so the
template the repo tells you to copy produces files the validator rejects.

🔑 The consequence is that the validator cannot currently be used as a gate:
it fails on ~90% of the corpus, so a real error would be indistinguishable from
the background noise. Whoever picks this up has to decide which side is wrong —
accepting a YAML date in the schema is one edit; requantifying 342 files and the
template is the other. Nobody owns this today.
