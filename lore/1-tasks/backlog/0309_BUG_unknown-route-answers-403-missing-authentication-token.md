---
id: "0309"
title: "An unknown path or method answers 403 'Missing Authentication Token' — answer 404 in the ErrorEnvelope shape"
type: BUG
status: backlog
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
