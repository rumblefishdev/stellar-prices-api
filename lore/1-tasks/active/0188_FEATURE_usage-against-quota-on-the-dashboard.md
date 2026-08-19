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
  - date: "2026-08-19"
    status: active
    who: akot
    note: >
      Implemented on branch `feat/0188_...`. One new Rust module
      (`portal/usage/`), one new `Gateway` call (`usage_of`, paginated), a
      `Throttled` error variant, one IAM statement in [[0187]]'s standalone
      policy, one CI assertion pinning it, and the unstyled usage section on
      the page. 24 new Rust tests (18 over HTTP against the mock control
      plane — which now serves `GetUsage`, wire-field `values` — and 6 unit),
      9 new frontend tests. Workspace: 497 Rust tests, 48 portal tests, 0
      failures; synth + all three OpenAPI verifications green. Six of eight
      acceptance criteria closed; "N requests move the number" and the
      deployed-stack halves wait on a deploy, like [[0187]]'s live curl.

      The carried-in question is answered: cold-start plan validation is
      **declined**, with the reasoning written at the grant in
      `api-gateway-stack.ts` — [[0187]] decision 22 already rejected it (a
      warm container still misses a plan that changes under it; the attach
      path disambiguates a dead plan id into `PlanNotFound` loudly), and
      `GetUsage` against a wrong plan id fails visibly on the first
      dashboard load.
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

- [x] **Ships closed.** With `PORTAL_ENABLED=false` ([[0183]]) this slice's
      routes return an empty `404`; with it on, they behave normally — every
      deploy goes straight to production. Asserted with a valid session
      presented, byte-identical to an unrouted path under the same prefix, and
      with zero control-plane calls made on the way
      (`a_closed_portal_answers_usage_with_an_empty_404`)
- [x] The endpoint returns used, remaining and limit for the current period from
      `GetUsage`, plus the period boundaries — `used` summed across the daily
      pairs, `remaining` from the latest day, `limit` reconstructed as
      `used + remaining` (the arithmetic [[0157]]'s close verified against the
      live plan), and the boundaries computed from **our** calendar-month rule.
      Paginated `GetUsage` responses are summed to exhaustion
- [ ] **(deferred to the deploy — cannot be closed from a keyboard, like
      [[0187]]'s live curl)** Making N requests with the key moves the number
      (allowing for the lag). What is asserted instead is that the `GetUsage`
      goes to the exact `(plan, keyId, period)` triple, and the local procedure
      in `packages/prices-api/README.md` §3b names the lag so the check is not
      misread as a failure
- [x] The page states when the figure was last refreshed, in wording agreed
      here (see Design Decisions #6) — and the timestamp is the moment of the
      `GetUsage`, not of the page load, so a cache- or stale-served figure
      dates itself honestly
- [x] Repeated dashboard loads do not produce one `GetUsage` call each — a
      60s in-process cache keyed by caller, covering the `GetApiKeys` lookup
      too (`a_second_load_is_served_from_the_cache` asserts exactly one of
      each across two loads; `the_cache_is_per_caller` that it never crosses
      users)
- [x] Throttling from the control plane backs off rather than erroring the
      page — the SDK's own retry/backoff first, then the last good answer with
      its original `as_of`; a cold-cache throttle is a `503` naming the
      condition
- [x] The 1 req/s rate limit and the reset date are both visible — as numbers,
      with the reset stated as our rule ("the 1st of each month, 00:00 UTC"),
      asserted in the frontend suite
- [x] Response is not cached at either layer — `Cache-Control: no-store` on
      every response the module produces (asserted per branch), and the
      gateway/CloudFront halves were already pinned by [[0187]]'s CI
      assertions, which cover this route because it shares the `{proxy+}`
      `GET` entry (`CachingEnabled: false` in `portalSettings`,
      `CachingDisabled` on the distribution)

## Notes

- Worth knowing while nearby: **`UpdateUsage`** (`op:replace` on `/remaining`)
  moves the quota counter directly without touching the key. That is the right
  tool for "this user needs more quota this month" — one control-plane call, no
  new secret to re-integrate — and it is a different operation from the rework
  in [[0191]], which the dashboard should not conflate.

## Implementation Notes

Backend in the existing `prices-api` axum router (ADR 0008), one new module so
the IAM addition is attributable, mirroring [[0187]]'s shape.

**New — `packages/prices-api/src/portal/usage/mod.rs`** (~470 lines with docs):
the route, `UsageState` (with the per-caller cache), the wall-clock deadline,
the pure `current_period`, and the response shape. **New —
`tests/portal_usage.rs`**: 18 tests over HTTP against the shared mock control
plane.

**Changed:**

- `portal/keys/gateway.rs` — `usage_of` (the `GetUsage` call, paginated to
  exhaustion like the listing), `KeyUsage` (+ reconstructed `limit()`), and
  `GatewayError::Throttled`, which `list_named` now also raises so a throttled
  lookup and a throttled usage read land in the same branch.
- `portal/mod.rs` — mounts the usage router; the stale "routes that do not
  exist yet" comment updated again.
- `tests/portal_keys/harness.rs` — the mock serves
  `GET /usageplans/{plan}/usage` (wire-field `values`, prefix-free), with
  knobs for throttling (sticky 429), failure (sticky 500), pagination, and a
  recorded `(keyId, startDate, endDate)` per call; `throttled()` beside
  `not_found()`/`conflict()`; `call_path` + `usage_router_with` helpers.
- Frontend — `web/portal/src/api/portal.ts` (`PortalUsage`, `fetchUsage`;
  `404 no_key` resolves to `null` rather than throwing) and `app.tsx` (a
  `Dashboard` wrapper lifting one boolean, the `Usage` section, and an
  `onKey` fact-only callback on `ApiKey`). 9 new tests in `app.spec.tsx`;
  every signed-in stub gained the `/usage` route.
- Infra — `api-gateway-stack.ts`: `PortalReadFreePlanUsage`
  (`apigateway:GET` on `/usageplans/{planId}/usage`) added to [[0187]]'s
  standalone policy, with the cold-start-validation decision written at the
  grant; `compute-stack.ts` comments updated (six calls, two granted in the
  gateway stack). **No gateway route change** — `{proxy+}` + `GET` +
  `portalSettings` already cover `/usage`, as the activation note predicted.
- CI — `verify-openapi-routes.mjs` check 5b: exactly one statement on the
  plan's `/usage` sub-resource, action exactly `apigateway:GET`.
- Docs — `packages/prices-api/README.md` §3b (local procedure: the lag and
  the cache are not bugs), `docs/runbooks/portal-oauth-deploy-prep.md` (five
  grants → six).

**Tests: 33 covering this task.**

| where | count | covers |
| --- | --- | --- |
| `portal/usage/mod.rs` | 6 | route shape ×2, period arithmetic ×4 (mid-month, boundaries, December, leap February) |
| `tests/portal_usage.rs` | 18 | the gate, the numbers + period, read-only ×2, no-session, no-rows, pagination, prefix hazard, duplicates-without-delete, cache ×3, throttle ×2, 502, deadline, unconfigured, GET-only |
| `app.spec.tsx` | 9 new (39 total) | numbers + limits + reset, the lag wording, fetch-on-mount, no-key, null-rows, fresh-key honesty, refresh, failure |
| `verify-openapi-routes.mjs` | 1 new assertion (check 5b) | the `GetUsage` grant exists and is the narrow form |

Verified: `cargo fmt --all --check`, `cargo clippy -p prices-api --all-targets`
(clean), `cargo test --workspace` (497 passed, 0 failed), `cargo check
--features lambda`, `nx run-many -t lint typecheck build test`, `nx
format:check --all`, `make -C infra synth-production`, `npm run openapi:lint`,
`openapi:verify-routes`, `openapi:verify-servers`.

## Issues Encountered

- **`GetUsage`'s wire field is `values`, not `items`.** The SDK's output
  member is named `items`, but its deserializer matches the JSON key
  `"values"` (checked against `aws-sdk-apigateway`'s `shape_get_usage.rs`
  rather than guessed from the docs). A mock answering with `items` would have
  handed every test an empty map — and the no-rows path would have covered
  everything, vacuously. The harness comment records the check.

- **The CI matcher for the `/usage` grant first matched `/usageplans/` too.**
  A bare `includes('/usage')` over the serialized resource matched every
  usage-plan ARN, [[0187]]'s `/keys` attach included, and the new check failed
  its first run with "found 2". Fixed by matching the sub-resource as a path
  suffix (`/usage"` with the closing quote) — and that first failure is the
  check's own non-vacuity evidence.

- **`GetUsage` reports no quota limit at all.** The response is daily
  `[used, remaining]` pairs and nothing else, so `limit` is reconstructed as
  `used + remaining` — consistent with [[0157]]'s close (`121 + 99 879 =
  100 000`). The alternative, `GetUsagePlan`, would cost a second grant for a
  number the arithmetic already yields.

## Design Decisions

### From Plan

1. **One statement, `apigateway:GET` on `/usageplans/{planId}/usage`, in
   [[0187]]'s standalone policy** — the task's stated IAM, exactly; declared in
   `ApiGatewayStack` because only it knows the plan id (the same cycle
   argument as the `/keys` attach).
2. **The period rendered is ours** — the calendar month, UTC, computed in a
   pure function with the December and leap-year cases unit-tested — and every
   place it surfaces words it as our rule, never as AWS behaviour (ADR 0010
   correction #2 stays open; [[0191]] owns the measurement).
3. **In-process cache + backoff, no gateway cache.** 60s TTL, keyed by the
   caller's session `sub` so it covers the `GetApiKeys` lookup as well as the
   `GetUsage`; throttling is separated into its own error variant and answered
   with the last good answer rather than an error page.
4. **The key lookup must not create** — the read-only property is asserted
   three ways (no create, no attach, no delete), on the happy path, the
   no-key path and the duplicates path.

### Emerged

5. **A key with no usage rows answers `null`/`null`/`null`, not zeros.** AWS
   having no row is the ordinary state minutes after issuance (`GetUsage` is
   not read-after-write), and while `used: 0` would be defensible, inventing
   `remaining` and `limit` would not. The three go absent together; the page
   renders "nothing recorded yet" with the same lag wording.
6. **The lag wording, decided once:** *"Last updated `<time, UTC>` — AWS
   reports usage with a delay, so requests made in the last few minutes may
   not be counted yet."* The timestamp is the backend's `as_of` — the instant
   of the actual `GetUsage` — so cached and stale-served answers date
   themselves. [[0193]] restyles this line; it does not re-decide it.
7. **Stale-serving is throttle-only.** A `Throttled` error serves the last
   good answer up to 15 minutes old (the entry-retention bound, which also
   bounds the cache's memory); every other control-plane failure is a `502`,
   because "AWS told us to slow down" has an honest fallback and "AWS is
   broken" should be visible. `no_key` is cached like any answer, so a
   keyless caller's refresh loop costs the same nothing.
8. **Duplicates rank by the reveal's rule and the loser is left alone.** The
   usage shown must belong to the key the reveal would hand out
   (`choose_winner`), but sweeping duplicates stays the issue flow's job —
   `DeleteApiKey` from a read path would put the module's most guarded
   operation somewhere a page load can reach.
9. **The frontend fetches usage on mount — the opposite of `/key`, on
   purpose.** [[0187]] decision 14's reasoning is re-derived rather than
   inherited: it forbade fetch-on-load because the key routes can create; this
   route cannot, by construction and by test, so a dashboard that shows usage
   without a button press is safe and is what "dashboard" means. The CSRF
   stance is likewise re-derived in the module docs (state-free `GET`,
   response unreadable cross-origin).
10. **`ApiKey` tells the dashboard a key exists — the fact only.** Straight
    after an issue, the backend's short cache can still answer `no_key` about
    a key the page is displaying; the page then says "your key is new,
    figures appear with a delay" instead of the false "you have no key". The
    callback carries no value and no id, so the credential still never leaves
    the component that masks it.
11. **Cold-start plan validation: declined** (the carried-in question). The
    reasoning is written at the grant in `api-gateway-stack.ts`: [[0187]]
    decision 22 already rejected it, and `GetUsage` against a stale plan id
    fails loudly on the first load rather than silently.
12. **`Throttled` applies to `list_named` too.** The key routes' behaviour is
    unchanged (their generic error arm still answers `502`), but a throttled
    lookup inside the usage flow now lands in the same stale-serve branch as
    a throttled `GetUsage` — the caller cannot tell which of the two calls
    AWS refused, and should not have to.

## Future Work

Nothing new spawned — every follow-up already has a task:

- The live "N requests move the number" observation → the deploy +
  [[0164]]'s evidence pass, like [[0187]]'s curl.
- Styling, and the dashboard composition around these numbers → [[0193]].
- Costing the portal's control-plane call volume with this cache in place,
  and the IAM audit of the now-six grants → [[0194]] (its check list already
  names `GetApiKey` + `GetUsage` per load with the in-process cache).
- Whether AWS's counter actually rolls at our stated instant → [[0191]]
  (ADR 0010 correction #2).
