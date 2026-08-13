---
id: "0161"
title: "Portal hosting — S3 + CloudFront for the onboarding site and the Swagger UI docs"
type: FEATURE
status: superseded
related_adr: []
related_tasks: ["0124", "0126", "0159", "0162", "0164"]
tags: [layer-infra, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, s3, cloudfront, swagger-ui, dns]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../docs/scf/api-endpoints.md"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      Epic AC 1 (portal at a documented URL) plus Tranche 3 AC 2's "Swagger UI
      deployed" half, folded in here because both are static assets behind the
      same distribution. Independent of the backend — can land early and serve
      a placeholder.
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Scope widened at the 2026-08-07 meeting: several frontends will share this
      domain, so the path layout becomes a convention rather than a one-off.
      First prefix is `/api-tokens/`. The API shares the distribution as a second
      origin, which settles [[0159]]'s route placement and removes CORS from
      portal traffic.
  - date: 2026-08-13
    status: superseded
    who: akot
    by: ["0184", "0195"]
    note: >
      Superseded by the epic's re-slice into vertical increments, split into
      the part that unblocks everything else and the part that does not.
      [[0184]] is the walking skeleton: private bucket, OAC, distribution, the
      behaviour table including `/api-tokens/api/*` **before** `/api-tokens/*`,
      and a placeholder page. That ordering and the API-as-second-origin stay in
      the first slice on purpose — they are what makes every later request
      same-origin, they cost a few lines, and retrofitting them would invalidate
      the session-cookie design. [[0195]] takes Swagger UI, the per-prefix SPA
      fallback (a CloudFront Function, real work, and needed only once the app
      has more than one route) and the custom domain and certificate.
---

> **Superseded 2026-08-13 by [[0184]] (skeleton + routing) and [[0195]] (Swagger
> UI, SPA fallback, domain).** The three silent failure modes documented below
> are inherited by [[0184]] verbatim.

# Portal hosting — S3 + CloudFront

## Summary

The place the portal and the API docs actually live: a private S3 bucket behind
a CloudFront distribution, with a URL we can write into the Tranche 3
submission. The epic fixes the hosting choice; this task builds it and decides
the path layout the two tenants share.

Deliberately independent of [[0162]] and [[0160]] — the distribution can go up
with a placeholder page and be filled in later, which also means the URL can be
documented and reviewed before the portal is finished.

## Context

The epic hosts the portal "alongside the OpenAPI/Swagger UI docs", and Tranche 3
AC 2 requires Swagger UI deployed. Both are static bundles, both want the same
CDN, and neither wants its own certificate and DNS record — so they share a
distribution and differ by path.

[[0124]] already exposes the spec itself at `GET /api-docs-json` from the API,
so Swagger UI here is a viewer pointed at that URL, not a second copy of the
document. That matters: a checked-in copy would drift, which is the exact
failure [[0124]] spent a task preventing.

## Implementation

**From the epic**

- Static site on S3 + CloudFront, portal and docs together.
- Reachable at a URL we publish.

**Follows from the epic, but not stated in it**

- **Private bucket, no public objects**, access via Origin Access Control only.
  A public bucket is the default way this ships wrong and it is audited by
  Tranche 3 AC 6.
- **Path layout — settled 2026-08-07.** Several frontends will share this
  domain, so this is a convention the next app joins, not a one-off:

  The rule, settled 2026-08-07: **`<app>/*` is that app's bundle, `<app>/api/*`
  is that app's backend.** Behaviours are evaluated in order and the first match
  wins, so each `/api/*` entry must be listed **before** the bundle entry it
  sits inside.

  ```
  <domain>/api-tokens/api/*  → API Gateway   (this portal's backend — MUST precede the next line)
  <domain>/api-tokens/*      → S3            (this portal's bundle)
  <domain>/docs/*            → S3            (Swagger UI bundle)
  <domain>/v1/*              → API Gateway   (partner data routes)
  <domain>/api-docs-json     → API Gateway   (root-level — the spec Swagger UI fetches)
  <domain>/health            → API Gateway   (root-level)
  <domain>/<next-app>/api/*  → API Gateway   (later frontends, same shape)
  <domain>/<next-app>/*      → S3
  ```

  One distribution, one certificate, one DNS record, one invalidation step. The
  convention scales: a new frontend adds two rows and invents nothing.

  Three ways this table goes wrong, all silent:

  - **Ordering.** `/api-tokens/api/*` after `/api-tokens/*` means every portal
    endpoint is served the SPA bundle instead of reaching the API. The response
    is a `200` full of HTML, which reads as a JSON parse error rather than a
    routing bug.
  - **Omitting the backend row entirely.** The endpoints fall through to S3, and
    the same-origin property that [[0159]]'s `SameSite=Lax` and the "no CORS"
    claim below both rest on never holds. The chosen prefix is recorded in
    [[0159]] as well — keep the two in step.
  - **The root-level API routes.** `/api-docs-json` and `/health` are mounted on
    the API root, not under `/v1`
    (`api-gateway-stack.ts` — `this.api.root.addResource('api-docs-json')`), so
    a table that only routes `/v1/*` sends them to S3. The Swagger UI AC below
    then fails: the page loads and its spec fetch 404s.
- **The API shares the distribution as a second origin** — this is what makes
  the portal's requests same-origin, so CORS ([[0126]]) never enters the picture
  for portal traffic and the session cookie can be `SameSite=Lax` ([[0159]]).
  Settled together with [[0159]]'s route placement, as one decision.
- **SPA fallback is per prefix, not global.** With one app it was enough to map
  403/404 to `/index.html`. With several, a refresh on `/api-tokens/dashboard`
  has to land on `/api-tokens/index.html` — the root document would boot the
  wrong app. **`customErrorResponses` cannot do this**: they are configured on
  the distribution and apply to every behaviour, so there is no per-prefix error
  mapping to reach for. The options are a CloudFront Function on viewer request
  rewriting the path, or handling it at the origin. Real work; it was not here
  before. **The function must skip `<app>/api/*`** — rewriting those to
  `index.html` would swallow every backend call the moment the API returned a
  404, turning a clean error into an HTML body.
- **Certificate in `us-east-1`** for CloudFront regardless of where the rest of
  the stack lives, and coordinated with [[0126]], which owns the domain
  decision for the API. Same zone, same ownership conversation — do not open a
  second one.
- **Deploy and invalidate in CI**, not by hand. A stale bundle behind a CDN is
  indistinguishable from a broken deploy, and Tranche 3 AC 7 wants the whole
  thing reproducible from CDK in a clean account.
- **Write the URL into `docs/scf/api-endpoints.md`** — the running record
  [[0124]] created for exactly this purpose, and the thing [[0164]] cites as
  evidence for epic AC 1.

## Acceptance Criteria

- [ ] Distribution serves the portal over TLS at a documented URL; bucket has no
      public access
- [ ] Swagger UI served from the same distribution, rendering the live spec from
      `/api-docs-json` rather than a checked-in copy
- [ ] Portal served from `/api-tokens/`; path layout recorded as a convention
      the next frontend can follow without inventing a second scheme
- [ ] API reachable through the same distribution, so portal requests are
      same-origin and no CORS is involved — including `/api-tokens/api/*` and
      the root-level `/api-docs-json`, not just `/v1/*`
- [ ] `/api-tokens/api/*` is ordered **before** `/api-tokens/*`; a call to an
      endpoint returns the API's response, never the SPA bundle
- [ ] The portal prefix's behaviour disables caching and forwards the session
      cookie; a signed-in request reaches the origin still signed in ([[0160]])
- [ ] A refresh on a sub-page of `/api-tokens/` returns that app's
      `index.html`, not the root document
- [ ] CI deploys the bundle and invalidates the cache; no manual step
- [ ] Certificate valid and auto-renewing; domain coordinated with [[0126]]
- [ ] URL recorded in `docs/scf/api-endpoints.md` as **the** base URL, and
      `API_BASE_URL` updated to match so [[0124]]'s OpenAPI `servers` block and
      the quickstart ([[0163]]) name the same origin. The raw invoke URL keeps
      working; only one of the two is documented and supported
- [ ] Everything expressed in CDK (Tranche 3 AC 7)

## Notes

- Cost is small but not zero: CloudFront has a free tier and this is a
  low-traffic site, so the meaningful line is the certificate (free with ACM)
  and the Route 53 hosted zone if a new one is needed.
- Sequencing with [[0159]]: the OAuth redirect URI must match the final
  hostname. Landing this task first gives that hostname something stable to
  point at.
