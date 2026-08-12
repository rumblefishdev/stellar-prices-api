---
id: "0160"
title: "Onboarding backend — issue key, reveal it, report usage, rework once a quota period"
type: FEATURE
status: backlog
related_adr: ["0007", "0008", "0010"]
related_tasks: ["0156", "0157", "0158", "0159", "0162", "0180"]
tags: [layer-backend, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, iam, dashboard]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../packages/prices-api"
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
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Meeting outcome: handlers go into the existing `prices-api` axum router,
      rework ships in Tranche 3, and the period boundary is settled. Opens #2,
      #3 and #4 are closed; #1 narrows to revocation only. Estimate drops from
      large to medium.
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      [[0156]] / ADR 0010 confirms the one-key model the rework cap depends on,
      so the endpoint shapes stand. Three corrections: `nameQuery` matching is
      undocumented, the quota-period boundary is our rule rather than inherited
      AWS behaviour, and `enabled=false`'s effect on usage counters is unknown —
      which blocks costing revocation. All measured by [[0180]].
---

# Onboarding backend — issue, reveal, usage, rework

## Summary

Everything the portal does with AWS on behalf of a signed-in Discord user. Four
operations, one Lambda, one IAM policy:

| Operation | Epic reference | AWS call |
| --- | --- | --- |
| Issue a key on first sign-in, no approval | "Agreed scope", key issued automatically | `CreateApiKey` + `CreateUsagePlanKey` |
| Show the key, now and later | "Key delivery" | `GetApiKey(includeValue=true)` |
| Usage against quota | "Usage dashboard" | `GetUsage` |
| Rework, once per quota period | "Rotation/revocation" | `DeleteApiKey` + issue again |

Epic references are by heading, not line number: the in-repo copy is
prettier-formatted and its line numbers drift from any other copy of the same
document.

None of these can happen in the browser: they need IAM credentials. **The
handlers live in the existing `prices-api` axum router** (2026-08-07 meeting) —
same Lambda, same crate, same build as the data routes, so `ErrorEnvelope` and
the `utoipa` spec generation come for free.

## Context

The epic settles two things that would otherwise be design work here. Key
delivery is **not** a one-time reveal — AWS stores key values retrievably, so
the dashboard can show the key whenever the user asks, and we never keep a copy
ourselves. And rework is **capped at once per quota period**, because a fresh
key gets a clean quota counter: `GetUsage` and quota are scoped to
`(usagePlanId, apiKeyId)`, so without the cap a user could burn the monthly
allowance and mint their way out of it. Capping rework to the quota's own period
makes AWS's native accounting sufficient and avoids building cumulative
cross-key aggregation.

**The boundary is settled (2026-08-07): a rework is allowed only when
`coalesce(last_rotated_at, created_at)` falls before the current quota period
start** — the 1st of the month, 00:00 UTC. Worked example from the meeting:
reworked on 3 August → next rework available 1 September. One date for the
dashboard to render.

**Correction (2026-08-10, [[0156]]): "the same instant the AWS quota period
rolls over" is not a sourced claim.** AWS documents neither the reset instant
nor its timezone; the only statement anywhere is an example caption, *"creates a
usage plan that resets at the beginning of the month"*. `offset` is a **request
count**, not a time shift. The rule stands as **our** product decision — keep
it — but do not justify it as inherited AWS semantics, and do not assume the two
boundaries coincide until [[0180]] has measured the rollover. If they turn out
to differ, the dashboard renders our date and the quota counter does its own
thing; that is a UX wrinkle, not a correctness bug, because the cap is ours to
define.

The `created_at` fallback is load-bearing, not defensive. Issuance never writes
`last_rotated_at`, so gating on that column alone leaves it null for every fresh
key — a user could take a key on 1 August, spend the whole 100 000, and rework on
2 August into a clean counter, since quota is scoped to `(usagePlanId,
apiKeyId)`. Any key acquired inside the current period has
`created_at >= periodStart`, so with the fallback it can never be reworked inside
that period, whether it came from issuance or from an earlier rework. One key per
period, one quota.

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
- **IAM scoped to the resources it needs**: `apigateway:POST` on `/apikeys`
  and on `/usageplans/{pricingApiFreePlanId}/keys`, `GET` on **`/apikeys`** (the
  collection — the reconciler lists it), on `/apikeys/{id}` and on
  `/usageplans/{pricingApiFreePlanId}/usage`, `DELETE` on `/apikeys/{id}`. Nothing
  wildcard — Tranche 3 AC 6 audits exactly this.
  The collection-level `GET` is not optional: without it every issue path and
  the reveal-path recovery below fail at runtime with `AccessDenied`, because
  both begin by listing keys by name.
- **Write down the limit of that scoping.** `POST /apikeys` cannot be narrowed:
  there is no resource ARN for "only keys this function created", and `DELETE`
  narrows no further than `/apikeys/*`. The policy will be as tight as the
  service allows and no tighter. Mitigation is tagging every created key and
  attaching it only to the self-service plan. Record this as a consciously
  accepted limit — an auditor who finds it unexplained will read it as an
  oversight.
- **These handlers now sit in the partner-facing Lambda.** Keep the portal
  routes in their own module and their own path prefix so the blast radius of a
  change is legible, and so the IAM additions above are obviously attributable
  to them rather than to the data routes.
- **Name the created key after the Discord user id** (and tag it). If the
  registry table is ever lost, the keys remain attributable; without it they are
  anonymous strings in an AWS account.
- **The rework refusal needs a real response**, not a generic error: `409` plus
  `next_eligible_at` in the `ErrorEnvelope` `details` field, so [[0162]] can
  render the date. See "Settled 2026-08-07" #3.
- **`GetUsage` lags.** It is not a real-time counter; the dashboard should say
  so rather than have a user conclude the numbers are wrong. Decide the wording
  once, here, and let [[0162]] render it.
- **Read the plan id from SSM** ([[0157]]), not from a hardcoded string or a
  cross-stack reference — `ComputeStack` is a dependency of `ApiGatewayStack`
  and cannot import from it.
- **Never log a key value**, including in error paths and X-Ray annotations.
- **Two of these four operations require a fresh eligibility proof; two do not
  (ADR 0010 §8).** Corrected 2026-08-10 — an earlier version of this bullet said
  none of them re-checks eligibility, which was wrong: **issue is one of the four**,
  so that rule left the gate unenforced on the only path that mints a key.

  | Operation | Eligibility proof | Why |
  | --- | --- | --- |
  | Issue | **required** | membership must hold *at the moment of issuance* |
  | Reveal | none | the key is already theirs; works forever |
  | Usage | none | same |
  | Rework | **required** | membership only; the age check is not repeated |

  The proof comes from [[0159]]'s re-authentication callback, which holds a fresh
  Discord token. **This task performs no Discord calls itself** — it consumes the
  verdict from the callback that invoked it. Reveal and usage stay pure AWS, so a
  Discord outage never breaks the dashboard for someone who already has a key.
- **Issue and rework must be safe under double-submit, and the store no longer
  helps.** ClickHouse has no conditional insert ([[0158]] "Accepted
  consequences"), so the guard is deterministic key naming plus the reconciler:
  look up `discord-<userId>-key` in API Gateway before creating — **exact-match
  the name in the client** — and converge on the earliest-created key if two
  appear. Rework records the new key id and `last_rotated_at` in one insert, and
  deletes the old key only afterwards.
  **Corrected 2026-08-10 ([[0156]]): `nameQuery` is not documented as a prefix
  match.** AWS's whole description of it is *"The name of queried API keys."* —
  no matching semantics at all. The client-side exact match is therefore the
  only guard that does not rest on undocumented behaviour; comment it as such so
  it is not later removed as redundant. [[0180]] measures the real behaviour.

## Settled 2026-08-07

The four items parked on 2026-08-06 resolve as follows.

1. **Rework ships in Tranche 3.** It is an atomic swap: the old key is deleted
   and a new one issued in the same operation, so the user is never without a
   working key. The cap blocks the *next* rework, not the replacement.
   Confirmation UX is [[0162]]'s: a modal stating that the old key stops working
   immediately, with the confirm button disabled until the user types
   `delete-key`.
2. **Period boundary** — `coalesce(last_rotated_at, created_at)` must fall before
   the current quota period start (1st of the month, 00:00 UTC). See Context for
   why the fallback is not optional.
3. **Refused rework returns `409`**, not `429`: `429` implies "retry shortly"
   when the wait can be weeks. Body is the existing `ErrorEnvelope`
   (`packages/prices-api/src/common/errors.rs`) with a new canonical code and
   `next_eligible_at` in `details` — the envelope already has the slot, so
   nothing new is invented.
4. **Registry pointing at a key that no longer exists in AWS** — do not re-issue
   blindly. Look the key up by its exact name (`discord-<userId>-key`) first,
   adopt it if present, create only if genuinely absent.

   **Hang this off the reveal path, not the issue path.** A hand-deleted key
   presents as a registry row with a populated `api_key_id`, so [[0158]]'s issue
   flow short-circuits at step 1 and its reconciler never runs — the user would
   keep receiving a dead key id indefinitely. The trigger is `GetApiKey`
   returning 404 on reveal: re-enter the issue flow from its lookup step, then
   write the adopted or newly created id back to the registry.

## Open

**Revocation.** Rework is capped, so a developer whose key leaks on the 3rd
cannot invalidate it until the 1st. `UpdateApiKey(enabled=false)` invalidates a
key in one call and is cheap to add. Deferred at the 2026-08-07 meeting and
recorded in the epic as a known gap — revisit if it bites in practice.

**Correction (2026-08-10, [[0156]]): "without touching the quota counter" is an
assumption, not a documented fact.** AWS says nothing about what disabling a key
does to accumulated usage. Note the delete-then-create reasoning does **not**
transfer here — that argument works only because `CreateApiKey` mints a *new*
`id`, and `enabled=false` keeps the `id` and `value` in place. If disabling
turned out to reset the counter, revocation would become a free quota reset and
would have to share the rework cap after all. [[0180]] #8 measures it; do not
ship revocation before it does.

Worth knowing while this is open: **`UpdateUsage`** (`PATCH
/usageplans/{id}/keys/{keyId}/usage`, `op:replace` on `/remaining`) moves the
quota counter directly without touching the key at all. That is the sharper tool
for "this user needs more quota this month" — one control-plane call, no new
secret for the user to re-integrate — and it is a different operation from
rework, which the dashboard currently conflates.

## Acceptance Criteria

- [ ] First sign-in issues a key attached to the self-service plan and writes
      the registry record; a second sign-in returns the same key, not a new one
- [ ] Key value retrievable repeatedly by its owner and by nobody else
- [ ] Usage endpoint returns requests used and remaining for the current quota
      period, sourced from `GetUsage`
- [ ] Rework issues a new key and deletes the old one in one operation; a second
      attempt in the same quota period is refused with `409` and
      `next_eligible_at`
- [ ] Issue and rework are unreachable without a fresh eligibility proof — a
      valid session alone does not suffice, verified by calling both endpoints
      directly with a session cookie and nothing else
- [ ] A user who has left the guild can still reveal their key and read usage,
      but is refused on rework
- [ ] Reworking on 3 August refuses until 1 September, and succeeds on 1
      September — the meeting's worked example, tested
- [ ] Reconciler adopts an existing `discord-<userId>-key` instead of creating a
      duplicate, and converges two concurrent first sign-ins onto one key
- [ ] No portal response is cached at **either** layer, verified against the
      synthesized template rather than assumed: `cachingEnabled: false` on every
      portal method at the gateway, **and** a CloudFront behaviour for the portal
      prefix that disables caching and forwards the session cookie ([[0161]]).
      CloudFront is now the outer layer and its default cache policy strips
      cookies — with the managed default the session never reaches the origin
      and every request reads as signed-out, while an un-`no-store`d key-reveal
      response would be served from the CDN to the next caller
- [ ] IAM policy names specific resources; no wildcard on `apigateway:*`, and
      the un-narrowable `POST /apikeys` is documented as an accepted limit
- [ ] Key values never appear in logs or traces
- [ ] Epic AC 2 and AC 4 satisfied end to end

## Notes

- The old key is **deleted** on rework, per the epic's reasoning. Anything using
  it stops working immediately — the modal in [[0162]] must say so before the
  user confirms, which is what the `delete-key` phrase is there to enforce.
- A user who reworks on the last day of a quota period gets a fresh counter and
  a period reset a day later. Not an exploit (the reset was coming anyway), but
  write it down so it is not re-raised as one.
- **These endpoints run on API Gateway's control plane**, which is throttled far
  more aggressively than the data plane and limited per *account* — the same
  budget our own CDK deploys draw on. A dashboard load already costs
  `GetApiKey` + `GetUsage`. Gateway-side caching is forbidden here for security
  reasons, so the mitigation is a short in-process cache for `GetUsage` plus
  backoff on throttling. Nobody has costed this yet; do it before the portal
  sees real traffic.
- [[0162]] consumes all four endpoints; keep the response shapes stable once it
  starts.
