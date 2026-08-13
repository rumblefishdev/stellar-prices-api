---
id: "0184"
title: "Portal reachable at a URL — private S3, CloudFront, routing table, placeholder page"
type: FEATURE
status: active
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
  - date: 2026-08-13
    status: active
    who: akot
    note: >
      Activated on `feat/0184_portal-hosting-skeleton`, branched off
      [[0183]]'s branch rather than `develop` — the `/api-tokens/api/*`
      gateway proxy this slice adds is what makes 0183's `config` route
      reachable in production, so the two are reviewed as a stack.
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

- [x] **Ships closed.** The placeholder states the portal is not yet available;
      the `/api-tokens/api/*` behaviour is in place but every route behind it is
      governed by [[0183]]'s flag — this distribution *is* production
- [x] `https://dojr4epgxo2qp.cloudfront.net/api-tokens/` returns our placeholder
      over TLS — `200 text/html`, 1727 bytes
- [x] Bucket has no public access; objects reachable only through OAC — all four
      block flags true on the live bucket, and a direct S3 URL returns `403`
- [x] `.../health` reaches the API, not S3 —
      `{"status":"ok","stack":"prices-production"}`
- [x] `.../api-tokens/api/anything` reaches the API and returns the API's 404,
      never the placeholder HTML — `404`, `0` bytes. This one line proves both
      halves: the portal is closed, **and** `/api-tokens/api/*` outranks
      `/api-tokens/*` (reversed, it would be a `200` full of HTML)
- [x] `/api-docs-json` reaches the API — returns the live OpenAPI 3.1.0 document
- [x] Distribution and bucket expressed in CDK; deploy invalidates
      — see the note below on what "from CI" turned out to mean here
- [x] URL recorded in `docs/scf/api-endpoints.md`

Also verified, beyond the written criteria: `/` redirects `302` to
`/api-tokens/`; `GET /api-tokens/api/config` answers `200 {"enabled":false}` with
`Cache-Control: no-store` while the portal is closed, which is what task 0185's
bundle will read; `/v1/assets` without a key returns the gateway's `403`, proving
portal traffic reaches API Gateway rather than S3 and that the viewer `Host`
header is not being forwarded.

> **The `/config` line describes the state after [[0183]] is deployed, not
> production as it stands.** Branches stack `develop → 0183 → 0184`, and 0183 is
> not merged, so the api-handler live today predates it: `PORTAL_ENABLED` is
> absent from the deployed Lambda's environment entirely and `/config` answers an
> empty `404` — via CloudFront and via execute-api alike. Nothing on this branch
> causes that and nothing here fixes it; `enabled: bool`, `config_handler` and
> the gate's `CONFIG_PATH` exemption are all 0183's code. It resolves when 0183
> merges and Compute is deployed.
>
> Worth knowing because the two states are otherwise indistinguishable: 0183
> makes a gated path and a nonexistent one byte-identical on purpose, so an empty
> `404` under the prefix proves nothing either way. `/config` is the single path
> where they differ — it is exempt from the gate and answers `200` in **both**
> flag states — which is what makes it the probe worth running after the next
> Compute deploy. Task 0185's bundle reads it and would today receive a `404`
> rather than `{"enabled":false}`.

## Implementation Notes

Five files, no Rust changes — [[0183]] already ships the behaviour behind the
gate; this slice only makes it reachable.

- `infra/src/lib/stacks/portal-hosting-stack.ts` — new stack. Private bucket
  (`BLOCK_ALL`, OAC, `RETAIN`), distribution with two origins, the behaviour
  table, a `BucketDeployment` that uploads and invalidates in one step, and the
  distribution domain published to
  `/prices/production/portal-distribution-domain` for [[0186]] and [[0195]].
- `infra/src/lib/stacks/api-gateway-stack.ts` — `ANY /api-tokens/api/{proxy+}`,
  keyless. This is the "door" [[0183]]'s note said would arrive here. The stage
  cache is kept off it by the `/*` `*` default entry, which now declares
  `cachingEnabled: false` — the per-route form is impossible, see the issue
  below.
- `tools/scripts/verify-openapi-routes.mjs` — see the issue below.
- `infra/assets/portal-placeholder/index.html`, `infra/src/lib/app.ts`,
  `infra/Makefile` (`deploy-production-portal`), `docs/scf/api-endpoints.md`.

Verified at synth: behaviour order (`/api-tokens/api/*` at index 0,
`/api-tokens/*` at index 1), all four `PublicAccessBlockConfiguration` flags
true, bucket policy grants only `cloudfront.amazonaws.com` `s3:GetObject`, API
origin carries `originPath: /production`, API behaviours resolve to
`CACHING_DISABLED` + `ALL_VIEWER_EXCEPT_HOST_HEADER`. `lint`, `typecheck`,
`format:check` and `openapi:verify-routes` all pass.

Verified against the deployed distribution — every response code in the
acceptance criteria above was measured, not assumed. Production was also checked
directly through execute-api after the failed first attempt (see below):
`/health` `200`, `/api-docs-json` `200`, `/v1/assets` `403`, and
`Prices-production-{ApiGateway,Compute}` both `UPDATE_COMPLETE`.

## Issues Encountered

- **`openapi:verify-routes` would have hard-failed CI, and not obviously.**
  The script compares gateway routes against the OpenAPI document in both
  directions, and it `exit(1)`s on any `ANY` method ("no OpenAPI equivalent").
  Declaring explicit verbs instead would have failed the other way, as
  `undocumented`. The portal is deliberately absent from the document
  (`portal/mod.rs`), so the script now skips the `/api-tokens/api/` prefix on
  the gateway side — and, symmetrically, **rejects** a portal path appearing in
  the document, so the skip cannot become a hole. Same stance the script already
  takes on documented `OPTIONS`.
- **A greedy `{proxy+}` cannot carry a stage method setting — first deploy
  failed on it.** The entry `{ resourcePath: '/api-tokens/api/{proxy+}',
  httpMethod: 'ANY', cachingEnabled: false }` synthesized fine and a read-only
  change set *accepted* it; the apply rejected it with `Invalid method setting
  path: /api-tokens/api/{proxy+}/ANY/caching/enabled`. API Gateway assembles the
  setting path as `/{resourcePath}/{httpMethod}/caching/enabled` and the `+` in
  a greedy segment makes it unparseable. Multi-segment paths are otherwise fine
  — `/v1/assets/{asset_identifier}/price` is deployed and works — so the `+` is
  the whole problem. The stack rolled back cleanly and production was never
  affected.

  Resolved by making the invariant explicit instead of per-route: the default
  `/*` `*` entry now declares `cachingEnabled: false`. That was **already** the
  effective behaviour, because the hand-written default entry never declared
  caching and API Gateway treats an undeclared method as uncached — but it was
  an accident of an omission rather than a stated rule, and one day someone
  would have written `true` there and silently switched on the cache for the
  portal's session traffic. The six routes that opt in are unaffected: a more
  specific entry wins. Zero behaviour change on what is deployed today, and the
  guarantee now survives an edit to the default.
- **`defaultRootObject` cannot serve `/api-tokens/index.html`.** It is a
  distribution-level property and applies to `/` only; per-directory index
  documents otherwise need an S3 *website* endpoint, which requires a public
  bucket and would trade away the OAC acceptance criterion. Resolved with a
  six-line CloudFront Function — see the decision below.

## Design Decisions

### From Plan

1. **Separate `PortalHostingStack`** rather than folding the bucket and
   distribution into `ApiGatewayStack`. It needs only `restApiId` from that
   stack, so the cross-stack export is one string, and the portal can be
   deployed and destroyed without touching the API.
2. **`ANY {proxy+}` rather than enumerated routes.** [[0183]] gates by prefix
   precisely so a later slice adds a route without editing the gate; enumerating
   at the gateway would reintroduce the per-slice CDK change it avoided.

### Emerged

3. **A directory-index CloudFront Function** (chosen with Adam, 2026-08-13).
   Six lines on viewer request: append `index.html` to a path ending in `/`.
   This is **not** [[0195]]'s SPA fallback, which rewrites unknown *sub-paths*
   per prefix and is real work. Associated with the S3 behaviours only —
   attaching it to an API behaviour would rewrite backend calls, which is the
   failure [[0161]] flagged.
4. **`/` 302-redirects to `/api-tokens/`.** The distribution root has no app of
   its own until [[0195]], and the alternative was a bare `AccessDenied` XML at
   the URL a reviewer is most likely to trim to. One line in the same function,
   removed when the root gets real content.
5. **Edge caching off on every API behaviour, not just the portal's.** The
   obvious `CACHING_OPTIMIZED` forwards no request headers, so `x-api-key` would
   be stripped and every keyed route would 403. Edge caching for the data routes
   is a separate decision with its own correctness argument; the gateway stage
   cache is untouched and still does the work.
6. **`apiBaseUrl` deliberately unchanged.** [[0161]] wanted the documented base
   moved to the distribution. Doing it now, when [[0195]]'s custom domain moves
   it again within weeks, would change the OpenAPI `servers` promise twice.
   Deferred to [[0195]], and the reasoning is written into
   `docs/scf/api-endpoints.md` so it is not re-litigated.
7. **`RETAIN` on the bucket.** The content is disposable, but it is the live
   origin of a live distribution; a stack delete that also empties it turns a
   rollback into an outage.

## Notes

- **Deployed 2026-08-13.** Distribution `EU8O3ADXFZP5U` at
  `dojr4epgxo2qp.cloudfront.net`; portal at
  `https://dojr4epgxo2qp.cloudfront.net/api-tokens/`. Bucket
  `prices-production-portalhosti-portalbucketf34416c0-ma76zxfgmn0x`. Stack
  creation took ~4.5 minutes including CloudFront propagation, not the 5–10 the
  plan budgeted.
- Cost: CloudFront free tier covers this; the meaningful line is the Route 53
  zone, and only once [[0195]] takes the domain.
- **"CI deploys and invalidates" had nothing to stand on.** There is no workflow
  that deploys infrastructure — `ci.yml` synthesizes only, and every deploy in
  this repo is `make -C infra deploy-production*` run by an operator. What the
  criterion is actually protecting against is a hand-run `aws s3 sync` that
  someone forgets to follow with an invalidation, and `BucketDeployment` closes
  that: upload and invalidation happen inside `cdk deploy`, or neither does.
  Building a deploy pipeline is a separate, much larger task and is not smuggled
  in here.
- **A change set is not a validator.** The deploy-time risk flagged before
  shipping did fire, and `cdk diff` gave no warning of it: `diff` builds a real
  read-only change set, and that change set *accepted* the method setting the
  apply then rejected. Worth remembering the next time a clean change set reads
  as reassurance about anything other than resource-level replacement.
- Sequencing: this was the first task of the epic and blocked nothing on
  Discord, AWS measurements or the backend.
