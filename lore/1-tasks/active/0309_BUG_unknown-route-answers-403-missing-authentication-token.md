---
id: "0309"
title: "An unknown path or method answers 403 'Missing Authentication Token' — answer 404 in the ErrorEnvelope shape"
type: BUG
status: active
related_adr: []
related_tasks: ["0306", "0183", "0194"]
tags: [layer-infra, api, priority-medium, effort-small]
links:
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../docs/scf/api-endpoints.md"
history:
  - date: "2026-09-23"
    status: backlog
    who: stkrolikiewicz
    note: >
      Raised by Stanisław during 0306: the published docs now explain the
      403, but the answer itself is still the wrong one.
  - date: "2026-09-23"
    status: active
    who: stkrolikiewicz
    note: "Activated; implementation on its own branch."
---

# An unknown path or method answers 403 "Missing Authentication Token"

## Summary

API Gateway answers a request for a path it does not map — or for a method
it does not map on a path it does — with `403 {"message": "Missing
Authentication Token"}`. A caller reads that as a key problem, which is why
[[0306]] had to write the explanation into the Quick Start and the OpenAPI
document. Answer `404` with an `ErrorEnvelope`-shaped body instead, and let
the docs describe that.

## Context

- Measured on production, 2026-09-23, with a valid key: `GET /v1/nope` →
  `403 {"message":"Missing Authentication Token"}`; `PATCH /v1/assets` → the
  same.
- `infra/src/lib/stacks/api-gateway-stack.ts` already overrides gateway
  responses (`THROTTLED`, `DEFAULT_5XX`, for the portal's CORS headers). A
  gateway response has no path dimension: whatever this sets is API-wide.
- The portal's `/api/{proxy+}` enumerates its verbs; an unlisted one gets the
  same 403 instead of the gated `404` (the stack's comment near the
  `PORTAL_API_METHODS` list, `docs/scf/api-endpoints.md` § Routes). This fixes
  that side too.

## Implementation

- `addGatewayResponse` for `ResponseType.MISSING_AUTHENTICATION_TOKEN`:
  `statusCode: '404'`, an `application/json` template of `{"code":
  "not_found", "message": "no such route"}`. Decide whether it also needs the
  portal's CORS headers, as `THROTTLED` got them — the bundle reads a status
  only from routes it calls, and it calls no unmapped one.
- A missing or wrong `x-api-key` is a different response type and must stay
  `403 {"message": "Forbidden"}`; measure it after the deploy rather than
  assume it.
- An unmapped method on a mapped path becomes `404`, not `405`. Accepted: the
  gateway cannot tell the two apart in this response type.
- Update what describes the 403 today: the Quick Start's 403 row, the
  `GatewayMessage` and `ErrorEnvelope` descriptions in
  `packages/prices-api/src/openapi/descriptions.rs`, `docs/scf/api-endpoints.md`,
  the stack's comments, and the line in
  `docs/runbooks/portal-oauth-deploy-prep.md`.
- Deploy the `ApiGateway` stack only; no Lambda or IAM change is expected.

## Acceptance Criteria

- [ ] `GET /v1/nope` and `PATCH /v1/assets` answer `404` with `{"code":
      "not_found", …}` on production
- [ ] A missing or wrong `x-api-key` still answers `403 {"message":
      "Forbidden"}`
- [ ] The Quick Start, the OpenAPI descriptions and `api-endpoints.md`
      describe the `404`, and none still sends a reader to the 403
      explanation
- [ ] The synthesized template is checked for the override, so a stack
      refactor cannot drop it silently

## Progress — 2026-09-23

**Infra half written on `fix/0309_…`, not yet pushed.** `UnknownRoute` in
`api-gateway-stack.ts`: `MISSING_AUTHENTICATION_TOKEN` → `404`,
`{"code":"not_found","message":"no such route"}`. **No CORS headers** — the
decision the plan left open: the bundle calls no unmapped route, and the
portal's one credentialed origin on an API-wide answer is what 0194's review
took off `DEFAULT_4XX`. Check 8 in `verify-openapi-routes.mjs` (already the CI
step that reads the synthesized ApiGateway template) holds the template to it;
the synthesized template mutated four ways — override dropped, `403`, a
non-JSON body, duplicated — fails it each time.

**`cdk diff --exclusively Prices-production-ApiGateway` against production:**
`+ GatewayResponse`, the `Deployment` replaced (`…13811535` → `…4bf28938`) and
`Stage.DeploymentId` repointed. No IAM, nothing else. CDK 2.257's
`GatewayResponse` both hashes itself into the deployment's logical id and makes
the deployment depend on it, so the new stage snapshot is taken after the
response exists — [[0255]]'s "control plane right, stage stale" does not apply
to an addition.

⚠️ **The docs half waits on [[0306]] (PR #343, open).** The texts this task
names — `GatewayMessage` and its description, the rewritten Quick Start — are
0306's: `GatewayMessage` does not exist on `develop` yet, and `QuickStart.tsx`
and `public/openapi.json` are rewritten there.

⚠️ **"Deploy the `ApiGateway` stack only" holds for the infra half alone.**
The OpenAPI descriptions are compiled into the api-handler and served at
`/api-docs-json`, so the docs half ships the way 0306 does: Compute,
`flush-production-cache`, then `sync-portal-explorer`. Shipping the two
together costs one round instead of two.
