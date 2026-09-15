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
