---
id: "0195"
title: "Swagger UI, per-prefix SPA fallback, and the custom domain"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0183", "0124", "0126", "0161", "0184", "0185", "0163", "0164"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M3, epic-self-service-onboarding, cloudfront, swagger-ui, dns, slice-12]
milestone: 3
links:
  - "../archive/0161_FEATURE_portal-static-hosting-s3-cloudfront.md"
  - "../../../docs/scf/api-endpoints.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Twelfth slice, the remainder of [[0161]]. Three separate pieces of hosting
      work that [[0184]] deliberately left out because none of them is needed to
      serve a page or reach the API. Kept together because they are all edits to
      the same distribution.
---

# Swagger UI, SPA fallback, custom domain

## Summary

**Story:** *as a developer, the docs live next to the portal, deep links work,
and both are on a hostname we would put in a submission.*

Three independent pieces, one distribution:

| Piece | Why it is not in [[0184]] |
| --- | --- |
| Swagger UI at `/docs/*` | Tranche 3 AC 2's other half; a second bundle, no dependency either way |
| Per-prefix SPA fallback | A CloudFront Function — real work, and pointless while [[0185]]'s app has one route |
| Custom domain + certificate | Needs the domain-ownership conversation [[0126]] owns; the raw distribution URL works until then |

They can land in any order and none blocks the backend slices.

## Implementation

**Swagger UI**

- Served from `/docs/*` on the same distribution, from the same bucket.
- It is a **viewer pointed at `GET /api-docs-json`** ([[0124]]), not a checked-in
  copy of the spec. A checked-in copy drifts, which is the exact failure 0124
  spent a task preventing.
- Note the spec route is mounted on the API **root**, not under `/v1`
  (`this.api.root.addResource('api-docs-json')`). [[0184]] already routes it; if
  it did not, this page would load and its spec fetch would 404.

**Per-prefix SPA fallback**

- With one app it was enough to map 403/404 to `/index.html`. With several, a
  refresh on `/api-tokens/dashboard` must land on `/api-tokens/index.html` — the
  root document would boot the wrong app.
- **`customErrorResponses` cannot do this.** They are configured on the
  distribution and apply to every behaviour; there is no per-prefix error mapping
  to reach for. The options are a CloudFront Function on viewer request rewriting
  the path, or handling it at the origin.
- **The function must skip `<app>/api/*`.** Rewriting those to `index.html` would
  swallow every backend call the moment the API returned a 404, turning a clean
  JSON error into an HTML body.
- This is what unblocks [[0185]]'s deferred note about sub-routes — until it
  lands, the app must not depend on deep links.

**Custom domain**

- Certificate in **`us-east-1`** for CloudFront regardless of where the rest of
  the stack lives.
- Coordinated with [[0126]], which owns the domain decision for the API. Same
  zone, same ownership conversation — do not open a second one.
- **The Discord redirect URI must be re-pointed at the cutover** ([[0186]],
  owned by Adam). Sequence it explicitly: sign-in breaks silently otherwise, and
  the failure surfaces as a Discord error page rather than as anything in our
  logs.
- Update `docs/scf/api-endpoints.md` and `API_BASE_URL` together, so [[0124]]'s
  OpenAPI `servers` block and the quickstart ([[0163]]) name the same origin. The
  raw invoke URL keeps working; only one of the two is documented and supported.

**Also here:** finish the path-layout table [[0184]] started, as the recorded
convention a second frontend joins — `<app>/*` is that app's bundle, `<app>/api/*`
is its backend, `/api/*` rows always before the bundle row they sit inside.

## Acceptance Criteria

- [ ] **Ships closed.** Sign-in is confirmed on the new hostname ([[0183]])
      before the domain is advertised anywhere
- [ ] Swagger UI is served from the same distribution and renders the **live**
      spec from `/api-docs-json`, not a checked-in copy
- [ ] A refresh on `/api-tokens/<sub-route>` returns that app's `index.html`, not
      the root document
- [ ] A 404 from `/api-tokens/api/*` returns the API's JSON error, not the SPA
      bundle
- [ ] Portal and docs are reachable on the custom domain over TLS, certificate
      auto-renewing
- [ ] The Discord redirect URI is updated and sign-in works on the new hostname
      before the old one stops being advertised
- [ ] `docs/scf/api-endpoints.md` and `API_BASE_URL` name the same origin as the
      OpenAPI `servers` block and the quickstart
- [ ] The path convention is written down where the next frontend will find it
- [ ] Everything in CDK (Tranche 3 AC 7)

## Notes

- Cost: the certificate is free with ACM; the meaningful line is a Route 53
  hosted zone if a new one is needed.
- [[0163]] and [[0164]] both cite the documented URL. If the domain slips, they
  cite [[0184]]'s distribution URL instead — which is why 0184 wrote it into
  `api-endpoints.md` on day one.
