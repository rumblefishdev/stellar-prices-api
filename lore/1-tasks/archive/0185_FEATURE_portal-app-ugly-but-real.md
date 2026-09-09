---
id: "0185"
title: "Portal app — ugly but real: Vite/React skeleton served from /api-tokens/, built by CI"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0183", "0162", "0184", "0186", "0193"]
tags: [layer-frontend, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, ui, vite, react, slice-2]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0162_FEATURE_portal-frontend-app.md"
history:
  - date: "2026-08-13"
    status: backlog
    who: akot
    note: >
      Second slice, carved out of [[0162]]. Explicitly the "working but ugly
      frontend" step: the app exists, deploys and routes, and looks like
      nothing. Styling is [[0193]]; every screen with content attaches to the
      backend slice that gives it something to show.
  - date: "2026-08-14"
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
  - date: "2026-08-17"
    status: active
    who: akot
    note: >
      Two review rounds on #218, both closed. Round 2 reported five findings, all
      accurate, but against a remote head three commits stale — three were
      already fixed. Live: the two `BucketDeployment`s had no `DependsOn` (taken
      back from [[0205]], decision 14) and `generated.ts` was `.prettierignore`d
      but not `.gitignore`d. Also fixed two things review missed: `index.html`
      shipped its comments to a public page (decision 15) and the config probe
      had no timeout (decision 16). 18 tests across 5 files, up from 4.
  - date: "2026-08-18"
    status: completed
    who: akot
    note: >
      COMPLETED via PR #218 (merged to `develop`). Six of the seven acceptance
      criteria met; the seventh — CI deploys on merge — was withdrawn with Adam
      on 2026-08-14 (decision 9), so the upload stays manual until [[0205]]. 18
      tests across 5 files, and CI runs them (decision 11). Live verification of
      the served bundle waits on [[0205]]'s deploys; styling is [[0193]], the
      sub-route refresh [[0195]]. [[0186]] landed on top of this slice and is
      archived alongside it.
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
- [x] `nx build` produces a static bundle; `nx test` runs and passes — 18 tests
      across 5 files, and **CI runs them** (see decision 11; it did not at first)
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

- `web/portal/src/base-path.ts` — `BASE_PATH` and `ROUTER_BASENAME` **derived**
  from it, so the pair cannot drift apart by hand.
- `web/portal/vite.config.mts` — `base`, the dev proxy (`server` **and**
  `preview`), and the `stripHtmlComments` build plugin.
- `web/portal/src/main.tsx` — `basename` on the router, the other half of `base`.
- `web/portal/src/api/portal.ts` — the one backend call, relative by
  construction, with a 10s probe timeout.
- `web/portal/src/app/app.tsx` — the page: three probe states, no button.
- Tests, 18 across 5 files: `app/app.spec.tsx` (9), `base-path.spec.ts` (3),
  `strip-html-comments.spec.ts` (3), `dev-proxy.spec.ts` (2), `main.spec.tsx`
  (1). The last one mounts the real entry point at the real URL, which is the
  only test that fails if `basename` is dropped.
- `infra/src/lib/stacks/portal-hosting-stack.ts` — `PORTAL_ASSET_DIR` repointed
  from [[0184]]'s placeholder to `../web/portal/dist`, the single
  `BucketDeployment` split in two, and a `DependsOn` between them.
- `infra/Makefile` — `build-portal`, and a `build-production` every production
  target now hangs off.
- `.github/workflows/ci.yml` — `web/**` in the `typescript` paths filter, and
  `test` in the target list that filter feeds.

Verified: `build`, `lint`, `typecheck`, `test` and `nx format:check --all` pass
for `portal` and for the CDK app; `make -C infra synth-production` succeeds
**from a tree with no `web/portal/dist`**, which is the property the Makefile
change exists to guarantee. Read off the synthesized template rather than the
source: both deployments carry the intended filters, cache headers and
invalidation paths, and `PortalBundle` now carries
`DependsOn: [PortalBundleAssetsAwsCliLayer…, PortalBundleAssetsCustomResource…]`.
Read off `dist/` rather than the config: `index.html` references
`/api-tokens/assets/…` and `/api-tokens/favicon.ico`, carries no comments, and
the bundle greps clean for `x-api-key`, `secret` and any external host.

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
   invalidates: new hashed assets are new URLs and were never cached. **Splitting
   them introduced an ordering hazard this decision did not see** — see 13 and
   14.
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

### Emerged from review

Two review rounds on PR #218. Everything below was found by review, not by the
tests — worth noting, because the suite was green throughout.

11. **CI runs `test`, and the suite now covers `base`/`basename`.** The
    `typescript` job ran `lint build typecheck` with no `test`, so the moment the
    PR was open the portal's suite guarded nothing: `portal` is the only project
    with a `test` target, and the pre-push hook that does run it is skippable.
    Worse, what the suite covered was not what this slice is *for* — `app.spec`
    mounted a `MemoryRouter` with no basename and `main.tsx` had no coverage at
    all, so dropping either half of the base path left every test green and the
    deployed app blank. `ROUTER_BASENAME` is now derived from `BASE_PATH`,
    `main.spec.tsx` mounts the real entry point at the real URL, and
    `base-path.spec.ts` loads the actual Vite config so the config's duplicate
    copy of `BASE_PATH` cannot drift. Both guards were mutation-tested.
12. **The dev proxy is on `preview` too.** `vite preview` is the only local way
    to run the built bundle, so it is the closest thing to a production
    rehearsal — and without a proxy it could only ever render "could not reach
    the portal backend", which is the one branch a rehearsal must not be stuck
    in.
13. **`distributionPaths` enumerates the unhashed objects instead of
    `/api-tokens/*`.** The wildcard purged the year-cached hashed assets on every
    deploy — the exact cost decision 7's comment claimed to avoid, avoided in the
    upload only. The list is `/api-tokens/`, `/api-tokens/index.html`,
    `/api-tokens/favicon.ico`, which is exactly the unhashed half of `dist/`.
    `/api-tokens/` is belt and braces: `DirectoryIndexFn` rewrites it on
    VIEWER_REQUEST, *ahead* of the cache lookup, so on the documented behaviour
    it never holds an entry — kept because an extra path is free and its absence
    would be invisible. [[0205]] can drop it once a live deploy confirms this.
14. **`portalBundle.node.addDependency(portalBundleAssets)` — kept here, not
    deferred to [[0205]].** `BucketDeployment` creates no implicit ordering
    between siblings (verified: `DependsOn` was `null` in the synthesized
    template), so CloudFormation ran the two concurrently. If the entry document
    won, CloudFront served a `max-age=0` `index.html` — which every viewer
    refetches at once — pointing at hashed assets not yet in the bucket, and the
    bucket grants `s3:GetObject` without `s3:ListBucket`, so the miss is a `403`
    and the app fails on its own JavaScript. The invalidation fires inside that
    window. It was briefly assigned to [[0205]] on the argument that the race can
    only bite on a deploy; taken back because **this task introduces the two
    deployments**, the fix is one line in a file this task already edits, and
    [[0205]] sits in `backlog/` — anyone deploying in between would have worn it.
15. **HTML comments are stripped from `index.html` at build.** Vite ships them
    verbatim, and this entry document was mostly commentary: which task puts a
    credential on the page, which sub-routes break before [[0195]], how S3
    answers a missing key. None of it is secret and all of it is free
    reconnaissance on a **public** page. Build-only, so the source keeps every
    word and `nx dev` still shows it; `vite preview` sees the stripped document
    like production does. 1.78 kB → 0.55 kB.
16. **A 10s timeout on the config probe, matched by `name` rather than
    `instanceof`.** `fetch` has no default timeout and nothing else bounds a
    connection that never reaches an origin, so a stalled handshake left the page
    on "Checking whether the portal is open…" forever — the spinner the failure
    branch exists to avoid. `AbortSignal.timeout` rejects with a platform
    `DOMException`, and an `instanceof` against it is a same-realm test: false
    across an iframe, and false under jsdom. Matched on `name === 'TimeoutError'`
    instead.

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
- **`instanceof Error` is false across realms.** The first cut of decision 16
  guarded the timeout branch with `error instanceof Error && error.name === …`.
  Under jsdom the `DOMException` the environment provides does not descend from
  the test realm's `Error`, so every timeout fell through to the generic
  "could not be reached". The test caught it, which is the point — the same
  fragility is real in a browser iframe, it is just not visible there.
- **A review can be correct and still be stale.** The round-2 review of PR #218
  reported five findings, all accurate — against `origin/…`, which was three
  commits behind the local branch, so three of them had already been closed by
  round 1. Nothing was wrong with the review. **Push before asking for one.**

## Future Work

Nothing new spawned — every follow-up this slice leaves behind already has a
task:

- The withdrawn deploy criterion, and live verification that `/api-tokens/`
  serves the bundle from CloudFront → [[0205]].
- A hard refresh on a sub-route (`/api-tokens/dashboard`), which needs the
  per-prefix SPA fallback → [[0195]].
- Styling. The page is deliberately unstyled → [[0193]].
- Somewhere for the app to render content → [[0186]] (landed), then [[0187]].
