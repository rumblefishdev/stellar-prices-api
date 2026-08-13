---
id: "0184"
title: "Portal reachable at a URL — private S3, CloudFront, routing table, placeholder page"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0183", "0124", "0126", "0161", "0185", "0194", "0195"]
tags: [layer-infra, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, s3, cloudfront, slice-1]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0161_FEATURE_portal-static-hosting-s3-cloudfront.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      First slice of the re-sliced epic, carved out of [[0161]]. Deliberately
      the smallest thing that produces a URL: everything in 0161 that is not
      needed to serve one HTML file and reach the API is in [[0195]].
---

# Portal hosting — skeleton

## Summary

**Story:** *as a reviewer, I open a URL and get a page we own, served over TLS.*

That is the entire slice. No app, no auth, no API calls from the page — a
placeholder `index.html` behind a private bucket and a distribution, plus the
behaviour table that everything after this depends on.

Epic AC 1 ("portal accessible at a documented URL") is satisfiable here, weeks
before the portal does anything, which is the point: the URL can go into
`docs/scf/api-endpoints.md` and be reviewed while the rest is built.

## Context

Full detail lives in the superseded [[0161]] — read it, do not restate it. This
task takes only the load-bearing half.

Two things stay in slice 1 even though nothing yet uses them, because
retrofitting them invalidates decisions made downstream:

- **The API as a second origin on the same distribution.** This is what makes
  every later portal request same-origin. Without it [[0186]]'s `SameSite=Lax`
  cookie and the "no CORS" property are both wrong, and both are hard to notice
  until they fail in a browser rather than in `curl`.
- **`/api-tokens/api/*` ordered before `/api-tokens/*`.** Behaviours match in
  order. Get this wrong and every backend call is served the SPA bundle: a `200`
  full of HTML that reads as a JSON parse error, not as a routing bug.

## Implementation

- Private S3 bucket, no public objects, Origin Access Control only. A public
  bucket is the default way this ships wrong and Tranche 3 AC 6 audits it.
- CloudFront distribution, behaviours in this order:

  ```
  /api-tokens/api/*  → API Gateway   (MUST precede the next line)
  /api-tokens/*      → S3
  /v1/*              → API Gateway
  /api-docs-json     → API Gateway   (root-level — not under /v1)
  /health            → API Gateway   (root-level)
  ```

  `/docs/*` and the `<next-app>/*` convention are [[0195]]'s; the rule that
  produced this shape (`<app>/*` is a bundle, `<app>/api/*` is its backend) is
  recorded in [[0161]] and is worth keeping in a comment here.
- A placeholder `index.html` saying what this will be. It is thrown away by
  [[0185]] — do not invest in it.
- Everything in CDK (Tranche 3 AC 7). Deploy + invalidate from CI, not by hand.
- Write the distribution URL into `docs/scf/api-endpoints.md`.

**Deferred out of this slice, on purpose:** Swagger UI, the per-prefix SPA
fallback (a CloudFront Function — real work, and pointless while the app has one
route), the custom domain and its `us-east-1` certificate, and the caching /
cookie-forwarding policy for the portal prefix. The last one is only correct to
write once there is a session to forward — it lands with [[0186]] and is audited
by [[0194]].

## Acceptance Criteria

- [ ] **Ships closed.** The placeholder states the portal is not yet available;
      the `/api-tokens/api/*` behaviour is in place but every route behind it is
      governed by [[0183]]'s flag — this distribution *is* production
- [ ] `https://<distribution>/api-tokens/` returns our placeholder over TLS
- [ ] Bucket has no public access; objects reachable only through OAC
- [ ] `https://<distribution>/health` reaches the API, not S3
- [ ] `https://<distribution>/api-tokens/api/anything` reaches the API and
      returns the API's 404, never the placeholder HTML
- [ ] `/api-docs-json` reaches the API (it is mounted at the API root, so a
      table routing only `/v1/*` sends it to S3)
- [ ] Distribution and bucket expressed in CDK; CI deploys and invalidates
- [ ] URL recorded in `docs/scf/api-endpoints.md`

## Notes

- Sequencing: this is the first task of the epic and blocks nothing on Discord,
  AWS measurements or the backend. It can start today.
- Cost: CloudFront free tier covers this; the meaningful line is the Route 53
  zone, and only once [[0195]] takes the domain.
