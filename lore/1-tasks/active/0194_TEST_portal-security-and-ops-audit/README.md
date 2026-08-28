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

- [ ] Every portal method has `cachingEnabled: false`, and every portal response
      carries `Cache-Control: no-store` — the gateway half is settled; the
      response half is **(host)**, because `EA2TLS5SS5M87` maps 403 and 404 to
      `/index.html` with status `200` (`CustomErrorResponses`), which today
      would swallow the portal's JSON errors and [[0183]]'s gate `404`
- [ ] **(host)** The portal prefix's CloudFront behaviour on `EA2TLS5SS5M87`
      disables caching **and** forwards the session cookie; a signed-in request
      reaches the origin signed in. Today that distribution has **no origin
      pointing at our API at all**, `/api/*` is `GET`/`HEAD` only and sits
      behind the `production-soroban-explorer-basic-auth` function
- [ ] The full `methodSettings` array contains every portal route in **both**
      arms of the `cacheEnabled` branch — flip `apiGatewayCacheEnabled` off in a
      synth and diff
- [ ] Anonymous sign-in routes carry their own method-level throttle and are not
      behind `apiKeyRequired`
- [ ] **(host)** The portal's API behaviour precedes its bundle behaviour in
      `EA2TLS5SS5M87`'s order, whatever the two prefixes end up being — the
      ordering rule of [[0161]], not the literal `/api-tokens/` pair
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

## Hosting preconditions (new, 2026-08-28)

None of these is this task's to build; all of them gate its sign-off. A plain
`aws s3 sync` of `web/portal/dist` to `s3://production-soroban-explorer-api-spa/api`
satisfies none of them and produces a blank page — the built `index.html`
carries **absolute** `/api-tokens/assets/…` URLs, which on that host match no
behaviour, fall to the Explorer SPA bucket, and come back as Explorer's
`index.html` with status `200`.

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
