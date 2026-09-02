---
id: "0195"
title: "Swagger UI, and the two hosting leftovers the cutover did not close"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0183", "0124", "0126", "0161", "0184", "0185", "0163", "0164", "0194", "0235"]
tags: [layer-infra, layer-backend, priority-medium, effort-medium, milestone-M3, epic-self-service-onboarding, cloudfront, swagger-ui, dns, slice-12]
milestone: 3
links:
  - "../archive/0161_FEATURE_portal-static-hosting-s3-cloudfront.md"
  - "../archive/0194_TEST_portal-security-and-ops-audit/README.md"
  - "../archive/0235_REFACTOR_portal-prefix-api-tokens-to-api.md"
  - "../../../docs/scf/api-endpoints.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Twelfth slice, the remainder of [[0161]]. Three separate pieces of hosting
      work that [[0184]] deliberately left out because none of them is needed to
      serve a page or reach the API. Kept together because they are all edits to
      the same distribution.
  - date: 2026-09-01
    status: active
    who: akot
    note: >
      Activated and **rewritten against develop**. Two of the original three
      pieces are gone, delivered elsewhere by [[0194]]'s decision A on
      2026-08-31: the custom domain landed as
      `prices-api.sorobanscan.rumblefish.dev` (created in `ApiGatewayStack`,
      `apiBaseUrl` + OpenAPI `servers` + `PUBLIC_API_BASE_URL` all updated),
      and the per-prefix SPA fallback stopped being ours when the page moved to
      the Explorer's distribution, whose own `api-spa-routing` function does the
      rewriting. The premise the whole task rested on — "three edits to the same
      distribution" — no longer holds: the page and the backend are on two
      different hosts and neither is `PortalHostingStack`'s distribution. What
      survives is Swagger UI (still unbuilt, and it now belongs on a different
      host than planned), plus two leftovers the cutover created rather than
      closed: a live unauthenticated copy of the portal on
      `dojr4epgxo2qp.cloudfront.net`, and the Explorer's basic auth still on in
      front of the real page.
  - date: 2026-09-01
    status: active
    who: akot
    note: >
      Plan approved by Adam (four decisions, see Design Decisions) and the
      code landed on `feat/0195_swagger-ui-spa-fallback-and-custom-domain`:
      Swagger UI as a lazy React route `/docs` (`swagger-ui-react`), "Try it
      out" off, `Access-Control-Allow-Origin: *` on both copies of the spec,
      `PortalHostingStack` removed from `app.ts` with its Makefile targets,
      CI filter and the CloudFront section of `verify-openapi-routes.mjs`;
      docs, runbook, epic and overview brought into line and the path
      convention re-homed in `api-endpoints.md`. Rust 419/419 (2
      tests changed/added), portal 184/184 (+5), infra synth green,
      `openapi:verify-routes` and `verify-servers` green. What is left is the
      operator half: destroy the shadow distribution and its two buckets,
      deploy the api-handler (the CORS header), sync the bundle.
  - date: 2026-09-01
    status: active
    who: akot
    note: >
      `swagger-ui-react` lived a day. Adam saw it locally and rejected the
      result ("tragiczny, rozjeżdżający się"): a white-page stylesheet
      fought into a dark one, texts overflowing, and every parameter drawn
      as a disabled form because "Try it out" was off — which read as broken
      dropdowns. Decision: "zrób A w stylu swaggera" — an own renderer of
      the live document in the portal's design system, keeping Swagger UI's
      information architecture. Built as `web/portal/src/docs/openapi.ts`
      (model + helpers, 20 unit tests) and `ApiReference.tsx` (the page,
      9 tests), on the quick start's pieces lifted into
      `landing/DocPrimitives.tsx`. Dependency gone; the docs chunk is 19 kB
      instead of 1.29 MB. Portal 198/198, lint/typecheck/build green;
      verified in Chrome against the dev server (rail, rows, parameters,
      responses, schemas, hash links).
  - date: 2026-09-01
    status: active
    who: akot
    note: >
      Adam's audit request: every endpoint, text and description — fill the
      empty ones, verify the rest against what the code does, check the
      characters, and nothing of ours (task numbers, section marks) may be
      published. Done at the SOURCE: every `#[utoipa::path]` summary,
      description, parameter and response text and every `ToSchema` doc
      comment in `assets/{dto,handlers,queries_ch}.rs`, `backfill/*`,
      `oracles/*`, `batch/*`, `ops/mod.rs`, `common/errors.rs` and
      `openapi/mod.rs` rewritten integrator-facing, checked against the
      handlers, the ClickHouse MV and the identity parser. 29 empty
      descriptions filled (27 schema fields, 2 operations), 22 with internal
      references cleaned, 0 of either left in the served document. Rust
      419/419, Redocly lint valid, `verify-routes` green.
  - date: 2026-09-01
    status: active
    who: akot
    note: >
      ⚠️ **Corrected the same day, and the correction is the lesson.** The
      audit above rewrote the reader-facing text by editing the DOC COMMENTS
      on the DTOs and handlers — 215 comment lines replaced. Adam: the
      comments are the implementation history, they are not to be touched;
      the docs page keeps the new text. Both hold, because the two never
      had to be the same string: every doc comment was restored from HEAD
      verbatim and the published text moved into
      `src/openapi/descriptions.rs`, a `Modify` pass applied when the
      document is built. Operation summaries, parameter and response text
      stayed as `#[utoipa::path(...)]` attributes, which are published
      strings and not comments. Verified: the rebuilt document is
      TEXTUALLY IDENTICAL to the one Adam accepted (0 differing keys over
      every summary, description, parameter, response, schema and field),
      `git diff` removes no `///` line from any source file, Rust 420/420
      (+1 guard test), lint and `verify-routes` green.
---

# Swagger UI, and the two hosting leftovers the cutover did not close

## Summary

**Story:** *as a developer, the docs are a page and not a JSON blob, and there
is exactly one portal on exactly one host.*

The original task bundled three edits to one distribution. Two of them were
delivered by other work and the distribution they targeted is no longer the one
serving anything:

| Original piece | State on develop (2026-09-01) |
| --- | --- |
| Swagger UI at `/docs/*` | **Open** — but `/docs/*` is not a path we own any more; see below |
| Per-prefix SPA fallback | **Dissolved** — the page is on the Explorer's distribution, its `api-spa-routing` function (their PR #437) already rewrites every extensionless path under `/api/` to `/api/index.html`. Our own `DirectoryIndexFn` allow-list still covers the shadow distribution. Nothing to build; one thing to verify once basic auth is off |
| Custom domain + certificate | **Done ([[0194]], 2026-08-31)** — `prices-api.sorobanscan.rumblefish.dev`, REGIONAL, DNS-validated cert created in `ApiGatewayStack`, A + AAAA in `Z10396861CRMUIWWA8TL9`, base path = root. `apiBaseUrl`, the OpenAPI `servers` block, `PUBLIC_API_BASE_URL` and the Discord redirect URI all name it |

What is actually left is four items, and two of them did not exist when this
task was written.

## Context — what changed under this task

[[0235]] moved the prefix `/api-tokens` → `/api`; [[0194]]'s **decision A**
(2026-08-31, Adam) then moved the *page* to the Explorer's bucket and
distribution and the *backend* to our own API hostname:

- **page** — `https://sorobanscan.rumblefish.dev/api/`, served from
  `s3://production-soroban-explorer-api-spa/api/` through Explorer
  distribution `EA2TLS5SS5M87`, behaviour `/api/*`, static SPA
- **backend + spec** — `https://prices-api.sorobanscan.rumblefish.dev`,
  called **cross-origin, same-site**, no CloudFront in front of it
- **our distribution** — `dojr4epgxo2qp.cloudfront.net`
  (`PortalHostingStack`) is still deployed and still serving

So `/docs/*` on "the same distribution" would put the docs on a host nobody
visits and would need a root-level behaviour on a distribution whose root is
the block explorer's. The docs have to go somewhere else, which is the one
genuine design decision this task still carries.

**Measured 2026-09-01** (plain `curl`, no credentials):

```
https://sorobanscan.rumblefish.dev/api/            401   (Explorer basic auth)
https://sorobanscan.rumblefish.dev/api/dashboard   401
https://prices-api.sorobanscan.rumblefish.dev/api-docs-json   200 application/json
https://prices-api.sorobanscan.rumblefish.dev/api/config      200 {"enabled":true,...}
https://dojr4epgxo2qp.cloudfront.net/api/          200 text/html      ← the shadow portal
https://dojr4epgxo2qp.cloudfront.net/api/config    200 {"enabled":true,"rate_limit_per_second":1}
https://dojr4epgxo2qp.cloudfront.net/api/dashboard 200 text/html      ← deep link already works there
https://dojr4epgxo2qp.cloudfront.net/api/auth/login 303
https://dojr4epgxo2qp.cloudfront.net/docs/         403
```

Also measured: `GET /api-docs-json` with `Origin: https://sorobanscan.rumblefish.dev`
returns **no `Access-Control-Allow-Origin`** — the gateway's CORS answer covers
the portal routes, not the spec route.

## Implementation

### 1. Swagger UI — decide the host first

`/docs/*` as originally written is not available. Three options, in the order
they were considered:

- **(a) Served by the API itself at `https://prices-api…/docs`** —
  *recommended.* A small HTML page from the axum handler (or an S3-free
  static asset embedded in the binary) whose spec URL is the **same-origin**
  `/api-docs-json`. No CloudFront, no second bucket, no Explorer-side change,
  and no CORS: the measurement above says a cross-origin fetch of the spec
  would fail today and adding `Access-Control-Allow-Origin` to an anonymous,
  gateway-cached document is a change worth avoiding. `links.ts` already
  says `API_REFERENCE` is "the one constant to re-point".
- **(b) A route in the portal bundle** (`/api/docs`, served from the
  Explorer's bucket). Nicer navigation, but the page then fetches the spec
  cross-origin — so it requires (a)'s CORS change anyway, on the one route
  that is anonymous and cached for an hour.
- **(c) `/docs/*` on `dojr4epgxo2qp`.** Only viable if item 3 keeps that
  distribution, and it publishes docs on a hostname nothing else advertises.

Whichever is chosen: it is a **viewer pointed at the live `/api-docs-json`**
([[0124]]), never a checked-in copy of the spec — the drift 0124 spent a task
preventing. Re-point `API_REFERENCE` in `web/portal/src/landing/links.ts`
(and its comment, which names this task) in the same change.

### 2. The shadow portal on `dojr4epgxo2qp.cloudfront.net`

`PortalHostingStack` is still in `infra/src/lib/app.ts` and still deployed. Its
distribution serves a **complete, working, unauthenticated copy of the portal**
— `/api/config` answers `{"enabled":true}`, the bundle loads, deep links
resolve, `/api/auth/login` 303s — while the host the task signed off on is
behind the Explorer's basic auth. The portal is "closed" on one host and open on
the other. (Sign-in on the shadow copy does not complete: the Discord redirect
URI names the API host, so the callback lands elsewhere. Key issue and reveal
would work for anyone who gets a session by other means.)

Decide, and write the decision down:

- **retire** — drop the stack from `app.ts`, delete the distribution and the
  bundle bucket. The `/api/*` origin it fronts (execute-api) stays reachable;
  only the second front door goes. This is the honest end state of decision A;
- or **keep** it as a fallback front door and put something in front of it
  (its own basic auth, or `PORTAL_ENABLED`-style gating) so there is not an
  ungated portal on the internet.

Whichever way it goes, `docs/scf/api-endpoints.md`'s CloudFront section and
`portal-hosting-stack.ts`'s header comment describe a distribution that is
either gone or demoted, and must say which.

### 3. The Explorer's basic auth

`enableApiSpaBasicAuth: false` in the `soroban-block-explorer` repo's
`production.json` — the single item [[0194]] left open. It gates *public
availability* of the page, not correctness. Sequence it **after** item 2: with
both hosts open at once there are two live portals, one of them on a URL nobody
audited.

Preconditions that belong to this hand-off and are not ours: the gating guild
SSM parameter is still the **test guild** ([[0194]] Design Decision 1, a
`put-parameter --overwrite` away) and must move before the portal is advertised.

### 4. Docs and the path convention

- `docs/scf/api-endpoints.md` — three stale passages: the CloudFront section
  still says "until the custom domain lands (task 0195) [the documented base]
  is still the execute-api URL"; the OpenAPI section's `GET` line and `curl`
  example still name `02mabge71l.execute-api…`; the Pending list still says
  Swagger UI "lands with the custom domain and the per-prefix SPA fallback".
- **The path convention.** [[0161]]'s `<app>/*` + `<app>/api/*` was superseded
  on 2026-08-31 by the flat `/api/` ([[0235]]'s closing note records that the
  `/api/api/` layout lived three days). The current rule — bundle paths are a
  short fixed carve-out list, the backend is the catch-all, carve-outs first —
  is written in `portal-hosting-stack.ts`'s header. If item 2 retires that
  file, **the convention must be re-homed** into `docs/scf/api-endpoints.md`
  before it is deleted, together with the fact that on a shared host the root
  is not ours. `lore/` records are not rewritten ([[0235]]'s rule).

## Acceptance Criteria

- [x] Swagger UI renders the **live** spec from `/api-docs-json`, not a
      checked-in copy, and its host is a recorded decision — a React route of
      the portal (decision 1 below; neither a/b/c as first listed, see
      Emerged)
- [x] `API_REFERENCE` in `links.ts` points at that page, and its comment no
      longer says the page does not exist
- [ ] The shadow portal on `dojr4epgxo2qp.cloudfront.net` is gone —
      `curl https://dojr4epgxo2qp.cloudfront.net/api/` does not return a
      working portal to an anonymous caller (**operator step, pending**: the
      code no longer declares the stack; the deployed one is destroyed by
      hand, see Operator steps)
- [x] The decision on `PortalHostingStack` (retire) is written down, with what
      happens to the bundle bucket and the execute-api origin — decision 3 and
      `docs/scf/api-endpoints.md`
- [x] The path convention lives somewhere that survives the stack's removal
      and names the flat `/api/` layout — `docs/scf/api-endpoints.md`, "The
      path convention"
- [x] `docs/scf/api-endpoints.md` names `prices-api.sorobanscan.rumblefish.dev`
      everywhere it names a base, including the OpenAPI section's examples
- [x] The published document describes every operation, parameter, response
      and schema field, correctly and for an integrator — no empty
      descriptions, no internal references (Adam's audit, 2026-09-01)
- [ ] After basic auth is off: a refresh on `https://sorobanscan.rumblefish.dev/api/dashboard`
      and on `/api/docs` returns the **portal's** `index.html` — verifiable
      behind the credentials already; measured in the operator steps
- [ ] ~~`enableApiSpaBasicAuth: false` is requested/landed in the Explorer
      repo~~ — **not this task's, by Adam's decision (2026-09-01)**: the
      switch stays on, and when to flip it is not ours to decide. Recorded,
      not blocking
- [x] Everything that is ours is in CDK (Tranche 3 AC 7) — and what is not
      ours any more is not in CDK either

### Dropped from this task (delivered elsewhere)

- ~~Custom domain + `us-east-1` certificate~~ — landed as a **REGIONAL**
  domain on the REST API ([[0194]]); `us-east-1` was a CloudFront requirement
  and CloudFront is no longer in the path. Cert lives in the API's region.
- ~~Re-point the Discord redirect URI at the cutover ([[0186]])~~ — done
  2026-08-31, it names the API host and sign-in was walked end to end.
- ~~A CloudFront Function for per-prefix SPA fallback~~ — the Explorer's
  function does it for the page; ours already does it for the shadow host.
- ~~`docs/scf/api-endpoints.md` + `API_BASE_URL` + `servers` agree~~ — done
  by [[0194]] except for the three stale passages in item 4.

## Implementation Notes

**Web (`web/portal`)**

- `src/docs/openapi.ts` — the slice of OpenAPI 3.1 the reference reads
  (types for `utoipa`'s dialect) and the helpers: `deref`, `typeLabel`
  (`integer (int32)`, `AssetListItem[]`, `Stream | null`), `linkedComponent`,
  `exampleOf` (examples → enums → placeholders, depth-bounded on structure
  not on reference hops), `groupByTag` (document order, undeclared tags
  last), `requiresKey` (operation override, then document default, `[{}]`
  is anonymous), `displaySummary` (drops the ``` `GET /x` — ``` prefix the
  row already shows), `statusTone`, `primaryMedia`, `apiKeyHeader`.
  `openapi.spec.ts`: 20 assertions over `openapi.fixture.ts`, one of every
  construct the handler emits.
- `src/docs/ApiReference.tsx` — the page, in Swagger UI's shape: version
  chips, "Base URL & authentication" (server + `x-api-key` strips), one
  `DocSection` per tag, one collapsible `Row` per operation (method badge,
  path, summary, lock when keyed) opening to description, a three-column
  `FieldTable` of parameters (required/location chips, enum members and
  bounds as chips under the description, types linked to their schema),
  request body with example, responses with toned status chips and a
  generated 200 example, then a Schemas section of the same rows with
  property tables. `Prose` renders the doc comments' backticks, `**bold**`
  and wrapped `*` lists; `JsonView` colours examples like the quick start.
  `useOpenApi` fetches with `accept` only (no preflight); `useOpenRows`
  opens whatever row the URL hash names, so the rail's links land on
  content. Loading and error states, "Try again", raw-JSON link.
  `ApiReference.spec.tsx`: 9 tests over a stubbed `fetch`.
- `src/landing/DocPrimitives.tsx` — `CopyButton`, `SectionTitle`,
  `DocSection`, `DocCard`, `Code`, `ValueStrip`, `Toc` (now with nested
  entries) and `DocPage` (grid, rail, glow, headline) lifted out of
  `quickstart/QuickStart.tsx`, which now imports them; the quick start's
  own tests are unchanged and green.
- `src/app/app.tsx` — `DocsRoute` at `/docs`, `React.lazy` + `Suspense`: a
  19 kB chunk, split so the landing page does not carry the reference (and
  `links`) on a first visit.
- `src/landing/links.ts` — `DOCS_ROUTE`, `API_REFERENCE` re-pointed;
  `DashboardChrome.tsx` ("OpenAPI Docs" is a `RouterLink` with
  `aria-current`), `Chrome.tsx` (footer "Documentation" is a route),
  `Documentation.tsx` (the "OpenAPI Specification" card opens the rendered
  reference). Comments in `index.html`, `api-origin.ts`, `vite.config.mts`,
  `api/portal.ts`, `app.spec.tsx` no longer describe the retired
  distribution — `api/portal.ts`'s module header only after the review caught
  that the first pass had rewritten the body and left the header standing.
- `app.spec.tsx` — two route tests (`/docs` under each bar); the open-portal
  stubs answer `/api/api-docs-json` with the fixture.
- `swagger-ui-react` and `@types/swagger-ui-react` were added and removed
  again the same day; `package.json` is as it was.

**Rust (`packages/prices-api`)**

- `src/lib.rs` — `serve_spec` adds `Access-Control-Allow-Origin: *`; both
  mounts share the closure, so the root copy and `/api/api-docs-json` carry
  it. The routes are mounted OUTSIDE `portal::cors_layer`, so nothing else
  writes a CORS header on them.
- `tests/portal.rs` — `the_cors_layer_stops_at_the_portal_prefix` modified
  (it asserted the spec had no allow header; it now asserts no
  **credentials** header outside the prefix, which is the layer's own
  fingerprint) and `the_openapi_document_is_readable_from_any_origin` added:
  `*` on both copies, with and without an `Origin`, with and without a
  configured portal origin.

**Infra**

- `src/lib/app.ts` — `PortalHostingStack` gone (`void apiGateway` keeps the
  instantiation's side effects and says why nothing imports it).
- `src/lib/stacks/portal-hosting-stack.ts` → `.trash/`.
- `Makefile` — `build-production` no longer needs the bundle;
  `build-portal`, `deploy-production-portal`, `destroy-production-portal`
  removed; the ordering note on `destroy-production-apigateway` inverted;
  `sync-portal-explorer` is the one deploy path and says so.
- `tools/scripts/verify-openapi-routes.mjs` — the CloudFront section (check
  2: behaviour order, methods, origin path, carve-outs, `DirectoryIndexFn`
  rewrite table, managed policies, log cookies) removed; the prefix check is
  two-way (Rust ↔ script) and its remedy names the places that still carry
  the prefix. Everything on the gateway, compute and spec side is unchanged.
- `.github/workflows/ci.yml` — the `portal-hosting-stack.ts` path filter and
  its comment gone.
- Comments: `api-gateway-stack.ts` (execute-api stays, 0126 decides),
  `types.ts` (`apiBaseUrl` names the custom domain), `portal/auth/mod.rs`.

**Docs**

- `docs/scf/api-endpoints.md` — the CloudFront section replaced by "Portal
  hosting — two hosts, no distribution of ours" with the path convention
  re-homed; OpenAPI section on the custom domain with a CORS bullet;
  Pending list current.
- `docs/runbooks/portal-oauth-deploy-prep.md` — prerequisites name the two
  hosts; §6 is the record of the 2026-08-31 move and the procedure for the
  next.
- `infra/README.md` (SSM row), `docs/epics/self-service-onboarding.md` (scope
  bullet, row 12), `docs/prices-api-general-overview.md` (two cells).
- `lore/` records untouched ([[0235]]'s rule).

**Rust — the published text (Adam's audit, 2026-09-01)**

⚠️ **Where the text lives, and why it is not on the types.** `utoipa`
publishes a doc comment verbatim, and the two audiences want opposite
things: a doc comment is written for the next maintainer and carries the
implementation history — which task moved a column, which measurement
settled a classification, which defect a guard exists for — while the
document is read by an integrator who has none of that context and to whom
our task numbers and section marks mean nothing. The first pass rewrote the
comments and destroyed the first reader's record; Adam stopped it. The text
now lives in **`src/openapi/descriptions.rs`** (a `Modify` pass over the
built document, so `bin/extract_openapi` gets it too) and **every doc
comment is back at its HEAD content, byte for byte**. Operation summaries
and parameter/response descriptions need no table — they were already
`#[utoipa::path(...)]` attribute strings, i.e. published text rather than
commentary, and are edited in place.

The content below is what that table and those attributes say. Each line was
checked against the code that produces the value:

- `openapi/mod.rs` — `info.description`, the five tag descriptions, the
  `/api-docs-json` summary and responses.
- `ops/mod.rs` — `/health` says what body it returns and what it does NOT
  claim (data freshness).
- `assets/handlers.rs` — descriptions for all four asset/price operations
  (two were empty), every parameter (identifier forms spelled out; `search`
  is case-sensitive and misses unresolved Soroban codes; `granularity`
  defaults per timeframe and the 5000-candle rule; `start`/`end` forms;
  `min_volume_usd` semantics incl. the 0…1e15 bound), every response with
  its error `code`; 401 vs 403 distinguished (in-app gate vs API gateway).
- `assets/dto.rs` — `PriceResponse`, `AssetDetail`, `AssetListItem`,
  `AssetListResponse`, `Candle`, `OhlcvResponse`: every field described,
  including the 27 that had nothing (`open`/`high`/`low`/`close`/`vwap`,
  the listing's identity and change columns, `data`/`has_more`, the
  backfill ledgers…). Facts verified: `asset_type` `classic` includes the
  native asset (`contract_address = ''`); `change_7d_pct` baseline is the
  7-to-5-day band; `sources` keys are venue names with `{price,
  volume_24h}`; candles ascend; batch results follow request order and
  answer duplicates; `method`/`derived` are `null` in XLM mode; the peg
  path sets `derived = true` on every field; `asset_code` is empty for
  unresolved Soroban assets (0210, stated honestly rather than hidden).
- `assets/queries_ch.rs` — the six enum docs (`SortCol`, `Order`,
  `TypeFilter`, `BaseCurrency`, `Granularity`, `Timeframe`).
- `backfill/*`, `oracles/*`, `batch/*`, `common/errors.rs` — likewise;
  `ErrorEnvelope.code` now lists the seven codes.
- Characters: `C…`/`G…` with a real ellipsis, `→`/`−`/`×` in the two
  formulas, em dashes throughout, straight quotes inside code spans, no
  emoji, no `⚠️`. Nothing internal: no task/ADR/PR numbers, no `§`, no
  "overview", no ClickHouse/MV/serde/utoipa vocabulary. A scan over the
  served document (`spec-dump2.md` in the scratchpad) reports 0 hits and 0
  empty descriptions; the renderer's `scrubInternal` stays as the safety
  net.

**Verification**: Rust 420/420 (`cargo test -p prices-api`); portal lint,
typecheck, 200/200 tests, build; infra lint, typecheck, build; `make
synth-production` (five stacks); `openapi:verify-routes` (9 routes + 3 portal
skipped); `openapi:verify-servers`; `cargo fmt --check`; prettier on every
touched file. In Chrome against `nx dev portal`: the rail, every row, the
parameter and property tables, the examples, and a rail link opening the
row it names. Not verified visually at 375 px — the window manager here
ignores resizes; the layout is the quick start's, which task 0193 measured
there, and the tables collapse to one column below `sm`.

## Design Decisions

### From Plan

1. **The reference is a route of the portal (`/docs`), rendered by our own
   code in Swagger UI's shape — not `swagger-ui-dist` and, after one day,
   not `swagger-ui-react` either.** Three steps, each Adam's: the boss's
   link suggested the standalone static bundle; Adam preferred a React
   component so it could sit in the portal's chrome; then he saw
   `swagger-ui-react` running and rejected it — its stylesheet is written
   for a white page, its layout does not bend, and with "Try it out" off it
   renders every parameter as a disabled form that reads as a broken
   dropdown. "Zrób A w stylu swaggera": an own renderer of the live
   document, keeping Swagger's information architecture (tags, collapsible
   operations, parameters, responses, schemas at the foot) in the design
   system the quick start was built in. A route rather than a folder for
   the same reason as before: the explorer's routing function rewrites
   every extensionless path under `/api/` to the SPA, so `/api/docs` reaches
   a route and a static folder only `/api/docs/index.html`. The cost of
   owning the renderer is `openapi.ts` — ~300 lines written for `utoipa`'s
   dialect and labelled `any` where it meets something else — against a
   dependency that could not be styled into the page.
2. **Nothing on the page sends a request** (Swagger's "Try it out" has no
   counterpart here). `/v1` answers no CORS (measured: `OPTIONS /v1/assets`
   → 403), so a "try it" would fail on preflight and read as a broken API;
   examples are generated from the schemas instead. Adding a request runner
   is [[0126]]'s call once `/v1` answers CORS — Oskar activated 0126 the
   same day; the two tasks touch the same gateway and are reconciled by
   scope: 0126 owns `/v1`, this task owns the spec route.
3. **`PortalHostingStack` is destroyed and removed, both buckets deleted.**
   Adam: "ma być tylko jeden portal". The distribution served a complete,
   ungated copy of the portal (`/api/config` → `{"enabled":true}`, deep
   links, `/api/auth/login` → 303) while the real page sat behind basic
   auth. Both buckets are `RETAIN` and are removed by hand after the stack
   (Adam: both at once, logs included). The execute-api origin stays
   reachable; [[0126]] decides its retirement.
4. **`Access-Control-Allow-Origin: *` on `GET /api-docs-json`**, not the
   portal's origin. Anonymous, public, no cookie, no key — an origin
   restriction protects nothing and costs every partner's browser-side
   tooling. The constant value is what makes it safe under the gateway's
   3600 s stage cache; a reflected origin there would be served to the next
   caller. It is the one `*` on the API; the portal's routes keep one
   credentialed origin.
5. **The explorer's basic auth is not touched.** Adam: it stays, and the
   decision when to flip it is not his. Recorded in the task and the docs as
   what gates public availability; not an AC.

### Emerged

6. **The modified Rust test asserts the credentials header, not the allow
   header.** `the_cors_layer_stops_at_the_portal_prefix` used to prove the
   layer's reach by the absence of `Access-Control-Allow-Origin` on
   `/api-docs-json`; decision 4 puts that header there from the handler. The
   layer's own fingerprint is `Access-Control-Allow-Credentials`, which only
   it writes, so the test now checks that on `/health` and both spec copies,
   and the allow-header absence only on `/health`. Not a regression —
   the property under test (the layer stops at the prefix) is unchanged.
7. **The "OpenAPI Specification" landing card opens the rendered reference,
   not the raw JSON.** Its copy promises "Full Swagger UI included"; the raw
   document is linked from the reference page's header instead. The four
   cards that were placeholders for the quick start's sections still open the
   reference — [[0163]]'s to re-point.
8. **`Documentation`/`Chrome` links became `RouterLink`s** ("OpenAPI Docs" in
   the signed-in bar, "Documentation" in the footer): an in-app route as a
   bare `href` reloads the bundle, and the footer already had the basename
   argument written down for the dashboard link.
9. **The quick start's page pieces became a shared module** rather than
   being copied: `DocPrimitives.tsx` holds what task 0193 built off the
   Figma frame, and `QuickStart.tsx` imports it. The only change on the way
   out is that section ids are strings and `Toc`/`DocPage` take their
   sections as a prop — a second interpretation of the frame is how two
   pages of one portal end up two pixels apart everywhere.
10. **Examples are generated, and say `"string"` where the document gives
    no example.** `utoipa` carries `example` on a handful of fields; the
    rest is placeholders by type. Real sample values belong in the Rust
    schema attributes (where the quick start's samples came from by hand),
    not in the renderer — a follow-up for whoever owns the response
    shapes, noted rather than spawned.
11. **`void apiGateway;` in `app.ts`** rather than dropping the binding: the
    line documents that nothing imports the stack's exports any more, which
    is the fact that unblocks `destroy-production-apigateway`.
12. **The rail's operation entries open their row.** A `#op-…` or
    `#schema-…` hash opens that row on load and on `hashchange`, so a link
    from the rail (or a pasted URL) lands on content rather than on a
    collapsed header — Swagger UI does the same with `#/tag/operationId`.
13. **Vite's 500 kB chunk warning** fires on the main bundle (590 kB) and
    is left firing: the reference is its own 19 kB chunk, and the size is
    the landing page's, not this task's.
14. **Descriptions are scrubbed of the project's bookkeeping before they
    render** (`scrubInternal`): task, ADR and PR numbers, `§` marks into the
    internal overview. Adam, on seeing "(task 0135)" and "general-overview
    §3.3 / §4.2" on the page: that is our knowledge, not to be published.
    The scrub is the renderer's safety net; the SOURCE is the handler's doc
    comments, which `utoipa` copies into `/api-docs-json` verbatim — 22
    descriptions in the served document carry such references today, and
    that document is public on its own. Cleaned at the source the same
    day, at Adam's request, as part of a full audit of the published text
    (see Implementation Notes, "Rust — the published text").

## Issues Encountered

- **`swagger-ui-react`, one day.** Wired, themed under `.swagger-ui`, the
  disabled parameter forms restyled — and still wrong: overflowing labels
  in the accordion headers, a layout that does not bend, forms that cannot
  be used. Replaced by the renderer (decision 1); the theme work was
  discarded with it.
- **`exampleOf` bottomed out one object too early**: the depth bound
  counted reference hops as nesting, so `AssetListResponse → data[] →
  AssetListItem → stream → Stream` rendered `stream: null`. The bound now
  counts properties and items only.
- **`oneOf: [{type: "null"}, {$ref}]`** — the first pass never recognised
  the `null` half (`splitType` strips `"null"` before the comparison), so
  optional references labelled as `any | null | Stream`. `isNullSchema`
  now decides.
- **Grid rules at three heights**: the field tables put the row rule on
  each cell, and `alignItems: 'start'` let a short cell draw its rule above
  the row's end — Adam's first screenshot. Cells stretch to the row now.
- **jsdom's rail**: the placeholder page has an empty rail of its own, and
  `findByRole('navigation')` resolved to it before the document arrived;
  the test waits for a heading first. Clearing `location.hash` in
  `afterEach` is a "navigation" to jsdom and throws; `history.replaceState`
  instead. (The two `Not implemented: navigation` lines in the portal's
  test output predate this task — `app.spec.tsx`, the sign-in link tests.)
- **`import/first`** — the lazy `import()` declaration first sat between two
  import groups in `app.tsx`; moved below the last import.
- **`DashboardNavbar`'s link map** destructured `href` from a union that no
  longer had one once all three links became routes; the map now takes `to`
  only.
- **A stale `Prices-production-PortalHosting.template.json`** lingered in
  `infra/cdk.out` from an earlier synth (cdk does not clean the directory);
  harmless to the checks, moved to `.trash/`.
- **`origin/develop` moved under the task**: [[0126]] was activated and
  rewritten by Oskar the same morning. Read before branching; no overlap —
  it owns `/v1` CORS and the execute-api retirement, this task the spec
  route and the distribution.

## Operator steps (not in any diff)

In this order, and **the destroy before the merge** — after the code is on
`develop` there is no stack definition left to destroy with:

1. `cdk destroy Prices-production-PortalHosting` from a checkout that still
   has the stack (a worktree at `origin/develop`), then delete the two
   `RETAIN`ed buckets by hand (bundle + access logs: empty, then remove).
   Verify: `curl -s -o /dev/null -w '%{http_code}' https://dojr4epgxo2qp.cloudfront.net/api/`
   is not `200` (CloudFront answers `404`/`403` for a deleted distribution
   once propagated), and the SSM parameter
   `/prices/production/portal-distribution-domain` is gone.
2. `make -C infra deploy-production-compute` — the api-handler with the CORS
   header; the target flushes the stage cache. Verify:
   `curl -sI -H 'Origin: https://x.example' https://prices-api.sorobanscan.rumblefish.dev/api-docs-json | grep -i access-control-allow-origin`
   → `*`.
3. `make -C infra sync-portal-explorer` — the bundle with the `/docs` route.
   Verify (with the explorer's staging credentials):
   `https://sorobanscan.rumblefish.dev/api/docs` renders the reference and the
   spec loads.
4. Discord application: remove the stale redirect URI naming
   `dojr4epgxo2qp.cloudfront.net`, if one is still registered.

## Notes

- Cost: nothing new. The certificate is ACM's, the hosted zone already exists
  and is the Explorer's. Retiring `PortalHostingStack` *removes* a
  distribution.
- [[0163]] and [[0164]] cite the documented base URL; it is now
  `prices-api.sorobanscan.rumblefish.dev` and stable, so neither is blocked on
  this task any more.
- The wildcard `*.sorobanscan.rumblefish.dev` cert in eu-central-1 is attached
  to nothing and ineligible for renewal (expires 2026-12-06). Not ours, not
  this task's, worth telling the Explorer team.
