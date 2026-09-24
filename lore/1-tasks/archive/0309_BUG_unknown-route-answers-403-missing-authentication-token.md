---
id: "0309"
title: "An unknown path or method answers 403 'Missing Authentication Token' — answer 404 in the ErrorEnvelope shape"
type: BUG
status: completed
related_adr: []
related_tasks: ["0306", "0183", "0194", "0255", "0205"]
tags: [layer-infra, api, priority-medium, effort-small]
links:
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../docs/scf/api-endpoints.md"
  - "../../../tools/scripts/verify-openapi-routes.mjs"
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
  - date: "2026-09-24"
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped and checked live. PR #348 merged 11:51 CEST; ApiGateway 12:19,
      Compute 12:21 (cache flushed), portal 12:23. 4/4 criteria: six
      unknown-route probes answer 404 `not_found`, keyless and wrong-key
      stay 403 Forbidden, the published texts describe the 404, and check 8
      of verify-openapi-routes holds the template (four mutations fail it).
      One gateway response, no IAM; two OpenAPI descriptions and two Quick
      Start rows changed.
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

- [x] `GET /v1/nope` and `PATCH /v1/assets` answer `404` with `{"code":
      "not_found", …}` on production — measured 2026-09-24 12:20
- [x] A missing or wrong `x-api-key` still answers `403 {"message":
      "Forbidden"}` — same run
- [x] The Quick Start, the OpenAPI descriptions and `api-endpoints.md`
      describe the `404`, and none still sends a reader to the 403
      explanation — `/api-docs-json` 12:22, portal bundle 12:23
- [x] The synthesized template is checked for the override, so a stack
      refactor cannot drop it silently — check 8 of `verify-openapi-routes.mjs`

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

## Progress — 2026-09-24

**Docs half on the branch, after #343 merged.** `GatewayMessage.message` loses
its "`403` reading `Missing Authentication Token` means the path does not
exist" sentence; `ErrorEnvelope`'s description names the gateway's unknown-route
`404` as its second source; the Quick Start's 403 row loses the same sentence
and its 404 row says "or no such route". `public/openapi.json` re-extracted:
those two descriptions are the only content change. The Quick Start lede
("the gateway's 403 and 429 carry a message only") and the `Authorization:
Bearer` → 403 verdict stay true.

**Production before the deploy, measured 09:30 CEST** (no key needed —
route matching runs before the key check):

| request | now | expected after |
|---|---|---|
| `GET /v1/nope`, no key and a wrong key | `403` `MissingAuthenticationTokenException` | `404` `not_found` |
| `PATCH /v1/assets` | same `403` | `404` |
| `GET /api/` on the API host, `PATCH /api/key` | same `403` | `404` |
| `GET /v1/assets`, no key and a wrong key | `403 {"message":"Forbidden"}`, `ForbiddenException` | unchanged |

The same probes after the deploy settle AC 1–2; AC 3 once Compute and the
portal sync have shipped the texts.

**Compute is current except for this task.** Adam's 0216 rollout (Compute,
11:30 CEST) shipped `develop` with #343 and #345: the live `/api-docs-json`
differs from this branch's extract in exactly the two descriptions above. So
0309's Compute deploy carries those two plus whatever merges meanwhile — read
the diff first.

## Shipped — 2026-09-24 (CEST)

From `develop` `b7bddad2` (#348 merged 11:51), Lambdas built locally from the
same commit and checked (12 aarch64 bootstraps; the api-handler carries the new
`ErrorEnvelope` text and not the removed sentence). Diffs before the deploy:
ApiGateway as on 2026-09-23; Compute only the two functions' `S3Key` — the
ledger-processor's source is unchanged, its local build is not byte-identical.
No IAM in either.

1. **ApiGateway, 12:19:51** (22 s). 13 s later the answers were mixed — two of
   the six unknown-route probes still `403` — and by 12:20:24 ten samples of
   each were all `404`: here the new stage deployment took between 13 and
   33 s to reach every node, so a probe right after a gateway deploy should
   be sampled, not read once. Full run at 12:20:41: `/v1/nope` (no key, a wrong key,
   `Accept: text/html`), `PATCH /v1/assets`, `GET /api/` and `PATCH /api/key`
   all `404 {"code":"not_found","message":"no such route"}`,
   `content-type: application/json`; `/v1/assets` with no key or a wrong key
   `403 {"message":"Forbidden"}`; `/health` `200`. The `404` still carries
   `x-amzn-ErrorType: MissingAuthenticationTokenException` — API Gateway's
   header, left as is.
2. **Compute, 12:21:50** (17 s), stage cache flushed. `/api-docs-json` serves
   the new descriptions and equals the extracted document. The
   ledger-processor ran 10–13 invocations a minute through the deploy with 0
   errors; all 61 `prices-production-*` alarms OK.
3. **Portal, 12:23** — `sync-portal-explorer`, guild check ok, bundle
   `index-BCrE5KYY.js`: the Quick Start's 404 row names "no such route", the
   403 row's `Missing Authentication Token` sentence is gone, and the bundled
   `openapi.json` equals the live `/api-docs-json`.

## Implementation Notes

- **Infra** (`7f40dc55`): `UnknownRoute` in `api-gateway-stack.ts`, after the
  two CORS responses; the comments on `PORTAL_API_METHODS` and the
  `/api/{proxy+}` block now name the `404`. Check 8 in
  `verify-openapi-routes.mjs` — exactly one `MISSING_AUTHENTICATION_TOKEN`
  response, `404`, a JSON body with `code: not_found` and a `message` — and
  its check-3 and unroutable messages say `404` instead of `403`.
- **Docs** (`cac16123`, after #343): `ErrorEnvelope` and
  `GatewayMessage.message` in `descriptions.rs`, the Quick Start's 403 and
  404 rows, `public/openapi.json` re-extracted (two lines); plus
  `api-endpoints.md` (a paragraph on routes outside the table),
  `portal-oauth-deploy-prep.md` and the doc comment on `with_web_origin` in
  `portal/auth/mod.rs` (`/api/` on the API host), which went with the infra
  commit.
- **Tests:** none modified. Rust OpenAPI suite 13/13 + 2/2, portal 253
  tests with lint and typecheck, `redocly lint`, synth + check 8, all green
  before the push; CI 4/4 on #348.

## Design Decisions

### From Plan

1. **The plan's shape**: `404 {"code":"not_found","message":"no such route"}`
   from a `MISSING_AUTHENTICATION_TOKEN` gateway response, API-wide.
2. **An unmapped verb gets `404`, not `405`** — accepted in the plan.
3. **No CORS headers** (the plan left it open): the bundle calls no unmapped
   route, and the portal's one credentialed origin on an API-wide answer is
   what 0194's review took off `DEFAULT_4XX`.

### Emerged

4. **Check 8 in the existing verifier**, not a new script: it is already the
   CI step that reads the synthesized ApiGateway template.
5. **The docs half waited for #343** rather than stacking on 0306's branch —
   Stanisław's call; one PR against `develop`.
6. **Shipped as ApiGateway + Compute + portal**, not "ApiGateway only" as the
   plan said: the OpenAPI descriptions live in the api-handler.
7. **ApiGateway before Compute**, so the minutes between them had the docs
   still explaining the old `403` rather than promising a `404` the gateway
   did not give yet.
8. **Historical mentions left alone**: "a reader's first request 403'd" in
   `Terminal.tsx`, `QuickStart.tsx:52` and the spec describe the past; only
   statements about current behaviour changed.

## Issues Encountered

- **Two tasks numbered 0309.** A CSP task created as 0309 in another worktree
  at 11:40 on 2026-09-23 was never pushed (the push was refused), and this
  task took 0309 at 12:58. The CSP task still needs a free number.
- **A synth that proved nothing.** The Lambda-asset stub directory in the
  session scratchpad was gone the next morning, the synth failed on the
  redirect, and the verifier "passed" against the previous day's `cdk.out`.
  Caught from the redirect error; re-run after recreating the stubs, with the
  template's timestamp checked.
- **Gateway propagation**: see Shipped, step 1.
- **`gh pr edit` fails** on GitHub's Projects (classic) deprecation; the PR
  body was edited with `gh api --method PATCH …/pulls/348`.

## Future Work

- [[0205]] can close: the `cdk diff`s of 2026-09-23 and 24 showed no change
  to `/api/{proxy+}`, so the greedy proxy is live, and the stack comment
  calling the intermediate pair "CURRENTLY DEPLOYED" is stale.
- [[0255]]: a candidate mechanism, not written there (Oskar's task) —
  changing a `GatewayResponse`'s `ResponseType` replaces the resource, and the
  new `Deployment` snapshots the API before the cleanup phase deletes the old
  response, so the stage keeps serving it. The same would apply to removing
  `UnknownRoute`: roll back with one extra `aws apigateway create-deployment`.

