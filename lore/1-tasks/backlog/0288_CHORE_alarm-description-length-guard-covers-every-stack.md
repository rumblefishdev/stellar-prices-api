---
id: "0288"
title: "The 1024-character alarm-description synth guard only walks ObservabilityStack — the ten -errors alarms in EventBridgeStack can exceed the CloudWatch cap and fail mid-deploy"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0223", "0243"]
tags: [layer-infra, priority-low, effort-small, observability, cloudwatch, alarms]
links:
  - "../../../infra/src/lib/stacks/observability-stack.ts"
  - "../../../infra/src/lib/lambda-baseline.ts"
history:
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

- [ ] Every `AWS::CloudWatch::Alarm` in every production stack is length-checked
      at synth.
- [ ] A deliberately over-long `-errors` description fails synth with the
      guard's own message, recorded in this task.

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
