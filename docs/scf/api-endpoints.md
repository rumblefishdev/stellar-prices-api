# Live API surface — endpoints and access model

Running record of every route the production API Gateway maps, its auth
posture, and its cache TTL. Kept current as M2 tasks land so the Milestone 2
package (task 0128) can cite it instead of re-deriving the surface from the CDK
source at submission time.

**Production base:** `https://prices-api.sorobanscan.rumblefish.dev`
(since 2026-08-31, task 0194 — the API's own hostname, a REGIONAL custom
domain on the REST API mapped at the root, so there is no stage path). The
execute-api origin
`https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production` still
answers and is no longer the documented base.

The base URL is configured once, in `infra/envs/production.json`
(`apiBaseUrl`), and flows from there to three places: the api-handler Lambda's
`API_BASE_URL` environment variable, the `servers` block of the published
OpenAPI document, and — asserted equal by `web/portal/src/landing/links.spec.ts`
— the portal's snippets (`PUBLIC_API_BASE_URL`).

## Routes

| Route                                     | Auth          | Gateway cache TTL |
| ----------------------------------------- | ------------- | ----------------- |
| `GET /health`                             | Anonymous     | uncached          |
| `GET /api-docs-json`                      | **Anonymous** | 3600 s¹           |
| `GET /v1/assets`                          | `x-api-key`   | 60 s              |
| `GET /v1/assets/{asset_identifier}`       | `x-api-key`   | 60 s              |
| `GET /v1/assets/{asset_identifier}/price` | `x-api-key`   | 10 s              |
| `GET /v1/assets/{asset_identifier}/ohlcv` | `x-api-key`   | 60 s              |
| `GET /v1/oracles/{asset_identifier}`      | `x-api-key`   | 60 s              |
| `GET /v1/backfill/status`                 | `x-api-key`   | 60 s              |
| `POST /v1/prices/batch`                   | `x-api-key`   | uncached          |
| `GET /api/{proxy+}`                       | Anonymous²    | uncached          |
| `POST /api/{proxy+}`                      | Anonymous²    | uncached          |
| `DELETE /api/{proxy+}`                    | Anonymous²    | uncached          |

¹ Gateway TTL. The handler sends `max-age=300` — see **Cache** below for why the
two differ.

² The onboarding portal's backend (task 0184). Keyless at the gateway because a
visitor signing in to obtain a key does not have one — the same argument that
makes `/api-docs-json` anonymous. It is **not** open: while `PORTAL_ENABLED` is
`false`, every path under it returns an empty `404`, byte-identical to a path
that was never deployed (task 0183). `GET /api/config` is the one
exception and answers `{"enabled": false}` in both states. Deliberately absent
from the OpenAPI document — the portal describes itself to its own bundle, not
to integrators.

One greedy `{proxy+}` resource, so later slices add routes at any depth without
a CDK change, with the **verbs enumerated** rather than collapsed into `ANY`.
The verbs are the part that looks arbitrary and is not: API Gateway names a
stage method setting `/{resourcePath}/{httpMethod}/{setting}` and rejects the
whole stage update if it cannot resolve one — and `ANY` is never a resolvable
`{httpMethod}` there, whatever the path looks like. A greedy segment is fine, a
path parameter is fine, depth is fine; `ANY` is not. So a route mapped as `ANY`
can carry neither a cache setting nor a throttle, and these routes need both.

The cost is that **a verb not in the list** — `PATCH`, say — gets the gateway's
`403 Missing Authentication Token` instead of the gated `404`. Paths stay free;
verbs do not. Adding one is a line in `PORTAL_API_METHODS` and a deploy.

They also carry their own method-level throttle — **10 req/s, burst 40** — which
is not decoration. Being keyless puts them outside the usage plan, so they
inherit neither the per-key rate (1 req/s) nor the monthly quota, and without an
entry of their own they would fall to the stage default of 200 req/s drawable
with no key at all. Uncached at both layers by requirement, so every request is
a billed gateway request **and** a billed Lambda invocation — task 0183's gate is
middleware inside the handler, so even a closed portal pays full price to answer
`404`. This bounds rate, not volume, and it is a global cap rather than
per-caller; task 0194 costs the traffic before the flag is flipped.

Source of truth: `infra/src/lib/stacks/api-gateway-stack.ts`. This table is
documentation and is not itself asserted — what CI enforces is that the
**gateway and the published document** describe the same routes, via
`tools/scripts/verify-openapi-routes.mjs` (derived from the synthesized template
and the extracted spec) plus the faster in-process check in
`packages/prices-api/tests/openapi.rs`. If this table drifts from either, fix
the table.

## Portal hosting — two hosts, no distribution of ours (tasks 0194, 0195)

**Portal:** `https://sorobanscan.rumblefish.dev/api/`
**API reference:** `https://sorobanscan.rumblefish.dev/api/docs`

The portal is a page on one host that calls an API on another:

| What               | Where                                                                                                                                                         | Owned by                                                                                                     |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------ |
| the page (bundle)  | `s3://production-soroban-explorer-api-spa/api/`, behaviour `/api/*` of the block explorer's distribution `EA2TLS5SS5M87` (alias `sorobanscan.rumblefish.dev`) | `soroban-block-explorer` (their stack); this repo syncs the bundle with `make -C infra sync-portal-explorer` |
| the backend + spec | `https://prices-api.sorobanscan.rumblefish.dev` — the custom domain above, no CloudFront in front of it                                                       | `ApiGatewayStack` here                                                                                       |

Decided 2026-08-31 (task 0194, "decision A"). The bundle is built with
`VITE_PORTAL_API_ORIGIN` set to the API hostname, so every backend call is
**cross-origin and same-site**: the session cookie is `SameSite=Lax` and rides
along because the two hosts share a registrable domain, and the portal's
routes answer CORS for exactly one origin, with credentials
(`portal::cors_layer` in the handler and `addCorsPreflight` on `/api/{proxy+}`
at the gateway — both name `portalWebOrigin` from `infra/envs/production.json`).

**Our own distribution is gone.** Task 0184's `PortalHostingStack` — a private
bucket and a CloudFront distribution (`dojr4epgxo2qp.cloudfront.net`) fronting
both the bundle and the execute-api origin — was the portal's first home and,
after the move above, a second, ungated copy of the same portal on a hostname
nobody audited. Task 0195 destroyed it on 2026-09-01 and removed the stack, its
`deploy-production-portal` target and the
`/prices/production/portal-distribution-domain` SSM parameter. The
execute-api origin it fronted still answers on its own URL; see task 0126 for
whether that is announced as retired or kept as an alias.

### The path convention

Everything of ours on the shared host lives under **one prefix, `/api/`**, with
no sub-prefix for the bundle or the backend: `/api/login` is a page,
`/api/docs` is a page, `/api/auth/login` is the backend, `/api/api-docs-json`
is the OpenAPI document. On that host the root belongs to the block explorer,
so nothing of ours may assume it.

This replaced task 0161's `<app>/*` + `<app>/api/*` convention on 2026-08-31,
which for an app called "api" produced `/api/api/…` (task 0235's record of the
three days that layout lived). The rule that decides which side of the split
is enumerated:

- **the bundle's paths are a short, fixed list** — `/api/`, `/api/index.html`,
  `/api/favicon.ico`, `/api/assets/*` and the app's routes — and
- **the backend is the open-ended side** (five slices added routes; none
  touched infrastructure), so it gets the catch-all.

The failure modes are asymmetric, which is why it is this way round: a bundle
path that reaches the API gets a loud JSON `404`; a backend call that reaches
a static host gets a `200` full of HTML that only surfaces as a JSON parse
error in a browser (`web/portal/src/api/portal.ts` names the URL when it
happens).

On the explorer's host the split needs no routing table at all: their `/api/*`
behaviour is S3, and their viewer-request function rewrites **every path whose
last segment has no `.`** to `/api/index.html`. That is what makes a hard
refresh on `/api/dashboard` — or on `/api/docs` — boot the portal, and it is
also why the API reference is a **route of the portal** rather than a static
`docs/` folder in the bundle: a folder would have been reachable only as
`/api/docs/index.html`. Adding a page to the app therefore needs nothing on
the hosting side; adding a backend route needs nothing either, because the
gateway maps `/api/{proxy+}` and the bundle calls the API host directly.

What the shared host does NOT give us, and what still gates the portal's
public availability: the same function answers `401` to anyone without the
explorer's staging credentials while `enableApiSpaBasicAuth` is on in their
`production.json`. Turning it off is the explorer team's call, not this
repo's.

## OpenAPI specification (task 0124)

```
GET https://prices-api.sorobanscan.rumblefish.dev/api-docs-json
```

Anonymous — no API key needed. Verify with:

```bash
curl -sS https://prices-api.sorobanscan.rumblefish.dev/api-docs-json \
  | jq '{openapi, servers, paths: (.paths | keys)}'
```

The same bytes are served at `/api/api-docs-json` (the portal's alias, task
0194), and rendered as the portal's API reference at
`https://sorobanscan.rumblefish.dev/api/docs` (task 0195) — Swagger UI's
shape (tags, collapsible operations, parameters, responses, schemas) in the
portal's own design system, reading the live document on every visit.

- **Format:** OpenAPI 3.1.0, generated from the axum routes by `utoipa`, so it
  cannot drift from the implementation.
- **Auth posture:** anonymous, deliberately. An API description is public
  documentation; gating it behind a key the reader does not have yet is a
  self-service dead end. `/health` set the anonymous-route precedent. The
  document contains nothing key-specific, which is what makes it safe to cache
  for all callers.
- **CORS:** `Access-Control-Allow-Origin: *`, on both copies (task 0195). The
  portal's API reference is a page on `sorobanscan.rumblefish.dev` that
  `fetch`es the document from the API host, and so may a partner's
  browser-side tooling.
  `*` rather than the portal's origin because the document is public and
  carries no credential, and because the value must be a **constant**: the
  gateway's stage cache would serve a reflected origin to the next caller.
  This is the only `*` on the API — the portal's own routes answer one
  origin with credentials, and the `/v1` data routes answer no CORS at all
  until task 0126.
- **`x-api-key` scheme:** declared in `components.securitySchemes` and required
  document-wide, with `/health` and `/api-docs-json` opting out explicitly. A
  reader can therefore see exactly which routes need a key without being told.
- **Cache:** 3600 s at the gateway stage cache; the handler sends
  `Cache-Control: public, max-age=300`. The document is byte-identical for the
  life of a deployment, but the caches are not — they outlive the build that
  filled them. The gateway entry is flushed on deploy
  (`make -C infra flush-production-cache`, wired into `deploy-production`); a
  partner's HTTP cache cannot be, so the client-facing window is short enough
  that a reader picks up a new release within 5 minutes rather than an hour.
  Revalidation costs nothing — those requests land on the gateway cache.
- **Lint:** `npm run openapi:lint` extracts the served document and runs
  Redocly's `recommended-strict` ruleset over it; wired into the `rust` CI job.
  Strict, not `recommended`, because plain `redocly lint` exits 0 on warnings —
  a gate that only fails on errors would not have blocked the very regressions
  it was added for. Config in `redocly.yaml`, documented exceptions in
  `.redocly.lint-ignore.yaml`.

The `extract_openapi` binary remains the way to get the document without a
deployment (client generation, spec diffs in CI):

```bash
npm run openapi:extract   # → target/openapi.json, servers stamped from config
```

## Pending

- ~~**Custom domain** (task 0126)~~ — landed 2026-08-31 (task 0194):
  `prices-api.sorobanscan.rumblefish.dev`, `apiBaseUrl` and `servers` updated
  in the same change as this file.
- ~~**Swagger UI** (task 0195)~~ — landed 2026-09-01 as the portal's API
  reference, `https://sorobanscan.rumblefish.dev/api/docs`, rendering the
  live `/api-docs-json` in the portal's own design system. Nothing on it
  sends a request until the data routes answer CORS (task 0126).
- **Onboarding portal** — open (`PORTAL_ENABLED=true` since task 0194) at
  `https://sorobanscan.rumblefish.dev/api/`, behind the block explorer's
  basic auth until their `enableApiSpaBasicAuth` is turned off; that switch
  and the move from the test guild to the Stellar guild (task 0179) both
  precede advertising the URL.
- **CORS on `/v1`** (task 0126) — no browser can call the data routes yet;
  the portal's API reference sends no requests for exactly this reason.
- **`info.license`** (task 0155) — currently emitted empty; the licensing
  decision is open.
