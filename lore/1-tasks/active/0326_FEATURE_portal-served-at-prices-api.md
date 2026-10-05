---
id: "0326"
title: "Serve the portal at /prices-api/ on the explorer's host"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0194", "0195"]
tags: ["portal", "hosting", "effort-small"]
links: []
history:
  - date: 2026-10-01
    status: active
    who: stkrolikiewicz
    note: >
      Task created. The portal moves from sorobanscan.rumblefish.dev/api/ to
      /pricing-api/; the explorer answers 301 on the old path
      (soroban-block-explorer task 0608).
  - date: 2026-10-02
    status: active
    who: stkrolikiewicz
    note: >
      /pricing-api/ went live (#384, sync + explorer Delivery deploy), then
      the name changed to /prices-api/: it matches the product, this repo and
      the API host prices-api.sorobanscan…, and "pricing" reads as a price
      list or a valuation engine. The explorer 301s /pricing-api… too.
  - date: 2026-10-02
    status: active
    who: stkrolikiewicz
    note: >
      /prices-api/ live ~10:55 UTC: sync-portal-explorer uploaded the bundle
      under prices-api/ (#385), then the explorer's Delivery deploy routed it.
      Portal and every 301 verified on production.
---

# Serve the portal at /prices-api/ on the explorer's host

## Summary

`https://sorobanscan.rumblefish.dev/api/` reads as the explorer's own API. The
portal moves to `/prices-api/`; the explorer turns every `/api…` URL into a
`301` to the same path under `/prices-api`, query string kept
(soroban-block-explorer task 0608), so nothing shared before the move breaks.

## Status: Active

**Current state:** `/prices-api/` live since 2026-10-02 ~10:55 UTC; `/api…`
and `/pricing-api…` answer `301` there. Next: Compute (`PORTAL_HOME`) with the
next regular release, after the asset_id migration ends (no AWS deploys
meanwhile, Adam, 2026-10-02); a Discord sign-in round-trip by a person; then
drop the `api/` and `pricing-api/` bucket prefixes.

## Context

The bundle bakes its prefix in (Vite `base`, router `basename`), the sync puts
it under the same key prefix in the explorer's bucket (CloudFront hands S3 the
viewer path unchanged), and the backend's OAuth callback lands the popup on
`<portalWebOrigin>/api/`. All three move. Backend routes on the API's own
hostname (`/api/auth/*`, `/api/key`, `/api/usage`, `/api/config`, the session
cookie's `Path=/api/`) do not: that host has no portal bundle on it.

## Implementation Plan

1. `web/portal/src/base-path.ts` + `vite.config.mts`: `BASE_PATH` →
   `/prices-api/`; specs that pin bundle paths follow.
2. `packages/prices-api/src/portal/auth/mod.rs`: `PORTAL_HOME` →
   `/prices-api/`.
3. `infra/Makefile` `sync-portal-explorer`: upload to and invalidate
   `prices-api/`.
4. Current-state docs (README, `docs/scf/api-endpoints.md`, overview,
   runbook, skill) name the new URL. Dated evidence (SCF milestone-2 files)
   stays as submitted; the `301` keeps those links working.

### Deploy order (production, manual)

1. Here: `make -C infra sync-portal-explorer` — new bundle under
   `prices-api/`; live paths keep serving what they serve meanwhile.
2. Explorer: Delivery stack (task 0608) — `/prices-api/` live, `/api…` and
   `/pricing-api…` → `301`.
3. Here: Compute (`PORTAL_HOME`). Until then the popup lands on `/api/?…` and
   the `301` carries it, query string included.

## Acceptance Criteria

- [x] Portal boots at `/prices-api/`, `/prices-api/dashboard`,
      `/prices-api/docs` (hard refresh included)
- [ ] Discord sign-in round-trip lands on `/prices-api/?…` (after Compute;
      until then via the `301`)
- [x] `/api/?utm_source=stellar-skill` → `301` `/prices-api/?utm_source=stellar-skill`
- [ ] Old `api/` and `pricing-api/` prefixes dropped from the bucket once
      the above holds

## Design Decisions

### Emerged

1. **Backend `/api/` untouched.** Only `PORTAL_HOME`, the landing, moves; the
   OAuth `redirect_uri`, the session cookie's `Path=/api/` and every backend
   route live on the API host, where nothing moved. 112 Location literals in
   `packages/prices-api/tests/*` follow `PORTAL_HOME`.
2. **Dated SCF evidence left as submitted** (`docs/scf/milestone-2-*`,
   `docs/epics/*`, the "landed" entries in `api-endpoints.md`): they record
   what was true then, and the `301` keeps their links working.
3. **Skill URL updated** (`skills/stellar-prices-api/SKILL.md`, `utm_source`
   kept). Any copy published outside this repo (task 0318) still points at
   `/api/`, which the `301` covers.

4. **`/prices-api/`, not `/pricing-api/`** — see the 2026-10-02 history
   entry. Usage plan names (`pricing-api-<tier>-<env>`) and the SSM
   parameter `/prices/<env>/pricing-api-free-plan-id` are AWS resource
   names, not URLs, and stay.

## Notes

- The dev proxy keeps its enumerated backend regex: still correct, just no
  longer forced by a shared prefix.
