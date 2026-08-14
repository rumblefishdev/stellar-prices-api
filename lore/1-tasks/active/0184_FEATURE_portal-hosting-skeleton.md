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
  - date: 2026-08-14
    status: active
    who: akot
    note: >
      Record reconciled with the code after review. The last two commits had
      replaced the greedy `{proxy+}` with two non-greedy levels plus a
      method-level throttle, and added access logs, upload `Cache-Control` and
      the trailing-slash redirect, none of which were written down: decision 2
      is marked superseded and decisions 8–11 record what replaced it.
      `docs/scf/api-endpoints.md` and the docblock in `verify-openapi-routes.mjs`
      described the old mapping and were corrected. Still open, and the reason
      this is not `done`: the branch is ahead of the deployed distribution.
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

> **The `/config` line was false when it was written and became true by
> accident.** Branches stack `develop → 0183 → 0184`, so for most of this task
> the api-handler live in production predated 0183: `PORTAL_ENABLED` was absent
> from the deployed Lambda's environment entirely and `/config` answered an empty
> `404`, via CloudFront and via execute-api alike. What changed it is not a merge
> — it is that the 2026-08-14 gateway deploy pulled `Prices-production-Compute`
> in as a dependency stack, because the Makefile targets do not pass
> `--exclusively`. `enabled: bool`, `config_handler` and the gate's `CONFIG_PATH`
> exemption are all 0183's code, now live ahead of 0183's own merge. See the note
> under Implementation Notes.
>
> Worth knowing because the two states are otherwise indistinguishable: 0183
> makes a gated path and a nonexistent one byte-identical on purpose, so an empty
> `404` under the prefix proves nothing either way. `/config` is the single path
> where they differ — it is exempt from the gate and answers `200` in **both**
> flag states — which is what makes it the probe worth running after any Compute
> deploy. It is also the one task 0185's bundle reads, so it is the difference
> between that slice booting and that slice seeing a `404`.

## Implementation Notes

Eight files, no Rust changes — [[0183]] already ships the behaviour behind the
gate; this slice only makes it reachable.

- `infra/src/lib/stacks/portal-hosting-stack.ts` — new stack. Private bucket
  (`BLOCK_ALL`, OAC, `RETAIN`), distribution with two origins, the behaviour
  table, a `BucketDeployment` that uploads and invalidates in one step and
  stamps `Cache-Control: public, max-age=0, must-revalidate` on what it
  uploads, a
  private access-log bucket with a 90-day expiry, and the distribution domain
  published to `/prices/production/portal-distribution-domain` for [[0186]] and
  [[0195]].
- `infra/src/lib/stacks/api-gateway-stack.ts` — `GET`, `POST` and `DELETE` on
  `/api-tokens/api/{proxy+}`, keyless, each with a method-level throttle of its
  own (10 req/s, burst 40) and `cachingEnabled: false`. This is the "door"
  [[0183]]'s note said would arrive here. The stage cache is *also* off by
  default for anything that does not opt in, via the `/*` `*` entry — see the
  issue below for why both statements exist.
- `tools/scripts/verify-openapi-routes.mjs` — see the issue below. Beyond the
  route comparison it now guards the CloudFront routing table: the first
  behaviour matching a portal backend path must be `/api-tokens/api/*`, must
  target the execute-api origin, must allow the write verbs, and that origin
  must carry a single-segment `originPath`. All four fail silently in
  production and are free to assert at synth.
- `.github/workflows/ci.yml` — `portal-hosting-stack.ts` added to the `rust`
  paths filter, which is the only job that runs those guards. Without it,
  reordering the behaviour table is a pure `infra/**` change and the guard
  never executes.
- `infra/assets/portal-placeholder/index.html`, `infra/src/lib/app.ts`,
  `infra/Makefile` (`deploy-production-portal`), `docs/scf/api-endpoints.md`.

Verified at synth: behaviour order (`/api-tokens/api/*` at index 0,
`/api-tokens/*` at index 1), all four `PublicAccessBlockConfiguration` flags
true, bucket policy grants only `cloudfront.amazonaws.com` `s3:GetObject`, API
origin carries `originPath: /production`, API behaviours resolve to
`CACHING_DISABLED` + `ALL_VIEWER_EXCEPT_HOST_HEADER`, and the assembled
`methodSettings` array carrying all three portal entries (one per verb) with
their throttle in **both** arms of the `cacheEnabled` branch. `lint`, `typecheck`, `format:check`
and `openapi:verify-routes` all pass.

Verified against the deployed distribution — every response code in the
acceptance criteria above was measured, not assumed. Production was also checked
directly through execute-api after the failed first attempt (see below):
`/health` `200`, `/api-docs-json` `200`, `/v1/assets` `403`, and
`Prices-production-{ApiGateway,Compute}` both `UPDATE_COMPLETE`.

> **Production does not match this branch, and the gateway is in an
> intermediate state.** Every acceptance criterion above still holds live, but:
>
> - the gateway maps `ANY {proxy}` + `{proxy}/{sub}` with **no throttle** —
>   neither the original `{proxy+}` nor decision 12's shape, left there by the
>   2026-08-14 deploy attempt. Behaviour is correct at depth 1–2 and `403` at
>   depth 3.
> - the access-log bucket, `Cache-Control` on the uploaded objects and the
>   trailing-slash redirect are code-only; `/api-tokens` answers `403
>   AccessDenied` rather than `302`.
> - `Prices-production-Compute` **was** deployed (2026-08-14 09:42) as a side
>   effect of `cdk deploy` pulling in dependency stacks — the Makefile targets
>   do not pass `--exclusively`. So [[0183]]'s handler and `PORTAL_ENABLED=false`
>   are live ahead of that task's merge, and `/config` now answers
>   `200 {"enabled":false}` with `no-store`.
>
> Getting to decision 12's shape needs **two** gateway deploys and then the
> portal, because `{proxy}` and `{proxy+}` cannot both be children of
> `/api-tokens/api` mid-update (see the issue below). Concretely:
>
> 1. `deploy-production-apigateway` from a working tree with the `portalProxy`
>    block and `portalSettings` commented out — this deletes the live `{proxy}`
>    and `{proxy}/{sub}` resources. **The portal prefix answers `403` for the
>    length of the gap**, which is why it happens before 0185 has a bundle that
>    calls it and not after.
> 2. `deploy-production-apigateway` from the branch as committed — creates
>    `{proxy+}`, its three verbs and the per-verb throttle.
> 3. `deploy-production-portal` — access logs, upload `Cache-Control`, the
>    redirect function.
>
> Step 1 is a *local* edit that must not be committed; the alternative
> (deleting the two resources with `aws apigateway delete-resource`) leaves the
> stack drifted against a template that still declares them, which is worse.
>
> Probes worth re-running afterwards: `/api-tokens` → `302`, a `Cache-Control`
> header on the placeholder, `/api-tokens/api/a/b/c` → empty `404` (greedy
> matches any depth again), and a throttle entry per verb in the deployed stage.
> Then delete this note, and the matching one in `docs/scf/api-endpoints.md`.

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
- **`ANY` cannot carry a stage method setting. The `+` was never the problem —
  and believing it was cost an outage.** The entry
  `{ resourcePath: '/api-tokens/api/{proxy+}', httpMethod: 'ANY',
  cachingEnabled: false }` synthesized fine and a read-only change set
  *accepted* it; the apply rejected it with `Invalid method setting path:
  /api-tokens/api/{proxy+}/ANY/caching/enabled`. The obvious reading — API
  Gateway assembles the path as `/{resourcePath}/{httpMethod}/{setting}`, a `+`
  makes it unparseable — is **wrong**, and it survived two commits because
  `/v1/assets/{asset_identifier}/price/GET/...` works and appeared to confirm
  it.

  Measured on 2026-08-14 against three throwaway REST APIs, one variable at a
  time:

  | setting path                            | verdict  |
  | --------------------------------------- | -------- |
  | `/lit/GET/caching/enabled`              | accepted |
  | `/pg/{p}/GET/caching/enabled`           | accepted |
  | `/g/{proxy+}/GET/caching/enabled`       | accepted |
  | `/g/{proxy+}/GET/throttling/rateLimit`  | accepted |
  | `/p/{proxy+}/POST/throttling/rateLimit` | accepted |
  | `/lit/ANY/caching/enabled`              | REJECTED |
  | `/pa/{p}/ANY/caching/enabled`           | REJECTED |
  | `/deep/{p}/{s}/ANY/caching/enabled`     | REJECTED |
  | wildcard on the verb alone              | REJECTED |

  A greedy segment is fine, braces are fine, depth is fine, and the only
  wildcard the service accepts is the both-segment `/*` `*`. `ANY` is the whole
  problem. The fix is to enumerate verbs on a single `{proxy+}` —
  `PORTAL_API_METHODS`, currently `GET`/`POST`/`DELETE`, which covers every
  route in the epic. The residual cost moves from paths to verbs: an unlisted
  verb gets the gateway's `403` rather than the gated `404`.

  It took three commits to get there, and two of the three artefacts survive in
  the code because they answer different questions.

  *First*, on the wrong diagnosis, the invariant was moved off the route and
  onto the default: the `/*` `*` entry now declares `cachingEnabled: false`.
  That was **already** the effective behaviour, because the hand-written default
  entry never declared caching and API Gateway treats an undeclared method as
  uncached — but it was an accident of an omission rather than a stated rule,
  and one day someone would have written `true` there and silently switched on
  the cache for the portal's session traffic. The seven routes that opt in
  (`/api-docs-json` and the six `/v1` reads) are unaffected: a more specific
  entry wins. Zero behaviour change on what is deployed, and the guarantee now
  survives an edit to the default.

  *Then* review reopened it: the wildcard states a caching default but cannot
  express a throttle, and a stage-wide default throttle is exactly what the
  portal needed to escape. Still believing the `+` was the obstacle, the mapping
  dropped it for two non-greedy levels (decision 8) — which is what broke
  production, below.

  *Finally*, with the table above measured rather than inferred, the greedy
  segment came back and `ANY` went (decision 12). The wildcard stays: it states
  the caching default for routes that do not exist yet, which is worth having on
  its own.

  The narrow lesson is the one the table states and the one both wrong turns
  cost: braces are fine, depth is fine, a greedy `+` is fine. `ANY` is what
  breaks, and it breaks *every* per-route stage setting, not just caching.
- **The migration off `{proxy+}` broke the portal prefix in production for ~20
  minutes (2026-08-14).** Worth recording in full, because two separate AWS
  behaviours combined and neither is obvious.

  *First*, `A sibling ({proxy+}) of this resource already has a variable path
  part -- only one is allowed`. A resource may have at most one variable child,
  and CloudFormation creates before it deletes, so `{proxy}` and `{proxy+}`
  would have coexisted mid-update. Any change **replacing** a path-parameter
  resource therefore needs two deploys: one that removes it, one that adds the
  replacement. `cdk diff` shows this as an unremarkable create + delete.

  *Second*, the two-phase deploy was run on the wrong diagnosis (above), so
  phase 1 removed `{proxy+}` and phase 2 was rejected for the same `ANY` reason
  as the original attempt. That left the gateway with **no** portal resources,
  and — surprisingly — `/api-tokens/api/*` answered `500 {"message": "Internal
  server error"}` rather than the `403` an unmapped path normally gets. The
  stage's method settings were clean, so this looks like a stale deployment left
  by the rollback; a subsequent deploy that recreated the resources cleared it.

  Resolved by redeploying the resources without the rejected method settings.
  Data routes (`/v1/*`, `/health`, `/api-docs-json`) and the portal page were
  unaffected throughout, and nothing consumes the portal prefix yet.

  Two things to carry forward: **budget two deploys whenever a path-parameter
  resource changes shape**, and do not treat "the rollback completed" as "the
  stage is serving what it served before".
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
   **Superseded by decision 8** — the *property* survives, the greedy segment
   does not.

### Emerged

3. **A directory-index CloudFront Function** (chosen with Adam, 2026-08-13). A
   handful of lines on viewer request: append `index.html` to a path ending in
   `/` — later joined by decisions 4 and 11 in the same function.
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
8. **Two non-greedy levels — `{proxy}` and `{proxy}/{sub}` — and a throttle of
   their own** (from review, 2026-08-14; **superseded the same day by decision
   12**, because the reason for dropping the greedy segment turned out to be
   false and the throttle it was supposed to enable never applied). The routes are
   keyless by design, so they sit outside the usage plan and inherit neither the
   per-key 1 req/s nor the monthly quota; without an entry of their own they draw
   the stage default of 200 req/s with no key at all, at a full Lambda invocation
   each — [[0183]]'s gate is middleware *inside* the handler, so even a closed
   portal pays to answer `404`. A per-route entry is the only way to bound that,
   and a `+` makes one impossible (see the issue above), so the greedy segment
   had to go. Sized at 10 req/s to match `API_DOCS_THROTTLE`, burst 40 because
   one portal page load is several calls. Every route in the epic sits at depth 1
   or 2, so decision 2's property is intact: a later slice still adds a route
   without touching CDK. **The cost is a depth limit** — a depth-3 path matches
   neither level and gets the gateway's `403` instead of the gated `404`. Written
   into the code as a warning, with the instruction to add a third level rather
   than reach back for `{proxy+}`. What this buys is cost control, not
   availability: the cap is global rather than per-caller and bounds rate rather
   than volume. A volume alarm and a WAF rule would close those two gaps and are
   deliberately not built here — [[0194]] costs the traffic before the flag is
   flipped.
9. **CloudFront access logs on from day one**, to a private bucket with a 90-day
   expiry, cookies **excluded**. CloudFront cannot backfill, so a distribution
   that was not logging during an incident has nothing to reconstruct it from —
   and the two things most likely to need reconstructing both arrive here
   (`/api-tokens/api/*` is anonymous; [[0186]] puts OAuth callbacks through the
   same behaviours). Cookies stay out for the same reason they arrive: from
   [[0186]] the portal's cookie *is* the session, and logging it would write a
   usable credential to S3 in plaintext for 90 days to answer a question the
   request line already answers. `BUCKET_OWNER_PREFERRED` is required, not
   stylistic — standard logging delivers via ACL grants and S3's current default
   disables ACLs, which makes delivery fail silently.
10. **`Cache-Control: public, max-age=0, must-revalidate` on the uploaded
    objects.** An
    invalidation clears the edge; it cannot reach a browser. Without a header the
    objects carry none, the S3 behaviours fall to CDK's default 24-hour
    `CACHING_OPTIMIZED` and browsers apply heuristic caching on top — so someone
    who opens today's placeholder keeps being told the portal is unavailable for
    a day after [[0185]] ships the real app, with nothing on screen to explain
    why. Right for one unhashed `index.html`; [[0185]] should split it, a year
    for content-hashed assets and this for the entry document.
11. **Three named paths redirect to their trailing-slash form** — `/`,
    `/api-tokens`, `/api-tokens/api` — as a lookup table in the same function as
    decision 3. `/api-tokens/*` does not match `/api-tokens`, so the bare prefix
    fell to the default behaviour and S3 answered `403 AccessDenied` XML: it
    grants `s3:GetObject` and not `s3:ListBucket`, so a missing key reads as
    forbidden rather than absent. That is the same bare XML decision 4 exists to
    prevent, at the URL a reviewer reaches by trimming one character off the
    documented one. `/api-tokens/api` is in the list for the mirror reason —
    it lands on S3 via the bundle behaviour, and the redirect puts it back on
    the API behaviour.

    **A fixed list rather than a rule**, after review (2026-08-14). The first
    version redirected any path whose last segment had no file extension, which
    was wrong twice: it interpolated `request.uri` into `Location`, and since
    CloudFront does not collapse a leading `//`, `//evil.com/x` would have
    produced a protocol-relative redirect to another origin — an open redirect
    on the host that will serve [[0186]]'s OAuth callback, which is the standard
    first link in a code-interception chain. It also broke [[0185]]: a real SPA
    route like `/api-tokens/keys` would have been rewritten in the address bar
    to a trailing-slash form that [[0195]]'s fallback would then have to undo.
    Interpolating nothing makes the first structurally impossible instead of
    validated, and the second disappears with it. Verified by running the
    emitted function code over `//evil.com/path`, `/\evil.com/path`,
    `/constructor` and `/api-tokens/keys`: all pass through untouched.
12. **Back to one greedy `{proxy+}`, with the verbs enumerated instead of `ANY`**
    (2026-08-14, after the failed deploy; supersedes decisions 2 and 8). The
    throttle needs a per-route method setting, a method setting cannot name
    `ANY`, and — measured, not assumed this time — it *can* name a greedy
    resource. So the greedy segment comes back and `ANY` goes, which is the
    combination that actually satisfies [[0186]]'s acceptance criterion.
    `PORTAL_API_METHODS` is `GET`/`POST`/`DELETE`: `GET` for `/config`,
    [[0186]]'s OAuth redirect and callback and [[0188]]'s `/usage`; `POST` for
    [[0187]]'s issue, [[0191]]'s rework and sign-out; `DELETE` for [[0192]] if it
    prefers that to a `POST`. Decision 8's depth-3 caveat disappears entirely —
    a greedy segment matches any depth — and is replaced by a narrower one: an
    unlisted **verb** gets the gateway's `403` instead of the gated `404`. That
    is a better trade, because a new path is what a slice adds routinely and a
    new verb is what it notices at design time.

## Notes

- **Deployed 2026-08-13.** Distribution `EU8O3ADXFZP5U` at
  `dojr4epgxo2qp.cloudfront.net`; portal at
  `https://dojr4epgxo2qp.cloudfront.net/api-tokens/`. Bucket
  `prices-production-portalhosti-portalbucketf34416c0-ma76zxfgmn0x`. Stack
  creation took ~4.5 minutes including CloudFront propagation, not the 5–10 the
  plan budgeted. **That deploy is not this branch** — decisions 8–11 landed after
  it; see the note under Implementation Notes for what is code-only and in which
  order to redeploy.
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
