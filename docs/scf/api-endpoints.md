# Live API surface — endpoints and access model

Running record of every route the production API Gateway maps, its auth
posture, and its cache TTL. Kept current as M2 tasks land so the Milestone 2
package (task 0128) can cite it instead of re-deriving the surface from the CDK
source at submission time.

**Production base:**
`https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production`

The base URL is configured once, in `infra/envs/production.json`
(`apiBaseUrl`), and flows from there to two places: the api-handler Lambda's
`API_BASE_URL` environment variable, and the `servers` block of the published
OpenAPI document. It includes the `/production` stage path — an execute-api URL
without it serves nothing.

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
| `GET /api/api/{proxy+}`                   | Anonymous²    | uncached          |
| `POST /api/api/{proxy+}`                  | Anonymous²    | uncached          |
| `DELETE /api/api/{proxy+}`                | Anonymous²    | uncached          |

¹ Gateway TTL. The handler sends `max-age=300` — see **Cache** below for why the
two differ.

² The onboarding portal's backend (task 0184). Keyless at the gateway because a
visitor signing in to obtain a key does not have one — the same argument that
makes `/api-docs-json` anonymous. It is **not** open: while `PORTAL_ENABLED` is
`false`, every path under it returns an empty `404`, byte-identical to a path
that was never deployed (task 0183). `GET /api/api/config` is the one
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

## Portal distribution — CloudFront (task 0184)

**Portal:** `https://dojr4epgxo2qp.cloudfront.net/api/`

Distribution `EU8O3ADXFZP5U`, deployed 2026-08-13. The domain is also published
to SSM at `/prices/production/portal-distribution-domain`, which is where task
0186 (Discord redirect URI) and task 0195 (custom domain) should read it from
rather than copying it.

> **Ahead of the deploy.** This section describes what task 0184's branch
> synthesizes. Three of its properties are not live yet — the trailing-slash
> redirect, CloudFront access logs, and `Cache-Control` on the uploaded objects
> — so `/api` answers `403 AccessDenied` today rather than redirecting.
> The gateway is a fourth case: it currently maps the portal as
> `ANY /api/api/{proxy}` and `.../{proxy}/{sub}` with **no throttle**, an
> intermediate state left by the 2026-08-14 deploy attempt (see task 0184).
> Moving it to the `{proxy+}` above means replacing a path-parameter resource,
> which API Gateway will not do in one update — deploy once with the portal
> resources removed, then again with them. **Task 0205 owns those deploys and
> deleting this note.**

A second, equivalent way to reach the API. One CloudFront distribution fronts
two origins: a private S3 bucket holding the portal bundle, and the API Gateway
stage above. That is not a convenience — it is what makes every portal request
**same-origin**, which is what lets the session cookie (task 0186) be
`SameSite=Lax` and keeps CORS out of portal traffic entirely.

Behaviours, in precedence order. CloudFront takes the **first** match, so the
order is part of the configuration, not a presentation choice:

| Path pattern     | Origin      | Notes                            |
| ---------------- | ----------- | -------------------------------- |
| `/api/api/*`     | API Gateway | must precede the row below       |
| `/api/*`         | S3          | the portal bundle                |
| `/v1/*`          | API Gateway | data routes                      |
| `/api-docs-json` | API Gateway | root-level, **not** under `/v1`  |
| `/health`        | API Gateway | root-level                       |
| _(default)_      | S3          | `/` redirects to `/api/` for now |

The convention, settled 2026-08-07 and meant for the frontends that follow:
**`<app>/*` is that app's bundle, `<app>/api/*` is that app's backend.** A new
frontend adds two rows and invents nothing.

That ordering is no longer a hand-checked property: `openapi:verify-routes`
takes the first pattern that matches a portal backend path out of the
synthesized distribution and fails CI unless it is `/api/api/*` pointing
at the execute-api origin. The same check asserts the prefix is identical in the
handler (`PORTAL_API_PREFIX`), in the CDK routing table (`PORTAL_BACKEND`) and
in the script itself, so moving it in one place cannot leave the other two
behind.

**Three paths redirect, and only three.** A pattern like `/api/*` does
not match `/api`, so the bare prefix would fall to the default behaviour
and S3 would answer `403 AccessDenied` XML — it grants `s3:GetObject` and not
`s3:ListBucket`, so a missing key reads as forbidden rather than absent. A
viewer-request function redirects the bare prefixes to their trailing-slash
form, which is what a reviewer trimming the documented URL should get:

| Request    | Result                                        |
| ---------- | --------------------------------------------- |
| `/`        | `302` → `/api/` → the portal page             |
| `/api`     | `302` → `/api/` → the portal page             |
| `/api/api` | `302` → `/api/api/` → the gateway, **not** S3 |

The targets are a fixed list rather than a rule like "any path with no file
extension". That generalisation was written first and rejected twice over: it
interpolates `request.uri` into `Location`, and CloudFront does not collapse a
leading `//`, so `//evil.com/x` would have redirected to a **different origin**
— an open redirect on the origin that will host task 0186's OAuth callback. It
would also fight task 0185's router, rewriting `/api/keys` to a
trailing-slash form in the address bar that task 0195's SPA fallback would then
have to undo. A new frontend adds its prefix to the list.

`/api/api/` reaches API Gateway but matches no method on the resource, so
it answers `403 {"message":"Missing Authentication Token"}` — the gateway's
standard response for an unmapped path, the same as `/v1`. It is deliberately
**not** the empty `404` described below: that applies to paths _under_ the
prefix, which `{proxy+}` maps and the handler then gates.

**Access logs are on**, to a private bucket with a 90-day expiry. Cookies are
excluded: from task 0186 the portal's cookie is the session itself, and logging
it would write a usable credential to S3 in plaintext. CloudFront cannot
backfill logs, so this has to be on before the incident, not after.

Three properties worth knowing before changing anything here:

- **The bucket is private.** No public objects, no ACLs; the distribution reads
  it through Origin Access Control and nothing else can.
- **API behaviours are uncached at the edge and forward everything except
  `Host`.** Forwarding the viewer's `Host` to an execute-api origin 403s every
  request; the obvious `CACHING_OPTIMIZED` policy would strip `x-api-key` and
  403 every keyed route. The gateway's own stage cache (table above) is
  unaffected.
- **The execute-api base keeps working.** Only one of the two is documented and
  supported as _the_ base URL, and until the custom domain lands (task 0195)
  that is still the execute-api URL at the top of this file — `apiBaseUrl` and
  the OpenAPI `servers` block are unchanged by this task, so a reader is not
  handed a base that changes twice in a month.

Deploy and cache invalidation are one operation
(`make -C infra deploy-production-portal`); there is no separate upload step to
forget.

## OpenAPI specification (task 0124)

```
GET https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/api-docs-json
```

Anonymous — no API key needed. Verify with:

```bash
curl -sS https://02mabge71l.execute-api.eu-central-1.amazonaws.com/production/api-docs-json \
  | jq '{openapi, servers, paths: (.paths | keys)}'
```

- **Format:** OpenAPI 3.1.0, generated from the axum routes by `utoipa`, so it
  cannot drift from the implementation.
- **Auth posture:** anonymous, deliberately. An API description is public
  documentation; gating it behind a key the reader does not have yet is a
  self-service dead end. `/health` set the anonymous-route precedent. The
  document contains nothing key-specific, which is what makes it safe to cache
  for all callers.
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

- **Custom domain** (task 0126) — when it lands, `apiBaseUrl` in
  `infra/envs/production.json` changes and `servers` follows automatically. This
  file's base URL must be updated in the same change.
- **Swagger UI** (task 0195) — served from the portal distribution at `/docs/*`,
  rendering the live `/api-docs-json` rather than a checked-in copy. Lands with
  the custom domain and the per-prefix SPA fallback.
- **Onboarding portal** — the distribution and its routing table are live (task
  0184); the portal itself ships closed behind `PORTAL_ENABLED` and is opened by
  task 0194's audit.
- **`info.license`** (task 0155) — currently emitted empty; the licensing
  decision is open.
