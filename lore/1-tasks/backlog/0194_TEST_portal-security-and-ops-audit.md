---
id: "0194"
title: "Portal security and ops audit — caching, IAM, throttles, logs, against Tranche 3 AC 6"
type: TEST
status: backlog
related_adr: ["0007", "0010"]
related_tasks: ["0183", "0159", "0160", "0184", "0186", "0187", "0188", "0189", "0191", "0192", "0164"]
tags: [layer-infra, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, security, audit, iam, slice-11]
milestone: 3
links:
  - "../archive/0160_FEATURE_onboarding-backend-endpoints.md"
  - "../archive/0159_FEATURE_discord-oauth-sign-in.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Eleventh slice. Not a bucket for deferred hardening — each slice ships its
      own security requirement, including the two that are not deferrable
      (`no-store` on key reveal, throttles outside the `cacheEnabled` branch).
      This task exists because those requirements are spread across seven
      slices and one of them is a wholesale-assigned array: the audit is of the
      **assembled** result, which no individual slice can see.
---

# Portal security and ops audit

## Summary

**Story:** *as the person submitting Tranche 3, I can show that the portal's
final assembled configuration is correct — not that each slice intended it to
be.*

Everything here is already required by an earlier slice. What is new is checking
the composition, because three of these are properties of the whole array or the
whole policy and are invisible from inside any one task.

## Why an audit rather than a checklist item in each slice

Three failure modes are structural, not per-slice:

- **`methodSettings` is keyed by `resourcePath + httpMethod` and assigned
  wholesale** in `api-gateway-stack.ts`. Seven slices add routes to it. The array
  the stack actually synthesises is the only thing worth checking.
- **The stack builds the full array only inside `if (cacheEnabled)`**, and its
  `else` emits just `[stageWideThrottle, apiDocsSettings]`. Entries added to the
  `if` arm alone vanish wherever `apiGatewayCacheEnabled` is false — leaving
  anonymous, keyless sign-in routes unthrottled in exactly the configuration
  where every request is a billed Lambda invocation. The existing code comments
  this trap.
- **Two caching layers, and CloudFront is the outer one.** Its default cache
  policy strips cookies, so with the managed default the session never reaches
  the origin and every request reads as signed-out — while an un-`no-store`d
  key-reveal response would be served from the CDN to the next caller. Neither
  layer is checkable from the other.

## Checks

Verify against the **synthesized CloudFormation template and the deployed
stack**, not against the source, and not by assumption:

- [ ] Every portal method has `cachingEnabled: false`, and every portal response
      carries `Cache-Control: no-store`
- [ ] The portal prefix's CloudFront behaviour disables caching **and** forwards
      the session cookie; a signed-in request reaches the origin signed in
- [ ] The full `methodSettings` array contains every portal route in **both**
      arms of the `cacheEnabled` branch — flip `apiGatewayCacheEnabled` off in a
      synth and diff
- [ ] Anonymous sign-in routes carry their own method-level throttle and are not
      behind `apiKeyRequired`
- [ ] `/api-tokens/api/*` precedes `/api-tokens/*` in the deployed distribution's
      behaviour order
- [ ] The assembled IAM policy names specific resources — no wildcard on
      `apigateway:*` — and the un-narrowable `POST /apikeys` is documented in the
      code as an accepted limit with its mitigation (tagging + attachment to the
      self-service plan only)
- [ ] The collection-level `GET /apikeys` is present; without it the reconciler
      fails at runtime with `AccessDenied` and only under concurrency
- [ ] No API key value appears in any CloudWatch log group or X-Ray trace —
      grepped, including error paths
- [ ] The Discord client secret is in Secrets Manager and in no environment
      variable, and no secret is in the static bundle
- [ ] Both SSM parameters are operator-seeded; a `cdk deploy` does not restore a
      committed guild id
- [ ] The portal bucket has no public access and is reachable only through OAC
- [ ] Control-plane call volume per dashboard load is known and bounded.
      **Corrected 2026-08-20 by [[0190]]'s measurement — it is four calls, not
      two:** `GetApiKeys` + `GetApiKey` on the reveal (`keys::lookup`) and
      `GetApiKeys` + `GetUsage` on the usage route (`usage::fetch`), the two
      listings being the same query for the same user in the same load. Measured
      against the real account: ~1.14 s of control-plane time per cold load, on
      an account budget of 10 rps / burst 40 shared with `cdk deploy` (observed
      14-day peak 12/s, 42/min). Only the usage half is cached, so a warm load
      still costs two. Cost it here against real traffic, and note [[0190]]'s two
      cheaper remedies before any storage is considered: de-duplicate the shared
      listing, and give the reveal the cache the usage route already has

## Opening the portal

**This task owns the flip.** `PORTAL_ENABLED` goes to `'true'` in `compute-stack.ts` here and nowhere
else — not as a side effect of anyone finishing their own slice. Preconditions,
all of them:

- [ ] [[0189]] has passed: a non-member is refused, and a Discord `429`/`5xx`
      does not read as "not a member"
- [ ] Every check in the list above passes
- [ ] Keys created while the flag was off are enumerated and deleted. There is no
      separate incubation plan (decided 2026-08-13), so those keys are real keys
      on the real free-tier plan and would otherwise survive into its accounting
      as anonymous strings. They come from local runs against production
      credentials, not from the closed portal — the flag lives in the Lambda

Note what the flip is **not** gated on: [[0193]] (looks presentable) and
[[0195]] (custom domain) can both land after it. Opening a plain-looking portal
that works is a smaller risk than leaving a finished one closed.

## Acceptance Criteria

- [ ] Every check above passes against the deployed production stack, with the
      evidence captured in a form [[0164]] can cite
- [ ] `PORTAL_ENABLED` is flipped to `'true'` here, with every precondition
      above met and recorded — and the flip is reversible by the same one-word
      diff plus a deploy
- [ ] Any failure is fixed in the slice that owns it, and the fix is re-verified
      here rather than patched locally
- [ ] Tranche 3 AC 6 ("no secrets in env vars", least-privilege IAM) is
      answerable from this task's output alone

## Notes

- Deliberately a `TEST`, not a `FEATURE`. If this task ends up writing code, the
  slice that should have written it is the one to change.
- Runs after [[0191]] (0192 merged into it) and before [[0164]]. [[0164]] verifies the user-visible
  flow; this verifies the configuration underneath it.
