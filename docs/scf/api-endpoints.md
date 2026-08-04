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

| Route                                | Auth          | Gateway cache TTL |
| ------------------------------------ | ------------- | ----------------- |
| `GET /health`                        | Anonymous     | uncached          |
| `GET /api-docs-json`                 | **Anonymous** | 3600 s            |
| `GET /v1/assets`                     | `x-api-key`   | 60 s              |
| `GET /v1/assets/{asset_identifier}`  | `x-api-key`   | 60 s              |
| `GET /v1/assets/{id}/price`          | `x-api-key`   | 10 s              |
| `GET /v1/assets/{id}/ohlcv`          | `x-api-key`   | 60 s              |
| `GET /v1/oracles/{asset_identifier}` | `x-api-key`   | 60 s              |
| `GET /v1/backfill/status`            | `x-api-key`   | 60 s              |
| `POST /v1/prices/batch`              | `x-api-key`   | uncached          |

Source of truth: `infra/src/lib/stacks/api-gateway-stack.ts`. The table is
asserted against the published document by
`packages/prices-api/tests/openapi.rs::spec_route_coverage_matches_the_deployed_gateway_both_ways`,
which fails if a route exists on one side and not the other.

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
- **Cache:** 3600 s at the gateway stage cache, matching the handler's
  `Cache-Control: public, max-age=3600`. The document is byte-identical for the
  life of a deployment.
- **Lint:** `npm run openapi:lint` extracts the served document and runs
  Redocly's recommended ruleset over it; wired into the `rust` CI job. Config in
  `redocly.yaml`, documented exceptions in `.redocly.lint-ignore.yaml`.

The `extract_openapi` binary remains the way to get the document without a
deployment (client generation, spec diffs in CI):

```bash
npm run openapi:extract   # → target/openapi.json, servers stamped from config
```

## Pending

- **Custom domain** (task 0126) — when it lands, `apiBaseUrl` in
  `infra/envs/production.json` changes and `servers` follows automatically. This
  file's base URL must be updated in the same change.
- **Swagger UI + onboarding portal** — Tranche 3, out of scope for M2.
- **`info.license`** (task 0144) — currently emitted empty; the licensing
  decision is open.
