---
id: "0185"
title: "Portal app — ugly but real: Vite/React skeleton served from /api-tokens/, built by CI"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0183", "0162", "0184", "0186", "0193"]
tags: [layer-frontend, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, ui, vite, react, slice-2]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0162_FEATURE_portal-frontend-app.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Second slice, carved out of [[0162]]. Explicitly the "working but ugly
      frontend" step: the app exists, deploys and routes, and looks like
      nothing. Styling is [[0193]]; every screen with content attaches to the
      backend slice that gives it something to show.
  - date: 2026-08-14
    status: active
    who: akot
    note: >
      Activated once [[0183]] and [[0184]] landed on `develop` (#207, #209).
      **Read [[0184]]'s open note before assuming what is live:** it is merged
      but not fully deployed — the gateway still maps the intermediate
      `{proxy}` + `{proxy}/{sub}` pair rather than `{proxy+}`, and the access
      logs, upload `Cache-Control` and trailing-slash redirect are code-only.
      So `/api-tokens/api/*` answers at depth 1-2 and `403`s at depth 3 until
      three deploys run. This slice's `/api-tokens/api/health` probe sits at
      depth 1, so it works either way — but do not read a depth-3 `403` as a
      bug in this task.
---

# Portal app — ugly but real

## Summary

**Story:** *as a developer on this team, I can change a line of TSX, push, and
see it live at `/api-tokens/` — so every slice after this has somewhere to land.*

An unstyled React app with one route that renders a heading and calls
`/api-tokens/api/health`. No design, no MUI, no auth, no key. The value is the
pipeline, not the page.

## Context

[[0162]] bundled the framework decision, the base-path plumbing, both screens,
five refusal states, the rework modal and a mobile pass into one effort-large
task that could not start until the whole backend existed. This slice takes only
the parts with no upstream dependency, which turns out to be most of the risk:
base path and routing are the two things that are painful to retrofit and
trivial to get right on the first commit.

**Ugly is a requirement here, not a concession.** If this slice produces
something presentable, it has taken work from [[0193]] and delayed [[0186]].

## Implementation

- **Stack, settled 2026-08-07 — mirror `soroban-block-explorer`:** React 19,
  Vite 7, TypeScript 5.9, `react-router-dom` 7, Vitest 4, orchestrated by Nx
  (`@nx/react`, `@nx/vite`). MUI is on that list too but is **not** installed
  here — it arrives with [[0193]]. Plain HTML elements until then.
- **`base: '/api-tokens/'` in `vite.config.ts` and `basename="/api-tokens"` on
  the router, from the first commit.** Without them every asset and route URL
  points at the domain root and the app breaks the moment it is not served from
  `/`. This is a build setting; [[0184]] cannot fix it from CloudFront.
- **Copy the explorer's dev-proxy pattern** — `vite.config.ts` proxies API paths
  through the dev server, and injects any dev `x-api-key` **server-side in the
  Node config**, never into the client bundle. Same-origin locally, matching what
  [[0184]] gives us in production.
- **Generate API types from `/api-docs-json`** ([[0124]]), do not hand-maintain
  them. The explorer hand-maintains `libs/api-types`; that is the one place we
  diverge from it, because our API publishes its own contract.
- Start with the app alone. No `libs/ui`, no `libs/api-types` split until a
  second frontend actually exists.
- CI builds the bundle, syncs it to [[0184]]'s bucket and invalidates.
- No third-party scripts, ever — no analytics, no CDN fonts. This page will
  render a credential and the CSP should stay trivial.

## Acceptance Criteria

- [ ] **Ships closed.** The app reads `GET /api-tokens/api/config` ([[0183]])
      and renders "not yet available" with no sign-in button while `enabled` is
      false — the bundle is publicly reachable from the first deploy
- [ ] `nx build` produces a static bundle; `nx test` runs and passes
- [ ] The app is served at `/api-tokens/`, assets resolve, and a hard refresh on
      `/api-tokens/` returns the app
- [ ] The page shows the result of a live call to `/api-tokens/api/health`,
      proving the same-origin path end to end
- [ ] No API key, no secret and no third-party script in the bundle
- [ ] CI deploys on merge; no manual upload step
- [ ] It looks bad and nobody has spent time on that

## Notes

- A refresh on a **sub-route** (`/api-tokens/dashboard`) is knowingly broken
  until [[0195]] adds the per-prefix SPA fallback. With one route there is
  nothing to break yet; do not add routes that depend on it before then.
- Nx here is 22.7.0 against the explorer's 22.6.1 — close enough to share config
  shapes, worth a glance if a generator misbehaves.
