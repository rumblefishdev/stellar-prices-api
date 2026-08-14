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

- [x] **Ships closed.** The app reads `GET /api-tokens/api/config` ([[0183]])
      and renders "not yet available" with no sign-in button while `enabled` is
      false — the bundle is publicly reachable from the first deploy
- [x] `nx build` produces a static bundle; `nx test` runs and passes — 4 tests
- [x] The app is served at `/api-tokens/`, assets resolve, and a hard refresh on
      `/api-tokens/` returns the app — verified at build: the emitted
      `index.html` references `/api-tokens/assets/…`. **Not yet verified live**,
      which needs [[0205]]'s deploys
- [x] ~~The page shows the result of a live call to
      `/api-tokens/api/health`~~ — **amended to `/api-tokens/api/config`**; that
      route does not exist and could not be made to answer. See decision 4
- [x] No API key, no secret and no third-party script in the bundle — asserted
      in a test for the key, and by inspection of `dist/` for all three
- [ ] ~~CI deploys on merge; no manual upload step~~ — **withdrawn, with Adam,
      2026-08-14.** See decision 9
- [x] It looks bad and nobody has spent time on that

## Notes

- A refresh on a **sub-route** (`/api-tokens/dashboard`) is knowingly broken
  until [[0195]] adds the per-prefix SPA fallback. With one route there is
  nothing to break yet; do not add routes that depend on it before then.
- Nx here is 22.7.0 against the explorer's 22.6.1 — close enough to share config
  shapes, worth a glance if a generator misbehaves.

## Implementation Notes

The app is `web/portal`, an `@nx/react` project: React 19, Vite, TypeScript 5.9,
Vitest 4, `react-router-dom` 7. One route, plain elements, no MUI.

- `web/portal/vite.config.mts` — `base: '/api-tokens/'` and the dev proxy.
- `web/portal/src/main.tsx` — `basename` on the router, the other half of `base`.
- `web/portal/src/api/portal.ts` — the one backend call, relative by
  construction.
- `web/portal/src/app/app.tsx` + `app.spec.tsx` — the page and four tests.
- `infra/src/lib/stacks/portal-hosting-stack.ts` — `PORTAL_ASSET_DIR` repointed
  from [[0184]]'s placeholder to `../web/portal/dist`, and the single
  `BucketDeployment` split in two.
- `infra/Makefile` — `build-portal`, and a `build-production` every production
  target now hangs off.
- `.github/workflows/ci.yml` — `web/**` in the `typescript` paths filter.

Verified: `build`, `lint`, `typecheck` and `test` pass for `portal` and for the
CDK app; `make -C infra synth-production` succeeds **from a tree with no
`web/portal/dist`**, which is the property the Makefile change exists to
guarantee; the synthesized template carries both deployments with the intended
filters, cache headers and invalidation.

## Design Decisions

### From Plan

1. **`base` + `basename` in the first commit.** Both, not either: `base` covers
   assets and `basename` covers routes. They differ by a trailing slash on
   purpose — Vite concatenates without one and emits `/api-tokensassets/…`,
   react-router warns if given one.
2. **Dev proxy with the key injected server-side**, ported from
   `soroban-block-explorer`. `loadEnv(mode, root, '')` reads it in the Node
   config; only `VITE_`-prefixed vars reach the bundle.
3. **No third-party scripts.** The reference app carries Google Tag Manager in
   its `index.html`; that is the one thing deliberately not copied from it.

### Emerged

4. **The same-origin probe is `/config`, not `/api-tokens/api/health`.** The
   criterion named a route that does not exist: the portal backend maps exactly
   one path (`portal/mod.rs`), and [[0183]]'s gate answers an empty `404` on
   every other path under the prefix — so a `/health` probe would render a
   failure whether or not anyone implemented it, and implementing it would mean
   adding a route whose only job is to be gated. `/config` is exempt from the
   gate and answers `200` in **both** flag states, which is exactly what a
   liveness probe from the bundle needs, and it is the route [[0183]] built for
   this bundle to read.
5. **`web/portal`, not `web/`.** [[0184]]'s routing convention has several
   frontends sharing one distribution; the next one should not have to move this
   one first. `web/*` joins the npm workspaces.
6. **`react-router-dom` bumped 6 → 7, Vite left at 8.** The task pins the stack
   to the explorer's. The router is a real choice — [[0195]]'s SPA fallback will
   be written against v7, and v6 warns about the exact APIs this app uses — so
   it was bumped. Vite came out of the generator at 8 against the explorer's 7
   and was left alone: the task itself anticipates drift, and what "mirror the
   explorer" buys is config shape, not version lockstep.
7. **Two `BucketDeployment`s, splitting [[0184]]'s decision 10 as it asked to be
   split.** Content-hashed assets get a year and `immutable`; the unhashed entry
   document keeps `max-age=0, must-revalidate`. The asset deployment sets
   `prune: false` deliberately — a viewer holding the previous `index.html` still
   requests the old chunk names, and deleting them the moment a new build lands
   turns an open tab into a blank page. Only the entry-document deployment
   invalidates: new hashed assets are new URLs and were never cached.
8. **Every production Makefile target builds the portal, not just the portal's.**
   `cdk` synthesizes the whole app whichever stack is named, so an unbuilt
   frontend fails `synth-production` and every per-stack deploy alike. This is
   [[0141]]'s footgun arriving for the frontend: the bundle is packaged off disk
   with no freshness check, so a stale one deploys quietly and reports success.
9. **"CI deploys on merge" withdrawn rather than built** (with Adam,
   2026-08-14). [[0184]] established there is no infrastructure deploy workflow
   in this repo at all — `ci.yml` synthesizes only, and every deploy is
   `make -C infra` run by an operator. What the criterion actually protects
   against is a hand-run `aws s3 sync` that nobody follows with an invalidation,
   and `BucketDeployment` already closes that inside `cdk deploy`. Building a
   deploy pipeline is a separate, much larger task and is not smuggled in here.
10. **`tsconfig.app.json` emits to `out-tsc/app`, not `dist`.** Vite owns `dist`
    and empties it on every build, which deleted the declarations
    `tsconfig.spec.json` references and broke `typecheck` after any `build`.

## Issues Encountered

- **The generated-types instruction rests on a premise that does not hold.** The
  task says to generate API types from `/api-docs-json` rather than
  hand-maintain them. Every portal endpoint is **deliberately absent** from that
  document — [[0184]]'s `verify-openapi-routes.mjs` fails CI if one appears in
  it, because the document describes the public data API to integrators and the
  portal describes itself to its own bundle. So there is nothing there to
  generate this app's own calls from, in this slice or in [[0186]]-[[0192]].
  Kept the mechanism (`npm run portal:api-types`, `openapi-typescript`) because
  the `/v1` types will be worth having when a page renders them; dropped the
  973-line emitted file, which nothing imported and would only drift. The one
  type the app needs, `PortalConfig`, is hand-written against `portal/mod.rs`
  and flagged as such.
- **`npm install` failed inside the generator.** `@nx/react`, `@nx/vite` and
  `@nx/web` installed with `^22.7.0` resolve to 22.7.8, which peer-conflicts
  with `@nx/eslint@22.7.0` on `@nx/jest` and aborts mid-generate, leaving a
  half-written project tree. All three are pinned exactly.
- **`typecheck` and `build` fought over `dist/`** — see decision 10. It passes
  the first time and fails the second, which is the worst version of this bug:
  it looks like a flake.
