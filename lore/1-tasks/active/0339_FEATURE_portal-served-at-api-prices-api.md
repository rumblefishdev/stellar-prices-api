---
id: "0339"
title: "Serve the portal at /api/prices-api/ on the explorer's host"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0326", "0194"]
tags: ["portal", "hosting", "effort-small"]
links:
  - https://github.com/rumblefishdev/soroban-block-explorer/pull/699
history:
  - date: 2026-11-03
    status: active
    who: stkrolikiewicz
    note: >
      Task created. The product portals gather under /api/ on the explorer's
      host (team decision): /api/prices-api/ now, /api/x402-facilitator/ and
      /api/sorobanscan/ later, /api/ a hub. Explorer side:
      soroban-block-explorer task 0660, PR #699.
---

# Serve the portal at /api/prices-api/ on the explorer's host

## Summary

The portal moves from `https://sorobanscan.rumblefish.dev/prices-api/` to
`/api/prices-api/`, under the hub at `/api/` that will also list the other
product portals. The explorer (task 0660, PR #699) serves
`/api/prices-api(/*)` from the bucket prefix `api/prices-api/` and answers
`/prices-api…`, `/pricing-api…` and every other `/api/…` with one `301` to the
same path under `/api/prices-api`, query string kept. Same kind of move as
[[0326]].

## Status: Active

**Current state:** code and docs on the branch; nothing deployed. Next: the
deploy order below, explorer PR #699 first.

## Context

The bundle bakes its prefix in (Vite `base`, router `basename`), the sync puts
it under the same key prefix in the explorer's bucket (CloudFront hands S3 the
viewer path unchanged), and the backend's OAuth callback lands the popup on
`<portalWebOrigin>/prices-api/`. All three move. On the API's own host nothing
moves: `/api/auth/*`, `/api/key`, `/api/usage`, `/api/config`, the session
cookie's `Path=/api/`, the Discord `redirect_uri`, `portalWebOrigin`.

## Implementation Plan

1. `web/portal/src/base-path.ts` + `vite.config.mts`: `BASE_PATH` →
   `/api/prices-api/`; specs that pin bundle paths follow.
2. `packages/prices-api/src/portal/auth/mod.rs`: `PORTAL_HOME` →
   `/api/prices-api/`; every test asserting a landing follows.
3. `infra/Makefile` `sync-portal-explorer`: upload to `api/prices-api/`,
   invalidate `/api/prices-api/*`.
4. Current-state docs (README, `docs/scf/api-endpoints.md`, overview,
   runbook, skill) name the new URL. Dated SCF evidence stays as submitted.

### Deploy order (production, manual)

1. Here: `make -C infra sync-portal-explorer` — the new bundle under
   `api/prices-api/`. Nothing serves it until step 2; the old `prices-api/`
   keeps serving.
2. Explorer PR #699: Delivery (CDK) deploy, then the explorer SPA.
3. Here: Compute with the new `PORTAL_HOME`. **Never before step 2:** until
   then `/api/prices-api/?signin=…` would `301` to `/prices-api/prices-api/`.
   Until step 3 the old home `/prices-api/?signin=…` reaches the portal
   through the `301`.

Then:

- The old `prices-api/` bucket prefix is dead: drop it, like explorer task
  0631 does for `api/` and `pricing-api/`. Any delete of the old `api/`
  prefix must now exclude `api/prices-api/`, or it takes the live portal.
- A person completes a Discord sign-in round-trip on production.

## Acceptance Criteria

- [ ] Portal boots at `/api/prices-api/`, `/api/prices-api/dashboard`,
      `/api/prices-api/docs` (hard refresh included)
- [ ] Discord sign-in round-trip lands on `/api/prices-api/?…` (after
      Compute; until then via the `301`), done by a person on production
- [ ] `/prices-api/?utm_source=stellar-skill` → one `301` to
      `/api/prices-api/?utm_source=stellar-skill`
- [ ] Old `prices-api/` prefix dropped from the bucket once the above holds

## Design Decisions

### From Plan

1. **Backend `/api/` untouched.** Only `PORTAL_HOME`, the landing, moves.
   The API host and the explorer host now both have an `/api/`, but they are
   different hosts and nothing routes between them; the session cookie is
   host-only, so its `Path=/api/` never reaches the explorer's host. ~110
   Location literals in `packages/prices-api/tests/portal_*.rs` follow
   `PORTAL_HOME`.

### Emerged

2. **`PORTAL_HOME` on the API host is no longer "nothing".** A relative
   landing (no `PORTAL_WEB_ORIGIN`) on the API host would now fall into the
   backend's own `/api/` namespace instead of the gateway's `404`. Production
   always sets the origin; the doc comment says so instead of claiming a 404.
3. **Dev proxy spec pins the real bundle paths.** `dev-proxy.spec.ts` listed
   `/api/login`, `/api/@vite/client`… from the 0194 layout; it now lists
   `/api/prices-api/…`, the paths Vite actually serves. The enumerated
   backend regex is load-bearing again: the bundle shares `/api/`.
4. **Dated SCF evidence left as submitted** (`docs/scf/milestone-*`): the
   `301` keeps their links working, as in [[0326]].
5. **Task created on the branch, not on `develop`.** Asked for explicitly
   (nothing pushed straight to `develop`), against the usual rule in
   `lore/CLAUDE.md`.

## Implementation Notes

Checked locally on 2026-11-03: `cargo test -p prices-api` (lib 271 + every
`portal_*` suite green), `cargo clippy -p prices-api --all-targets -D
warnings`, `cargo fmt --all --check`, `nx run-many -t lint typecheck test`
on `portal` and the CDK project. `make -C infra build-portal-explorer`: every
asset URL in `dist/index.html` is under `/api/prices-api/`; `vite preview`
boots `/api/prices-api/`, `/api/prices-api/docs` and `/api/prices-api/dashboard`.

## Notes

- The usage plan names (`pricing-api-<tier>-<env>`) and the SSM parameter
  `/prices/<env>/pricing-api-free-plan-id` are AWS resource names, not URLs.
