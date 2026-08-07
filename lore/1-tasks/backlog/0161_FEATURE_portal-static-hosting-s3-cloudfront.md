---
id: "0161"
title: "Portal hosting — S3 + CloudFront for the onboarding site and the Swagger UI docs"
type: FEATURE
status: backlog
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
---

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

  ```
  <domain>/api-tokens/*  → S3   (this portal — first tenant)
  <domain>/docs/*        → S3   (Swagger UI)
  <domain>/v1/*          → API Gateway (data routes)
  <domain>/<next-app>/*  → S3   (later frontends, same shape)
  ```

  One distribution, one certificate, one DNS record, one invalidation step.
  Write the rule down so the second frontend does not invent a second scheme.
- **The API shares the distribution as a second origin** — this is what makes
  the portal's requests same-origin, so CORS ([[0126]]) never enters the picture
  for portal traffic and the session cookie can be `SameSite=Lax` ([[0159]]).
  Settled together with [[0159]]'s route placement, as one decision.
- **SPA fallback is per prefix, not global.** With one app it was enough to map
  403/404 to `/index.html`. With several, a refresh on `/api-tokens/dashboard`
  has to land on `/api-tokens/index.html` — the root document would boot the
  wrong app. That needs either a behaviour per prefix or a CloudFront Function
  rewriting the path. Real work; it was not here before.
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
      same-origin and no CORS is involved
- [ ] A refresh on a sub-page of `/api-tokens/` returns that app's
      `index.html`, not the root document
- [ ] CI deploys the bundle and invalidates the cache; no manual step
- [ ] Certificate valid and auto-renewing; domain coordinated with [[0126]]
- [ ] URL recorded in `docs/scf/api-endpoints.md`
- [ ] Everything expressed in CDK (Tranche 3 AC 7)

## Notes

- Cost is small but not zero: CloudFront has a free tier and this is a
  low-traffic site, so the meaningful line is the certificate (free with ACM)
  and the Route 53 hosted zone if a new one is needed.
- Sequencing with [[0159]]: the OAuth redirect URI must match the final
  hostname. Landing this task first gives that hostname something stable to
  point at.
