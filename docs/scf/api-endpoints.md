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
| `ANY /api-tokens/api/{proxy+}`            | Anonymous²    | uncached          |

¹ Gateway TTL. The handler sends `max-age=300` — see **Cache** below for why the
two differ.

² The onboarding portal's backend (task 0184). Keyless at the gateway because a
visitor signing in to obtain a key does not have one — the same argument that
makes `/api-docs-json` anonymous. It is **not** open: while `PORTAL_ENABLED` is
`false`, every path under it returns an empty `404`, byte-identical to a path
that was never deployed (task 0183). `GET /api-tokens/api/config` is the one
exception and answers `{"enabled": false}` in both states. Mapped as one greedy
`ANY` so later slices add routes without a CDK change, and deliberately absent
from the OpenAPI document — the portal describes itself to its own bundle, not
to integrators.

Source of truth: `infra/src/lib/stacks/api-gateway-stack.ts`. This table is
documentation and is not itself asserted — what CI enforces is that the
**gateway and the published document** describe the same routes, via
`tools/scripts/verify-openapi-routes.mjs` (derived from the synthesized template
and the extracted spec) plus the faster in-process check in
`packages/prices-api/tests/openapi.rs`. If this table drifts from either, fix
the table.

## Portal distribution — CloudFront (task 0184)

**Portal:** `https://dojr4epgxo2qp.cloudfront.net/api-tokens/`

Distribution `EU8O3ADXFZP5U`, deployed 2026-08-13. The domain is also published
to SSM at `/prices/production/portal-distribution-domain`, which is where task
0186 (Discord redirect URI) and task 0195 (custom domain) should read it from
rather than copying it.

A second, equivalent way to reach the API. One CloudFront distribution fronts
two origins: a private S3 bucket holding the portal bundle, and the API Gateway
stage above. That is not a convenience — it is what makes every portal request
**same-origin**, which is what lets the session cookie (task 0186) be
`SameSite=Lax` and keeps CORS out of portal traffic entirely.

Behaviours, in precedence order. CloudFront takes the **first** match, so the
order is part of the configuration, not a presentation choice:

| Path pattern        | Origin      | Notes                                   |
| ------------------- | ----------- | --------------------------------------- |
| `/api-tokens/api/*` | API Gateway | must precede the row below              |
| `/api-tokens/*`     | S3          | the portal bundle                       |
| `/v1/*`             | API Gateway | data routes                             |
| `/api-docs-json`    | API Gateway | root-level, **not** under `/v1`         |
| `/health`           | API Gateway | root-level                              |
| _(default)_         | S3          | `/` redirects to `/api-tokens/` for now |

The convention, settled 2026-08-07 and meant for the frontends that follow:
**`<app>/*` is that app's bundle, `<app>/api/*` is that app's backend.** A new
frontend adds two rows and invents nothing.

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
