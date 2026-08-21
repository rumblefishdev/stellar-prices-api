---
id: "0188"
title: "Usage against quota on the dashboard — GetUsage, rendered honestly"
type: FEATURE
status: completed
related_adr: ["0010"]
related_tasks: ["0183", "0157", "0160", "0187", "0193", "0194"]
tags: [layer-backend, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, api-gateway, dashboard, slice-5]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0160_FEATURE_onboarding-backend-endpoints.md"
history:
  - date: "2026-08-13"
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
  - date: "2026-08-21"
    status: completed
    who: akot
    note: >
      Shipped in PR #227 (merged to `develop` as `a76d8a9`). 33 tests cover
      this slice — 24 Rust (18 over HTTP against the mock control plane, 6
      unit), 9 frontend, and one CI assertion pinning the `GetUsage` grant to
      its narrow form; workspace 497 Rust tests, 0 failures. Seven of eight
      acceptance criteria closed; "N requests move the number" cannot be
      closed from a keyboard and waits on the deploy with [[0187]]'s live
      curl and [[0164]]'s evidence pass.

      Four review passes produced 20 findings: 18 fixed, one refuted as
      designed (stale-serve stays throttle-only — an outage should be
      visible), one accepted and handed on (`Instant` TTLs stretch across a
      frozen Lambda container; the failure direction is serving-older, not
      calling-more, and it joins [[0194]]'s costing pass). Two are worth
      carrying forward. A cached "no key" survived the issue that falsified
      it, and the eviction that fixed it then lost a race against an
      in-flight lookup — the answer is per-caller eviction epochs where the
      absence of a mark *proves* no eviction happened, rather than being
      assumed to. And `remaining` is a running balance, so a day whose
      `remaining` rises marks AWS's own period reset: counting from there
      makes `used` and `remaining` come from one period whatever instant it
      began, and warns — the only evidence this system can produce about the
      instant ADR 0010 correction #2 is open on.

      Nothing spawned: the live observation belongs to [[0164]], styling to
      [[0193]], the control-plane call-volume costing and the IAM audit to
      [[0194]], and the quota-roll instant to [[0191]].
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

## Review Findings

Two independent post-implementation reviews: a spec-conformance pass against
the task, the epic, ADR 0010 and tasks 0183–0187/0193/0194, and an adversarial
correctness review. **Nine fixed, one refuted-as-designed, four noted for the
owning tasks.** No scope violation found by either.

| # | severity | what | resolution |
| --- | --- | --- | --- |
| S1 | Medium | The frontend usage timeout (10s) tied with the backend's own 10s deadline, so the page would report a generic timeout instead of the backend's `503` | `USAGE_TIMEOUT_MS = 15s`, between the probe's 10 and the key's 20 |
| S2 | Low | `fetchUsage` read **any** 404 as "no key" — including [[0183]]'s empty gate 404, reachable when the portal closes under an open tab — rendering "you have no API key" about a key that may exist | 404 is "no key" only with the `no_key` envelope code; anything else is a stated failure. Tested |
| S3 | Low | CI check 5b matched `/usage` as a substring (its first run matched `/usageplans/` and failed on "found 2" — its own non-vacuity evidence) and pinned neither the wildcard-free resource nor the declaring stack | Suffix match, wildcard refusal, and a compute-template refusal added |
| S4 | Nit | The standalone policy is still named `…-portal-attach-key` while carrying the usage grant | Kept — renaming an `AWS::IAM::Policy` is a resource replacement bought for cosmetics; a comment at the construct says so for [[0194]] |
| S5 | Nit | The 1 req/s / reset-rule lines rendered only alongside a usage figure, so a keyless visitor — who they inform most — never saw them | Rendered in the no-key state too (without a next-reset date, which comes with an answer) |
| S6 | Nit | The decided wording says "UTC"; `toUTCString()` spells it "GMT", and [[0193]] may not re-decide the line | Suffix corrected at render; test pins "UTC" |
| R1 | Confirmed | The usage section's catch bypassed `describeFailure`, so an expired session read "answered 401" here next to "sign out and sign in again" in the key section | Routed through the shared helper; tested |
| R2 | Confirmed | A cached "no key" survived the issue that falsified it — the page's own refetch and any reload inside the TTL told a key-holder they had no key | `UsageCache::invalidate_no_key`, held by the key routes via `KeysState::with_usage_cache`, evicts exactly that answer on every successful issue/reveal. Integration-tested inside the TTL |
| R3 | Plausible | The negative-value clamp plus `limit = used + remaining` can render an invented limit on a nonsense row; the clamp comment claimed to prevent what it produces | Comment rewritten to state the degradation honestly; behaviour kept — the response carries no true limit to fall back on and no such row has been observed |
| R4 | Plausible | Serving a stale answer during a throttle did not re-stamp it, so every load during a throttle event still fired both control-plane calls — the opposite of backing off | The served entry is re-stamped (original `as_of` kept), making the next TTL of loads cache hits; tested, including that AWS is then left alone |
| R5 | Plausible | Cached entries carried period fields never re-checked against the calendar, serving last month's period (and a past `resets_at`) for up to a minute after the boundary — 15 under the throttle fallback | Entries answering for a different `period_start` are treated as expired; "no key" is period-independent. Unit-tested |
| R6 | Confirmed | Decision #12 (a throttled key **lookup** lands in the stale-serve branch) was asserted nowhere — reverting `list_named`'s `Throttled` arm kept the suite green | `throttle_list` knob on the mock + a test that answers `502` if the arm is reverted |
| R7 | Plausible | `usage_of` always queried a future `endDate` (month end), which only the mock — accepting any string — had ever validated | The query now ends **today** (future days carry no data anyway); the rendered `period_end` stays the month boundary. Test updated to pin the split |
| — | Refuted | "Stale-serve should cover every failure, not just throttling" | As designed (decision #7): a throttle has an honest fallback, an outage should be visible |

**A third pass** (Adam-run `/code-review` against the finished branch)
produced 10 verified findings; the four correctness ones are fixed, the rest
dispositioned:

| # | severity | what | resolution |
| --- | --- | --- | --- |
| C1 | Confirmed | The deadline's `503` arm never consulted the stale cache — a throttle manifesting as SDK-retry LATENCY (each per-call budget spent before any 429 surfaces) was cancelled by the timeout and answered with the exact error page the throttle arm exists to prevent | The elapsed arm now serves (and re-stamps) the last good answer, `503` only with nothing cached; tested with an injected delay |
| C2 | Confirmed | Write-after-eviction race: an in-flight lookup that snapshotted a keyless listing could `remember()` its `NoKey` **after** the concurrent issue's `invalidate_no_key` ran as a no-op — resurrecting the fixed R2 for a full TTL | Per-caller eviction epochs: `invalidate_no_key` bumps unconditionally, and a `NoKey` computed under an older epoch is discarded instead of stored (real answers exempt — a listing that saw the key cannot be falsified). Unit-tested step by step |
| C3 | Confirmed | `Instant`-based TTLs stretch across a frozen Lambda container (CLOCK_MONOTONIC does not reliably advance while frozen) | **Not fixed here, accepted** — `as_of` keeps the rendered age honest, the failure direction is serving-older not calling-more, and a `SystemTime` stamp brings its own non-monotonicity; noted for [[0194]]'s costing pass |
| C4 | Confirmed | The `keyOnScreen` effect refetched on every flip, blanking already-rendered numbers into a loading flicker for a body the backend cache answers unchanged | Refetch guarded to the no-key state (latest view read through a ref, so the effect fires on the transition alone and cannot loop); tested |
| C5 | Confirmed | A short daily pair (`[121]`, `[]`) defaulted `remaining` to 0, collapsing the reconstructed limit to `used` — a barely-used key rendered as quota-exhausted | Malformed rows are warn-and-skipped like the id-less key in `list_named`; all-malformed degrades to "nothing recorded". The mock now serves raw rows so both are tested |
| C6–C10 | Plausible | `Throttled` drops the SDK message; check 5b will false-fail on a future `UpdateUsage` grant; `fetchUsage` re-implements `getJson`; the auth preamble is duplicated from the key routes; the 502 envelope is hand-assembled (no `errors::bad_gateway`) | Dispositioned below with the earlier deferrals — C9/C10 join the 0192 extraction note, C6 its observability half, C7/C8 recorded here for whoever touches those files next |

Noted for owning tasks, deliberately not fixed here: ~~the `1 req/s` page
literal duplicates `pricingApiFreePlanRateLimit` with no drift check
([[0193]]/[[0194]] — the response or `/config` is where the number would
belong)~~ **— fixed in the fourth pass below, and the guess was right: it went
on `/config`**; the throttle classification idiom exists at two of six SDK call
sites and [[0192]]'s
revocation should extract a shared classifier rather than copy it; `no_store`
is a per-branch discipline across three portal modules and could become a
response layer on the gated prefix; a throttle with **nothing** cached still
lets a refresh loop reach AWS (bounded by SDK backoff and human rates —
[[0194]] costs it).

**A fourth pass** (PR #227 review by Oskar Karcz, five findings, all low
severity) — **all five fixed.** Two of them corrected earlier passes of this
same task, which is the part worth recording: C4's ref-read and C2's epoch guard
were each right about the race they were written for and wrong at one edge.

| # | what | verdict | resolution |
| --- | --- | --- | --- |
| O1 | The `Rate limit: 1 request per second` literal can go stale against `pricingApiFreePlanRateLimit`, the per-env value `addUsagePlan` enforces — the one number on the panel not coming from the backend | Confirmed | `compute-stack.ts` passes it as `PORTAL_RATE_LIMIT` (same config key `api-gateway-stack.ts` feeds the plan), `AppConfig` reads it, `/config` serves it as `rate_limit_per_second`, and the page renders THAT. Absent → the line is omitted, never defaulted: a fallback figure is the same silent staleness one layer down. Synth-verified: the template carries `PORTAL_RATE_LIMIT = 1` beside the plan's `RateLimit: 1` |
| O2 | `fetchUsage` discarded the backend's error envelope for every non-404, so `usage_unavailable`'s authored message never reached the page — and the extra 5s of `USAGE_TIMEOUT_MS` was spent waiting for an answer that was then thrown away | Confirmed | `readEnvelope` + `failureMessage` prefer the backend's `message`, falling back to `${url} answered ${status}` when there is no envelope (the gate's empty 404, a proxy's page). Applied to `getJson` and `issueKey` too — one wording per cause, which is R1's rule extended to the failures the backend writes itself. `describeFailure` still owns the 401 sentence |
| O3 | The keyed refetch could never fire if the mount-time load was still in flight: the `keyOnScreen` transition happened while the view was `'loading'`, and the effect's deps never moved again. The section sat on "your key is new" until the visitor found Refresh | Confirmed | Supersedes **C4's** ref-read. The effect now watches `view.state` — the honest dependency — and a `refetchedForKey` ref makes it fire at most once per mount, which is what C4's ref was avoiding the loop with. Both properties tested: the deferred-response race recovers with no Refresh press, and a refetch answered `no_key` again does not re-trigger |
| O4 | If AWS's MONTH quota rolls at any instant other than our calendar 1st 00:00 UTC (undocumented — ADR 0010 correction #2), the query spans two AWS periods: `used` sums both while `remaining` is the current one, so `limit = used + remaining` renders a quota no plan has | Confirmed, and understated — `used` is wrong too, `limit` is its consequence | Neither of the review's two suggestions taken. Bounding the query start at the key's first row does not touch the boundary, and a note would leave the wrong number on screen. Instead `summarize_days` uses the invariant AWS's own shape gives: `remaining` is a running balance, so a day whose `remaining` RISES is a reset — count from it. Both figures then come from one period whatever instant it began. It also `warn!`s the sighting, which is the only evidence this system can produce about the instant ADR 0010 is open on |
| O5 | The epoch guard read `unwrap_or(0)`, so a mark pruned at `STALE_KEEP` mid-lookup compared as a moved epoch and discarded a legitimate `NoKey` — buying a `GetApiKeys` on every load after it. Plus: `invalidate_no_key` wrote to `epochs` without pruning, and `remember` (the only pruner) is not reached on the error paths | Confirmed | Corrects **C2**. `epoch_of` returns `Option<u64>` and no mark now matches any epoch — safe by construction, not by luck: `invalidate_no_key` stamps `bumped_at` as it bumps, so any eviction inside the window leaves a mark too fresh to prune, and absence therefore *proves* no eviction happened. `invalidate_no_key` prunes too, closing the leak |

No infra test accompanies O1: this repo has no CDK assertion harness, and adding
one for a single env var would be the larger change. The wiring is instead
drift-free by construction — one config key, read once, passed unconditionally —
and `cdk synth` was run to confirm the value reaches the template.

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
