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
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      Second review pass over PR #277, plus the remaining decisions. (1) The
      test added by the half-mechanism fix pinned the LAYER, not the surface:
      it asserts the wildcard on /health, which production answers from a
      gateway MOCK and never routes to the Lambda, so the suite still touched
      no /v1 path at all - the same failure shape one level up. `1b20a68` adds
      a test on POST /v1/prices/batch (rejected before state.ch(), so it needs
      no ClickHouse) that asserts the 400 and the invalid_query code as well as
      the header, because the layer wraps the fallback too and a header-only
      assertion would pass against a deleted route. Verified by detaching the
      layer. (2) EXECUTE-API DECIDED: keep it serving, do not disable yet - the
      blocker is not technical (disableExecuteApiEndpoint is one CDK line,
      present in aws-cdk-lib 2.257.0) but that the submitted M1 evidence, form
      answers and video scenario all cite execute-api URLs; disabling breaks
      links we gave SCF reviewers, silently from our side. Downstream of the
      M1-docs question, with the trigger named. (3) The 0122 AC was REWORDED:
      it asked to "re-verify" cache behaviour that has never been measured -
      0122 is still in backlog - so it could never have closed. Narrowed to the
      one property this task's change could break (both hostnames map to one
      stage, so they must share one cache) with a runbook handed over, and the
      TTL matrix left in 0122, where two drifts against §6 are now recorded.
      (4) The PortalHosting origin fix is deliberately NOT written: PR #276
      deletes that file and is still open, so the fix would buy a conflict and
      nothing else.
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      PR #276 MERGED at 12:10Z and it collided after all - not in
      api-gateway-stack.ts as the pre-check predicted, but in lib.rs and
      tests/portal.rs, which its review-fix commit reached. Merged as
      `a96e297`. 🔴 The dangerous half merged CLEANLY: 0195 amended the same
      CORS test from the other side, and git silently kept its assertion that
      /health carries NO allow-origin into a tree where data_cors_layer gives
      it `*`. Committing without reading the auto-merged middle would have
      shipped a red test, and the obvious "fix" would have deleted the
      wildcard. Resolved on the property both tasks still need - the portal's
      single origin and its credentials appear nowhere else. 0195 also handed
      over a better witness (/v1/assets/not-an-asset/price, a handler-side 400
      with no DB call), now covered alongside the POST batch case. ⚠️ Also
      corrected by measurement: data_cors_layer's comment claimed an overlap
      would emit TWO allow-origin headers. It does not - CorsLayer OVERWRITES,
      one value comes back in every arrangement, and HeaderMap::get cannot see
      the difference. The real failure is that the portal's credentialed origin
      is silently replaced by `*`; four existing tests already catch it. A test
      I had written to assert "exactly one allow-origin" was DROPPED, because
      no ordering makes it fail. The PortalHosting origin AC closes as MOOT:
      portal-hosting-stack.ts no longer exists.
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      🔴 CORRECTION, raised by the operator. Earlier today I recorded decision 5
      as "execute-api: KEEP it serving, do not disable yet" and ticked its AC.
      That REVERSED the migrate-then-retire call already taken with the
      operator and recorded in decision 3 - which says explicitly "not a
      permanent alias" - and the AC only ever offered two outcomes, "announced
      and dated" or "explicitly kept as a permanent alias". I chose neither: I
      invented a third and closed on it. The M1-docs dependency I found is real
      (the submitted evidence, form answers and video scenario all cite
      execute-api URLs, so announcing retirement before those are migrated or
      marked historical breaks links given to SCF reviewers) but that is a
      PREREQUISITE and an ordering constraint, not grounds to keep the endpoint
      indefinitely, and reversing a settled decision was not mine to do.
      Decision 5 rewritten as sequencing under the standing decision; the AC is
      RE-OPENED. What is outstanding is the DATE, which is the operator's.
      Live state unchanged and confirmed on the wire: execute-api still answers
      200.
  - date: 2026-09-02
    status: active
    who: okarcz
    note: >
      🔴 REVIEW CAUGHT PR #277 SHIPPING HALF THE MECHANISM. addCorsPreflight
      emits only the OPTIONS mock; the GET/POST are Lambda PROXY integrations,
      so the real response's allow-origin can only come from the handler - and
      the Rust CorsLayer is portal-scoped, with a test asserting data routes
      carry none. Preflight 204, then the browser blocks the actual GET, with
      `curl` passing throughout: the exact defect this task exists to close,
      nearly re-shipped inside its own fix. Every local signal was green and
      this task's own text already said "verify from a real browser, not only
      curl". Fixed in 8321e9a with data_cors_layer() - `*` via
      AllowOrigin::any(), no credentials, a CONSTANT so the stage cache cannot
      serve one caller's origin to the next. Rewrote the test that pinned the
      old world; added one pinning the new answer across two origins. Corrected
      a factually wrong max-age comment (browsers key the preflight cache per
      origin regardless of the response). Left to PR #276: /api-docs-json's own
      `*`. Added to [[0255]]: a THROTTLED preflight answers 429 carrying the
      portal's origin and credentials, so under throttle this feature is
      intermittently broken, not merely broken on errors.
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

### 5. execute-api: RETIRE stands — what follows is the ORDER, not a reversal

⚠️ **CORRECTED. The first version of this section was headed "KEEP it serving —
do not disable it yet" and closed the AC. That reversed a decision already taken
with the operator, on the agent's own reasoning, and recorded the reversal as
settled. It was not the agent's call to make.**

**The standing decision is decision 3, unchanged: MIGRATE-THEN-RETIRE, and
explicitly NOT a permanent alias.** The AC offers exactly two outcomes —
*"announced and dated, or explicitly kept as a permanent alias"* — and this
section only ever had grounds for the first. It is still **OPEN**, because
"announced and dated" needs a date, and the date is the operator's.

What this section legitimately contributes is the **one blocker on the way**,
which is a sequencing fact rather than a reason to keep the endpoint:

`disableExecuteApiEndpoint` is a
one-line CDK property (confirmed present in `aws-cdk-lib` 2.257.0,
`RestApiProps.disableExecuteApiEndpoint`), it does not disturb the custom domain
mapping, and nothing deployed consumes execute-api as an origin. Flipping it is
trivial. What stops it is what still POINTS at it:

- `docs/scf/milestone-1-evidence.md` — the base URL in the evidence table, plus
  a worked `curl` on `/v1/backfill/status`;
- `docs/scf/milestone-1-form-answers.md` — two reviewer-runnable commands;
- `docs/scf/milestone-1-video-scenario.md` — the API base shown on screen.

Those are the URLs **we handed to SCF reviewers in a submitted milestone**.
Disabling the endpoint turns every one of them into a connection error, and it
does so silently from our side — nobody here would see it. A reviewer
re-checking M1 would.

🔑 **So this decision is DOWNSTREAM of the open question in the last AC** — are
the M1 evidence docs frozen as submitted, or maintained? Both answers permit
retirement, by different routes, and neither is ours to pick alone:

- **maintained** → migrate those URLs to the custom domain first, then disable;
- **frozen** → mark them explicitly historical ("as submitted on <date>; the
  API now serves at …"), then disable.

⚠️ **This is a PREREQUISITE, not a reprieve.** The endpoint is on today because
the M1 URLs have not been dealt with yet — not because it has been granted alias
status. Decision 3 says it goes.

**The remaining question for the operator is only the DATE**, and it has one
dependency: the M1-docs question is answered and every cited execute-api URL is
either migrated or explicitly marked historical. Then set
`disableExecuteApiEndpoint: true` on the `RestApi` in `api-gateway-stack.ts` and
deploy — CDK-expressible, so Tranche 3 AC 7 is unaffected.

⚠️ **Do not disable it before [[0121]] runs.** The load test's own script is
already portable — `packages/prices-api/loadtest/price_load.js:62` reads
`BASE_URL` from the environment and hardcodes nothing — so 0121 needs no code
change, only the right value passed. But a run launched from the stale
instructions on that branch would hit a disabled host and read as an outage
rather than a wrong URL.

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
- [ ] The custom domain did not fragment or bypass the stage cache.
      ⚠️ **REWORDED 2026-09-02 — the old text asked to "re-verify [[0122]]'s
      cache-hit behaviour", and there is nothing to re-verify: 0122 is still in
      `backlog/` and has never run.** There is no baseline this task can
      compare against, so the AC as written could never have closed. What 0126
      actually owns is the ONE property its own change could have broken — both
      hostnames map to the same stage, so they must share one cache — and the
      full TTL/hit-rate matrix stays where it already lives, in 0122. Runbook
      below; needs a key, so it is the operator's to run
- [x] Execute-api URL: the DECISION is made — **migrate the CloudFront origin
      to the custom domain, then retire** (decision 3 above). ⚠️ The decision
      is recorded; the MIGRATION is implementation and is tracked by the two
      criteria that follow
- [x] ~~`PortalHostingStack`'s API origin points at `config.apiDomain.domainName`~~
      with **no `originPath`**, `ALL_VIEWER_EXCEPT_HOST_HEADER` unchanged, and
      `verify-openapi-routes.mjs` updated to assert the new shape.
      ⚠️ That stack is NOT DEPLOYED (verified 2026-09-02), so this is a
      correctness fix for the day it first is — **not** a prerequisite for
      retiring execute-api, and not verifiable in a browser until then.
      ⚠️ **Deliberately NOT implemented here.** PR #276 (task 0195, another
      owner) DELETES `portal-hosting-stack.ts` outright and was still open on
      2026-09-02. Writing the origin fix into a file a pending PR removes buys
      a merge conflict and nothing else. **This criterion is resolved by
      whichever lands: #276 merging deletes its subject; #276 being abandoned
      makes the fix real work.**
      ✅ **RESOLVED 2026-09-02 12:10Z — #276 merged and
      `infra/src/lib/stacks/portal-hosting-stack.ts` no longer exists.**
      ⚠️ **"Moot" was the first word written here and it UNDERSTATES it — the
      goal is met, by a different mechanism than this task designed.** The
      portal no longer has a CloudFront origin to point at anything: it is
      served from the Explorer's distribution and reaches the backend through a
      BUILD-TIME `VITE_PORTAL_API_ORIGIN` (`web/portal/src/api-origin.ts`),
      which `infra/Makefile:127` derives from `apiDomain.domainName` in
      `envs/production.json` — the custom domain. `check-portal-api-origin`
      fails the build unless that value is `https://<host with a dot>`, so it
      cannot silently fall back to the relative same-origin layout, which on
      the shared host would return `index.html` for `/api/config`.
      **So: nothing points at execute-api, and the `originPath: '/production'`
      stage-prefix trap left the codebase with the file.** Holding the fix was
      the right call — writing it would have conflicted against a deletion.
      ⚠️ Not verified on the wire: `sorobanscan.rumblefish.dev/api/` answers
      `401` to an anonymous probe, so which origin the CURRENTLY DEPLOYED
      bundle was built with is unconfirmed. It is a build-time constant, so it
      only changes when `make sync-portal-explorer` runs
- [ ] Execute-api retirement: **announced and dated.** ⚠️ Briefly ticked on
      2026-09-02 on the strength of a "keep it serving" decision the agent
      recorded on its own — which REVERSED the migrate-then-retire call already
      taken with the operator, and is not an outcome this AC offers. Re-opened.
      The retirement stands; what is missing is the DATE, and one prerequisite
      before it: the submitted M1 evidence, form answers and video scenario all
      cite execute-api URLs, so those must be migrated or explicitly marked
      historical first or the announcement breaks links given to SCF reviewers.
      The flip itself is `disableExecuteApiEndpoint: true`, one CDK line
      (`aws-cdk-lib` 2.257.0), and it does not disturb the custom domain
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

### 🔴 The first cut shipped HALF the mechanism — caught in review of PR #277

Recorded because the near-miss is the instructive part: **the fix for this task
nearly re-shipped the defect this task exists to close.**

`addCorsPreflight` creates the `OPTIONS` mock and **nothing else**. The
`GET`/`POST` methods are Lambda PROXY integrations, so the real response's
`Access-Control-Allow-Origin` can only come from the handler — and the Rust
`CorsLayer` is scoped to the portal sub-router (`portal/mod.rs:240`), with a
test that actively asserted data routes carry none.

So the sequence was: preflight `204` ✅ → browser sends the real `GET` →
response has no allow-origin → **browser blocks it**, with the same opaque
`TypeError: Failed to fetch` as before. Net user-visible behaviour unchanged.
Confirmed on prod against `/api-docs-json`, a Lambda proxy route: `200`, no CORS
header.

🔑 **Every local signal was green.** Synth showed seven correct `OPTIONS`;
`verify-openapi-routes` passed; the portal suite passed. `curl -X OPTIONS`
would have passed too — which is exactly what this task's own Implementation
section warned about: *"Verify from a real browser, not only curl."* The warning
was written here and still nearly missed, because the preflight is the visible
half and the response header is the half nothing tests.

Fixed in `8321e9a`: `data_cors_layer()` over the data router — `*` via
`AllowOrigin::any()`, no credentials, matching headers and max-age. The value is
a CONSTANT deliberately: a reflected `Origin` cached in the stage cache would be
served to the next caller, the bleed [[0118]] measured on production. Each half
now carries a comment pointing at the other so neither can travel alone.

Two tests changed: `the_cors_layer_stops_at_the_portal_prefix` asserted the OLD
world (a data route with no allow-origin — true while `/v1` was uncallable, and
the reason it was), so it is rewritten to pin what still holds — the portal's
single origin and credentials never appear on a data route. A new test sends two
unrelated origins and asserts the same `*` both times.

Also corrected: `DATA_PREFLIGHT_MAX_AGE`'s rationale claimed an uncredentialed
`*` is cached across origins. It is not — browsers key the preflight cache per
origin regardless of the response.

### Second review round — the new test pinned the layer, not the surface

`8321e9a` closed the half-mechanism, and the test it added asserts the wildcard
on **`/health`**. That looked like the data surface and is not: `/health` is a
gateway `MockIntegration` (`api-gateway-stack.ts:311`), so **production never
routes it to this Lambda at all.** The test therefore proved `data_cors_layer`
was attached and nothing about the routes the task exists to fix. Every other
CORS assertion in `tests/portal.rs` rides on `/health` or `/api-docs-json` — so
after the fix, the suite still touched no `/v1` path.

The gap it leaves is the same shape as the one it was written to catch: a data
router re-nested after the `.layer()` call in `app()`, or a new `/v1` route
registered below it, keeps `/health` green while every browser call goes back to
being blocked — and `curl` stays green throughout.

Fixed in `1b20a68`: `a_real_v1_route_carries_the_wildcard_on_its_own_response`
drives `POST /v1/prices/batch` with `{"assets": []}`, which `post_batch` rejects
before `state.ch()` is reached — so it runs under `AppState::without_ch` like
the rest of the suite, no ClickHouse required. It is also the one route a
browser preflights unconditionally.

🔑 **It asserts the `400` and the `invalid_query` code as well as the header,
and that is the load-bearing part.** The layer wraps the fallback too, so an
unrouted path answers `404` carrying the same `*` — a header-only assertion
would pass against a route that had been deleted. The status is what proves the
request was ROUTED rather than merely wrapped.

Verified by detaching the layer: both data tests fail, the other 21 pass. 23
green with it attached; `cargo clippy --all-targets` and `cargo fmt` clean.

### ⚠️ Coordination — #276 MERGED 2026-09-02 12:10Z, and it did collide

The pre-merge check said #276's only hunks in `api-gateway-stack.ts` were
comments far from the `/v1` block. That held — but #276 grew a review-fix
commit (`94c55aa`) and landed on **`packages/prices-api/src/lib.rs` and
`packages/prices-api/tests/portal.rs`**, which is exactly where 0126's
half-mechanism fix lives. Merged into this branch as `a96e297`.

🔴 **The dangerous part merged CLEANLY.** 0195 amended
`the_cors_layer_stops_at_the_portal_prefix` from one side while 0126 rewrote it
from the other; git raised markers around the doc comments and the new tests,
and silently kept 0195's assertion that **`/health` carries NO
allow-origin** — in a tree where `data_cors_layer` gives it `*`. Committing the
conflicted files without reading the auto-merged middle would have shipped a red
test, and "fixing" it the obvious way (deleting the wildcard) would have undone
the task.

Resolved by keeping the property BOTH tasks still need — the portal's single
origin and its credentials never appear anywhere else — and dropping "no
allow-origin", which after today describes nothing this repo serves: 0195 gave
both OpenAPI copies `*` and 0126 gave `/v1` and `/health` `*`.

**0195 also handed this task a better test subject.** Its
`/v1/assets/not-an-asset/price` is a real data route that 400s in handler
validation before any DB call, so it runs without ClickHouse. The wildcard test
now covers it alongside the `POST /v1/prices/batch` case, giving one GET and one
POST data route.

⚠️ Two things checked because #276 could have broken them, both fine:
`openapi:verify-routes` still skips `OPTIONS` on the gateway side (the rule
survived the script's rewrite and still names this task), and the OpenAPI routes
carry 0195's `*` **once** — `data_cors_layer` is declared before they are
registered, and an axum layer covers only the routes already added.

### 🔬 Corrected by measurement: overlapping CORS does NOT duplicate the header

`data_cors_layer`'s own comment claimed that layering it over the portal would
emit **two** `Access-Control-Allow-Origin` headers, "which browsers reject
outright". **Measured on 2026-09-02 — it does not.** `CorsLayer` OVERWRITES:

| arrangement | `/api-docs-json` | `/health` | `/api/config` |
|---|---|---|---|
| layer before the spec routes (shipped) | `*` ×1 | `*` ×1 | portal origin |
| layer moved below the spec routes | `*` ×1 | `*` ×1 | portal origin |
| layer moved outside `auth::apply` | `*` ×1 | `*` ×1 | **`*`, credentials GONE** |

🔑 **The real failure is quieter than the one that was written down.** Nothing
duplicates; the portal's credentialed single origin is silently REPLACED, and
`HeaderMap::get` returns one value in every arrangement, so no test that reads
the header could tell the difference. Four existing portal tests do catch it, so
the constraint was already pinned — only the reasoning was wrong. Comment fixed
in place, the old claim named, same as `DATA_PREFLIGHT_MAX_AGE` before it.

⚠️ **A test asserting "exactly one allow-origin" was written and then DROPPED.**
With overwrite semantics there is no ordering that makes it fail. A test that
cannot fail is the thing this task exists to stop shipping, and keeping it
because it was already written would have been the same mistake in a new place.

⚠️ **#276 also settles decision 3 by removing its subject.** Its own comment
says the distribution that fronted execute-api as an origin "was retired by task
0195", and leaves the retire-or-alias call to this task. So the origin-migration
criterion above is moot the moment #276 merges, and the execute-api question
reduces to: disable the endpoint, or keep it. Nothing is gated on it.

## 📕 OPERATOR HAND-OVER — the two checks an agent cannot run

Both need a real API key, so they are yours. Neither is blocking: the first
closes a reworded AC, the second is the top AC and also needs PR #277 deployed.

### A. Does the custom domain share the stage cache? (~2 min, read-only)

The claim to test is narrow: `prices-api.sorobanscan.rumblefish.dev` and the
execute-api hostname are two doors onto **one stage**, so a response warmed
through either should be served to the other. If they did NOT share, every
cached route would silently halve its hit rate the day consumers moved over —
and nothing would look wrong, which is this task's recurring failure shape.

⚠️ **API Gateway REST does not emit `X-Cache`** — that is CloudFront's header,
and expecting it is 0122's AC 7. The evidence here is the CloudWatch counter
plus response time, and it should be reported as exactly that.

1. **[local machine]** Put a key in the shell without printing it. Take it from
   the portal, or from Secrets Manager — never paste it into a file or a task
   note:
   ```bash
   read -rs KEY && export KEY
   ```
2. **[local machine]** Pick the shortest-TTL cached route so a stale entry from
   an earlier probe cannot confuse the reading. `/price` is 10s
   (`CACHE_TTL.price`, `api-gateway-stack.ts:56`):
   ```bash
   CUSTOM=https://prices-api.sorobanscan.rumblefish.dev
   EXEC=https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production
   R=/v1/assets/native/price
   ```
3. **[local machine]** Warm through **execute-api**, then read through the
   **custom domain** inside the 10 s window. The cross-hostname direction is the
   whole point — same-host twice proves only that a cache exists:
   ```bash
   curl -s -o /dev/null -w 'warm  %{time_total}s\n' -H "x-api-key: $KEY" "$EXEC$R"
   curl -s -o /dev/null -w 'read  %{time_total}s\n' -H "x-api-key: $KEY" "$CUSTOM$R"
   ```
   ✅ **Checkpoint:** the second timing should drop to roughly the round-trip
   floor. If it matches the first, suspect per-domain fragmentation and say so
   rather than rounding it off — a single pair is weak evidence on its own.
4. **[local machine]** Confirm against the counter, which is the evidence that
   does not depend on timing noise. Run it after the pair above:
   ```bash
   aws cloudwatch get-metric-statistics --namespace AWS/ApiGateway \
     --metric-name CacheHitCount --dimensions Name=ApiName,Value=prices-production-api \
     --start-time "$(date -u -d '10 min ago' +%FT%TZ)" --end-time "$(date -u +%FT%TZ)" \
     --period 60 --statistics Sum --region eu-central-1
   ```
   ✅ **Checkpoint:** a non-zero `Sum` in the minute you ran step 3.
5. **[local machine]** Wait past the TTL and confirm the entry really expires —
   a cache that never expires also reads as "Hit" and would be a freshness bug
   on a 10 s contract:
   ```bash
   sleep 15 && curl -s -o /dev/null -w 'after ttl  %{time_total}s\n' \
     -H "x-api-key: $KEY" "$CUSTOM$R"
   ```

**Final test command** — the one line that answers the AC, both hostnames on the
same warmed entry:
```bash
for H in "$EXEC" "$CUSTOM"; do curl -s -o /dev/null -w "$H  %{time_total}s\n" -H "x-api-key: $KEY" "$H$R"; done
```

📌 Record the numbers here, and put anything about **TTL values or hit rates**
into [[0122]] instead — including the drift already spotted below.

### B. The browser check for AC 1 — after PR #277 deploys

⚠️ Do not attempt this before the deploy, and expect it to be **partially
blocked by [[0255]]**: the preflight and the `200` should now both work, but any
`/v1` 4xx still carries the portal's single origin, so a third-party page that
hits a 403 or 429 sees a CORS mismatch and reads it as a dead network.

1. **[any browser, on a page that is NOT `sorobanscan.rumblefish.dev`]** — the
   origin has to be a third party or the test proves nothing. A blank tab on
   `https://example.com` with devtools open is enough:
   ```js
   await (await fetch('https://prices-api.sorobanscan.rumblefish.dev/v1/assets/native/price',
     { headers: { 'x-api-key': '<key>' } })).json()
   ```
   ✅ **Checkpoint:** an object, not `TypeError: Failed to fetch`. In the
   Network tab the `OPTIONS` is `204` and the `GET` carries
   `access-control-allow-origin: *`. **Both halves must be visible** — the
   preflight alone passing is precisely the near-miss recorded above.
2. **[same tab]** Repeat once per route group (`/v1/assets`,
   `/v1/assets/{id}`, `/price`, `/ohlcv`, `/v1/oracles/{id}`,
   `/v1/backfill/status`, and `POST /v1/prices/batch`) — the AC says *every*
   data route, and the preflight is declared per resource.
3. **[same tab]** Then send it **without** the key and watch the 403 fail on
   CORS. That is 0255, not a regression, and it is worth seeing once so the
   report of 0255 is first-hand.

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
