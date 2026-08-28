---
id: "0235"
title: "Move the portal's prefix from /api-tokens to /api — the whole portal, bundle and backend alike"
type: REFACTOR
status: active
related_adr: ["0010"]
related_tasks: ["0161", "0184", "0186", "0194", "0195", "0205"]
tags: [layer-frontend, layer-backend, layer-infra, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, portal, routing]
milestone: 3
links:
  - "../active/0194_TEST_portal-security-and-ops-audit/README.md"
  - "../backlog/0195_FEATURE_swagger-ui-spa-fallback-and-custom-domain.md"
history:
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Spawned from [[0194]] and activated the same day. Adam is deploying the
      portal to `https://sorobanscan.rumblefish.dev/api/` by syncing the bundle
      to `s3://production-soroban-explorer-api-spa/api`, and decided the whole
      portal moves with it: `/api-tokens` → `/api`, bundle and backend alike,
      keeping the shape inside the prefix. [[0194]] is a TEST and says in its
      own Notes that code belongs in the slice that owns it — this is that
      slice.
  - date: "2026-08-28"
    status: active
    who: akot
    note: >
      Substitution applied to 38 files — Rust constants and tests, the web
      bundle and its two `BASE_PATH` copies, both infra stacks including
      `DirectoryIndexFn`'s allow-lists and `destinationKeyPrefix`, the two
      verifier scripts, and three live docs. `lore/` deliberately untouched:
      16 files there still name the old prefix and are records. Verified on a
      fresh `synth-production`: gateway resource `/api/api/{proxy+}` with all
      three `methodSettings` entries at 10/40 and caching off, CloudFront
      `/api/api/*` ahead of `/api/*`, built bundle emitting
      `/api/assets/…`, `openapi:verify-routes` reporting the prefix agrees
      across handler, routing table and check, and `openapi:verify-servers`
      green. Rust suite green; portal 156/156 (one flaky failure seen once
      while `cargo test` saturated the CPU, not reproduced in four runs).
---

# Move the portal's prefix from /api-tokens to /api

## Summary

**One substitution, applied everywhere the prefix is baked in.** The portal is
served under `/api-tokens/` today — the bundle at `/api-tokens/*` and its
backend at `/api-tokens/api/*`. Adam's target is
`https://sorobanscan.rumblefish.dev/api/index.html`, so the prefix becomes
`/api` and the shape inside it is unchanged:

| | before | after |
|---|---|---|
| bundle | `/api-tokens/` | `/api/` |
| app routes | `/api-tokens/login`, `/dashboard`, `/quick-start` | `/api/login`, `/api/dashboard`, `/api/quick-start` |
| backend | `/api-tokens/api/*` | `/api/api/*` |
| callback | `/api-tokens/api/auth/callback` | `/api/api/auth/callback` |
| session cookie | `Path=/api-tokens/` | `Path=/api/` |
| pending cookie | `Path=/api-tokens/api/auth/` | `Path=/api/api/auth/` |
| gateway resource | `/api-tokens/api/{proxy+}` | `/api/api/{proxy+}` |
| S3 key prefix | `api-tokens/` | `api/` |

`/api/api/` is not a typo. [[0161]]'s convention is `<app>/*` for the bundle and
`<app>/api/*` for that app's backend, and the app is now called `api` — so the
doubling is the convention applied to this name, not a mistake to tidy away.
Collapsing the two (backend directly at `/api/auth/callback`) was considered
and rejected: it puts backend routes as siblings of SPA routes, so CloudFront
needs one behaviour per backend route instead of one for the whole namespace,
every new route becomes an infrastructure change, and a future SPA route can
collide with an endpoint name.

## Context

The prefix is a **build-time and deploy-time** fact in seven places, and the
three layers must agree or the failure is silent:

- **Rust** — `PORTAL_API_PREFIX`, `CONFIG_PATH`, `LOGIN_PATH`, `CALLBACK_PATH`,
  `ME_PATH`, `LOGOUT_PATH`, `PORTAL_HOME`, `SESSION_PATH`, `PENDING_PATH`
- **Web** — `BASE_PATH` (and its copy in `vite.config.mts`, which
  `base-path.spec.ts` pins), `PORTAL_API`, the route table in `links.ts`
- **Infra** — `PORTAL_API_RESOURCE_PATH` and the three `methodSettings` entries
  keyed by it, `PORTAL_BACKEND` / `PORTAL_BUNDLE`, `DirectoryIndexFn`'s
  `REDIRECTS` and `APP_ROUTES` allow-lists, `distributionPaths`,
  `destinationKeyPrefix`
- **Out of band** — the Discord redirect URI, and the `redirect_uri` field of
  the `prices/production/portal-discord-oauth` secret, which the loader rejects
  unless it ends in `CALLBACK_PATH`

**The timing is lucky and worth naming.** The OAuth secret does not exist yet
([[0194]]'s audit, check 9), so this rename costs no cutover: the secret is
created once, already carrying the new callback, instead of being created with
the old one and rewritten by `put-secret-value` under [[0186]]'s §6 dance.

## Implementation

- One substitution of the literal `api-tokens` → `api` across code, tests and
  live docs. Every occurrence outside `lore/` is a path segment — verified
  before the change, no identifier, tag, bucket or secret name contains it
- `lore/` is **not** rewritten: archived tasks are records of what was true
- The deployed bucket keeps its old `api-tokens/*` objects after the next
  deploy, because `prune` is scoped to `destinationKeyPrefix`. Harmless (no
  behaviour routes to them once the CloudFront table moves) but worth a manual
  cleanup

## Acceptance Criteria

- [x] No occurrence of `api-tokens` outside `lore/` and `.trash/` — 38 files
- [x] `cargo test -p prices-api` and the portal's Vitest suite pass — Rust green, portal 156/156
- [x] `openapi:verify-routes` passes — it asserts the gateway↔spec mapping and
      the portal behaviour's ordering against the synthesized template
- [x] A synth diff shows the gateway resource, all three `methodSettings`
      entries and both CloudFront behaviours moved together
- [ ] The Discord registration carries the new callback before the secret is
      created ([[0194]] step 1)
- [ ] The three [[0194]] checks marked `(host)` are re-run after the deploy

## Notes

- Open tasks that still name the old prefix in their own text — [[0195]],
  [[0205]] — are left alone here and updated by their owners. [[0194]]'s audit
  report is a dated measurement and is not rewritten.
