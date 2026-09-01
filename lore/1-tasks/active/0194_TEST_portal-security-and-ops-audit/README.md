---
id: "0194"
title: "Portal security and ops audit — and the three changes it forced: the flip, the flat /api/ prefix, and the API's own hostname"
type: TEST
status: active
related_adr: ["0007", "0010"]
related_tasks: ["0183", "0159", "0160", "0184", "0186", "0187", "0188", "0189", "0191", "0192", "0164"]
tags: [layer-infra, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, security, audit, iam, slice-11]
milestone: 3
links:
  - "../../archive/0160_FEATURE_onboarding-backend-endpoints.md"
  - "../../archive/0159_FEATURE_discord-oauth-sign-in.md"
  - "./audit/2026-08-28-report.md"
history:
  - date: "2026-08-13"
    status: backlog
    who: akot
    note: >
      Eleventh slice. Not a bucket for deferred hardening — each slice ships its
      own security requirement, including the two that are not deferrable
      (`no-store` on key reveal, throttles outside the `cacheEnabled` branch).
      This task exists because those requirements are spread across seven
      slices and one of them is a wholesale-assigned array: the audit is of the
      **assembled** result, which no individual slice can see.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Activated by Adam once [[0193]] merged (PR #249, `d53cfc2`). Every slice
      this audit composes has now shipped, so the assembled `methodSettings`
      array, the `cacheEnabled` branch and the CloudFront policy can be read
      off a real synthesized template rather than predicted from source.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      First full audit run, read-only, against the deployed stacks and a synth
      at `79dd55c` — `audit/2026-08-28-report.md`. Ten of twelve checks PASS
      and the assembled gateway/CloudFront/IAM/bucket composition is
      byte-identical between `develop` head and production. Two FAIL, both
      operator steps the runbook lists that were never run: the Discord OAuth
      secret does not exist in Secrets Manager (0186, runbook §2) and neither
      eligibility SSM parameter exists (0189, §2a). With the flag on, either
      absence fails Lambda init and takes `/v1` down, so the recommendation is
      NO-GO until both are seeded and re-verified. One real portal key from a
      local run sits on the free plan and is listed for deletion before the
      flip. Checks 1 and 2 each keep a half that only the open portal can
      close; the commands are in the report. Converted to a directory to hold
      the evidence.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Two decisions by Adam, both recorded here rather than in the audit
      report, which is a dated measurement and stays as taken. (1) **This task
      signs off at `https://sorobanscan.rumblefish.dev/api/`, not at the
      distribution domain** — the bundle is synced to the Explorer
      distribution's `api-spa` bucket, so three of the twelve checks are now
      properties of `EA2TLS5SS5M87` and must be re-run there. That couples this
      task to [[0195]] and to a change in the `soroban-block-explorer` repo:
      today that distribution has no origin for our API, allows `GET`/`HEAD`
      only under `/api/*`, sits behind a basic-auth function, and rewrites 403
      and 404 to `index.html` with status 200. A bare `s3 sync` of the current
      bundle produces a blank page — its `index.html` carries absolute
      `/api-tokens/assets/…` URLs. Written up as **Hosting preconditions**.
      (2) **`PORTAL_ENABLED` flipped to `'true'`** in `compute-stack.ts`. The
      commit is the one-word diff the flag was designed to be; the **deploy is
      still gated**, because the OAuth secret and both eligibility parameters
      do not exist and a missing one fails Lambda init on the function that
      also serves `/v1`. The comment at the flag now says so.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Checkbox state synchronised with the audit's own findings, which had
      lived only in the report's table — the task read as though nothing had
      been done. Four checks are now ticked, and only those that are settled
      AND survive both [[0235]] and the host change: IAM (6), the collection
      grant (7), the key-leak sweep (8) and the control-plane costing (12).
      Six carry ⏳ with what was measured and why it does not close: checks 1,
      3 and 4 passed but on `/api-tokens/api/{proxy+}` paths that [[0235]]
      moves, so they need re-measuring after the deploy; checks 2 and 5 were
      verified on our distribution and have to be re-run against
      `EA2TLS5SS5M87`; check 11 passed on OUR bucket while the sign-off bucket
      is `production-soroban-explorer-api-spa`, whose public-access block and
      policy status spot-check clean but whose policy, OAC scoping and
      anonymous-GET behaviour are unaudited. Two carry ❌ — the absent secret
      and the two absent parameters, unchanged.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Blocker B2 closed. Both eligibility parameters seeded by hand at 12:14Z
      with Adam's approval — guild `1536303837785362432` (the test guild) and
      `5` minutes — and check 10 flips to PASS: `LastModifiedUser` is the SSO
      operator role rather than `AWSCloudFormation`, which is exactly what the
      check is about, and the guild id is a 19-digit snowflake so the cold-start
      probe's shape test will pass rather than turning every visitor into
      `could not verify`. B1 (the OAuth secret) is still open and is Adam's to
      run. Separately, PR #264 merged as `9c8a331`, so `develop` now carries
      [[0235]]'s prefix move and the deploy is split in two: the prefix first
      with the portal still closed, the flag second.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Prefix deploy went out at 13:16-13:31Z (all six stacks), portal still
      closed. Re-ran every check [[0235]] had invalidated, against the deployed
      stack: check 3 and check 4 now PASS on `/api/api/{proxy+}` and are ticked;
      check 1's gateway half passes with the three entries at 10/40 and caching
      off and no old-prefix entry surviving, while its response half stays
      blocked by the closed portal. Check 9's bundle half re-scanned on the 13
      newly deployed files — zero real key values, zero third-party references,
      the only suspicious strings being base64 slices and [[0233]]'s design
      placeholder. Check 11's host half fully audited on
      `production-soroban-explorer-api-spa` and PASSES. **One prediction was
      wrong and is corrected in the body: the orphaned `api-tokens/*` objects
      are NOT inert — `/api-tokens/` still returns 200 with the old bundle via
      the default behaviour and `DirectoryIndexFn`'s trailing-slash rewrite,
      and that stale app then 403s on its own backend.** Four objects to delete
      before opening.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Blocker B1 closed — the Discord OAuth secret was created at 13:53Z with
      [[0235]]'s callback and validated by shape without reading its values, and
      the api-handler's IAM grant matches the generated ARN suffix. Check 9
      flips to PASS. ⚠️ The `client_secret` reached a chat transcript on the way
      in, so it is recorded as compromised and needs rotating in the Developer
      Portal followed by `put-secret-value`; the flip is not blocked on that,
      but Tranche 3 sign-off should not claim the secret was never exposed.
      Issues Encountered written up while the detail is fresh: the exposure, the
      wrong inert-objects prediction, Zig as an undocumented build prerequisite,
      and CI discarding the Lambda assets it builds.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Two deletions with Adam's approval, both verified after the fact. The four
      orphaned `api-tokens/*` objects are gone, so the previous portal no longer
      answers — `/api-tokens/` and `/api-tokens/index.html` now `403` while
      `/api/` stays `200`. And the one key created while the flag was off,
      `smdesqkg5j`, is deleted: no `discord-*` key remains in the account and the
      free plan holds only the CDK-managed `t61phbbhhj`. That closes the third
      gate precondition. With [[0189]] met and blockers B1 and B2 closed, the
      only preconditions still open are the two `(host)` checks, which wait on
      the Explorer distribution.
  - date: "2026-09-01"
    status: active
    who: akot
    note: >
      Check 3 re-run a third time and still PASS — 14 entries vs 6, all four
      `/api/{proxy+}` methods byte-identical in both arms of the
      `cacheEnabled` branch. It needed re-running because `addCorsPreflight`
      added `OPTIONS` to the wholesale-assigned array on 08-31, after the
      08-28 reading: the failure mode this check exists for, arriving from
      this task's own commit. Summary, H1, the "why an audit" rationale and
      the sign-off-host section amended where decision A had made them false.
  - date: "2026-09-01"
    status: active
    who: akot
    note: >
      Retitled and given a "What this task actually shipped" section, because
      the title said `TEST` while seven commits carried code and three of them
      changed the portal's architecture: the flip (`PORTAL_ENABLED` true, in
      production), the flat `/api/` prefix (supersedes [[0161]]'s convention
      for this app and [[0235]]'s three-day-old layout), and the API's own
      hostname with CORS (supersedes the same-origin property [[0184]] and
      [[0186]] were built on). Plus the tag-on-create IAM fix without which no
      key could ever be issued, the example rewrite that closes [[0233]]'s
      portal half, and the branch's own code review — six findings, all fixed
      in `5635af9`. Rust 405 → 416, portal 157 → 170. Nothing about the
      checks' verdicts changed; the record of what merging this branch does
      did.
  - date: "2026-09-01"
    status: active
    who: akot
    note: >
      Oskar's review of PR #268 (seven findings, read at `e7704ae`) assessed
      and answered on the branch: all seven valid, two already fixed in the
      unpushed `5635af9`, five fixed here. Finding 1 became a design decision
      — a portal source failing at cold start now closes the portal instead
      of panicking the Lambda that serves `/v1` — recorded as Emerged #4 with
      the evidence the original stance lacked. Spawned [[0249]] (the
      api-handler error alarm it never had, plus the new log line) and
      [[0250]] (the CloudTrail detective control the 08-31 review left as
      prose). Also disarmed a calendar bomb in `app.spec.tsx` that turned CI
      red today on every branch. Rust 416 → 418, portal 170 → 179.
---

# Portal security and ops audit — and the three changes it forced

## Summary

**Story:** *as the person submitting Tranche 3, I can show that the portal's
final assembled configuration is correct — not that each slice intended it to
be.*

The story still holds and it is still what the checks answer. What the summary
below promised about *scope* did not survive contact with the measurements, and
the sentence is kept with its correction rather than quietly rewritten.

⚠️ **Amended 2026-09-01: "everything here is already required by an earlier
slice" is false.** It was true of the twelve checks and false of the task. The
audit measured three things that could not pass as built, and Adam's answer in
each case was to change the portal rather than to file a finding: the portal
was **opened** (the flip is this task's by design), the URL layout was
**flattened** to `/api/`, and the API was given **its own hostname** with CORS.
No earlier slice required any of those — two of them supersede properties
earlier slices were built on. A fourth, the tag-on-create IAM grant, was
required by [[0187]] and had never worked. The full index is the next section;
read it before the title.

Original — everything here is already required by an earlier slice. What is new
is checking the composition, because three of these are properties of the whole
array or the whole policy and are invisible from inside any one task.

## What this task actually shipped — read this before the title

**The title says `TEST` and "audit". Seven of the branch's commits carry code,
and three of them changed the portal's architecture.** That is not scope creep
discovered late: each one was a decision Adam made *because* the audit measured
something that could not pass as built. The record below is the honest index of
what merging this branch does, because "audit" is not a description anyone
would deploy carefully on.

The `Notes` at the bottom say a `TEST` that writes code should hand the work to
the slice that owns it. That rule was consciously not followed here, twice
(the flat prefix and the custom domain), and the reason is written at each
decision: both were prefix/host changes whose cost was one deploy on our side
*only* while the Explorer repo had not yet built against the old shape. A week
later they would have been a coordinated change across two repos.

### The three architectural changes

| # | commit | what it changed | what it supersedes |
|---|---|---|---|
| 1 | `4ebb0ea` + `8562cf1` | **The portal is open.** `PORTAL_ENABLED: false → true`, deployed to production 2026-08-31 10:43Z (`ComputeStack` alone). Four cold-start sources had to exist first, and two of them did not — see the flip section | the flag's own gate; nothing else |
| 2 | `a0aaa13` | **The flat prefix.** `/api/` is the whole portal — `/api/login` is a page, `/api/auth/login` is the backend. Bundle and backend share one prefix, so the *bundle* is enumerated as ten CloudFront carve-outs ahead of an `/api/*` catch-all to the API. Touches Rust route constants, the gateway resource `/api/{proxy+}`, `portal-hosting-stack.ts`, the Vite dev proxy, `verify-openapi-routes.mjs`, `links.ts` and the docs. New `OPENAPI_PATH` alias at `/api/api-docs-json` | [[0161]]'s `<app>/*` + `<app>/api/*` convention, **for this app**. [[0235]]'s `/api/api/` layout, which had shipped three days earlier |
| 3 | `c8fa31d` | **The API has its own hostname.** `prices-api.sorobanscan.rumblefish.dev`, a REGIONAL custom domain with its own DNS-validated certificate and A/AAAA records; the bundle on the shared host calls it **cross-origin, same-site**. Brings with it: `addCorsPreflight` on `/api/{proxy+}` (MOCK, one origin, credentials) joining `portalSettings` in **both** arms, CORS headers on the gateway's own error responses, a `CorsLayer` scoped to the portal routes, `PORTAL_WEB_ORIGIN`, an absolute `AuthState.home`, a third leg on the CSRF check, `credentials: 'include'` in the bundle, and `make sync-portal-explorer` | the **same-origin, no-CORS** property [[0184]] and [[0186]] were built on. The four Explorer-side requirements in `audit/2026-08-31-explorer-distribution-requirements.md`, which is now superseded to one item |

### The four smaller code changes

- **`a5c920e` — the api-handler could never create a key in production.**
  `CreateApiKey` with `tags` is authorised as `apigateway:PUT` on
  `/tags/…/apikeys/*`, separate from `POST /apikeys`, and the role had `POST`
  only. Three per-resource IAM audits read the policy comment's "deliberately
  NOT here" as a statement of intent and agreed with it. The first real
  browser sign-in found it in one press. `PortalTagApiKeysOnCreate`, conditioned
  on `aws:RequestTag/ManagedBy` and a `aws:TagKeys` allow-list.
- **`64e22da` — every code example on the portal was the design's API, not
  ours.** The hero, the landing endpoints and the whole quick start showed
  `GET /v1/prices/XLM-USDC`, `/pools`, `/history`, `source: "soroswap"` — routes
  that do not exist. With a freshly issued key, the quick start's own "First
  request" answered `403`. Rewritten against the seven live routes on the API's
  hostname, with `apiBaseUrl` feeding the OpenAPI `servers` block. This closes
  [[0233]]'s portal half.
- **`5635af9` — the branch's own `/code-review`, six findings.** Full table in
  "Code review of the whole branch" below. The two that reach a visitor: two
  quick-start snippets whose on-screen code did not compile while their Copy
  button wrote correct code, and three sign-in refusals that answered a
  **browser** with a JSON envelope — leaving the OAuth popup open on raw text
  with no way back.
- **Test suites grew with all of it:** Rust 405 → 416, portal 157 → 170, plus
  a new carve-out assertion in `openapi:verify-routes`.

### The operator actions, which are not in any diff

Four production changes this task performed by hand, each of which a check
had assumed was already true:

- the Discord OAuth secret **created** (it did not exist), then **rotated**
  after its `client_secret` leaked into a working transcript, then rewritten
  twice more as the callback path and then the callback *host* moved;
- both eligibility SSM parameters **seeded** (neither existed — the runbook's
  own ticked criterion had been true of the runbook and false of the account);
- one stray portal-tagged API key from a local run **deleted** off the live
  free-tier plan;
- the orphaned `api-tokens/*` bundle objects **deleted**, after this task
  predicted they were inert and measured that they were not — they were still
  serving the previous portal at `200`.

### What is left for other tasks

- **[[0195]]** is reduced to one item: `enableApiSpaBasicAuth: false` in the
  Explorer repo, which gates *public availability*, not correctness.
- **The gating guild is still the test guild** — a `put-parameter --overwrite`
  away, and it must happen before the portal is advertised (Design Decision 1).
- **The Discord redirect URI** now names the API host and is registered.
- **A CloudTrail detective control** for a `PUT /tags` not preceded by a
  `CreateApiKey` — spawned by the review's finding 4, which is unfixable in IAM.
  Filed as [[0250]] on 2026-09-01; it had been prose only.
- **An error alarm on the api-handler** — on `Errors`, which it never had,
  and on the `portal closed at cold start` log line that the PR review's
  finding 1 introduced. Filed as [[0249]].

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

⚠️ **Amended 2026-08-31 (decision A): the third one no longer exists, and the
first two were right.** There is one caching layer in front of the portal's
routes now, not two — the backend answers on its own hostname with no
CloudFront between it and the browser, so cookie-stripping and CDN-cached key
reveals are both out of reach by construction. What replaced that failure mode
is CORS: a header that is right on our API and wrong at the edge is invisible
from either side in exactly the same way, which is why check 2 became a CORS
check rather than being dropped.

The first two held up, and grew a fourth member: `OPTIONS` joined
`portalSettings` with the preflight, so the two-arm `methodSettings` diff check
counts 14 against 6 rather than 13 against 5. That is the shape this section
predicted — a slice adding a route to a wholesale-assigned array — arriving one
more time, from this task itself.

## The host this task closes against

**`https://sorobanscan.rumblefish.dev/api/` — not the distribution domain.**
Decided 2026-08-28 by Adam. The portal's bundle is synced to
`s3://production-soroban-explorer-api-spa/api`, which is origin 2 of the
**Explorer** distribution `EA2TLS5SS5M87` (alias `sorobanscan.rumblefish.dev`),
behaviour `/api/*`. `dojr4epgxo2qp.cloudfront.net` — the distribution
`PortalHostingStack` creates and the one the 2026-08-28 audit measured — is
where the configuration was verified, and it is **not** where this task signs
off.

⚠️ **Amended 2026-08-31 by decision A, and the sign-off host is now two
hosts.** The *page* is still served by `EA2TLS5SS5M87` at
`https://sorobanscan.rumblefish.dev/api/` and that is still where this task
signs off. Every *call the page makes* now goes to
`prices-api.sorobanscan.rumblefish.dev` — a REGIONAL custom domain on our own
REST API — cross-origin and same-site. So the paragraph below is right about
which host the visitor types and wrong about which distribution answers the
portal's routes: none does. See "The backend on its own host".

The consequence as it was stated, and how it resolved: three of the twelve
checks are properties of *the distribution that serves the portal*, so they must
be re-run against `EA2TLS5SS5M87` before this task closes, and the assembled
configuration they check does not exist there yet. That work is [[0195]]'s and
it lives partly in the `soroban-block-explorer` repo — see **Hosting
preconditions** below. This task does not do it and does not sign off without
it. **It resolved by removing the dependency rather than by waiting for it:**
with the backend on its own hostname, check 1's `no-store` is measured where the
browser reads it, check 2 became a CORS check on our own API, and check 5 is
not applicable — none of the three needs anything from the Explorer repo, and
[[0195]] is down to `enableApiSpaBasicAuth: false`.

## Checks

Verify against the **synthesized CloudFormation template and the deployed
stack**, not against the source, and not by assumption. Checks marked **(host)**
are properties of the serving distribution and are answered against
`EA2TLS5SS5M87`; the rest are answered against our own stacks and were settled
on 2026-08-28:

- [x] ✅ **2026-08-31 11:49–12:35Z: PASS on the sign-off layout.** Measured on
      `prices-api.sorobanscan.rumblefish.dev` — the host the bundle actually
      calls — every portal route carries `cache-control: no-store`
      (`/api/config` 200, `/api/key` 401, `/api/auth/login` 303, logout 204),
      and no CloudFront sits between the browser and those answers, so no
      `CustomErrorResponses` can rewrite them. In the browser the JSON `401`
      and `404 no_key` reached the app as themselves (dashboard rendered
      "Not issued" from the envelope). Earlier amendment —
      ⏳ **Amended 2026-08-31 (decision A): the (host) half changes shape.**
      The portal's responses no longer pass through `EA2TLS5SS5M87` at all —
      the bundle calls `prices-api.sorobanscan.rumblefish.dev` directly, so
      `CustomErrorResponses` cannot touch a portal `403`/`404` and the
      `no-store` measured on execute-api is the `no-store` the browser sees.
      Re-measure once on the new hostname after the ApiGateway deploy; then
      this closes. Earlier reading — **2026-08-31, AFTER THE FLIP: origin half now fully PASS; only the
      (host) half is left.** With the portal open the routes finally emit real
      responses, so `no-store` is observable for the first time — and every one
      of them carries it, on success and error paths alike, measured on
      `02mabge71l.execute-api…/production`:

      | route | method | status | `Cache-Control` |
      |---|---|---|---|
      | `/api/api/config` | GET | 200 | `no-store` |
      | `/api/api/auth/login` | GET | 303 | `no-store` |
      | `/api/api/auth/callback` | GET | 400 | `no-store` |
      | `/api/api/auth/logout` | POST | 204 | `no-store` |
      | `/api/api/key` | GET / POST | 401 | `no-store` |
      | `/api/api/key/rework` | POST | 403 | `no-store` |
      | `/api/api/usage` | GET | 401 | `no-store` |
      | `/api/api/does-not-exist` | GET | 404 | **none** — axum's fallback, not a portal response |

      Note the path: revoke is `/api/api/key/rework` (`keys/mod.rs:125`), not
      `/key/revoke` as the prose above and [[0192]] call it.

      ⚠️ **`/key/rework` answering `403` is the (host) risk made concrete.**
      `EA2TLS5SS5M87`'s `CustomErrorResponses` maps 403 to `/index.html` with
      status `200`, so on the sign-off host that revoke refusal would reach the
      browser as Explorer's SPA reporting success. That is no longer a
      hypothetical about [[0183]]'s gate `404` — it is a live status on a live
      route.

      Earlier reading — **2026-08-28, RE-RUN after the prefix deploy: gateway
      half PASS on the new paths.** `get-stage` after 13:30Z: 13 entries, the three
      `api/api/{proxy+}` verbs all `cachingEnabled: false`, rate 10, burst 40,
      and **no entry naming the old prefix survives**. `/api/api/config` carries
      `cache-control: no-store` through CloudFront and the gateway alike. The
      response half stays BLOCKED — the other routes are still [[0183]]'s empty
      `404`, and they carry **no** `Cache-Control` at all (the [[0183]] note
      below). Earlier reading, now superseded: **gateway half PASS, response
      half BLOCKED + invalidated by [[0235]].** The three portal `methodSettings` entries read
      `cachingEnabled: false` on the deployed stage, and `/config` carries
      `no-store` through both layers; every other route is [[0183]]'s empty
      `404`, so its `no-store` is unobservable while closed. The entries were
      measured at `/api-tokens/api/{proxy+}` and move to `/api/api/{proxy+}` —
      re-measure after the deploy. Report E1.
      Every portal method has `cachingEnabled: false`, and every portal response
      carries `Cache-Control: no-store` — the gateway half is settled; the
      response half is **(host)**, because `EA2TLS5SS5M87` maps 403 and 404 to
      `/index.html` with status `200` (`CustomErrorResponses`), which today
      would swallow the portal's JSON errors and [[0183]]'s gate `404`
- [x] ✅ **2026-08-31 11:49Z + browser walk: PASS as replaced.** On
      `prices-api…`, `Origin: https://sorobanscan.rumblefish.dev` gets
      `Access-Control-Allow-Origin` naming that origin plus
      `Allow-Credentials: true` and `Vary: origin`; `evil.example` and no
      `Origin` get neither; the preflight answers `204` with the marker header
      allowed. In Adam's Chrome the session cookie set by the callback on
      `prices-api…` was sent on every cross-host `fetch` (`/api/auth/me` 200
      as `kotryba`, `/api/key` 200) — the same-site property, live. Earlier
      amendment — ⏳ **Amended 2026-08-31 (decision A): moot as written, replaced by a
      CORS check.** There is no CloudFront behaviour in front of the portal's
      backend any more, so "disables caching and forwards the cookie" has no
      subject. What stands in for it: on `prices-api…`, `GET /api/auth/me`
      with `Origin: https://sorobanscan.rumblefish.dev` and a session cookie
      answers signed-in with `Access-Control-Allow-Origin` naming that origin
      and `Allow-Credentials: true`; with any other `Origin` it carries
      neither. Both asserted in `tests/portal.rs`; measure once live after the
      deploy. Earlier reading — **2026-08-31: MEASURED at the sign-off host, and it FAILS — there is
      no API origin at all.** `get-distribution-config` on `EA2TLS5SS5M87`
      returns two origins and **both are S3** (`…-spa`, `…-api-spa`); nothing
      points at `02mabge71l.execute-api…`. Behaviour `/api/*` targets the S3
      bucket with `GET`/`HEAD` only and carries the
      `production-soroban-explorer-basic-auth` function, which answers `401` to
      every unauthenticated request under the prefix. So the check's subject —
      a behaviour that disables caching and forwards the session cookie — does
      not exist to be measured. Full requirement written up for the owning repo
      in `audit/2026-08-31-explorer-distribution-requirements.md`. Earlier
      reading — **2026-08-28: not started at the sign-off host.** Verified on
      `dojr4epgxo2qp.cloudfront.net` (report E2): `Managed-CachingDisabled` +
      `Managed-AllViewerExceptHostHeader` (`CookieBehavior: all`), and 13 of 13
      probe requests reached the origin — nothing served from the edge. None of
      that transfers.
      **(host)** The portal prefix's CloudFront behaviour on `EA2TLS5SS5M87`
      disables caching **and** forwards the session cookie; a signed-in request
      reaches the origin signed in. Today that distribution has **no origin
      pointing at our API at all**, `/api/*` is `GET`/`HEAD` only and sits
      behind the `production-soroban-explorer-basic-auth` function
- [x] ✅ **2026-09-01, RE-RUN a third time — after the preflight, which this
      task itself added to the array.** Synth both ways on this branch: **14
      entries vs 6**, and all **four** `/api/{proxy+}` entries — `GET`, `POST`,
      `DELETE` and now `OPTIONS` — byte-identical in both arms
      (`CachingEnabled: false`, rate 10, burst 40). The `ON ONLY` set is still
      exactly the cache TTL table plus `/api-docs-json`, none of which carries
      a portal route. Worth stating why this re-run happened at all: the
      08-28 reading was taken before decision A, and `addCorsPreflight` added
      a method to the wholesale-assigned array three days later — the exact
      failure this check exists for, arriving from the audit's own commit.
      A PASS measured before the last change to the thing it measures is not
      a PASS. Earlier reading — **2026-08-28, RE-RUN after the prefix deploy: PASS on the new paths.**
      Synth both ways at `develop`: 13 entries vs 5, and all three
      `/api/api/{proxy+}` entries byte-identical in both arms
      (`CachingEnabled: false`, 10/40). Nothing portal-shaped is missing from
      the `cacheEnabled: false` arm. Earlier reading: PASS, then invalidated by
      [[0235]]. Synth with the flag both ways: 13 entries vs 5, and all three portal entries byte-identical in
      both arms; the `ON ONLY` set is exactly the cache TTL table, which carries
      no throttle. Re-run the two-arm diff on the new paths. Report E3.
      The full `methodSettings` array contains every portal route in **both**
      arms of the `cacheEnabled` branch — flip `apiGatewayCacheEnabled` off in a
      synth and diff
- [x] ✅ **2026-08-28, RE-RUN after the prefix deploy: PASS on the new paths.**
      `get-resources` after 13:30Z: `/api/api/{proxy+}` carries `GET`, `POST`
      and `DELETE`, every one `apiKeyRequired=False`, `authorizationType=NONE`,
      with throttle 10/40 from `get-stage`. Earlier reading: `get-resources`:
      `apiKeyRequired=False` on all three verbs, with throttle 10/40 from
      `get-stage`. Measured on the old resource path. Report E4.
      Anonymous sign-in routes carry their own method-level throttle and are not
      behind `apiKeyRequired`
- [x] ✅ **Not applicable by decision (A, 2026-08-31), and verified so:**
      `get-distribution-config` on `EA2TLS5SS5M87` after the walk still shows
      only S3 origins and `/api` + `/api/*` → the SPA bucket; nothing of ours
      is ordered there and nothing needs to be. Earlier amendment —
      ⏳ **Amended 2026-08-31 (decision A): no longer applies.** There is no
      API behaviour on `EA2TLS5SS5M87` to order and there will not be one;
      the only rows under `/api` are the bundle's. Closes with the deploy as
      "not applicable, by decision". Earlier reading — **2026-08-31: MEASURED, FAILS — there is no API behaviour to order.**
      `EA2TLS5SS5M87`'s table is `/assets/*`, `/static/*`, `/api/*`, then
      default; all three named behaviours target S3. The ordering rule cannot be
      satisfied until the `/api/api/*` behaviour exists, and it must be inserted
      **ahead of** `/api/*`. Earlier reading — **2026-08-28: not started at the
      sign-off host.** Correct on our distribution (report E5) and on the
      post-[[0235]] synth; `EA2TLS5SS5M87` has no API behaviour at all yet.
      **(host)** The portal's API behaviour precedes its bundle behaviour in
      `EA2TLS5SS5M87`'s order, whatever the two prefixes end up being — the
      ordering rule of [[0161]], not the literal `/api-tokens/` pair
- [x] ✅ **Amended again 2026-08-31 by this task's own `/code-review`: the
      PASS stands, and one sentence of it was over-read.**
      `PortalDisableOwnApiKeys` is conditioned on `aws:ResourceTag/ManagedBy =
      prices-portal`, and the fix above gave the same role
      `PortalTagApiKeysOnCreate` — `apigateway:PUT` on `/tags/…/apikeys/*`
      conditioned on `aws:RequestTag`, which is a condition on the value being
      written, not on which key it is written to. So the two statements are
      not independent: this role can tag any key in the account
      `ManagedBy=prices-portal` and then satisfy the revoke's guard against
      it. Two calls. **IAM cannot close it** — there is no condition key that
      distinguishes tagging a key as it is created from tagging one that
      already exists, and the create cannot name a key that does not exist yet
      (limit 1). Recorded rather than fixed, in `compute-stack.ts` at the
      statement itself: the `ResourceTag` guard is a guard against *our own*
      code disabling a key it did not make — which is the failure mode that
      actually happens — and not a containment boundary against a compromised
      handler. What bounds that instead: limit 1's existing mitigation, a
      free-tier plan this role cannot detach keys from, and the fact that a
      `PUT /tags` not preceded by a `CreateApiKey` is visible in CloudTrail.
      That last one is a detective control nobody has built; it is the
      follow-up this check spawns, not a blocker on it — the check asks for
      named resources and no `apigateway:*`, and both remain true.
      Earlier amendment — **the 08-28 reading was a PASS on shape and a
      FAIL on function, and only the browser walk could tell.** The policy
      named specific resources, as the check asks — and could not create a
      key: `CreateApiKey` with `tags` needs `apigateway:PUT` on
      `/tags/<url-encoded /apikeys/*>`, a fourth grant the comment declared
      deliberately absent. Now present, conditioned on
      `aws:RequestTag/ManagedBy = prices-portal` and `aws:TagKeys ⊆
      {ManagedBy, IssuedBy}`; limit 4 in the policy comment records what the
      condition cannot promise. Still no `apigateway:*`, synth == deployed
      (12:03:41Z). Earlier reading — **2026-08-28: PASS, and unaffected by [[0235]] or the host change.**
      Two inline policies plus `AWSLambdaBasicExecutionRole`; the only
      `apigateway:` actions in any deployed template are `GET`, `POST`,
      `DELETE`, `PATCH`, the string `"apigateway:*"` occurs nowhere, `PATCH` is
      tag-conditioned, and synth == deployed. `POST /apikeys` is documented as
      limit 1 of 3 with its mitigation at `compute-stack.ts`. Report E6 — which
      also records the `GET`/`DELETE` condition decision this task was left.
      The assembled IAM policy names specific resources — no wildcard on
      `apigateway:*` — and the un-narrowable `POST /apikeys` is documented in the
      code as an accepted limit with its mitigation (tagging + attachment to the
      self-service plan only)
- [x] ✅ **2026-08-28: PASS, unaffected.** `PortalCreateAndListApiKeys` grants
      `apigateway:GET` on the collection ARN. Report E7.
      The collection-level `GET /apikeys` is present; without it the reconciler
      fails at runtime with `AccessDenied` and only under concurrency
- [x] ✅ **2026-08-28: PASS, unaffected.** Live key values held in memory only
      and compared against: Logs Insights over all 11 groups for 31 days
      (3 586 940 records, 1.57 GB, 0 matched the 40-char prefilter), 471 X-Ray
      traces / 1 392 segments, 47 CloudFront log lines, and the CloudTrail
      bodies of 2 054 control-plane events. No API Gateway execution logs exist
      — `dataTraceEnabled` is false on every method. Report E8.
      No API key value appears in any CloudWatch log group or X-Ray trace —
      grepped, including error paths
- [x] ✅ **2026-08-28: FAIL → PASS, created the same day.** Was absent
      (`ResourceNotFoundException` in `eu-central-1` and `us-east-1` alike) —
      blocker B1, owner [[0186]], runbook §2. Created 2026-08-28T13:53Z as
      `…:secret:prices/production/portal-discord-oauth-s5Qz1H`, carrying all
      four fields, a `redirect_uri` that ends in `CALLBACK_PATH`
      (`…/api/api/auth/callback`, [[0235]]'s value) and a 64-character signing
      key. The api-handler's `ReadPortalOauthSecret` grant names
      `…portal-discord-oauth-*`, which matches. ⚠️ **The `client_secret` was
      pasted into a chat transcript during this task and must be rotated in the
      Discord Developer Portal, then updated with `put-secret-value`** — see
      the note in Issues Encountered. The other two halves PASS: every
      secret-shaped env var across 11 live functions and 6 templates is a NAME,
      and the deployed bundle carries no secret. **Re-scanned 2026-08-28 after
      [[0235]] rebuilt it** (13 files under `api/`): zero real API key values,
      zero third-party or CDN references. Three 40-character runs are slices of
      an inline base64 asset, and `sf_live_k8mN…Sw4` is the design's placeholder
      key that [[0233]] owns — neither is a credential. Report E9.
      The Discord client secret is in Secrets Manager and in no environment
      variable, and no secret is in the static bundle
- [x] ✅ **2026-08-28: FAIL → PASS, seeded the same day.** Both parameters were
      absent (`ParameterNotFound`, and no `PutParameter` naming either in 90
      days of CloudTrail) — blocker B2, owner [[0189]], runbook §2a, whose own
      ticked criterion had been true of the runbook and not of the account.
      Seeded 2026-08-28T12:14Z by `AWSReservedSSO_AdministratorAccess/adam.kot`
      — operator, not `AWSCloudFormation`, which is the property this check
      asks for. `discord-guild-id` = the **test** guild (19-digit snowflake, so
      it passes the cold-start shape probe; see the design decision on the
      knowingly-wrong landing copy), `min-account-age-minutes` = `5`, matching
      the Stellar guild's own `verification_level: 2` per ADR 0010 §3. The
      restore-half still PASSES: no `AWS::SSM::Parameter` for either name in any
      template, and the Lambda receives only the parameter NAMES. Report E10.
      Both SSM parameters are operator-seeded; a `cdk deploy` does not restore a
      committed guild id
- [x] ✅ **2026-08-31: PASS — the visitor is served our bundle.** After the
      12:33Z sync and invalidation `I7AL28YN5B1JBOA9AZGFOC2KSO`, `api/index.html`
      in `production-soroban-explorer-api-spa` references `index-DLma6J82.js`,
      and that is what Adam's browser rendered at `/api/`, `/api/login`,
      `/api/dashboard` and `/api/quick-start` (hard loads included). Bucket
      posture unchanged from the 08-28 audit. What the earlier reading left
      open — ⏳ **(host)** **2026-08-28: PASS on our bucket, and the sign-off bucket
      is a different one.** `prices-production-portalhosti-portalbucket…`:
      `BLOCK_ALL`, `IsPublic false`, `BucketOwnerEnforced`, OAC sigv4 scoped to
      this distribution by `AWS:SourceArn`, anonymous GET → `403`. Report E11.
      The bundle now also lands in `production-soroban-explorer-api-spa`, whose
      public-access block, policy status, **policy, OAC scoping and anonymous
      GET were all audited on 2026-08-28 and PASS**: `BLOCK_ALL` on all four
      flags, `IsPublic false`, a single `s3:GetObject` grant to
      `cloudfront.amazonaws.com` conditioned on
      `AWS:SourceArn = …distribution/EA2TLS5SS5M87`, and an anonymous GET on
      `api/index.html` answering `403`. What remains for this check at sign-off
      is only that the bundle actually served from there is the portal's —
      and **as of 2026-08-31 the object half PASSES**: the bucket holds 13
      objects under `api/`, synced 09:43Z, and `api/index.html` is ours
      (`<title>Stellar Prices API — API keys</title>`, assets at
      `/api/assets/…`, and no Google Tag Manager — Explorer's document has
      one). The check stays open because what a *visitor* is served at `/api/`
      today is still Explorer's `index.html`, not this object.
      The portal bucket has no public access and is reachable only through OAC
- [x] ✅ **2026-08-28: PASS, unaffected.** Measured 4 calls ≈ 1.3 s cold, 2 ≈
      0.64 s warm, against 2 054 CloudTrail events in 14 days (peak 12/s, 37/min,
      no throttling) and 43-423 `/v1` requests per day. Costed: at
      `PORTAL_THROTTLE`'s ceiling the portal alone would consume 100 % of the
      account's non-adjustable 10 rps control-plane budget, which is a ceiling
      no guild-gated portal will approach. [[0190]]'s two remedies recorded
      ahead of any storage. Report E12.
      Control-plane call volume per dashboard load is known and bounded.
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

## ⚠️ Found after the prefix deploy: the old portal is still served

**Corrected prediction.** [[0235]] and this task both recorded that the
`api-tokens/*` objects left in the bundle bucket would be inert because
"nothing routes to them". That is wrong, and it was measured wrong: probing
`https://dojr4epgxo2qp.cloudfront.net/api-tokens/` on 2026-08-28 after the
deploy returns **`200` with the OLD bundle**.

The path no longer matches any behaviour, so it falls to `DefaultCacheBehavior`
— which is the S3 origin. `DirectoryIndexFn`'s trailing-slash branch rewrites
`/api-tokens/` to `/api-tokens/index.html`, that key still exists, and S3 serves
it. The stale app then calls `/api-tokens/api/config`, which **is** unmapped and
answers `403`, so a visitor holding the old bookmark gets the previous portal
stuck on its "could not reach the backend" state.

Four objects, all disposable and all re-creatable by a deploy:

```
api-tokens/index.html                    556 B    2026-08-27
api-tokens/favicon.ico                15 086 B    2026-08-27
api-tokens/assets/index-BDDTU4A6.js  248 284 B    2026-08-27
api-tokens/assets/index-x1XGuNl0.css       1 B    2026-08-27
```

`prune` is scoped to `destinationKeyPrefix`, so no deploy will ever remove them.
**Deleted 2026-08-28** with Adam's approval, prefix `api-tokens/` only. The
bucket now holds the 13 `api/` objects and nothing else, and
`https://dojr4epgxo2qp.cloudfront.net/api-tokens/` answers `403` — S3 masking
the missing key, which is the honest answer for a path nothing serves — while
`/api/` stays `200`.

## Hosting preconditions (new, 2026-08-28)

None of these is this task's to build; all of them gate its sign-off. A plain
`aws s3 sync` of `web/portal/dist` to `s3://production-soroban-explorer-api-spa/api`
satisfies none of them and produces a blank page — the built `index.html`
carries **absolute** `/api-tokens/assets/…` URLs, which on that host match no
behaviour, fall to the Explorer SPA bucket, and come back as Explorer's
`index.html` with status `200`.

### The target layout, and why the callback keeps the old prefix

The bundle is synced to `s3://production-soroban-explorer-api-spa/api`, which is
origin 2 of `EA2TLS5SS5M87` under behaviour `/api/*` with no `OriginPath`, so
the S3 key `api/index.html` is the URL `/api/index.html`. That fixes the
**bundle** prefix at `/api/`. It does not fix the **backend** prefix, and the
two do not have to match — which is what makes one layout much cheaper than the
other:

| | bundle | backend | what changes |
|---|---|---|---|
| **A (minimal)** | `/api/` | `/api-tokens/api/` | `BASE_PATH` (web, 2 copies) and `PORTAL_HOME` (Rust). Nothing else in Rust, nothing in the gateway, nothing in `methodSettings`. The session cookie is already scoped `Path=/api-tokens/`, so it is still sent to every backend call and never to the bundle — which does not need it. `CALLBACK_PATH` is unchanged, so the secret loader's `ends_with` rule is satisfied by `https://sorobanscan.rumblefish.dev/api-tokens/api/auth/callback` |
| **B (convention)** | `/api/` | `/api/api/` | [[0161]]'s `<app>/*` + `<app>/api/*` shape, but it moves `PORTAL_API_PREFIX`, `CALLBACK_PATH`, `PORTAL_HOME`, `SESSION_PATH`, `PENDING_PATH`, the gateway resource path `/api-tokens/api/{proxy+}` and its three `methodSettings` entries, plus the Discord registration. Every one of those is a deploy, and the gateway pair is the change [[0184]] records breaking production for twenty minutes |

**Superseded again 2026-08-31: neither A nor B — the prefix is flat.** See
"The prefix, flattened" below: bundle and backend both live directly under
`/api/`, with no sub-prefix for either. The paragraph that follows is the
2026-08-28 record.

**Superseded 2026-08-28: B is what shipped, not A.** The code now reads
`BASE_PATH = '/api/'` (`web/portal/src/base-path.ts` and `vite.config.mts`),
`PORTAL_API_PREFIX = "/api/api/"` and `PORTAL_HOME = "/api/"`
(`portal/mod.rs`, `portal/auth/mod.rs`), `CALLBACK_PATH =
"/api/api/auth/callback"` and the gateway resource `/api/api/{proxy+}`
(`api-gateway-stack.ts`) — [[0235]]'s prefix deploy, and the paths the re-run
checks above were measured on. The recommendation below is kept as the record
of why it was a live choice; it is no longer the plan. The Discord redirect URI
must therefore be registered as
`https://sorobanscan.rumblefish.dev/api/api/auth/callback`.

**A was the recommendation** and it is what the redirect URI below assumed. It is
asymmetric on purpose; [[0161]]'s convention is worth re-deciding once, in 0195,
rather than paid for during a cutover.

Either way, these remain and are not solved by choosing a layout:

- **Deep links and refresh.** With the bundle at `/api/`, the app's routes are
  `/api/login`, `/api/dashboard`, `/api/quick-start`. On `EA2TLS5SS5M87` a
  refresh on one resolves to a missing S3 key, and `CustomErrorResponses` turns
  the 403 into **Explorer's** `index.html` with status `200` — the visitor gets
  the block explorer, not the portal. `PortalHostingStack` solves this with the
  `DirectoryIndexFn` allow-list; that function is on OUR distribution and does
  not exist on Explorer's. This is [[0195]]'s per-prefix SPA fallback, and here
  it is a requirement rather than a nicety
- The four Explorer-side items listed below (origin, methods, basic-auth,
  `CustomErrorResponses`)

- [x] ✅ **Done ([[0235]], 2026-08-28).** The bundle is built for the prefix it
      is served from — `BASE_PATH = '/api/'` in `web/portal/src/base-path.ts`
      and its copy in `vite.config.mts` (`base-path.spec.ts` fails if they
      drift). What is NOT done is the sync of that bundle to
      `s3://production-soroban-explorer-api-spa/api`
- [x] ✅ **Done 2026-08-31.** The bundle is synced to
      `s3://production-soroban-explorer-api-spa/api/` — 13 objects at 09:43Z,
      and it is the right one (`api/index.html` carries the portal's title and
      `/api/assets/…` URLs). This closes the sync half of the first item above.
> **Amended 2026-08-31 (decision A):** the four Explorer-side items below —
> origin, methods, basic-auth, `CustomErrorResponses` — reduce to **one**:
> basic-auth off. The backend is reachable on its own hostname and the bundle
> calls it there; see "The backend on its own host".

- [x] ✅ **Superseded by decision A** — the backend is reachable on its own
      hostname and the bundle calls it there (same-site, not same-origin).
      ~~The portal's backend is reachable **same-origin** on
      `sorobanscan.rumblefish.dev`~~ — superseded: a second origin for
      `02mabge71l.execute-api.eu-central-1.amazonaws.com` and a behaviour ahead
      of the bundle row, with `ALLOW_ALL` methods, `CachingDisabled` and
      `AllViewerExceptHostHeader`. Same-origin is not a preference — [[0186]]'s
      session cookie is `SameSite=Lax` and the design keeps CORS out of portal
      traffic entirely. Owner: `soroban-block-explorer` repo, requested by [[0195]]
- [x] ✅ **Superseded by decision A** — `POST` reaches the backend on
      `prices-api…` directly (logout `204`, rework preflight `204`, key issue
      walked); no Explorer behaviour is involved. ~~`POST` and `DELETE` reach the portal's backend there — the current
      `/api/*` behaviour allows `GET`/`HEAD` only, so key issue, rework, revoke
      and sign-out cannot work. Owner: same~~
- [ ] ⏳ **The one item that survives decision A, re-scoped:** the basic-auth
      function no longer sits in front of any portal *route* (those are on
      `prices-api…`), only in front of the *page* — `enableApiSpaBasicAuth:
      true` in the Explorer's `production.json` answers `401` to anyone
      without the staging credentials. It gates public availability, not this
      task's correctness; tracked as a follow-up for the Explorer repo. ~~The
      basic-auth CloudFront function does not intercept the portal's routes.
      Owner: same~~
- [x] ✅ **Superseded by decision A** — no portal error response passes
      through `EA2TLS5SS5M87` any more (check 1). ~~`CustomErrorResponses` does not rewrite the portal's error responses (see
      the first check). Owner: same — and it is distribution-wide, so it affects
      the Explorer SPA too~~
- [x] ✅ **Met by the Explorer's PR #437 (deployed 2026-08-31):**
      `api-spa-routing` redirects bare `/api` → `/api/`, rewrites extensionless
      paths to `/api/index.html`; verified in the browser on `/api`,
      `/api/dashboard` and `/api/quick-start` as hard loads. Original —
      **NEW, found 2026-08-31:** a per-prefix SPA fallback exists for `/api/*`,
      and `/api` (no trailing slash) redirects to `/api/`. Neither works today:
      `/api/` finds only a zero-byte `api/` placeholder key, and `/api` does not
      match the `/api/*` pattern at all, so it falls to `DefaultCacheBehavior`
      and serves **Explorer's** SPA at `200`. Owner: same. Our own distribution
      solves both with `DirectoryIndexFn`, which does not exist on
      `EA2TLS5SS5M87`
- [x] ✅ **Done in code ([[0235]], 2026-08-28), one item outstanding.** The
      prefix change is carried through `PORTAL_API_PREFIX` and `CALLBACK_PATH`
      (Rust), the gateway resource path and its three `methodSettings` entries,
      and the session cookie's `Path` — layout B, [[0161]]'s convention.
      ⚠️ **The Discord redirect URI is the one place it is not carried
      through**: it is registered in the Developer Portal, not in this repo, and
      it must read `https://sorobanscan.rumblefish.dev/api/api/auth/callback`
      before the first sign-in. Owner: Adam, at the same time as the
      `client_secret` rotation. **Amended 2026-08-31:** the callback is now
      `…/api/auth/callback` (flat prefix, below) — registered once at
      2026-08-31 with the old path, so it needs registering again

## Opening the portal

**This task owns the flip.** `PORTAL_ENABLED` goes to `'true'` in
`compute-stack.ts` here and nowhere else — not as a side effect of anyone
finishing their own slice. **Committed 2026-08-28 and DEPLOYED 2026-08-31
10:43Z** — see "The flip is live" below. The gate mattered because with the flag
true the
handler resolves **four** sources at cold start and a missing one is an `Init
Errors` event on the Lambda that also serves `/v1`. Three are the ones this list
already tracked; the fourth is `PORTAL_FREE_PLAN_PARAM` —
`/prices/{env}/pricing-api-free-plan-id`, published by `ApiGatewayStack`, which
deploys *after* `ComputeStack`. It is the only one of the four no operator
seeds, so it is the one that goes missing on a fresh environment or a replaced
usage plan. All four are verified below.

Preconditions, all of them:

- [x] [[0189]] has passed: a non-member is refused, and a Discord `429`/`5xx`
      does not read as "not a member" — on the evidence available while closed
      (2026-08-28 report, gate §1)
- [x] ✅ **2026-08-31:** every check passes or is not applicable by decision
      — checks 1, 2 and 11 measured on the layout actually serving the
      portal (the API's own hostname + the Explorer's static `/api/*`),
      check 5 verified not applicable. Every check in the list above passes, the three **(host)** ones against
      `EA2TLS5SS5M87`
- [x] ✅ **2026-08-31, as amended:** four of the five superseded by decision
      A and one met by #437; the basic-auth gate is the Explorer's
      public-release switch, tracked as a follow-up, not a precondition of
      the portal working. Every hosting precondition above is met
- [x] ✅ **2026-08-28: all three now exist** — created the same day after this
      line was first written, when none of them did. The OAuth secret at
      `…:secret:prices/production/portal-discord-oauth-s5Qz1H` (13:53Z, four
      fields, parsed — check 9), `/prices/production/discord-guild-id` and
      `/prices/production/min-account-age-minutes` (12:14Z, operator-seeded —
      check 10). ⚠️ **The deploy that carries the flip is still gated**, on the
      `client_secret` rotation recorded in Issues Encountered — the stored value
      is the one that leaked into a transcript. Rotate, `put-secret-value` all
      four fields with `session_signing_key` unchanged, then deploy
- [x] ✅ **Done 2026-08-28.** Keys created while the flag was off are
      enumerated (one, `smdesqkg5j`, `discord-1534…537-key`, created
      2026-08-26T15:11:57Z, `ManagedBy=prices-portal`, 155 requests in August)
      **and deleted** at 14:0xZ. A fresh listing confirms no `discord-*` key
      remains and that the free plan `71t9im` holds only `t61phbbhhj`, the
      CDK-managed key. The other two in the account are the Explorer partner
      key and [[0121]]'s load-test key, neither of which is the portal's. There is no
      separate incubation plan (decided 2026-08-13), so those keys are real keys
      on the real free-tier plan and would otherwise survive into its accounting
      as anonymous strings. They come from local runs against production
      credentials, not from the closed portal — the flag lives in the Lambda

### Deploy readiness — measured 2026-08-31

The deploy itself is now **technically unblocked**; what still gates it is the
`client_secret` rotation, which is a security decision, not a missing piece.

**All four cold-start sources exist in production** (`eu-central-1`, account
`750702271865`):

| # | source | value | seeded |
|---|---|---|---|
| 1 | `prices/production/portal-discord-oauth` | 4 fields, `client_secret` 32 ch, `session_signing_key` 64 ch | operator, 2026-08-28 |
| 2 | `/prices/production/pricing-api-free-plan-id` | `71t9im` | `ApiGatewayStack`, v1, 2026-08-12 |
| 3 | `/prices/production/discord-guild-id` | `1536303837785362432` (test guild) | operator, 2026-08-28 |
| 4 | `/prices/production/min-account-age-minutes` | `5` | operator, 2026-08-28 |

Source 2 is the one the checks never covered, and it turns out to have been
published two weeks before the portal work started — so the ordering hazard
described above is real but not live in this environment.

**The stored `redirect_uri` names the wrong host.** It reads
`https://dojr4epgxo2qp.cloudfront.net/api/api/auth/callback`, not
`sorobanscan.rumblefish.dev`. This does **not** block the deploy: `secret.rs`
validates with `ends_with(CALLBACK_PATH)` only, which is host-agnostic by
design, so the cold start succeeds. It does block the first real sign-in at the
sign-off host, and it has to be changed in the same `put-secret-value` as the
rotation — together with the Discord Developer Portal registration, which is the
other copy of the same string.

**Build assets are fresher than production, for the two functions that matter.**
`target/lambda/prices-api/bootstrap` was rebuilt 2026-08-31 09:50 and carries
`/api/api/auth/callback`; `prices-ledger-processor` is from 2026-08-28 14:32.
Both deployed functions were last modified 2026-08-28 13:17, so `ComputeStack`
ships forward, not backward — the [[0141]] footgun does not bite here.

### ⚠️ Deploy `Prices-production-Compute` ALONE — never `deploy-production`

    make deploy-production-compute     # ComputeStack + flush-production-cache

`make deploy-production` deploys `--all` and would ship nine EventBridge
functions off local disk. `target/lambda/oracle-worker` is from **2026-08-13**,
while `prices-production-oracle` was deployed 2026-08-28 14:21 and now carries
[[0231]]'s `metrics.rs` — so `--all` would regress the oracle worker by two
weeks, remove the metrics [[0231]]'s alarms fire on, and report success. Exactly
the [[0141]] shape, one stack over.

This branch was also **5 commits behind `origin/develop`** until 2026-08-31, and
in that state `cdk diff` proposed destroying four live Oracle alarms
(`OracleDarkFeedAlarm`, `OracleTimestampRejectedAlarm`,
`OracleWorkerDurationAlarm`, `OracleWorkerNoInvocationsAlarm`), their two
outputs, and the `PublishOracleMetrics` IAM statement — none of it a decision,
all of it staleness. Merged, and the diff is now:

| stack | diff |
|---|---|
| Secrets, **ApiGateway**, **PortalHosting**, Observability | no differences |
| **Compute** | `PORTAL_ENABLED: false → true` + 2 asset hashes — **this is the deploy** |
| EventBridge | 9 asset hashes, all stale-local — **do not deploy** |

`ApiGateway` and `PortalHosting` showing no differences is itself evidence:
[[0235]]'s prefix work is fully deployed, so the flip does not carry a gateway
change with it.

**Amended 2026-08-28.** The original text here said the flip was not gated on
[[0193]] or [[0195]], on the reasoning that opening a plain-looking portal that
works beats leaving a finished one closed. [[0193]] has since merged, and the
decision to sign off at `sorobanscan.rumblefish.dev/api/` rather than at the
distribution domain **couples this task to [[0195]]**: the host that must be
verified is the one 0195 delivers. The flag itself is still independent of both
— it lives in the Lambda, which answers on either host — so the flip can be
deployed as soon as the secret and the parameters are seeded. What waits for
0195 is this task's sign-off, not the portal being open.

## The flip is live — 2026-08-31

Deployed with `make deploy-production-compute` at 10:43Z after the
`client_secret` rotation. `ComputeStack` only, for the reason recorded above;
`cdk diff` immediately before the deploy was byte-identical to the one taken
after the `develop` merge — `PORTAL_ENABLED: false → true` plus two asset
hashes, no removals, no IAM change.

**The secret was rotated first.** New version `3ff8ea37…` is `AWSCURRENT`, the
leaked `42a83ae9…` is `AWSPREVIOUS` and is now inert — Discord invalidated that
value at rotation, so the transcript exposure is closed. `session_signing_key`
carried through unchanged (64 ch), so no session was invalidated;
`redirect_uri` was corrected to the sign-off host in the same write. All four
rules in `secret.rs::parse` were checked against the stored value before the
deploy, not after.

**Cold start passed all four reads.** `Errors` 0 / `Invocations` 4 on
`prices-production-api-handler` over the deploy window — the failure mode this
task spent its length worrying about did not occur.

Measured on `02mabge71l.execute-api…/production` immediately after:

| probe | before | after |
|---|---|---|
| `/api/api/config` | `{"enabled":false,…}` | `{"enabled":true,…}`, still `cache-control: no-store` |
| `/api-docs-json` (`/v1` router canary) | `200` | `200`, 0.17-0.19 s |
| `/v1/assets` keyless | `403` | `403` |
| `/api/api/auth/login` | [[0183]]'s empty `404` | **`303`** to `discord.com/oauth2/authorize` |
| `/api/api/key`, `/api/api/usage` | [[0183]]'s empty `404` | **`401`** (no session) |

The `303` carries `client_id=1537116138427781190`, `scope=identify
guilds.members.read`, `response_type=code`, a `state`, and
`redirect_uri=https://sorobanscan.rumblefish.dev/api/api/auth/callback` — so the
rotated secret is wired end to end.

**Consequence, stated so it is not discovered later:** because `redirect_uri`
now names the sign-off host, the flow can **no longer** be walked on
`dojr4epgxo2qp.cloudfront.net`. Discord will bounce every callback to
`sorobanscan.rumblefish.dev`, which has no backend behaviour until [[0195]]
lands. That is the intended end state, not a regression — but it means there is
now **no host on which a full sign-in can be tested** until 0195 delivers. The
backend is open; the door it opens onto is not built yet.

## The prefix, flattened — 2026-08-31

**Decided by Adam, 2026-08-31, implemented here:** `/api/` is the whole
self-service portal on the shared host, and there is no sub-prefix for the
backend. `/api/login` is a page, `/api/auth/login` is the backend,
`/api/api-docs-json` is the OpenAPI document. Nothing of ours lives at the root
of `sorobanscan.rumblefish.dev`, because the root is the block explorer's.

This is a `TEST` task writing code, against its own note. Adam's explicit call
("zrób to teraz w tym tasku"), made with the timing argument in view: the
Explorer repo had not yet built its behaviour for the old prefix, so this was
the last moment a prefix change cost one deploy on our side rather than a
coordinated change across two repos.

### Why not a sub-prefix

The rename started as "avoid `/api/api`", which [[0161]]'s `<app>/*` +
`<app>/api/*` convention produced for an app that is itself called "api". Two
layouts were on the table — a sibling `/portal-api/*` and a nested
`/api/portal/*` — and both lost to a rule stated by Adam that decides more than
this rename: **the prefix `/api` is the portal, and everything after it is
just `/api/<rest>`.** A sibling takes a second top-level prefix in someone
else's namespace; a nested sub-prefix is the thing being avoided under a
different name.

### What flat costs, and what it buys

Bundle and backend now share a prefix, so **one side has to be enumerated**.
The bundle is: `/api/`, `index.html`, `favicon.ico`, `assets/*`, and the three
SPA routes in both slash forms — ten CloudFront rows, carved out to S3 **ahead
of** an `/api/*` catch-all that goes to the API. That inverts the old table
(backend row ahead of a bundle catch-all), and the inversion is the point:

- the backend is the open-ended side (five slices added routes; none touched
  the CloudFront table) and now needs no row at all;
- a bundle path missing from the list reaches the API and fails **loud** — a
  JSON `404` in the network tab — where the old shape's failure was the silent
  `200 text/html` that took this task most of a day to diagnose.

Costs, stated: the session cookie's `Path` can no longer be narrower than
`/api/` (it rides on asset requests; `HttpOnly`, and S3 ignores it); the
bundle list is maintained in `portal-hosting-stack.ts` (`PORTAL_APP_ROUTES`,
which generates both the rows and `DirectoryIndexFn`'s allow-list),
`verify-openapi-routes.mjs` (asserts every carve-out resolves to S3 and
`/api/probe` to the API), and `links.ts`; and the Vite dev proxy becomes a
regex over the backend's top-level segments, because a plain `/api` rule would
swallow Vite's own `/api/@vite/…`.

### What changed

One substitution `/api/api` → `/api` across 27 files (the [[0235]] method; two
escaped regexes in `app.spec.tsx` needed a second pass), then by hand:

- **Rust** — `PORTAL_API_PREFIX = "/api/"`, every `*_PATH` under it,
  `PENDING_PATH = "/api/auth/"`, `SESSION_PATH` unchanged at `/api/`. New
  `OPENAPI_PATH = "/api/api-docs-json"`: `lib.rs` mounts the one spec handler
  at both paths, `gate_portal` and `auth::is_exempt` exempt it by name like
  `CONFIG_PATH`. Gateway resource is `/api/{proxy+}`, so the alias needs no
  infra. Tests: the bundle paths are now *inside* the prefix (a plain `404`
  in both states if one ever arrives), and the alias answers in both states
  byte-identical to the root copy.
- **Infra** — `api-gateway-stack.ts`: `/api/{proxy+}` with the same three
  `methodSettings`. `portal-hosting-stack.ts`: `PORTAL_BUNDLE_PATHS` rows
  before `PORTAL_BACKEND = '/api/*'`; `REDIRECTS` loses `/api/api`.
- **Web** — `PORTAL_API = '/api'`, `OPENAPI_JSON = '/api/api-docs-json'`,
  dev-proxy regex + a test that it proxies the backend segments and leaves
  `/api/@vite/client`, `/api/src/…`, `/api/login` and `/api/keys` alone.
- **Docs** — README, runbook, `docs/scf/api-endpoints.md`, the epic.
  `lore/` records are not rewritten ([[0235]]'s rule); [[0195]]'s convention
  text is now stale and is noted, not edited.

Verified: Rust 405 passed / 0 failed; portal 157/157; lint + typecheck green;
`openapi:verify-routes` green with the new carve-out check; `cdk diff`:
Compute = one asset hash (the rebuilt api-handler — the disk binary was
pre-change and diffed as "no differences" until rebuilt, [[0141]] again),
ApiGateway = `/api/api/{proxy+}` → `/api/{proxy+}` plus the three
`methodSettings`, PortalHosting = the ten rows + catch-all and the function.

### Deploy order, and the suffix that makes it safe

`secret.rs` validates `redirect_uri` with `ends_with(CALLBACK_PATH)`, and the
stored `…/api/api/auth/callback` **ends with** the new
`/api/auth/callback`. So the new binary accepts the old secret and the order
is **code first, then secret + Discord** — no cold-start window in which
`/v1` could go down. Until the secret is updated, sign-in starts with the old
callback and lands on a `404`; nothing crashes.

**The reverse is not true.** Old code with the new secret value fails
`ends_with` at init and takes `/v1` down. A rollback of this deploy must
revert the secret **before** the code.

Stacks: `Compute`, `ApiGateway`, `PortalHosting` by name — never
`deploy-production` (the EventBridge asset regression recorded above). The
Explorer bucket is then re-synced from `web/portal/dist` (the bundle bakes
`PORTAL_API` in). One resource moves in the gateway; [[0184]]'s "at most one
variable child" rule is not touched, because `/api/{proxy+}` and the departing
`/api/api` are a variable and a literal under the same parent, and [[0235]]
made the same-shaped move in one deploy.

### Deployed — 2026-08-31, 10:06–10:11Z

Compute 10:06Z (22 s), then ApiGateway (27 s) and PortalHosting (198 s,
CloudFront propagation) at 10:08–10:11Z, then the stage cache flushed. **Not in
one command, and the gap was a bad state** — see Issues Encountered:
`--require-approval broadening` stopped after Compute, so for ~90 s the new
Lambda (routes at `/api/*`) sat behind the old gateway (`/api/api/{proxy+}`)
and every portal call was a `404`. `/v1` was unaffected throughout; `Errors` 0
over the window (8 invocations, all of them this task's probes).

Secret: `redirect_uri` → `https://sorobanscan.rumblefish.dev/api/auth/callback`,
version `bcea66cb…` is `AWSCURRENT`, the rotated-but-old-path `3ff8ea37…` is
`AWSPREVIOUS`; the other three fields carried through unchanged (`client_id`
19, `client_secret` 32, `session_signing_key` 64). Bundle re-synced to
`s3://production-soroban-explorer-api-spa/api/` (new `index-B30WO_hN.js`; the
previous chunk left in place on purpose — no `--delete`, a visitor holding the
old `index.html` still resolves) and `/api/*` invalidated on `EA2TLS5SS5M87`
(`I4I30TL0ID0ZRUMEN9SMA157FN`, completed).

Measured after, on `02mabge71l.execute-api…/production` and on our own
distribution alike:

| probe | result |
|---|---|
| `GET /api/config` | `200` JSON `{"enabled":true,…}`, `no-store` |
| `GET /api/api-docs-json` | `200` JSON, byte-identical to `/api-docs-json` |
| `GET /api/auth/login` | `303`, `redirect_uri=…/api/auth/callback` |
| `GET /api/key`, `GET /api/usage` | `401` JSON, `no-store` |
| `POST /api/key/rework` | `403` JSON, `no-store` |
| `POST /api/auth/logout` | `204` |
| `GET /api/api/config`, `/api/api/auth/login` (old) | `404` JSON |
| `/api/`, `/api/index.html`, `/api/favicon.ico`, `/api/assets/*`, `/api/login`, `/api/dashboard/`, `/api/quick-start` (our CDN) | `200` from S3 — the carve-outs |
| `GET /api` (our CDN) | `302` → `/api/` |
| `GET /api/nope` (our CDN) | `404` JSON — the loud failure, as designed |
| `/api-docs-json`, `/health`, `/v1/assets` | `200`, `200`, `403` — unchanged |

⚠️ **Still owed, and only Adam can do it:** register
`https://sorobanscan.rumblefish.dev/api/auth/callback` in the Discord
Developer Portal. Until then a sign-in started anywhere is refused by Discord at
the authorize step with its own error page — nothing in our logs.

## The backend on its own host — 2026-08-31

**Decision A, Adam, 2026-08-31 ("A, rób dalej").** The API gets a hostname —
`prices-api.sorobanscan.rumblefish.dev`, a REGIONAL custom domain on the
REST API mapped at the root to the `production` stage — and the bundle at
`https://sorobanscan.rumblefish.dev/api/` calls it **directly**: cross-origin,
same-site. The Explorer distribution stays exactly as PR #437 left it (static
`/api/*` SPA + routing function); it needs no API origin, and the
requirements document in `audit/` is superseded (banner added there).

### Why this and not the CloudFront proxy

The measured failure ("`/api/config` answered `200`, not JSON") is #437's
routing function rewriting `/api/config` → `/api/index.html`. The proxy
layout needed four changes in the other repo, all still open; this one needs
none. It is also the block explorer's own pattern — its SPA on
`sorobanscan.rumblefish.dev` calls its API on `api-sorobanscan.rumblefishdev.com`
with `allowOrigins: [domainName]` — so the host now carries two apps built
the same way. Option B (custom domain on our own distribution, zero code) was
on the table and lost because it moves the page off the host decided on
2026-08-28.

### What "same-site" buys and what it costs

`sorobanscan.rumblefish.dev` and `prices-api.sorobanscan.rumblefish.dev`
share the registrable domain, so `SameSite=Lax` **does** send the session
cookie on a `fetch` between them — provided the request asks
(`credentials: 'include'`) and the answer allows it. That is the whole
arrangement:

| side | change |
|---|---|
| gateway | `addCorsPreflight` on `/api/{proxy+}` — one origin (`portalWebOrigin`), credentials, `X-Requested-With`, max-age 1h; MOCK, no Lambda. `OPTIONS` joins `portalSettings` (both arms — check 3's count is now 14 vs 6). `DEFAULT_4XX/5XX` gateway responses carry the same CORS headers, so a `429` reads as a `429` in the browser and not as a network error |
| handler | `PORTAL_WEB_ORIGIN`: `CorsLayer` on the portal routes only (`AllowOrigin::list([ours])` + credentials predicate — `exact` would stamp our origin on every answer); `AuthState.home` = `{origin}/api/` so every landing is absolute; `is_same_origin_write` accepts `Sec-Fetch-Site: same-site` **only** with `Origin == PORTAL_WEB_ORIGIN` — the page is a sibling host now, and so is every other subdomain |
| bundle | `API_ORIGIN` from `VITE_PORTAL_API_ORIGIN` (empty → relative, as before: dev, preview and our distribution unchanged); `credentials: 'include'` on every call; `OPENAPI_JSON` → `https://prices-api…/api-docs-json` on that build |
| cookies | unchanged — `Path=/api/`, no `Domain`, set on the API host by the callback |
| infra | own DNS-validated certificate (the wildcard in eu-central-1 is `InUseBy: []`, renewal `INELIGIBLE`, expires 2026-12-06 — nobody's), A + AAAA in `Z10396861CRMUIWWA8TL9`, `make sync-portal-explorer` formalises the hand sync |

Cost, stated: the CSRF story now has three legs instead of two (marker
header, fetch metadata, **and** the origin allow-list), and they are
configured in two places that must agree — `portalWebOrigin` in
`production.json` feeds both. The redirect URI moves to the API host, which
is a Discord registration and a `put-secret-value`, both Adam's.

Verified: Rust 416 passed / 0 failed (new: CORS answers name one origin and
no other, no header without configuration, preflight allows the marker, the
layer stops at the prefix, a closed portal answers a preflight `404`; the
landing is absolute with an origin configured; a same-site revoke is
accepted from the configured origin and refused from a sibling, without
`Origin`, or without the marker); portal 159/159; lint, typecheck, clippy
`-D warnings`, `openapi:verify-routes` green.

### Deploy order

1. `make -C infra deploy-production-apigateway` — certificate (DNS validation,
   ~3–5 min), domain, records, `OPTIONS`, gateway responses. Nothing the
   bundle uses yet; `/v1` unaffected.
2. `make -C infra deploy-production-compute` — `PORTAL_WEB_ORIGIN` + the
   rebuilt api-handler. From here every landing is absolute; sign-in still
   fails at Discord until step 4.
3. `make -C infra sync-portal-explorer` — the absolute build to the Explorer
   bucket + `/api/*` invalidation.
4. **Adam:** Discord redirect URI →
   `https://prices-api.sorobanscan.rumblefish.dev/api/auth/callback`, then
   `put-secret-value` with the same `redirect_uri` (other three fields
   unchanged; `ends_with(CALLBACK_PATH)` still holds, so the code accepts old
   and new alike — no cold-start window).
5. **Explorer repo:** `enableApiSpaBasicAuth: false` before the portal is
   public. Until then `/api/` answers `401` to strangers.

### Deployed — 2026-08-31, 11:41–11:48Z

Steps 1–3 run by Adam's command, in order. `deploy-production-apigateway`
deployed **Compute first** (22 s, 11:41:57Z — cdk follows the cross-stack
dependency), then ApiGateway (203 s: certificate `CREATE_COMPLETE` 11:44:57Z,
domain 11:45:01Z, A/AAAA 11:45:35Z). `deploy-production-compute` afterwards
was "no changes" plus the stage cache flush. `sync-portal-explorer` at
11:48Z: new chunk `index-BFiUxyFn.js` carrying the API origin, invalidation
`I2V92N568S0479E2BCEPK5UIL2`. `Errors` 0 on the api-handler over the window.

Measured on `prices-api.sorobanscan.rumblefish.dev`:

| probe | result |
|---|---|
| `GET /api/config`, `Origin: https://sorobanscan.rumblefish.dev` | `200`, `ACAO` = that origin, `Allow-Credentials: true`, `Vary: origin`, `no-store` |
| same, `Origin: https://evil.example` / no `Origin` | `200`, **no** CORS header |
| `OPTIONS /api/key/rework` (POST + `x-requested-with`) | `204` from the gateway MOCK: `GET,POST,DELETE`, `Content-Type,Accept,X-Requested-With`, `max-age: 3600`, credentials |
| `GET /api/key` + Origin | `401 not_signed_in` **with** CORS |
| `GET /nope` + Origin | gateway `403` **with** CORS (gateway responses work) |
| `GET /api/auth/login` | `303` → Discord, `redirect_uri` still the **old** host — step 4 pending |
| `/v1/assets` keyless / `/api-docs-json` / `/health` | `403` / `200` / `200`, unchanged |

Check 1's (host) half and check 2's replacement are therefore **measured
and PASS**; check 5 is not applicable by decision.

**Step 4 done, 11:50–11:55Z.** Adam registered
`https://prices-api.sorobanscan.rumblefish.dev/api/auth/callback` in the
Developer Portal; the secret was rewritten in one pipeline (`get` → `jq
.redirect_uri` → `put` via stdin, no value on screen): version `e73d09a7…`
is `AWSCURRENT`, `bcea66cb…` is `AWSPREVIOUS`, four fields, `client_secret`
32 and `session_signing_key` 64 characters unchanged. The `303` carried the
old URI for another three minutes — see Issues Encountered — and reads
`redirect_uri=https://prices-api.sorobanscan.rumblefish.dev/api/auth/callback`
since 11:55:39Z, with `Errors` 0 across the recycle.

Still open: step 5 (Explorer's `enableApiSpaBasicAuth: false`) — until then
`/api/` answers `401` to strangers — and the browser walk-through of a full
sign-in and key issue, which is this task's remaining acceptance criterion.

### Every function walked — 2026-08-31, 12:05–12:35Z

After the key issue, the rest of the portal in Adam's Chrome on the sign-off
host, plus the shell measurements the browser cannot make:

| function | result |
|---|---|
| the issued key on `/v1` (value kept in the shell) | `assets`, `assets/native`, `…/price`, `backfill/status` → `200`, 0.19–0.45 s; keyless `403`; a burst of 8 → 6×`200` then `429` (plan 1 rps / burst 5) |
| dashboard after issue | Key ID, Issued/Last updated, Discord account, the once-per-period cap naming `1 September 2026`; Monthly Usage `0 / 100 000`, Resets 1 September (`GetUsage` still 0 after 13 calls — AWS's delay, as the panel says) |
| Quick start chrome | TOC anchors, cURL/JS/Python/Go tabs, Copy buttons — all work |
| OpenAPI Docs link | `https://prices-api…/api-docs-json`, opens |
| deep links on the Explorer host | `/api/dashboard` hard load, bare `/api`, `/api/login` while signed in → dashboard; #437's routing function + the SPA's redirects |
| sign out → landing; `/api/dashboard` signed out → landing; `/api/quick-start` signed out → public page | ✅ (`POST /auth/logout` shown as `503` by the extension once — gateway `5XXError` 0, Lambda `Errors` 0, `curl` → `204` with the cookie cleared, UI behaved as success; an extension artefact, not reproduced) |
| second sign-in | adopted the existing key `31z25psyn7` (Issued 31 August) — no second key, as designed |
| "Copy key" | click accepted; the "Copied" confirmation was not captured under automation — not a finding |
| api-handler, 4 h | 217 invocations, `Errors` 0, `Throttles` 0, p50 1.4 ms, p95 422 ms, max 1.25 s, 10 cold starts (init 261–467 ms) |

**Not walked: "Regenerate".** With a key issued today it revokes and issues
nothing until 1 September; the walk stopped short of leaving Adam keyless.
The write path it uses (`POST /api/key/rework`, same-site + `Origin`) is
covered by `tests/portal_rework.rs` and by the live preflight measurement.

**Content finding, fixed the same day (`64e22da`, deployed 12:33Z, bundle
synced, invalidation `I7AL28YN5B1JBOA9AZGFOC2KSO`).** The hero, the landing
endpoints section and the whole quick start rendered the DESIGN's API —
`GET /v1/prices/XLM-USDC`, `/pools`, `/history`, `liquidity`,
`source: "soroswap"` — on the execute-api origin. With a freshly issued key
the quick start's own "First request" answered `403 Missing Authentication
Token`. Every example now reads off the live API on
`https://prices-api.sorobanscan.rumblefish.dev/v1`: the seven real routes
with their query parameters, `/assets/native/price`'s fields (decimal
strings), the real error bodies, and `apiBaseUrl` → the hostname so the
OpenAPI `servers` block says the same (`openapi:verify-servers`,
`links.spec.ts`). Task 0233's portal half is closed by this.

## Code review of the whole branch — 2026-08-31

Run after the walk, over everything this task and [[0235]] landed: the flat
prefix, the custom domain, CORS, the tag-on-create IAM fix and the example
rewrite. Six findings, all fixed in one commit; none of them changes a check's
verdict, and one of them amends the *reasoning* under check 6 (recorded there).

| # | where | what |
|---|---|---|
| 1 | `QuickStart.tsx` | Two snippets declared `prices` and used `price` — the JS one a `ReferenceError` for anyone who retyped what was on screen, the Rust one not compiling. Left by the 08-31 example rewrite: the rename reached `text` and only part of `view` |
| 2 | `QuickStart.spec.tsx` (new) | Every snippet is authored **twice** — coloured JSX for the reader, a plain string for the Copy button — and nothing tied the two. Both Copy buttons wrote correct code, which is exactly why #1 could ship. The spec renders each `view` and compares its text to `text`; `SNIPPET_TABLES` is the list to extend when a snippet table is added |
| 3 | `api-gateway-stack.ts` | The CORS gateway response was `DEFAULT_4XX`, and a gateway response is scopable by `ResponseType` and by nothing else — so it stamped `portalWebOrigin` onto every keyless `/v1` `403`, API-wide, and onto requests carrying no `Origin` at all. Narrowed to `THROTTLED`, which is the one 4xx the bundle must tell apart from a dead network. `DEFAULT_5XX` kept, with why written down |
| 4 | `compute-stack.ts` | The revoke's `aws:ResourceTag` guard is not independent of the new tag-on-create grant — see check 6. Documented, not fixable in IAM; spawns the CloudTrail detective control as a follow-up |
| 5 | `vite.config.mts` | The dev proxy's `(/|$)` guard did not match a query directly on a segment, so `/api/config?fresh=1` fell through to Vite and the dev server answered its own `index.html` with a `200` — the same silent wrong-`200` shape this task spent a day diagnosing in production. `(/|\?|$)`, with the case and its negative (`/api/configuration`) in `dev-proxy.spec.ts` |
| 6 | `portal/auth/mod.rs` | Three refusals answered a **browser** with a JSON envelope: `unconfigured` `503`, `refuse_state` `400`, `refuse_discord` `502`. Every caller of all three is a top-level navigation — `/auth/login` is the URL the bundle opens as a popup, and `/auth/callback` is Discord returning — so the visitor got raw text in a window with no way back, and the popup never posted to its opener. All three now land: `?signin=not_open` for the unconfigured deployment, `?signin=failed` for the other two. The argument was already written down one function away and applied to half the traffic — the issue arm has redirected since [[0189]] because "`502` JSON is a dead end with no link back" |

`refuse_query` was deliberately left an envelope: Discord's callback always
carries `code` or `error`, so an arrival with neither is a hand-built URL, not
a visitor part-way through anything.

The three error-code constants the landings retired — `invalid_state`,
`discord_unavailable`, `sign_in_unconfigured` — are gone from the handler, so
no caller can be written against a code that no longer ships.

Verified: Rust 416 passed / 0 failed, clippy `-D warnings` green; portal
suite green including the two new specs; `synth-production` green with
`THROTTLED` in the template. **Not deployed** — findings 3 and 6 are a
production change and land with the merge.

## PR review — 2026-09-01, Oskar on #268

Seven findings, read at `e7704ae` — three commits behind this branch, because
`5635af9` (the branch's own review) had fixed #4 and #6 the evening before and
was never pushed. Every finding is valid; the table is what was done with each.

| # | finding | verdict | what changed |
|---|---|---|---|
| 1 | a portal source failing at cold start panics init on the Lambda that serves `/v1` | **valid, and understated** — next section | `AppConfig::load_portal_or_close`: closed, not crashed |
| 2 | `sync-portal-explorer` uploads with no `Cache-Control` | valid | two `s3 sync` calls mirroring the stack's two `BucketDeployment`s — `assets/*` `public, max-age=31536000, immutable`, everything else `public, max-age=0, must-revalidate` — assets first, as the stack orders them. A fresh build re-stamps every object's mtime, so the next sync re-uploads all of them with headers; nothing needs a one-off fix |
| 3 | `$(shell jq …)` bakes `https://` or `https://null` into the bundle | valid | `// empty` in the filter and a `check-portal-api-origin` prerequisite that refuses anything but `https://<host.with.dot>`. Exercised: the real value passes, `https://null` and `https://` are refused with the cause named |
| 4 | rendered JS/Rust snippets declare `prices` and use `price` | valid — already fixed in `5635af9` (its finding 1, `QuickStart.spec.tsx` as the tie) | pushed with this |
| 5 | "Copy example response" is not JSON | valid | all three venues spelled out in `value` and `raw` alike; the per-venue volumes sum to `volume_24h_usd`; the spec parses `RESPONSE_TEXT` and ties each field's `value` to its `raw` |
| 6 | dev proxy misses `/api/config?x=1` | valid — already fixed in `5635af9` (its finding 5) | pushed with this |
| 7 | `BUNDLE_PROBES` hand-lists the SPA routes | valid | the routes are read from `PORTAL_APP_ROUTES` in the stack source and from `<Route path>` in `app.tsx`, asserted equal, and each one's rewrite is asserted in the synthesized `DirectoryIndexFn`. A route added on one side only, or a stale synth, fails CI — all three cases exercised by hand before the change was kept |

### Finding 1 — what the review understated, and the decision

The comment on the block called the panic deliberate: "fail at deploy, in
`Init Errors`, not at a visitor's click". Measured against the code, three
parts of that were wrong:

- **`cdk deploy` does not fail on an init error.** The deploy succeeds; the
  failure is the next `/v1` caller's `502`. "Fail at deploy" was really "fail
  on the first partner request after the deploy".
- **It was not only a deploy hazard.** The extension client has a 2 s timeout
  and no retry (`prices_clickhouse::mtls`), the flip added three SSM reads to
  a cold start that previously made none, and Parameter Store's default
  throughput is 40 TPS for the whole account. A burst of cold starts —
  [[0121]]'s 100 rps ramp — is exactly where a throttled read would have
  taken `/v1` down.
- **Nobody is paged by `Init Errors`.** `observability-stack.ts` has no alarm
  on the api-handler's `Errors` at all; the only error alarm is the
  ledger-processor's. "Loud" was loud to whoever ran a probe.

Decision (Emerged #4 below): **closed, not crashed.** A failed read closes the
portal in that execution environment — flag off, all three sources dropped,
so no control-plane client survives — and logs `portal closed at cold start`
with the failing variable named. `/config` then answers `enabled: false`,
which is the probe the runbook already makes after every deploy, so a
misconfigured deploy is caught by the same step as before and `/v1` never
notices. `serve.rs` keeps its three panics: a developer who asked for the
portal wants to know now, and no partner is behind that process.

Cost, stated: an environment that failed a *transient* read stays closed for
its lifetime, where the panic discarded it and the next cold start retried.
That is a portal saying "not open" from one environment, traded for a `502`
on the data API. The alarm on the log line is [[0249]] — the same task that
gives the api-handler the error alarm it never had.

Verified: Rust 416 → 418 (two unit tests on the closed-after-failure
property), clippy `-D warnings` green with and without `--features lambda`
(`main.rs` only compiles with it); portal 179/179 after the calendar fix in
Issues Encountered; `format:check`, infra lint/typecheck and
`openapi:verify-routes` green against a fresh synth. **Not deployed** —
lands with the merge, like the 08-31 review's findings.

## Issues Encountered

- **Three portal tests started failing on 1 September 2026, by the calendar.**
  `app.spec.tsx`'s "replace my key" fixtures name real instants —
  `revoked_at: 2026-08-21`, `next_eligible_at: 2026-09-01` — and the app
  compares them to the real clock (`stillWaiting`, `revokedJustNow`,
  `describeNextPeriodStart`). From today `2026-09-01` is no longer ahead, so
  "After 1 September 2026, sign in again" rendered as "A new key can be
  issued now" and three assertions failed — on `develop` too, for everyone,
  with no code change. Fixed by faking `Date` (and only `Date`, so
  testing-library's polling keeps its real timers) at `2026-08-21T12:30Z` in
  that describe: 30 min past the fixture's revocation, inside the period
  whose end the fixtures name. Not a regression of this branch and not one of
  the review's findings; recorded because CI here was red for a reason no
  diff would show.
- **`cargo clippy --features lambda --all-targets` does not compile**, and
  never did: `gateway.rs`'s tests call `Gateway::against`, which is
  `#[cfg(not(feature = "lambda"))]`. CI runs clippy and tests without the
  feature and only *builds* the bins with it, so nothing notices. Not fixed
  here — noted so the next person to check `main.rs` under the feature runs
  `--lib --bins`, which is clean.

- **The api-handler had never been able to create a key in production, and
  every reading of its policy said it could.** Found 2026-08-31 by the first
  real sign-in on the sign-off host: `CreateApiKey` with `tags` is
  authorised as `apigateway:PUT` on `arn:aws:apigateway:eu-central-1::/tags/
  arn%3A…%2Fapikeys%2F*` — separate from `POST /apikeys` — and the role had
  `POST` only. Three attempts, three `AccessDeniedException`s, three
  "landing without one"; the dashboard rendered the honest `Not issued`
  card with the visitor's Discord id. The policy comment listed `PUT
  /tags/*` under "deliberately NOT here (the portal never re-tags a key)",
  true of re-tagging and false of creating, and check 6 on 08-28 read that
  sentence as the statement of intent it claims to be. The only portal key
  that ever existed (`smdesqkg5j`, 08-26) came from a local run under
  operator credentials, which is why nothing had noticed. Fix `a5c920e`,
  deployed with `--require-approval never` after reading the diff (one IAM
  statement, no asset change). **A per-resource IAM audit cannot replace one
  real call**; the AC that demanded the browser walk is the one that caught
  it.

- **A `put-secret-value` alone does not change what the api-handler sends.**
  `load_portal_oauth` reads the secret ONCE, at cold start, into
  `AppConfig::portal_oauth`; the Parameters-and-Secrets extension's cache is
  not the reason (it holds values for minutes, not for the container's life)
  — the value simply lives in memory. Three minutes after the write the
  `303` still carried the previous `redirect_uri`, and with steady `/v1`
  traffic a warm container can outlive any patience. Fix that leaves no
  drift: `aws lambda update-function-configuration --environment` with the
  function's **current** environment, verbatim — an update with identical
  values still recycles the execution environments (`LastModified` moved,
  `md5` of the variables did not). New value observed on the first request
  after `function-updated`. The runbook's cutover ordering (§6) should say
  this; a `cdk deploy` that shows "no changes" does NOT do it.
- **The Discord `client_secret` was exposed in a chat transcript.** On
  2026-08-28 the value was pasted into the working session rather than typed
  into a terminal, so it exists outside Secrets Manager in at least one
  conversation log. The secret was created with it to unblock the flip, and the
  value must be treated as compromised: reset it in the Developer Portal and
  replace the stored copy with `put-secret-value`, passing all four fields and
  **keeping `session_signing_key` unchanged** (rotating that one signs every
  visitor out). Rotating `client_secret` breaks nothing in flight — it is used
  only during the token exchange, so it takes effect on the next sign-in. Until
  it is rotated, anyone holding the transcript can impersonate the application
  against Discord.
- **The orphaned bundle objects were predicted inert and are not** — see the
  section above; `/api-tokens/` still serves the previous portal.
- **The open portal looks shut at the sign-off host, and the cause is a `200`.**
  Reported 2026-08-31: `https://sorobanscan.rumblefish.dev/api/index.html`
  renders the portal shell with no sign-in control, indistinguishable from
  `PORTAL_ENABLED=false`. The flag is `true` and the API answers
  `{"enabled":true,…}` directly. The chain: the page's relative
  `fetch('/api/api/config')` matches behaviour `/api/*`, which targets **S3**
  (there is no API origin on that distribution); S3 has no such key and answers
  `403`; `CustomErrorResponses` turns that into `/index.html` at status **`200`**
  on the *default* origin, i.e. Explorer's SPA. So the probe receives HTML with
  a success status — `!response.ok` passes, the JSON parse fails, and the app
  renders its "cannot reach the backend" state.

  Two things worth keeping. First, `portal.ts:220` **predicted this exact
  failure** in a comment ("a `200` that is not JSON is the signature of the most
  likely routing regression there is here") and wraps it with the status and URL
  rather than leaking a bare `SyntaxError` — the diagnosis took minutes because
  of that comment. Second, the reporter reached it via `/api/index.html`;
  `/api` alone is a *different* failure with the same appearance, because it
  matches no behaviour and serves Explorer directly. Requirements for the owning
  repo: `audit/2026-08-31-explorer-distribution-requirements.md`.

- **Production and `develop` disagree — no longer about the flag, but about
  everything after it.** Amended 2026-08-31 (Adam): `PORTAL_ENABLED` is
  `'true'` on `origin/develop` too (`7e73dc5` merged), so a deploy from
  `develop` would NOT close the portal. It would break it differently:
  `develop` still has the gateway at `/api/api/{proxy+}`, no custom domain,
  no CORS, no `PORTAL_WEB_ORIGIN` and no tag-on-create grant — so
  `cdk deploy` from there deletes the domain, certificate and records, moves
  the resource back, and every call from the bundle on the shared host
  fails again. Merging this branch is still what fixes it. The original
  note — `PORTAL_ENABLED` is
  `'true'` on this branch and live in production, and **`'false'` on
  `origin/develop`** — the flip was deployed from an unmerged branch. Nothing is
  wrong today, but the first `cdk deploy` run from `develop`, by CI or by anyone
  on the team, silently closes the portal again, because from `develop`'s point
  of view `false` is the intended state. Merging this branch is what fixes it;
  no second deploy is needed.

- **`cdk deploy A B C --require-approval broadening` deployed A and stopped.**
  The ApiGateway diff adds `Lambda::Permission` resources for the moved methods
  — an IAM change, so cdk asks for approval, and without a TTY it aborts
  silently after the first stack. My grep for `✅|❌|FAILED` hid the lowercase
  message. The result was the one intermediate state this task had reasoned
  about and meant to avoid: new Lambda under old gateway, ~90 s of portal-wide
  `404`. Recovered with `--require-approval never` for the remaining two,
  having already read the diff. **When a multi-stack deploy touches IAM, run
  it with `never` after reading the diff, or run the stacks one at a time and
  read every exit.**
- **`cdk diff` said "no differences" for Compute after a Rust change.** The
  api-handler asset is packaged off `target/lambda/prices-api/bootstrap`,
  which was the pre-change build from 09:50; the diff was honest about the
  disk and wrong about the source. Rebuilt with `cargo lambda build` and the
  hash changed. [[0141]], third sighting in one task.
- **A `/api/api` → `/api` substitution missed two regexes**, because the test
  had escaped the slashes (`\/api\/api\/config`). Caught by the portal
  suite, not by the grep that declared zero occurrences remaining.

- **A stale task branch turns `cdk diff` into a demolition plan.** On
  2026-08-31 this branch was 5 commits behind `origin/develop` and
  `make diff-production` proposed destroying four live Oracle alarms and an IAM
  statement — [[0231]]'s work, merged while this task was open. Nothing in the
  diff marks such a removal as "you are behind" rather than "you decided this",
  and the Makefile's own warning ("read it for removals rather than skimming it
  for additions") is exactly right and exactly not enough: the removals looked
  deliberate. Merging `develop` cleared all of them. **Merge before diffing,
  and diff before every production deploy.**
- **The local build needs Zig and nothing said so.** `cargo lambda build
  --arm64` cross-compiles through Zig on an x86_64 host; CI never hits this
  because it runs on a native ARM runner, and the comment on
  `API_HANDLER_ASSET_DIR` gives the command without the prerequisite. Cost an
  hour of the flip.
- **Nothing publishes the built Lambda assets.** CI builds all eleven natively
  and discards them, so a deploy ships whatever happens to be on the operator's
  disk. On 2026-08-28 that disk held binaries from 2026-08-13, and a `Compute`
  deploy would have regressed both the api-handler and the ledger-processor by
  two weeks while reporting success — the [[0141]] footgun, live.

## Design Decisions

### Emerged

1. **The gating guild is the test guild, and the landing page's copy is
   knowingly wrong for it.** Decided by Adam, 2026-08-28.
   `/prices/production/discord-guild-id` is seeded with
   `1536303837785362432` (`stellar_test`), not `897514728459468821`
   (`Stellar Developers`). The landing page states `discord.gg/stellardev` as
   the membership prerequisite — [[0189]]'s copy, rendered by [[0193]] and
   asserted by two tests — so with the portal open on a public URL, **every
   visitor who follows the instruction on the page is refused as
   `not_member`**; only people Adam invites to the test guild can get a key.

   Accepted rather than fixed, deliberately: the alternative is editing
   [[0189]]'s copy for a state that is meant to be temporary, and copy churn is
   what the "re-decide no copy" rule of [[0193]] exists to prevent. The refusal
   is also the honest one — the visitor really is not a member of the guild the
   gate checks — and it is the arm that does not accuse
   (`Eligibility::NotMember`, not `Unknown`).

   ⚠️ **This is a dated, temporary state and needs an owner and a date.**
   (See item 2 below for the prefix decision of 2026-08-31.)
   Switching to the production guild is `put-parameter --overwrite` and a ~5 min
   extension cache — no deploy, no code change. It must happen before the portal
   is advertised to anyone outside the demo, and before [[0164]] walks the
   user-visible flow as a stranger would. Until then the public URL is a door
   that says "join Stellar Developers" and then refuses Stellar Developers.

2. **The portal owns `/api/*` flat, and the bundle is a CDN carve-out.**
   Decided by Adam, 2026-08-31; the rule is "`/api` is the portal, then
   `/api/<rest>`". Supersedes [[0161]]'s `<app>/*` + `<app>/api/*` convention
   for this app. What emerged in implementation rather than in the decision:
   which side to enumerate. The bundle was chosen because it is the small,
   fixed side and because its failure mode is loud — full argument under "The
   prefix, flattened". The OpenAPI document is mounted a second time at
   `/api/api-docs-json` as a pure alias, exempt from gate and key check like
   `/config`, and deliberately absent from the OpenAPI document itself.

3. **The backend has a hostname of its own and the bundle calls it
   cross-origin.** Decided by Adam, 2026-08-31 (option A over B). Supersedes
   the "same-origin, no CORS" property [[0184]] and [[0186]] were built on,
   and the `audit/` requirements for the Explorer distribution. Emerged in
   implementation: the CORS answer is a **list** with a credentials
   predicate, never a reflection and never `exact`; the CSRF check gains an
   origin leg rather than dropping the fetch-metadata one; the bundle's API
   origin is a build-time variable with an empty default, so only the
   Explorer-bound build is absolute. See "The backend on its own host".

4. **A portal source failing at cold start closes the portal; it does not
   crash the Lambda.** Emerged from the PR review (finding 1) and decided
   here without asking, on evidence the original stance did not have: the
   api-handler has no error alarm, `cdk deploy` does not fail on an init
   error, and the flip made `/v1`'s cold start depend on three SSM reads
   against a 40 TPS account budget with no retry. Reverses the "fail at
   deploy, in `Init Errors`" comments in `config.rs`, `main.rs`,
   `compute-stack.ts` and the deploy-prep runbook, all rewritten. One commit,
   easy to drop if the loud stance is preferred — but then [[0249]]'s alarm
   is a precondition of the loud stance being loud, not a follow-up. Full
   reasoning on `AppConfig::load_portal_or_close`; the section "PR review —
   2026-09-01" has the measurements.

## Acceptance Criteria

- [x] ✅ **2026-08-31: met.** Nine checks settled on 08-28 and re-run after
      the prefix move; checks 1, 2 and 11 measured on 08-31 against the layout
      that serves `https://sorobanscan.rumblefish.dev/api/` — the Explorer's
      static `/api/*` for the page, `prices-api.sorobanscan.rumblefish.dev`
      for every call — and check 5 not applicable by decision A. Evidence:
      `audit/2026-08-28-report.md` plus the dated readings on each check and
      the three "Deployed" / "walked" sections above, all citable by [[0164]].
      Every check above passes against the deployed production stack — the
      three **(host)** checks against `EA2TLS5SS5M87`, serving
      `https://sorobanscan.rumblefish.dev/api/` — with the evidence captured in
      a form [[0164]] can cite. The 2026-08-28 report is the first half of that
      evidence and is explicitly **not** sufficient on its own: it measured
      `dojr4epgxo2qp.cloudfront.net`
- [x] ✅ **2026-08-31: deployed.** `PORTAL_ENABLED` is `'true'` and live —
      `Prices-production-Compute` updated 10:43Z, 22 s, `Errors` 0 over the
      window. Reversible by the same one-word diff plus a deploy; the secret's
      previous version is retained as `AWSPREVIOUS`.
- [x] ✅ **2026-08-31, 11:58–12:04Z, walked in Adam's Chrome, signed out
      on the new host by construction (no cookie existed on `prices-api…`).**
      Landing renders with "Get API Key" (config from `prices-api…` as JSON)
      → `/api/login` → "Sign in with Discord" → popup: `prices-api…/api/auth/
      login` → Discord (no re-consent) → callback on `prices-api…` sets the
      cookie → lands on `sorobanscan…/api/` → `postMessage` → dashboard as
      `kotryba`; `/api/auth/me` `200` across hosts. **The first sign-in
      issued no key** — `AccessDeniedException` on tag-on-create, the finding
      below — so the walk found exactly the kind of failure it exists to
      find. After the IAM fix (`a5c920e`, deployed 12:03:41Z), "Generate API
      Key" → `?issue=ok` → "Your API Key is ready — Just issued", `/api/key`
      and `/api/usage` `200`, rate-limit panel 1 req/s. Control plane:
      `31z25psyn7`, `discord-1534853384740540537-key`, enabled, tags
      `ManagedBy=prices-portal` + `IssuedBy=task-0187`, on plan `71t9im`,
      created 12:04:00Z, log `portal issued an API key`. The key value was
      never displayed or read.
      A complete sign-in and key issue is walked at
      `https://sorobanscan.rumblefish.dev/api/`, in a browser, signed out —
      the check that no configuration reading can replace
- [x] ✅ **2026-08-31: one failure, owned by [[0187]], fixed in its policy
      and re-verified here.** `PortalTagApiKeysOnCreate` in
      `compute-stack.ts` (`a5c920e`); re-verified by the walk above and by
      `AccessDeniedException` count 0 in the handler's log after the deploy.
      Any failure is fixed in the slice that owns it, and the fix is re-verified
      here rather than patched locally
- [x] ✅ **2026-08-31: answerable.** Tranche 3 AC 6 ("no secrets in env vars",
      least-privilege IAM) is carried by checks 6, 7 and 9, all PASS and all
      independent of the sign-off host. The env-var half was **re-verified on
      the deployed function after the flip**, not just at synth: all 14
      variables on `prices-production-api-handler` hold names or paths —
      `PORTAL_OAUTH_SECRET_NAME` and `MTLS_SECRET_NAME` are Secrets Manager
      names, the three `PORTAL_*_PARAM` are SSM paths, and no value is a
      credential. Report E6/E7/E9 plus this re-scan.

## Notes

- Deliberately a `TEST`, not a `FEATURE`. If this task ends up writing code, the
  slice that should have written it is the one to change.
- Runs after [[0191]] (0192 merged into it) and before [[0164]]. [[0164]] verifies the user-visible
  flow; this verifies the configuration underneath it.
