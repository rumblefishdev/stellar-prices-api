---
id: "0188"
title: "Usage against quota on the dashboard — GetUsage, rendered honestly"
type: FEATURE
status: active
related_adr: ["0010"]
related_tasks: ["0183", "0157", "0160", "0187", "0193", "0194"]
tags: [layer-backend, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, api-gateway, dashboard, slice-5]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0160_FEATURE_onboarding-backend-endpoints.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Fifth slice, third of [[0160]]'s four operations. Separate from [[0187]]
      because it is a different AWS call with its own freshness problem, its own
      period arithmetic and its own caching answer — and because the key is
      useful without it.
  - date: "2026-08-19"
    status: active
    who: akot
    note: >
      Activated on top of [[0187]] (#224, merged to `develop`), which this slice
      extends rather than sits beside: the usage figure is scoped to
      `(usagePlanId, apiKeyId)`, and the key id comes from [[0187]]'s reconciler.

      Three things settled there shape this one. **The gateway needs no change**
      — `{proxy+}` already covers `/usage`, `GET` is already a mapped verb, and
      `portalSettings` already sets `cachingEnabled: false` on it, with CI
      asserting all three. **The `GetUsage` grant belongs in ApiGatewayStack**,
      not ComputeStack: it needs the plan id, which only that stack knows, so it
      joins the standalone `iam.Policy` [[0187]] created for exactly that reason.
      And **the key lookup here must not create** — [[0187]] decision 14 keeps
      issuance behind an explicit press, so a dashboard that ran the full
      reconciliation on load would mint a production key for anyone who opened
      the page.

      Carried in from [[0187]]'s review, to be decided here rather than
      rediscovered: validating the usage plan at cold start (one `GetUsagePlan`,
      one grant in the same policy this slice opens) would turn a stale plan id
      from a runtime failure into an init failure.
---

# Usage against quota

## Summary

**Story:** *as a developer with a key, I can see how many requests I have used
this period, how many remain, and when it resets — so a `429` is something I can
diagnose myself instead of emailing you.*

Epic AC 4, and the one screen that turns a key into a dashboard.

## Context

`GetUsage` is a server-side AWS SDK call requiring IAM credentials — the epic
already records that it "cannot be called from the browser directly", which is
the original reason a backend exists at all. Quota is scoped to
`(usagePlanId, apiKeyId)`, so the numbers come straight from AWS with no
accounting of our own.

## Implementation

- `GET /api-tokens/api/usage` → `GetUsage` over the current quota period for the
  caller's key, returned as used / remaining / limit / period start / period end.
- **IAM:** `apigateway:GET` on `/usageplans/{freePlanId}/usage`. One statement
  added to [[0187]]'s policy.
- **`GetUsage` lags, and is not a read-after-write surface** — measured
  2026-08-12 (archived `0180/notes/R-apigw-namequery-quota-and-disable.md`). A
  dashboard that reads usage straight after a test request shows a stale figure.
  **Decide the wording once, here**, and render a "last updated" line; [[0193]]
  restyles it but does not re-decide it. A dashboard that looks broken is worse
  than one that admits a lag.
- **Short in-process cache for `GetUsage`, plus backoff on throttling.** Gateway
  caching is forbidden on portal methods for the security reason in [[0187]], and
  these are control-plane calls sharing an account-wide budget with our deploys.
  A dashboard load already costs `GetApiKey` + `GetUsage`; do not let a refresh
  loop compete with CI.
- **The period boundary rendered here is ours, not AWS's.** AWS documents
  neither the reset instant nor its timezone — the only statement anywhere is an
  example caption, *"creates a usage plan that resets at the beginning of the
  month"*, and `offset` is a **request count**, not a time shift (ADR 0010,
  correction #2, still open). Render "1st of the month, 00:00 UTC" as our stated
  rule. If the measurement in [[0191]] shows AWS's counter rolls at a different
  instant, that is a UX wrinkle to word around, not a correctness bug.
- **Show the limits as numbers, not prose:** 1 req/s ([[0157]]), and
  used-of-quota with the reset date.
- `cachingEnabled: false` and `Cache-Control: no-store`, same as [[0187]].
- Frontend: three numbers and a date, unstyled. [[0193]] makes it a dashboard.

## Acceptance Criteria

- [ ] **Ships closed.** With `PORTAL_ENABLED=false` ([[0183]]) this slice's
      routes return an empty `404`; with it on, they behave normally — every
      deploy goes straight to production
- [ ] The endpoint returns used, remaining and limit for the current period from
      `GetUsage`, plus the period boundaries
- [ ] Making N requests with the key moves the number (allowing for the lag)
- [ ] The page states when the figure was last refreshed, in wording agreed here
- [ ] Repeated dashboard loads do not produce one `GetUsage` call each
- [ ] Throttling from the control plane backs off rather than erroring the page
- [ ] The 1 req/s rate limit and the reset date are both visible
- [ ] Response is not cached at either layer

## Notes

- Worth knowing while nearby: **`UpdateUsage`** (`op:replace` on `/remaining`)
  moves the quota counter directly without touching the key. That is the right
  tool for "this user needs more quota this month" — one control-plane call, no
  new secret to re-integrate — and it is a different operation from the rework
  in [[0191]], which the dashboard should not conflate.
