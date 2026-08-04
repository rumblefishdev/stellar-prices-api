---
id: "0124"
title: "Expose the OpenAPI spec through API Gateway — /api-docs-json is unroutable in production"
type: FEATURE
status: active
related_adr: ["0008"]
related_tasks: ["0119", "0120", "0128"]
tags: [layer-infra, layer-backend, priority-medium, effort-small, milestone-M2, api-gateway, openapi, documentation]
milestone: 2
links:
  - "../../../packages/prices-api/src/lib.rs"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../docs/scf/milestone-1-evidence.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Closes the "Swagger"
      row of `milestone-1-evidence.md` Table 4, which states the axum router
      defines `/api-docs-json` but the API Gateway does not map it. Scoped to
      the **spec**; the Swagger **UI** and the onboarding portal stay
      Tranche 3 per overview §9.
  - date: 2026-08-04
    status: active
    who: akot
    note: >
      Promoted to active, picked up by Adam. Scope unchanged: the spec
      document over the deployed API, not the Swagger UI. First open
      question is the auth posture — the task recommends anonymous to
      match `/health` (`api-gateway-stack.ts:238`); the decision needs
      recording either way before implementation.
---

# Expose the OpenAPI spec through API Gateway

## Summary

`packages/prices-api/src/lib.rs` builds the OpenAPI spec at startup and serves
it at `GET /api-docs-json` — but API Gateway never maps that path, so in
production the spec is reachable only by running
`cargo run -p prices-api --bin extract_openapi` locally. The M1 submission said
so explicitly and deferred the fix to Tranche 2.

Scope here is the **specification document**, served over the deployed API. The
Swagger **UI** and the self-service onboarding portal (S3 + CloudFront) are
Tranche 3 per §9 and are out of scope.

## Context

The spec is generated from the axum routes via `utoipa`, so it cannot drift from
the implementation — which is exactly why it is worth exposing rather than
hand-maintaining a parallel document. It is also the input [[0120]] validates
responses against, and the natural home for the enumerations and ranges
[[0119]] adds.

Design decision needed: **should the spec require an API key?** Recommendation
is **no** — an API description is public documentation, and gating it behind the
key a developer does not yet have is a self-service dead end. `/health` is
already anonymous (`api-gateway-stack.ts:238`), so the precedent for an
unauthenticated route exists. Record the choice either way.

## Implementation

- Map the route through API Gateway to the existing axum handler. Prefer the
  existing proxy integration over a second mechanism so there is one source of
  truth.
- Decide and apply the auth posture (recommended: anonymous, matching
  `/health`).
- Cache it — the spec is static per deployment, so a long TTL is free and keeps
  it off the Lambda. Note that `apiKeyRequired: false` + caching means it must
  contain nothing key-specific (it does not).
- Stamp `servers` correctly. `lib.rs` already sets `spec.servers` from
  `config.base_url`; confirm the deployed value is the real invoke URL,
  including the stage path. **Watch the stage-prefix trap** — the same class of
  bug that required `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true` for the `/v1`
  routes will make the advertised `servers` URL wrong if `base_url` omits the
  stage.
- Sanity-check the emitted document: valid OpenAPI 3.0, every deployed route
  present, no route present that is not deployed, and the `x-api-key` security
  scheme declared so a reader knows what auth the *data* routes need.
- Once [[0126]] lands a custom domain, `servers` must follow it — note the
  ordering.

## Acceptance Criteria

- [ ] `GET <base>/api-docs-json` (or the agreed public path) returns the spec
      from the deployed production API
- [ ] Auth posture decided, applied, and recorded
- [ ] Document is valid OpenAPI 3.0 and passes a linter cleanly
- [ ] `servers` resolves to a URL that actually serves the API, stage path
      included — verified by fetching a route from it
- [ ] Route coverage matches the deployed router exactly, both directions
- [ ] Security scheme (`x-api-key`) declared for the key-gated routes
- [ ] Response cached with a TTL appropriate to a per-deployment-static document
- [ ] Path recorded in `docs/scf/` so [[0128]] can cite it

## Notes

- The `extract_openapi` binary stays useful for CI (spec-diff checks, client
  generation); this task adds a runtime surface, it does not replace the binary.
- Tranche 3 AC 2 requires *"OpenAPI spec passes `openapi-validator` lint with no
  errors; Swagger UI deployed"*. Doing the lint half now is cheap and de-risks
  M3 — hence its inclusion above.
