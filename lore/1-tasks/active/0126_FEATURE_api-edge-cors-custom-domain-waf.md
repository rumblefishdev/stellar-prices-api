---
id: "0126"
title: "API edge — CORS preflight, custom domain, and a recorded WAF decision"
type: FEATURE
status: active
related_adr: ["0008"]
related_tasks: ["0124", "0121", "0128", "0194", "0122"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M2, api-gateway, cors, dns, security]
milestone: 2
links:
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../docs/scf/milestone-1-evidence.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Closes the "Custom API
      domain, WAF, CORS preflight" row of `milestone-1-evidence.md` Table 4,
      which records all three as deliberately deferred from M1 with the API
      served on the raw execute-api URL.
  - date: 2026-09-01
    status: active
    who: okarcz
    note: >
      Activated after [[0194]] merged (PR #268). ⚠️ TWO OF THE THREE PARTS
      MOVED WHILE THIS SAT IN THE BACKLOG and the task body is rewritten to
      match. The custom domain is DONE — 0194 shipped it, verified live at
      https://prices-api.sorobanscan.rumblefish.dev with a valid cert and the
      stage path gone. CORS is still open for the DATA routes but this task's
      original design for it is now WRONG: 0194 added a single-origin,
      credentialed preflight for the portal, so a second policy has to be
      reconciled with it rather than decided from scratch. WAF is untouched
      and stands as written.
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      THREE OF THE FOUR OPEN DECISIONS SETTLED with the operator, recorded in
      full above with their rejected alternatives: /v1 CORS is `*` with no
      credentials (and the portal's single-origin answer differs because a
      credentialed response CANNOT use `*` - the browser rules leave no choice,
      which is the sentence the reconciliation AC wanted); WAF is NO, with four
      named reversal triggers; execute-api is MIGRATE-THEN-RETIRE, not a
      permanent alias, once it emerged that it is not a legacy URL at all but a
      load-bearing CloudFront origin (portal-hosting-stack.ts:191). Prod
      measured first: /v1 has no OPTIONS on any route (403
      MissingAuthenticationToken), routing survives the base-path mapping (the
      two DIFFERENT 403 error types prove the resource resolves), and the
      OpenAPI servers block on the wire matches apiBaseUrl. 🔴 FOUND: 0194's
      review already fixed this task's DEFAULT_4XX leak (narrowed to
      THROTTLED) but the fix IS NOT RUNNING - every /v1 403 still carries the
      portal's origin, even with no Origin header sent, and Vary: Origin is
      absent. The deploy ran 12:22Z, the fix committed 13:06Z. Inference from
      outside, not an AWS read; ownership of that deploy is the one question
      still open.
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      TWO CORRECTIONS, both from measuring instead of inferring, and both
      recorded rather than quietly rewritten. (1) The DEFAULT_4XX leak is NOT
      an undeployed fix. 0194's narrowing to THROTTLED is live in the control
      plane, `cdk diff --strict` shows no functional difference, and every /v1
      4xx STILL carries the portal's origin - including the OPTIONS preflight
      and requests sending no Origin at all. Narrowing the ResponseType does
      not scope the header. Spawned [[0255]]; this task's matching AC now
      depends on it and two earlier diagnoses of mine are recorded as wrong.
      (2) Decision 3's premise is half wrong: PortalHostingStack HAS NEVER
      BEEN DEPLOYED (`describe-stacks` -> does not exist; every resource `[+]`
      in the diff), so nothing in production consumes execute-api as a
      CloudFront origin and it IS the legacy alias I argued it was not. The
      migrate-then-retire decision stands on the narrower ground that the CDK
      origin is wrong for the day that stack first deploys - but it is no
      longer a prerequisite for retirement. ⚠️ Also noticed: Compute
      (09:00:25Z) and EventBridge (09:46:20Z) were deployed by someone else
      this morning; the Lambda asset diffs in `cdk diff` must NOT be read as
      stale deploys - see [[lambda-asset-diff-is-feature-unification]].
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      THE CORS WORK IS BUILT — PR #277, CDK only, not deployed. addCorsPreflight
      on all seven /v1 data routes: `*`, no credentials, x-api-key in
      allowHeaders, MOCK integration, 1h max-age. Folded into addGet rather than
      left as a separate call, so a future data route cannot silently ship
      without one. Synth confirms seven OPTIONS, every one ApiKeyRequired=false
      and Type=MOCK, with the portal's preflight unchanged; verify-routes,
      verify-servers, lint and typecheck green. The reconciliation AC is closed
      by writing the reason at DATA_CORS_ALLOW_ORIGINS - a credentialed answer
      CANNOT use `*`, so the two policies differ because the browser rules leave
      no choice. Both WAF deferral comments in the file now carry the decision.
      Left open deliberately: milestone-1-evidence.md is a FROZEN SUBMISSION
      RECORD and I will not rewrite what we told reviewers - that is a team
      question, and it governs the execute-api URLs in the other two M1 docs
      too. Also found while checking for conflicts: PR #276 (0195, another
      owner) DELETES portal-hosting-stack.ts, which moots this task's
      origin-migration criterion and reduces execute-api to disable-or-keep.
      ⚠️ The browser AC still cannot pass on a deploy alone - [[0255]] must land
      first, or a third-party page sees a CORS mismatch on every /v1 error.
---

# API edge — CORS, custom domain, WAF decision

## Summary

Three loosely-related edge concerns, grouped because they touch the same stack
and the same DNS/deploy step. **One is now done and the other two are what this
task is actually for.**

| concern | state (2026-09-01) |
|---|---|
| **Custom domain** | ✅ **DONE by [[0194]]** — see below. This task owns only the leftovers. |
| **CORS on the `/v1` data routes** | 🔴 **OPEN, and the original design here is now wrong.** The real work. |
| **WAF decision** | 🔴 **OPEN, unchanged.** A recorded decision, not necessarily a deployment. |

## ✅ The custom domain shipped with 0194 — what is left is small

Verified live 2026-09-01: `https://prices-api.sorobanscan.rumblefish.dev/health`
returns 200 with a valid certificate, and the execute-api URL still answers.
0194 settled everything this task listed as open — the hostname (in the shared
`sorobanscan.rumblefish.dev` zone, coordinated rather than assumed, as this task
asked), the ACM cert, the base path mapped to root so **`/production` is gone
from public URLs**, `apiBaseUrl`, `validateConfig`'s execute-api-only stage rule,
and `docs/scf/api-endpoints.md`.

⚠️ **§4's `api.prices.stellar.example.com` was a placeholder and is now dead.**

Leftovers this task still owns:

- the **execute-api retirement decision** — announced and dated, or explicitly
  kept as a permanent alias (it currently still serves);
- **re-verify [[0122]]'s cache-hit behaviour through the new path** — a custom
  domain changes the cache key surface;
- **re-verify routing** after the base-path mapping, given
  `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true` is already load-bearing for `/v1`.

## 🔴 CORS: there are now TWO policies on one gateway, and only one exists

This is the part that changed most, and reading the original text below without
this note would send someone the wrong way.

0194 added exactly one `addCorsPreflight`, on the **portal** proxy
(`/api/{proxy+}`):

```ts
allowOrigins: [config.portalWebOrigin],   // ONE origin, WITH credentials
allowHeaders: ['Content-Type', 'Accept', PORTAL_REQUEST_HEADER]
```

The **`/v1` data routes have no preflight at all**, so the original problem —
*no browser can call this API* — is untouched.

⚠️ **The design this task originally prescribed (`*`, no credentials,
`x-api-key` in allowed headers) is still right for the data routes and now
CONFLICTS with the portal's policy on the same gateway.** That is not a defect:
they are different surfaces with different threat models — the portal carries a
session cookie and must be single-origin; the data API is key-authenticated,
public, and read-only, so `*` costs nothing. But the two must be **reconciled
deliberately and the reason written down**, because the next reader will see two
different CORS answers in one stack and assume one is a mistake.

⚠️ Gateway-level `DEFAULT_4XX` / `DEFAULT_5XX` gateway responses already carry
portal CORS headers (`PortalCors4xx`/`PortalCors5xx`). Check whether those leak
the portal's single origin onto data-route errors before adding a second policy.

## Context

**CORS is the functional one.** Overview §4's base URL is
`https://api.prices.stellar.example.com/v1` and the Tranche 3 deliverable is a
browser-based onboarding portal with example queries. Without preflight
handling, **no browser can call this API** — every cross-origin request fails
before it reaches a handler. It is also the item most likely to be discovered by
the first external consumer rather than by us.

**The custom domain was presentational but load-bearing for the spec** — ✅ now
delivered by [[0194]]; the paragraph is kept for the reasoning, not as work.

**WAF is a genuine decision, not a default-yes.** The API is public, read-only,
key-gated, over public blockchain data, with no PII (§7) and API Gateway
throttling already in place at two levels (stage: 200 rps / 400 burst; per key:
100/200). AWS WAF adds ~$5-6/mo per web ACL plus per-request cost against a §10
budget of ~$108/mo total. **The deliverable is a recorded decision with
reasoning — not necessarily a deployed WAF.** A defensible "no, because X, and
here is what would change our mind" is a complete outcome.

## Implementation

**CORS**

- Decide the allowed-origin policy. For a public read API with key auth, `*` is
  the conventional and defensible choice — but note that `Access-Control-Allow-
  Origin: *` cannot be combined with credentials, and confirm nothing relies on
  cookies (nothing should; auth is a header key).
- `x-api-key` must be in `Access-Control-Allow-Headers` or every browser call
  fails preflight — the single most common way this ships broken.
- Implement preflight at the **gateway** (`OPTIONS` mock integration per
  resource) rather than per-handler, so an unauthenticated `OPTIONS` never
  invokes Lambda and never consumes quota. Confirm the `OPTIONS` method is
  **not** `apiKeyRequired` — a preflight cannot carry a key.
- Cache preflight responses (`Access-Control-Max-Age`).
- Verify from a real browser, not only curl. `curl -X OPTIONS` succeeding proves
  less than it appears to.

**Custom domain**

- Choose the hostname and confirm who owns the zone (this is a shared
  sub-account alongside BE — coordinate rather than assume). §4's
  `api.prices.stellar.example.com` is a placeholder, not a decision.
- ACM certificate in the right region for the endpoint type, API Gateway custom
  domain + base-path mapping, DNS record.
- Decide the **stage-path story**: mapping the base path to the stage removes
  `/production` from public URLs, which is nicer — and changes every documented
  URL. Update §4, [[0124]]'s `servers`, and the [[0128]] evidence together, and
  keep the execute-api URL working during the transition.
  - [[0124]] landed, so `servers` is now **one config value**: `apiBaseUrl` in
    `infra/envs/production.json`. Change it and the handler's `API_BASE_URL`
    and the published document follow automatically — no code edit. Two things
    to know: `validateConfig` in `infra/src/lib/types.ts` only enforces the
    `/production` stage suffix for `.execute-api.` hosts, so a custom domain
    with the base path mapped to the stage passes without changes; and the base
    URL is repeated as prose in `docs/scf/api-endpoints.md`, which must be
    updated in the same commit.
- Note for anyone touching the handler: the stage-prefix behaviour is already
  subtle here (`AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true` is required for `/v1`
  routing). Re-verify routing after the base-path mapping change.

**WAF**

- Write the decision: threat model, what throttling already covers, cost against
  the §10 budget, and the trigger that would reverse it (e.g. sustained abuse,
  a Stellar-side requirement, an incident).
- If yes: managed rule groups only, rate-based rule aligned with the existing
  throttle, and deploy in **count mode first** — a blocking WAF in front of an
  API whose load profile is about to be measured by [[0121]] will corrupt that
  measurement.

## ✅ Decisions settled 2026-09-02 — with the operator

Three of the four open questions are answered. They are recorded here as
decisions, not preferences, so the implementation below is now execution
rather than design. Measured evidence for the state they were taken against
is in "Measured on prod 2026-09-02".

### 1. `/v1` allowed origins: `*`, no credentials

**Decided.** The conventional answer for a public, key-authenticated,
read-only API, and here it is close to forced.

CORS protects a browser user's *ambient authority* — credentials the browser
attaches on its own. `/v1` has none: auth is an `x-api-key` header the caller
supplies deliberately, there is no cookie and no session. So a hostile page
calling `/v1` gets exactly what `curl` gets, and restricting the origin blocks
only browsers — the one client where it costs legitimate users something and
stops no attacker (a script or a server does not perform CORS at all).

Rejected, with reasons:

- **Mirror the request `Origin`.** Equivalent in effect, but the answer then
  varies per caller and needs `Vary: Origin`, which splits the API Gateway
  cache into one entry per origin. The cache is ON
  (`apiGatewayCacheEnabled: true`) and cache-key mistakes have already cost us
  once — [[0118]] shipped a parameter believing the gateway keyed on the query
  string, and production served one caller's narrowed response to the next.
  Measurable cost, no security gain.
- **Allowlist named origins.** Only coherent if we intend to control who builds
  against the API. We do not: keys are self-service through the portal and the
  onboarding page ships example queries. An allowlist makes every new consumer
  file a ticket.

⚠️ **Why this is NOT the same as the portal's answer, and why that is correct.**
`Access-Control-Allow-Origin: *` cannot be combined with credentials — browsers
reject the pairing outright. The portal carries a session cookie, so it is
*forced* to name exactly one origin; `/v1` carries none, so `*` is available and
costs nothing. **The two policies differ because the browser rules leave no
choice, not because one of them is a mistake.** That sentence is the deliverable
of the reconciliation AC — write it next to the `/v1` preflight in
`api-gateway-stack.ts`, not only here.

### 2. WAF: NO — recorded, with reversal triggers

**Decided: do not deploy.** The deliverable was always a reasoned decision
rather than a deployment, and the reasoning is:

- Public, read-only, key-gated API over public blockchain data. **No PII** (§7).
- No user input reaches a query as SQL — routes take an asset identity and a
  time window, both shape-validated before the DB is touched.
- Rate abuse is already bounded at two levels: stage 200 rps / 400 burst, and
  per key 100 / 200.
- ~$5-6/mo per web ACL plus per-request charges against a §10 budget of
  ~$108/mo total — a real share of it for coverage that largely duplicates the
  throttles.

**What a WAF would genuinely add, and why it still loses today:** the gap the
throttles leave is per-caller *volume*, not rate — a caller sitting at the
ceiling accumulates, and nothing notices (`api-gateway-stack.ts:205-213` records
this). That is a real gap. It is not worth a standing cost and a new dependency
for a threat nobody has observed.

**Reversal triggers — any one of these reopens it:**

1. Sustained abuse or a volume anomaly from a single caller.
2. A Stellar-side or SCF requirement naming a WAF.
3. An incident whose blast radius a rate-based rule would have bounded.
4. Adding any route that accepts free-form input or writes.

⚠️ If it is ever reversed: managed rule groups only, a rate-based rule aligned
to the existing throttle, and **count mode first** — a blocking filter in front
of an API whose load profile is about to be measured by [[0121]] corrupts that
measurement.

⚠️ Two code comments still defer this (`api-gateway-stack.ts:212` and `:279`,
the latter pointing at task 0056) and `milestone-1-evidence.md:959` lists WAF
with the domain and CORS as deferred to Tranche 2. All three are now stale and
must be updated to point here, or the next reader re-opens a settled question.
`:212` also says the right moment to decide is after 0194 has costed the
portal's traffic — 0194 landed 2026-09-01, so that condition is met.

### 3. execute-api: move the origin, THEN retire

**Decided: migrate, then retire** — rather than keeping it as a permanent alias.

⚠️ **CORRECTED 2026-09-02 — the premise this was decided on is HALF WRONG, and
the correction is recorded rather than the reasoning quietly rewritten.**

What was said: *execute-api is not a legacy public URL, it is load-bearing* —
`PortalHostingStack` uses it as a CloudFront origin
(`portal-hosting-stack.ts:191`) with `originPath: '/${config.envName}'`.

That is true **of the code and false of production.** `PortalHostingStack` HAS
NEVER BEEN DEPLOYED:

```
aws cloudformation describe-stacks --stack-name Prices-production-PortalHosting
  → Stack with id Prices-production-PortalHosting does not exist
```

Deployed stacks are ApiGateway, Compute, EventBridge, Observability, Secrets —
no PortalHosting, and `cdk diff` renders every one of its resources as `[+]`.
The portal bundle is served from the Explorer's distribution instead, which is
what `make sync-portal-explorer` exists for.

**So in production today, execute-api IS the legacy alias it was argued not to
be** — nothing consumes it as an origin, because the consumer does not exist.

**The decision still stands, for a narrower reason.** The origin is wrong in the
CDK either way and would bite the day `PortalHostingStack` is first deployed —
against a hostname whose `originPath` no longer applies. But the migration is
**no longer a prerequisite for retiring execute-api**, and the sequencing in
this task should not pretend it is.

The migration is small and CI-guarded:

- swap the origin hostname to `config.apiDomain.domainName`;
- **drop `originPath`** — the custom domain maps the base path to root, so
  leaving `/production` in prefixes every request and 403s the lot;
- **keep `ALL_VIEWER_EXCEPT_HOST_HEADER`.** It carries over unchanged and is
  doing two jobs: withholding the viewer's `Host` (an origin authenticates
  against its own hostname, so forwarding the viewer's 403s everything) and
  forwarding the session cookie ([[0186]]). `tools/scripts/verify-openapi-routes.mjs`
  asserts both the policy and the `originPath` against the synthesized template.

Reasoning, including the argument AGAINST — which is real and was weighed:

- **For.** Afterwards there is exactly one hostname; "execute-api retired"
  becomes true rather than aspirational; the `originPath` stage-prefix trap
  (same failure class as `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH`, silent when
  wrong) leaves the codebase; and the path our consumers use is the path we
  exercise every day, so a fault in the custom domain surfaces to us first.
- **Against.** It puts the DNS record, the ACM cert and the base-path mapping
  into the portal's critical path, where today they are not. A cert or DNS
  fault would take the portal down with the public API instead of only the
  public API.

The two are the same fact read opposite ways — *the portal does not currently
depend on the custom domain*. Isolation was judged worth less than having one
hostname and one exercised path. ⚠️ **This is the reasoning to re-read if a
cert or DNS fault ever does take out both**: the trade was made knowingly.

### 4. Resolved into [[0255]] — the narrowing is deployed and does NOTHING

This was carried as "a merged fix that never shipped". **It shipped.** The
gateway-response narrowing (`THROTTLED` instead of `DEFAULT_4XX`) is live in the
control plane and `cdk diff --strict` reports no functional difference — and
every `/v1` 4xx still carries the portal's origin.

So it is not a deploy question and there is nobody to hand it to: [[0194]]'s
code is correct and running. It is API Gateway behaviour nobody here
understands, and it is now **[[0255]]**.

⚠️ **Two wrong diagnoses were published before that**, both by inference rather
than measurement — "merged but never deployed" (a CEST/UTC timestamp misread)
and "saved but not published to the stage". Both died to `cdk diff` and
`describe-stacks`. Recorded because the pattern, not the conclusion, is the
thing worth not repeating.

## Acceptance Criteria

- [ ] Cross-origin `GET` from a browser page against every data route succeeds,
      preflight included. ⚠️ **Needs a deploy AND a real browser** — this task
      says it and it still holds: `curl -X OPTIONS` succeeding proves less than
      it appears to. ⚠️ **Blocked in practice by [[0255]]**: once `/v1` answers
      `*`, its 4xx still carry the portal's single origin, so a third-party
      page hitting a 403 or 429 sees a CORS mismatch and reads it as a dead
      network. The preflight alone does not make the API browser-usable
- [x] `x-api-key` is in the allowed-headers list; `OPTIONS` requires no API key
      and does not invoke Lambda — PR #277. Synth shows `OPTIONS` on all SEVEN
      data routes, every one `ApiKeyRequired: false` and `Type: MOCK`,
      `Allow-Headers: 'Content-Type,Accept,x-api-key'`
- [x] Allowed-origin policy decided and recorded — **`*`, no credentials**,
      settled 2026-09-02 with the reasoning and the two rejected alternatives
      in decision 1 above
- [x] Custom domain resolves and serves the API over TLS; certificate valid and
      auto-renewing — **delivered by [[0194]]**, verified 2026-09-01
- [x] The two CORS policies on this gateway (portal: one origin + credentials;
      data routes: `*`, no credentials) are reconciled DELIBERATELY, with the
      reason recorded — not left looking like one of them is a mistake.
      PR #277 writes it at `DATA_CORS_ALLOW_ORIGINS` in `api-gateway-stack.ts`,
      where the next reader hits it: a credentialed answer CANNOT use `*`, so
      the portal is forced to one origin and `/v1` is not
- [ ] Gateway-level `DEFAULT_4XX`/`DEFAULT_5XX` responses do not leak the
      portal's single `Access-Control-Allow-Origin` onto data-route errors.
      ⚠️ **CORRECTED 2026-09-02: 0194's review narrowed this to `THROTTLED`,
      the fix IS deployed, and it has NO EFFECT** — every `/v1` 4xx still
      carries the portal's origin. Not a deploy problem and not 0194's tail;
      tracked as **[[0255]]**. This AC cannot close until 0255 does
- [x] Every documented URL (§4, OpenAPI `servers`, evidence docs) updated
      consistently — **done by [[0194]]** (`apiBaseUrl`, `api-endpoints.md`)
- [x] Routing re-verified after the base-path mapping, given
      `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true` — measured 2026-09-02 below.
      A keyed `200` on a `/v1` route would be belt-and-braces; the resource
      resolves, which is what the base-path mapping could have broken
- [ ] [[0122]]'s cache-hit behaviour re-verified through the custom domain
- [x] Execute-api URL: the DECISION is made — **migrate the CloudFront origin
      to the custom domain, then retire** (decision 3 above). ⚠️ The decision
      is recorded; the MIGRATION is implementation and is tracked by the two
      criteria that follow
- [ ] `PortalHostingStack`'s API origin points at `config.apiDomain.domainName`
      with **no `originPath`**, `ALL_VIEWER_EXCEPT_HOST_HEADER` unchanged, and
      `verify-openapi-routes.mjs` updated to assert the new shape.
      ⚠️ That stack is NOT DEPLOYED (verified 2026-09-02), so this is a
      correctness fix for the day it first is — **not** a prerequisite for
      retiring execute-api, and not verifiable in a browser until then
- [ ] Execute-api retirement: since nothing deployed consumes it as an origin,
      the remaining question is only whether the endpoint itself is disabled.
      Decide and record; no portal round-trip is gated on it
- [x] WAF decision recorded with reasoning, cost, and a reversal trigger —
      **NO**, settled 2026-09-02, four triggers named (decision 2 above)
- [ ] The three stale deferrals updated to point at that decision. **Two done**
      in PR #277 (`api-gateway-stack.ts`, both comments). ⚠️ The third,
      `milestone-1-evidence.md:959`, is DELIBERATELY NOT TOUCHED — it is a
      frozen submission record ("Table 4 — Out-of-scope and known-open items")
      and rewriting what we told reviewers is not a call to make in passing.
      **Open question for the team: are the M1 evidence docs frozen as
      submitted, or maintained?** The same question governs the execute-api
      URLs still in `milestone-1-form-answers.md` and
      `milestone-1-video-scenario.md`
- [x] All of it expressed in CDK (Tranche 3 AC 7 requires clean-account
      reproducibility) — PR #277 is CDK only; no console step, no manual
      gateway edit

## 📏 Measured on prod 2026-09-02 — before any of this task's code

Probed from outside against `https://prices-api.sorobanscan.rumblefish.dev`.
Recorded because three ACs turn on what is actually RUNNING, and this task's
own history is a case study in a task file describing a world that had moved.

### `/v1` has no preflight — the functional gap, confirmed

```
OPTIONS /v1/assets/native/price
  Origin: https://example.com
  Access-Control-Request-Method: GET
  Access-Control-Request-Headers: x-api-key
→ 403  x-amzn-errortype: MissingAuthenticationTokenException
```

No `OPTIONS` method exists on any data route — `addGet` adds `GET` alone
(`api-gateway-stack.ts:349`), and the only `addCorsPreflight` in the stack is
the portal's. The control confirms the mechanism works where it is wired:

```
OPTIONS /api/config   Origin: https://sorobanscan.rumblefish.dev
→ 204
   access-control-allow-origin: https://sorobanscan.rumblefish.dev
   access-control-allow-headers: Content-Type,Accept,X-Requested-With
   access-control-allow-methods: GET,POST,DELETE
   access-control-max-age: 3600
   access-control-allow-credentials: true
```

### 🔴 The `THROTTLED` narrowing IS deployed — and does nothing → [[0255]]

Every `/v1` 4xx carries the portal's origin, **including on requests that sent
no `Origin` header at all**:

| request | status | `access-control-allow-origin` |
|---|---|---|
| `GET /v1/assets/native/price`, no key, `Origin: https://evil.example` | 403 `ForbiddenException` | `https://sorobanscan.rumblefish.dev` |
| same, **no `Origin` header** | 403 `ForbiddenException` | `https://sorobanscan.rumblefish.dev` |
| `GET /nope`, no `Origin` | 403 `MissingAuthenticationTokenException` | `https://sorobanscan.rumblefish.dev` |
| `OPTIONS /v1/assets/native/price` | 403 | `https://sorobanscan.rumblefish.dev` |
| `GET /health`, `Origin: https://evil.example` | 200 | *(none — correct)* |

`Access-Control-Allow-Credentials: true` and `Vary: Origin` ride along on all
four. And the control plane says none of these types is customised —
`DEFAULT_4XX`, `ACCESS_DENIED`, `INVALID_API_KEY` and
`MISSING_AUTHENTICATION_TOKEN` all read `responseParameters: {}`,
`defaultResponse: true`; only `THROTTLED` and `DEFAULT_5XX` are customised,
exactly as the merged code declares.

Not a deploy problem — ruled out explicitly:

- `cdk diff Prices-production-ApiGateway --strict` → only `AWS::CDK::Metadata`
  and two Output **descriptions** (a mojibake `?` becoming `→` / `—`).
- CFN `LastUpdatedTime` `2026-09-01T12:23:31Z`, stage deployment `vsrfht`
  `12:23:38Z` — both after PR #268 merged at `12:02Z`.
- `PortalHostingStack` is not deployed, so no CloudFront sits in the path.

**Full write-up, reproduction plan and the generalisable lesson: [[0255]].**

⚠️ Two wrong diagnoses were published before that one, both inferred rather than
measured. Kept as method, not as trivia: a CEST/UTC misread (`git log` prints
local, Lambda `LastModified` is UTC) produced "merged but never deployed", and
a guess at API Gateway internals produced "saved but not published". `cdk diff`
and `describe-stacks` killed both. **The measurement was right from the first
probe; every wrong answer came from explaining it instead of extending it.**

### Routing survives the base-path mapping

The two different 403s are the evidence, and they are more informative than a
200 would be:

- `/v1/assets/native/price` → **`ForbiddenException`** = the resource and method
  MATCHED, the API key is missing.
- `/nope` → **`MissingAuthenticationTokenException`** = no such resource.

Different errors mean the gateway is resolving `/v1` through the mapping. Also
verified anonymously end to end:

```
GET /api-docs-json → 200, 34,541 bytes
  servers: [{"url": "https://prices-api.sorobanscan.rumblefish.dev"}]
```

So [[0124]]'s `servers` block, `apiBaseUrl` and the live hostname all agree —
the "every documented URL updated consistently" AC holds on the wire, not just
in the repo.

### Still unmeasured

- **[[0122]]'s cache-hit behaviour through the custom domain.** The cache IS on
  (`apiGatewayCacheEnabled: true`, `infra/envs/production.json`). Needs a keyed
  request pair, which the agent cannot issue — hand-over for the operator.
- A keyed `200` on a `/v1` route.

## 🛠️ Implementation — PR #277 (branch `feat/0126_v1-cors-preflight-and-waf-decision`)

CDK only, not deployed. `infra/src/lib/stacks/api-gateway-stack.ts`.

### What shipped

`addCorsPreflight` on all seven data routes — `*`, **no credentials**,
`allowHeaders: ['Content-Type', 'Accept', 'x-api-key']`, MOCK, `maxAge` 1 h:

| resource | verbs answered |
|---|---|
| `/v1/assets` | `GET,OPTIONS` |
| `/v1/assets/{asset_identifier}` | `GET,OPTIONS` |
| `/v1/assets/{asset_identifier}/price` | `GET,OPTIONS` |
| `/v1/assets/{asset_identifier}/ohlcv` | `GET,OPTIONS` |
| `/v1/oracles/{asset_identifier}` | `GET,OPTIONS` |
| `/v1/backfill/status` | `GET,OPTIONS` |
| `/v1/prices/batch` | `POST,OPTIONS` |

### Design decisions

1. **The preflight is folded into `addGet`, not called separately.** A data
   route added later without one is invisible to `curl` and to every test in
   this repo, and surfaces only as "the API cannot be called from a browser" —
   the exact defect this task exists to close. Coupling them makes forgetting it
   require an edit rather than an omission. `batch` names its own, being the one
   non-GET — and the one a browser preflights unconditionally, since a JSON
   `POST` is never a "simple" request.
2. **`x-api-key` listed explicitly** although `apigateway.Cors.DEFAULT_HEADERS`
   already contains it. A default that silently stopped including it would take
   every browser integrator down and nothing here would show why.
3. **The `OPTIONS` methods take the wildcard stage entry** (uncached,
   `apiGatewayThrottleRate/Burst`) rather than the portal's tighter
   `PORTAL_THROTTLE`. The portal's verbs are keyless AND reach the Lambda;
   these reach a MOCK, which is the shape `/health` has carried on the stage
   default since it shipped. Caching them would be worse than pointless — the
   BROWSER already caches a preflight answer for `maxAge`.
4. **`milestone-1-evidence.md` left alone** — see the AC above.

### Verified locally

- `cdk synth` → seven `OPTIONS`, every one `ApiKeyRequired: false`, `Type: MOCK`,
  `Allow-Origin: '*'`, no `Allow-Credentials`, `Max-Age: '3600'`. The portal's
  preflight is byte-identical to before.
- `openapi:verify-routes` green — 9 routes agree in both directions. `OPTIONS`
  is skipped on the gateway side by a rule in
  `tools/scripts/verify-openapi-routes.mjs:147` that **already named this
  task** — someone pre-wired the gate for this change.
- `openapi:verify-servers`, `lint`, `typecheck` all green.

### ⚠️ Coordination

PR **#276** (task 0195, another owner) rewrites this same file and **deletes
`portal-hosting-stack.ts` entirely**. Checked before writing: its only two hunks
in `api-gateway-stack.ts` are comments at lines ~417 and ~870, nowhere near the
`/v1` block, so #277 does not collide.

⚠️ **#276 also settles decision 3 by removing its subject.** Its own comment
says the distribution that fronted execute-api as an origin "was retired by task
0195", and leaves the retire-or-alias call to this task. So the origin-migration
criterion above is moot the moment #276 merges, and the execute-api question
reduces to: disable the endpoint, or keep it. Nothing is gated on it.

## Notes

- 🔴 **The [[0121]] sequencing note has now FIRED, not merely been noted.** This
  task said *"settle the domain and any WAF before the load test, or the run
  measures an edge that is about to change"* — the domain changed on
  2026-09-01. A load test still pointed at
  `02mabge71l.execute-api…/production` measures a host that is no longer the
  documented base URL, and both hostnames answer 200, so nothing about the run
  would look wrong. 0121 is another owner's task: **tell them, do not edit it.**
- The custom domain also affects the API Gateway cache — verify [[0122]]'s hit
  behaviour still holds through the new path.
