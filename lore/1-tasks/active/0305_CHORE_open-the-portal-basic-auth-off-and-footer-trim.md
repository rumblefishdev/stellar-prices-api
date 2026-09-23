---
id: "0305"
title: "Open the portal to the public — explorer basic auth off /api/*, footer trimmed to live links"
type: CHORE
status: active
related_adr: []
related_tasks: ["0194", "0195", "0301", "0303"]
tags: [layer-frontend, portal, priority-high, effort-small]
links:
  - "../archive/0195_FEATURE_swagger-ui-spa-fallback-and-custom-domain.md"
  - "../archive/0194_TEST_portal-security-and-ops-audit/README.md"
  - "../../../web/portal/src/landing/Chrome.tsx"
  - "../../../docs/scf/api-endpoints.md"
history:
  - date: "2026-09-23"
    status: backlog
    who: stkrolikiewicz
    note: >
      Created after the go-public decision (Stanisław + Marek, 2026-09-22).
      The basic-auth switch was left open by 0194 and 0195 as "not ours to
      decide"; it is now decided.
  - date: "2026-09-23"
    status: active
    who: stkrolikiewicz
    note: "Activated; footer trim and logo check start on the branch."
---

# Open the portal to the public

## Summary

The portal bundle is served under `https://sorobanscan.rumblefish.dev/api/*`
on the block explorer's distribution, behind the explorer's basic-auth
CloudFront function. On 2026-09-22 Stanisław and Marek decided to take that
gate off. This task tracks the switch (explorer repo) and the portal-side
changes that go with it: the footer loses its two dead or duplicate links, and
the SorobanScan logo is verified on every page.

## Context

- [[0195]] decision 5 and [[0194]] left `enableApiSpaBasicAuth: true` in the
  explorer's `production.json` as the one thing gating public availability —
  recorded, never tracked as a task.
- On 2026-09-23 `GET /api/` still answers `401` from CloudFront; the explorer
  root answers `200`.
- The footer's `Status` has had no destination since the design (rendered as
  plain text), and `rumblefish.dev` duplicates the Rumble Fish mark next to it,
  which already links there.

## Implementation Plan

### Step 1: Explorer repo — basic auth off `/api/*`

In `soroban-block-explorer`: `enableApiSpaBasicAuth: false` in production
config, so `production-soroban-explorer-basic-auth` is detached from the
`/api/*` behaviour only. The explorer root stays as it is.

### Step 2: Portal footer

Remove `Status` and `rumblefish.dev` from the footer in
`web/portal/src/landing/Chrome.tsx`. The Rumble Fish mark keeps the link to
`rumblefish.dev`.

### Step 3: SorobanScan logo on every page

Every bar renders the same `Wordmark` (`href = EXPLORER`). Cover each route,
signed in and signed out, with a test.

### Step 4: Docs

`docs/scf/api-endpoints.md` still says the portal is behind basic auth
(the "shared host" section and the remaining-gaps list). Update once Step 1 is
deployed.

## Acceptance Criteria

- [ ] Footer shows neither `Status` nor the `rumblefish.dev` text link; the
      Rumble Fish mark still links to `https://rumblefish.dev`
- [ ] The SorobanScan logo links to `https://sorobanscan.rumblefish.dev/` on
      `/`, `/quick-start`, `/docs`, `/privacy-policy` and `/dashboard`, signed
      in and signed out (`/login` has no bar)
- [ ] Explorer: `enableApiSpaBasicAuth: false` deployed to production
- [ ] Without credentials: `/api/` answers `200`; a refresh on `/api/dashboard`
      and `/api/docs` returns the portal's `index.html` (0195's open AC)
- [ ] `docs/scf/api-endpoints.md` no longer describes the portal as gated
