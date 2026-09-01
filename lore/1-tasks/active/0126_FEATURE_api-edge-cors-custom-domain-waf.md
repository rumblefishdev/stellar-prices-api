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

## Acceptance Criteria

- [ ] Cross-origin `GET` from a browser page against every data route succeeds,
      preflight included
- [ ] `x-api-key` is in the allowed-headers list; `OPTIONS` requires no API key
      and does not invoke Lambda
- [ ] Allowed-origin policy decided and recorded
- [x] Custom domain resolves and serves the API over TLS; certificate valid and
      auto-renewing — **delivered by [[0194]]**, verified 2026-09-01
- [ ] The two CORS policies on this gateway (portal: one origin + credentials;
      data routes: `*`, no credentials) are reconciled DELIBERATELY, with the
      reason recorded — not left looking like one of them is a mistake
- [ ] Gateway-level `DEFAULT_4XX`/`DEFAULT_5XX` responses do not leak the
      portal's single `Access-Control-Allow-Origin` onto data-route errors
- [x] Every documented URL (§4, OpenAPI `servers`, evidence docs) updated
      consistently — **done by [[0194]]** (`apiBaseUrl`, `api-endpoints.md`)
- [ ] Routing re-verified after the base-path mapping, given
      `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true`
- [ ] [[0122]]'s cache-hit behaviour re-verified through the custom domain
- [ ] Execute-api URL: retirement announced and dated, OR explicitly kept as a
      permanent alias. It still serves today; the DECISION is what is missing
- [ ] WAF decision recorded with reasoning, cost, and a reversal trigger —
      deployed in count mode first if the answer is yes
- [ ] All of it expressed in CDK (Tranche 3 AC 7 requires clean-account
      reproducibility)

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
