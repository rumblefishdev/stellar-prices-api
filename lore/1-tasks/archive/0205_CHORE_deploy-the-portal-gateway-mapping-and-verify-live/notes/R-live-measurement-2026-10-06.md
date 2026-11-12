---
title: "Live re-measurement of the portal routes, 2026-10-06 — every criterion that still applies holds"
type: research
status: mature
tags: [portal, measurement, apigateway, cloudfront, production]
links:
  - "../README.md"
  - "../../../../../docs/scf/api-endpoints.md"
history:
  - date: "2026-10-06"
    status: mature
    who: akot
    note: >
      Run read-only against production on 2026-10-06 at about 12:10 UTC:
      public HTTP requests to both hosts, and AWS reads (API Gateway stage,
      CloudFormation stack list, CloudFront distribution config) as
      `AWS_PROFILE=stellar`. One point in time.
---

# Live re-measurement of the portal routes, 2026-10-06

## What changed since the task was written

The task describes the 2026-08-14 production: the portal on our own
CloudFront (`PortalHosting`, `dojr4epgxo2qp.cloudfront.net`) under
`/api-tokens/`, the gateway mapping the intermediate `{proxy}` +
`{proxy}/{sub}` pair. Neither exists any more:

- `Prices-production-PortalHosting` is `DELETE_COMPLETE` (2026-08-31 10:09
  UTC), deleted in [[0194]]'s cutover that moved the backend to the API's own
  host; [[0195]] removed the stack from the code (`e8ed5c0e`). No CloudFront
  distribution of ours serves the portal.
- The backend is `/api/{proxy+}` directly under the root on
  `prices-api.sorobanscan.rumblefish.dev` — [[0235]] moved the prefix off
  `/api-tokens` (`9d39207e`, 2026-08-28). `/api` is a new parent, so the
  one-variable-child conflict that made this task three deploys never arose:
  an ordinary gateway deploy created the greedy proxy and deleted the
  `/api-tokens` tree.
- The bundle is on the block explorer's distribution `EA2TLS5SS5M87`
  (`sorobanscan.rumblefish.dev`), under `/prices-api/` since [[0326]]
  (2026-10-02), synced by `make -C infra sync-portal-explorer`.

So the three deploys are not needed; what follows measures each criterion
against what replaced its subject.

## Results

| # | Criterion | Measured | Verdict |
|---|---|---|---|
| 1 | `/api-tokens` → `302` | `sorobanscan…/prices-api` → `301 Location: /prices-api/` (`FunctionGeneratedResponse`); `sorobanscan…/api/` → `301 /prices-api/`. `/api-tokens` on the API host → gateway `404 {"code":"not_found"}` | subject gone; replacement holds |
| 2 | entry document `public, max-age=0, must-revalidate` | `/prices-api/`, `/prices-api/index.html`, `/prices-api/dashboard`: `200 text/html`, 2738 B, `public, max-age=0, must-revalidate` | holds |
| 3 | `assets/*` `public, max-age=31536000, immutable` | the 2 assets `index.html` references and the 3 chunks the entry script loads lazily: `200`, `public, max-age=31536000, immutable` | holds |
| 4 | depth 3 → empty `404` | `/api/a`, `/api/a/b`, `/api/a/b/c`: `404`, 0 bytes (the handler's gate, not the gateway); `/api/auth/me` (depth 3, real route) `200 {"authenticated":false}` | holds |
| 5 | per-verb throttle 10/40, caching off | stage `production`, `api/{proxy+}/{GET,POST,DELETE,OPTIONS}`: rate 10, burst 40, `cachingEnabled: false`. `*/*` default 200/400 | holds |
| 6 | CloudFront access logs, no cookies | no distribution of ours. The explorer's `EA2TLS5SS5M87`: `Logging.Enabled: true`, `IncludeCookies: false`, bucket `production-soroban-explorer-cf-logs`, newest object `EA2TLS5SS5M87.2026-10-06-12.…gz` at 12:18 UTC | subject gone; explorer's holds |
| 7 | `/api/config` `200`, `no-store`; other paths empty `404` | `200 {"enabled":true,"rate_limit_per_second":1}`, `no-store`; with `Origin: https://sorobanscan.rumblefish.dev` adds `Access-Control-Allow-Origin` (that origin) + `Allow-Credentials: true`. `enabled: true` is [[0194]]'s opening, not a regression | holds for today's flag |
| 8 | `/health`, `/api-docs-json`, `/v1/assets` keyless | `200 {"status":"ok","stack":"prices-production"}`; `200` 58 863 B `public, max-age=300` (same at `/api/api-docs-json`); `403 {"message":"Forbidden"}` | holds |
| 9 | "ahead of the deploy" notes deleted | `docs/scf/api-endpoints.md`: already gone. Still present: `api-gateway-stack.ts` docblock of `PORTAL_API_METHODS`, `docs/runbooks/portal-oauth-deploy-prep.md` §preconditions, [[0184]]'s Implementation Notes | open at measurement |
| 10 | entry-document deployment `DependsOn` the asset one | no template. `sync-portal-explorer` runs the `assets/*` sync before the rest, sequentially in one recipe, then one invalidation — the race cannot occur | subject gone; ordering holds |
| 11 | cold cache: every asset `200`, none `403` | all 5 bundle files and the 3 icons `200`, most `Miss from cloudfront`. No invalidation was issued (a write), so "cold" is per-object misses, not a flushed distribution | holds |

Also seen: `OPTIONS /api/config` with `Origin` +
`Access-Control-Request-Method: DELETE` → `204`, `allow-methods: GET,POST,DELETE`,
`allow-headers: Content-Type,Accept,X-Requested-With`, `max-age: 3600`,
credentials true — a MOCK answer, no invocation. A verb outside the list
(`PATCH /api/config`) gets the gateway's `404 {"code":"not_found"}` ([[0309]]).

[[0185]]'s decision 13 (`/api-tokens/` in `distributionPaths`) has no subject:
the invalidation is now `'/prices-api/*'` on the explorer's distribution.

## How it was measured

`curl -s -D -` (GET; OPTIONS and PATCH where stated) against both hosts,
keeping the status line, `Location`, `Cache-Control`, `Content-Type`,
`Content-Length`, `Access-Control-*`, `X-Cache`, `x-amzn-ErrorType` and the
first bytes of the body. Asset list from the `src`/`href` attributes of
`/prices-api/`, lazy chunks from `assets/…` names inside the entry script.
AWS: `apigateway get-stages` on `02mabge71l`, `apigateway get-resources`,
`cloudformation list-stacks`, `cloudfront get-distribution-config
EA2TLS5SS5M87`, `s3api list-objects-v2` on the log bucket. The scripts stayed outside the repository.
