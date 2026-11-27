---
id: "0357"
title: "A dot segment through a keyless resource skips the API key"
type: BUG
status: active
assignee: akot
related_adr: []
related_tasks: ["0356"]
tags: [layer-backend, priority-high, effort-small, api, security]
links: []
history:
  - date: "2026-11-27"
    status: active
    who: akot
    note: >
      Created and started. Found in the review of #437 (task 0356) and confirmed on production:
      a keyed route answers with no API key through the portal's keyless `/api/{proxy+}`.
---

# A dot segment through a keyless resource skips the API key

## Summary

A keyed route requested with a dot segment after a keyless resource's prefix (`..` or its
percent-encoded forms) answers `200` with no API key, where the plain path answers `403`.
Any keyed route can be read this way, past the usage plans and quotas.

## Context

- API Gateway picks the resource, and with it whether a key is asked for, on the path as sent:
  `/api/../v1/...` matches the portal's keyless `/api/{proxy+}`.
- `lambda_http` builds the request's URI with `Url::parse`, which resolves dot segments
  (`..`, `%2e%2e`), so the router sees `/v1/assets/native/price` and serves the keyed route.
- Production sets no `API_KEYS`: the Lambda does not check keys itself, the gateway is the
  only check.
- #437 (task 0356) adds a second keyless greedy resource, `/x402/{proxy+}`, with the same hole.

Confirmed on production on 2026-11-27 with three spellings of the dot segment; the same
route without one answered `403`.

## Implementation

- `packages/prices-api/src/common/gateway.rs`: `left_its_resource(resource, path)`, true when
  the routed path does not start with the literal part of the gateway resource (up to its
  first `{`).
- `packages/prices-api/src/main.rs` (the Lambda only): a layer over the whole app reads the
  API Gateway request context's `resourcePath` and answers `404` (with a `warn` log line)
  when the routed path has left it. No list of keyless prefixes to keep in step with the
  gateway: every resource checks itself.

## Acceptance Criteria

- [ ] A keyed route reached with a dot segment through a keyless resource answers `404` on
      production; the same route still answers `403` without a key and `200` with one
- [ ] The portal under `/api/` and the keyed routes answer as before
- [ ] `left_its_resource` has unit tests for both sides

## Design Decisions

1. **Compare the resource, not the raw path.** The raw path would also catch dot segments,
   but API Gateway and `Url::parse` encode some characters differently (a buyer's
   `USDC%3AG…`), so a raw-versus-routed comparison could refuse legitimate requests. A dot
   segment is the only thing the gateway lets through that moves a path out of its resource.
2. **`404`, not `400`**: the path is no route under the resource that received it.
3. **In the Lambda's `main.rs`, not in `app`**: the context exists only there; `serve` and the
   tests have no gateway.

## Notes

- After the deploy: the three requests that showed it, plus one keyed request with a key and
  one portal request, as the evidence for the criteria.
