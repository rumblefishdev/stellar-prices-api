---
id: "0160"
title: "Onboarding backend — issue key, reveal it, report usage, rotate once a month"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0156", "0157", "0158", "0159", "0162"]
tags: [layer-backend, priority-high, effort-large, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, iam, dashboard]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      The four operations the portal performs on a signed-in user's behalf,
      kept in one task because they share a Lambda, an IAM policy and the
      registry record. Depends on [[0157]] (the plan to issue into),
      [[0158]] (the registry) and [[0159]] (who is calling). The rotation
      scope question is parked for the team — see "Open".
---

# Onboarding backend — issue, reveal, usage, rotate

## Summary

Everything the portal does with AWS on behalf of a signed-in Discord user. Four
operations, one Lambda, one IAM policy:

| Operation | Epic reference | AWS call |
| --- | --- | --- |
| Issue a key on first sign-in, no approval | line 22 | `CreateApiKey` + `CreateUsagePlanKey` |
| Show the key, now and later | lines 46–52 | `GetApiKey(includeValue=true)` |
| Usage against quota | lines 53–59 | `GetUsage` |
| Rotate, once per calendar month | lines 63–74 | `DeleteApiKey` + issue again |

None of these can happen in the browser: they need IAM credentials, which is
the reason the epic introduces a backend Lambda that the component list did not
previously contain.

## Context

The epic settles two things that would otherwise be design work here. Key
delivery is **not** a one-time reveal — AWS stores key values retrievably, so
the dashboard can show the key whenever the user asks, and we never keep a copy
ourselves. And rotation is **capped at once per calendar month**, because a
fresh key gets a clean quota counter: `GetUsage` and quota are scoped to
`(usagePlanId, apiKeyId)`, so without the cap a user could burn the monthly
allowance and mint their way out of it. Capping rotation to the quota's own
period makes AWS's native accounting sufficient and avoids building cumulative
cross-key aggregation.

## Implementation

**From the epic**

- Key issued automatically the first time a user completes sign-in.
- Key value returned on demand, not once.
- Usage read from `GetUsage` over the current quota period, rendered against the
  plan's quota.
- "Generate new key" available, refused if one was already issued this quota
  period.

**Follows from the epic, but not stated in it**

- **Never cache these responses — and in this stack that takes an explicit
  setting.** `deployOptions.cachingEnabled` is on, which is why
  `api-gateway-stack.ts` has to switch caching *off* per method for
  `/v1/prices/batch` and `/health`. The gateway cache has no cache-key
  parameters, so all callers collapse onto one entry: a cached key-reveal
  response would hand one user another user's key. Set `cachingEnabled: false`
  on every portal method and `Cache-Control: no-store` on every response.
- **IAM scoped to the two resources it needs**: `apigateway:POST` on `/apikeys`
  and on `/usageplans/{selfServicePlanId}/keys`, `GET` on `/apikeys/{id}` and on
  `/usageplans/{selfServicePlanId}/usage`, `DELETE` on `/apikeys/{id}`. Nothing
  wildcard — Tranche 3 AC 6 audits exactly this.
- **Name the created key after the Discord user id** (and tag it). If the
  registry table is ever lost, the keys remain attributable; without it they are
  anonymous strings in an AWS account.
- **The rotation refusal needs a real response**, not a generic error: which
  date the next rotation becomes available, so the portal can render it. Decide
  the status code and keep it consistent with the API's `ErrorEnvelope` shape.
- **`GetUsage` lags.** It is not a real-time counter; the dashboard should say
  so rather than have a user conclude the numbers are wrong. Decide the wording
  once, here, and let [[0162]] render it.
- **Read the plan id from SSM** ([[0157]]), not from a hardcoded string or a
  cross-stack reference — `ComputeStack` is a dependency of `ApiGatewayStack`
  and cannot import from it.
- **Never log a key value**, including in error paths and X-Ray annotations.
- Issue and rotate must be safe under double-submit: the conditional write from
  [[0158]] is the guard, and rotation updates `lastRotatedAt` in the same
  operation that records the new key id.

## Open — to settle before/while building

Parked deliberately (Adam, 2026-08-06) rather than assumed:

1. **Rotation vs revocation, and whether rotation is Tranche 3 scope at all.**
   The epic's heading says "Rotation/revocation" but only rotation is described,
   and rotation does not appear in the epic's acceptance criteria (lines
   126–135). The gap that matters: a developer whose key leaks mid-month is
   blocked by the once-a-month cap. **To settle with the team.**
2. **Where "once a month" is measured from** — calendar month, 30 days since
   `lastRotatedAt`, or the AWS quota period boundary. Recommendation: the quota
   period boundary, since that is the counter the rule exists to protect.
3. **Status code for a refused rotation** — `409` with the next eligible date
   reads better than `429`, which implies "retry shortly" when the wait is
   weeks.
4. **Registry pointing at a key that no longer exists in AWS** (deleted by hand
   in the console). Re-issue silently and log, or fail to support?
   Recommendation: re-issue, so our own operation does not leave a user with a
   dead portal.

## Acceptance Criteria

- [ ] First sign-in issues a key attached to the self-service plan and writes
      the registry record; a second sign-in returns the same key, not a new one
- [ ] Key value retrievable repeatedly by its owner and by nobody else
- [ ] Usage endpoint returns requests used and remaining for the current quota
      period, sourced from `GetUsage`
- [ ] Rotation issues a new key and deletes the old one; a second attempt in the
      same quota period is refused with the next eligible date *(subject to
      Open #1)*
- [ ] No portal response is cached at the gateway; verified against the
      synthesized template, not assumed
- [ ] IAM policy names specific resources; no wildcard on `apigateway:*`
- [ ] Key values never appear in logs or traces
- [ ] Epic AC 2 and AC 4 satisfied end to end

## Notes

- The old key is **deleted** on rotation, per the epic's reasoning. Anything
  using it stops working immediately — the portal must say so before the user
  confirms.
- A user who rotates on the last day of a quota period gets a fresh counter and
  a period reset a day later. Not an exploit (the reset was coming anyway), but
  write it down so it is not re-raised as one.
- [[0162]] consumes all four endpoints; keep the response shapes stable once it
  starts.
