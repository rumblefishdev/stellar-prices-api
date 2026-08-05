---
id: "0152"
title: "Self-service onboarding — Discord sign-in, automatic API key, usage dashboard, quickstart"
type: FEATURE
status: active
related_adr: ["0008"]
related_tasks: ["0124", "0126", "0121"]
tags: [layer-infra, layer-backend, layer-frontend, priority-high, effort-large, milestone-M3, tranche-3, api-gateway, onboarding, discord-oauth]
milestone: 3
links:
  - "sources/epic-self-service-onboarding.md"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../infra/src/lib/types.ts"
  - "../../../packages/prices-api/src/auth/mod.rs"
history:
  - date: 2026-08-05
    status: active
    who: akot
    note: >
      Opened from the epic doc handed over as
      `sources/epic-self-service-onboarding.md` (scope settled there — do not
      relitigate). Picked up by Adam. Umbrella task: the placement plan below
      splits into per-component tasks once the first slice lands.
---

# Self-service onboarding portal

## Summary

External developers get a Prices API key without a human in the loop: sign in
with Discord, key issued automatically and shown on-screen, dashboard shows
usage against quota, quickstart + example queries alongside. Tranche 3
deliverable and an RFP acceptance criterion ("Onboarding portal accessible at
documented URL; self-service API key request flow functional").

Full scope, resolved decisions and rationale live in
[sources/epic-self-service-onboarding.md](sources/epic-self-service-onboarding.md).
That document is settled input, not a draft to reopen.

## Context

Nothing in this repo serves a frontend today — `CicdStack` says so explicitly
("no S3 SPA, no CloudFront"), and `ApiGatewayStack` creates exactly **one**
hand-made partner key on a usage plan throttled at 100 req/s. This epic adds
the first browser-facing surface, the first Lambda that *writes* AWS resources
(API Gateway keys), and the first OLTP-shaped storage (Discord ID → key ID).

Two adjacent tasks touch the same ground: [[0124]] (spec over the gateway —
still on its own branch, **not yet in `develop`**, so this branch does not carry
`apiBaseUrl` or the `/api-docs-json` gateway mapping) and [[0126]] (custom
domain + CORS + WAF, which owns the hostname the portal is documented at).

## Placement plan

| Component | Where |
|-----------|-------|
| Onboarding backend (OAuth callback, key issue/rotate, `GetUsage`) | new Rust package `packages/onboarding-api/` — same axum + `lambda_http` shape as `prices-api`, separate Lambda so its IAM write scope never lands on the read path |
| Discord ID → key mapping | new DynamoDB table in a new `OnboardingStack` (ClickHouse is the wrong store for per-user OLTP reads) |
| Portal SPA | new `packages/onboarding-portal/` (first TS/web package; `packages/*` is already an npm workspace, so Nx picks it up) |
| Portal hosting (S3 + CloudFront + `BucketDeployment`) | new `infra/src/lib/stacks/portal-stack.ts`, wired in `app.ts` |
| Onboarding routes on the gateway | `api-gateway-stack.ts` — `apiKeyRequired: false`, uncached, separate resource tree from `/v1` |
| Rate-limit override (1 req/s + monthly quota) | `infra/envs/production.json` + `types.ts` + the usage-plan block in `api-gateway-stack.ts` |
| Quickstart + example queries | `docs/` and the portal; examples must be executed against the live API before they ship (AC 3) |
| CI/CD scopes (S3 write, CloudFront invalidation) | `cicd-stack.ts` — its comment that there is no frontend stops being true here |

## Implementation order

1. **Rate limits first** — smallest, independent, no new components: 1 req/s per
   key + monthly quota replacing the 100 req/s default (epic §"Rate limiting",
   AC 5). Ships and demos on its own.
2. **Storage + key issuance backend** — DynamoDB table, `onboarding-api`
   Lambda, Discord OAuth callback, `CreateApiKey` + usage-plan association.
3. **Usage + rotation** — `GetUsage` endpoint, key re-read via
   `GetApiKey(includeValue=true)`, once-per-calendar-month rotation guard.
4. **Portal + hosting** — SPA, S3/CloudFront stack, CI/CD deploy scopes.
5. **Quickstart + examples**, verified against the live API.

## Acceptance criteria

- [ ] Portal reachable at a documented URL (coordinate hostname with [[0126]])
- [ ] Discord sign-in → key issued and shown on-screen → key works live
- [ ] Quickstart + example queries present and verified against the live API
- [ ] Dashboard shows usage against quota, sourced from `GetUsage`
- [ ] Default key limits are 1 req/s + monthly quota, not 100 req/s
- [ ] Rotation rate-limited to once per calendar month

## Open items to confirm before build

Carried from the epic, flagged there as unverified:

- Does Stellar's Discord actually require member verification? The whole
  abuse story rests on it (epic §"Auth & key handling").
- One active key per Discord account — recommended, needs sign-off.
- Monthly quota number: epic suggests 50k–100k calls/month; pick one.
