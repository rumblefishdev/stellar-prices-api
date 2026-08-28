---
id: "0194"
title: "Portal security and ops audit — caching, IAM, throttles, logs, against Tranche 3 AC 6"
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

## The host this task closes against

**`https://sorobanscan.rumblefish.dev/api/` — not the distribution domain.**
Decided 2026-08-28 by Adam. The portal's bundle is synced to
`s3://production-soroban-explorer-api-spa/api`, which is origin 2 of the
**Explorer** distribution `EA2TLS5SS5M87` (alias `sorobanscan.rumblefish.dev`),
behaviour `/api/*`. `dojr4epgxo2qp.cloudfront.net` — the distribution
`PortalHostingStack` creates and the one the 2026-08-28 audit measured — is
where the configuration was verified, and it is **not** where this task signs
off.

The consequence, stated so it is not discovered later: three of the twelve
checks are properties of *the distribution that serves the portal*, so they must
be re-run against `EA2TLS5SS5M87` before this task closes, and the assembled
configuration they check does not exist there yet. That work is [[0195]]'s and
it lives partly in the `soroban-block-explorer` repo — see **Hosting
preconditions** below. This task does not do it and does not sign off without
it.

## Checks

Verify against the **synthesized CloudFormation template and the deployed
stack**, not against the source, and not by assumption. Checks marked **(host)**
are properties of the serving distribution and are answered against
`EA2TLS5SS5M87`; the rest are answered against our own stacks and were settled
on 2026-08-28:

- [ ] ⏳ **2026-08-28, RE-RUN after the prefix deploy: gateway half PASS on the
      new paths.** `get-stage` after 13:30Z: 13 entries, the three
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
- [ ] ⏳ **2026-08-28: not started at the sign-off host.** Verified on
      `dojr4epgxo2qp.cloudfront.net` (report E2): `Managed-CachingDisabled` +
      `Managed-AllViewerExceptHostHeader` (`CookieBehavior: all`), and 13 of 13
      probe requests reached the origin — nothing served from the edge. None of
      that transfers.
      **(host)** The portal prefix's CloudFront behaviour on `EA2TLS5SS5M87`
      disables caching **and** forwards the session cookie; a signed-in request
      reaches the origin signed in. Today that distribution has **no origin
      pointing at our API at all**, `/api/*` is `GET`/`HEAD` only and sits
      behind the `production-soroban-explorer-basic-auth` function
- [x] ✅ **2026-08-28, RE-RUN after the prefix deploy: PASS on the new paths.**
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
- [ ] ⏳ **2026-08-28: not started at the sign-off host.** Correct on our
      distribution (report E5) and on the post-[[0235]] synth; `EA2TLS5SS5M87`
      has no API behaviour at all yet.
      **(host)** The portal's API behaviour precedes its bundle behaviour in
      `EA2TLS5SS5M87`'s order, whatever the two prefixes end up being — the
      ordering rule of [[0161]], not the literal `/api-tokens/` pair
- [x] ✅ **2026-08-28: PASS, and unaffected by [[0235]] or the host change.**
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
- [ ] ❌ **2026-08-28: FAIL — the secret does not exist.**
      `describe-secret prices/production/portal-discord-oauth` →
      `ResourceNotFoundException`, in `eu-central-1` and `us-east-1` alike.
      Blocker B1, owner [[0186]], runbook §2. The other two halves PASS: every
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
- [ ] ⏳ **(host)** **2026-08-28: PASS on our bucket, and the sign-off bucket
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
      is only that the bundle actually served from there is the portal's.
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
Delete them before the portal opens — a URL that was the documented one until
today should 404, not serve a broken copy.

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

**A is the recommendation** and it is what the redirect URI below assumes. It is
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

- [ ] The bundle is built for the prefix it is served from — `BASE_PATH` in
      `web/portal/src/base-path.ts` and its copy in `vite.config.mts`
      (`base-path.spec.ts` fails if they drift). Owner: [[0195]]
- [ ] The portal's backend is reachable **same-origin** on
      `sorobanscan.rumblefish.dev`: a second origin for
      `02mabge71l.execute-api.eu-central-1.amazonaws.com` and a behaviour ahead
      of the bundle row, with `ALLOW_ALL` methods, `CachingDisabled` and
      `AllViewerExceptHostHeader`. Same-origin is not a preference — [[0186]]'s
      session cookie is `SameSite=Lax` and the design keeps CORS out of portal
      traffic entirely. Owner: `soroban-block-explorer` repo, requested by [[0195]]
- [ ] `POST` and `DELETE` reach the portal's backend there — the current
      `/api/*` behaviour allows `GET`/`HEAD` only, so key issue, rework, revoke
      and sign-out cannot work. Owner: same
- [ ] The basic-auth CloudFront function does not intercept the portal's routes.
      Owner: same
- [ ] `CustomErrorResponses` does not rewrite the portal's error responses (see
      the first check). Owner: same — and it is distribution-wide, so it affects
      the Explorer SPA too
- [ ] The prefix change is carried through every place it is baked in:
      `PORTAL_API_PREFIX` and `CALLBACK_PATH` (Rust), the gateway resource path
      and its `methodSettings` entries, the session cookie's `Path`, and the
      Discord redirect URI. Owner: [[0195]] with [[0161]] for the convention

## Opening the portal

**This task owns the flip.** `PORTAL_ENABLED` goes to `'true'` in
`compute-stack.ts` here and nowhere else — not as a side effect of anyone
finishing their own slice. **The one-word diff is committed** (2026-08-28); what
remains gated is the deploy that carries it, because with the flag true the
handler resolves the secret and both parameters at cold start and a missing one
is an `Init Errors` event on the Lambda that also serves `/v1`.

Preconditions, all of them:

- [x] [[0189]] has passed: a non-member is refused, and a Discord `429`/`5xx`
      does not read as "not a member" — on the evidence available while closed
      (2026-08-28 report, gate §1)
- [ ] Every check in the list above passes, the three **(host)** ones against
      `EA2TLS5SS5M87`
- [ ] Every hosting precondition above is met
- [ ] The Discord OAuth secret exists and parses, and both eligibility SSM
      parameters are seeded (runbook §2, §2a) — 2026-08-28: **none of the three
      exists**, and the deploy that carries the flip must not run until they do
- [ ] Keys created while the flag was off are enumerated (2026-08-28: one,
      `smdesqkg5j`, listed in the report) and deleted. There is no
      separate incubation plan (decided 2026-08-13), so those keys are real keys
      on the real free-tier plan and would otherwise survive into its accounting
      as anonymous strings. They come from local runs against production
      credentials, not from the closed portal — the flag lives in the Lambda

**Amended 2026-08-28.** The original text here said the flip was not gated on
[[0193]] or [[0195]], on the reasoning that opening a plain-looking portal that
works beats leaving a finished one closed. [[0193]] has since merged, and the
decision to sign off at `sorobanscan.rumblefish.dev/api/` rather than at the
distribution domain **couples this task to [[0195]]**: the host that must be
verified is the one 0195 delivers. The flag itself is still independent of both
— it lives in the Lambda, which answers on either host — so the flip can be
deployed as soon as the secret and the parameters are seeded. What waits for
0195 is this task's sign-off, not the portal being open.

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
   Switching to the production guild is `put-parameter --overwrite` and a ~5 min
   extension cache — no deploy, no code change. It must happen before the portal
   is advertised to anyone outside the demo, and before [[0164]] walks the
   user-visible flow as a stranger would. Until then the public URL is a door
   that says "join Stellar Developers" and then refuses Stellar Developers.

## Acceptance Criteria

- [ ] Every check above passes against the deployed production stack — the
      three **(host)** checks against `EA2TLS5SS5M87`, serving
      `https://sorobanscan.rumblefish.dev/api/` — with the evidence captured in
      a form [[0164]] can cite. The 2026-08-28 report is the first half of that
      evidence and is explicitly **not** sufficient on its own: it measured
      `dojr4epgxo2qp.cloudfront.net`
- [ ] `PORTAL_ENABLED` is flipped to `'true'` here **and deployed**, with every
      precondition above met and recorded — and the flip is reversible by the
      same one-word diff plus a deploy. (Committed 2026-08-28; deploy still
      gated on the secret and the two parameters)
- [ ] A complete sign-in and key issue is walked at
      `https://sorobanscan.rumblefish.dev/api/`, in a browser, signed out —
      the check that no configuration reading can replace
- [ ] Any failure is fixed in the slice that owns it, and the fix is re-verified
      here rather than patched locally
- [ ] Tranche 3 AC 6 ("no secrets in env vars", least-privilege IAM) is
      answerable from this task's output alone

## Notes

- Deliberately a `TEST`, not a `FEATURE`. If this task ends up writing code, the
  slice that should have written it is the one to change.
- Runs after [[0191]] (0192 merged into it) and before [[0164]]. [[0164]] verifies the user-visible
  flow; this verifies the configuration underneath it.
