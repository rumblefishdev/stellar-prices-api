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

- [ ] ⏳ **2026-08-31, AFTER THE FLIP: origin half now fully PASS; only the
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
- [x] ✅ **Done in code ([[0235]], 2026-08-28), one item outstanding.** The
      prefix change is carried through `PORTAL_API_PREFIX` and `CALLBACK_PATH`
      (Rust), the gateway resource path and its three `methodSettings` entries,
      and the session cookie's `Path` — layout B, [[0161]]'s convention.
      ⚠️ **The Discord redirect URI is the one place it is not carried
      through**: it is registered in the Developer Portal, not in this repo, and
      it must read `https://sorobanscan.rumblefish.dev/api/api/auth/callback`
      before the first sign-in. Owner: Adam, at the same time as the
      `client_secret` rotation

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
- [ ] Every check in the list above passes, the three **(host)** ones against
      `EA2TLS5SS5M87`
- [ ] Every hosting precondition above is met
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

## Issues Encountered

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
- [x] ✅ **2026-08-31: deployed.** `PORTAL_ENABLED` is `'true'` and live —
      `Prices-production-Compute` updated 10:43Z, 22 s, `Errors` 0 over the
      window. Reversible by the same one-word diff plus a deploy; the secret's
      previous version is retained as `AWSPREVIOUS`.
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
