---
id: "0250"
title: "Detect a PUT /tags on an API key that no CreateApiKey preceded — the revoke guard's IAM gap, closed by CloudTrail rather than by policy"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0194", "0187", "0191"]
tags: ["priority-low", "effort-small", "security", "iam", "observability", "epic-self-service-onboarding", "milestone-M3"]
milestone: 3
links:
  - "../../../infra/src/lib/stacks/compute-stack.ts"
history:
  - date: "2026-09-01"
    status: backlog
    who: akot
    note: >
      Spawned from [[0194]] future work — its own code review's finding 4
      (2026-08-31), which the task recorded as a follow-up in prose and never
      filed. Filed now so it stops being prose.
---

# A detective control for the one thing the revoke guard cannot promise

## Summary

`PortalDisableOwnApiKeys` in `compute-stack.ts` lets the api-handler disable
a key only when `aws:ResourceTag/ManagedBy = prices-portal`. [[0194]]'s
tag-on-create fix (`PortalTagApiKeysOnCreate`, `apigateway:PUT` on
`/tags/…/apikeys/*` conditioned on `aws:RequestTag`) gave the same role the
ability to **write** that tag — and IAM has no condition that distinguishes
tagging a key as it is created from tagging one that already exists. So the
two statements are not independent: a compromised handler can tag any key in
the account `ManagedBy=prices-portal` and then satisfy the guard against it.
Two calls. Recorded at the statement itself as not fixable in IAM; this task
is the control that makes it *visible* instead.

## Context

- The guard's purpose (per the comment in `compute-stack.ts`) is to stop
  *our own* code disabling a key it did not make — the failure that actually
  happens — not to contain a compromised handler. What bounds the latter:
  limit 1's mitigation, a free-tier plan the role cannot detach keys from, and
  CloudTrail.
- CloudTrail already records the control-plane calls ([[0194]] read 2 054 of
  them over 14 days). The signature is an `apigateway` tagging event (the
  `PUT /tags/<key ARN>` REST call; check the recorded `eventName` — it is
  `TagResource` or `UpdateTags` depending on the API surface, read one off a
  real create) whose caller is the api-handler's role and whose resource is
  a key that **no `CreateApiKey` by the same role produced** in the preceding
  seconds.

## Implementation

- Find how the trail is delivered (S3 only, or CloudWatch Logs). If Logs: a
  metric filter for tagging events from the api-handler role, and a scheduled
  Logs Insights correlation against `CreateApiKey` events; if S3 only: an
  EventBridge rule on `AWS API Call via CloudTrail` for the tagging event
  from that role, into a small Lambda that checks for the paired create.
- Alarm/notify on any unpaired tagging event. Expected rate: zero, ever —
  the portal never re-tags — so the threshold is 1.
- Do NOT widen or tighten the IAM: [[0194]] already established the
  statements are as narrow as IAM allows.

## Acceptance Criteria

- [ ] An induced unpaired tag write (operator, assuming the role in a test
      window, on a throwaway key) is detected and notified within 15 min.
- [ ] A real portal issuance (create + tag, paired) is not.
- [ ] The comment on `PortalTagApiKeysOnCreate` in `compute-stack.ts`
      points at the control.
