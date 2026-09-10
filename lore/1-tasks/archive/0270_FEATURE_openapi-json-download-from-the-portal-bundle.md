---
id: "0270"
title: "The \"OpenAPI JSON\" link renders the document instead of downloading it — `download` is ignored cross-origin, so ship the spec as a static file in the portal bundle"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0124", "0185", "0194", "0195", "0269"]
tags: [portal, frontend, openapi, docs, priority-low, effort-small]
links:
  - https://sorobanscan.rumblefish.dev/api/docs
  - https://prices-api.sorobanscan.rumblefish.dev/api-docs-json
history:
  - date: "2026-09-07"
    status: backlog
    who: adam-kot
    note: "Task created"
  - date: "2026-09-07"
    status: active
    who: akot
    note: "Activated; taken by akot. Approach settled: the spec ships as a static asset in the portal bundle."
  - date: "2026-09-09"
    status: completed
    who: akot
    note: >
      Merged as PR #299 (241a31e). One commit, +148/-5 over 6 files, two new
      (public/openapi.json, verify-openapi-bundled.mjs). 211 portal tests
      passing (+5). Four of five acceptance criteria met; the production URL
      is pending the next `make -C infra sync-portal-explorer`, the same sync
      task 0269 waits on. Emerged: the drift guard compares parsed JSON rather
      than bytes, and the branch needed a fast-forward to develop before
      extracting or the committed copy would have shipped stale.
---

# The "OpenAPI JSON" link should download the document, not render it

## Summary

The API reference page ends with a link to the raw OpenAPI document
(`web/portal/src/docs/ApiReference.tsx:1182`). Clicking it opens the JSON in a
browser tab. It should save a file. Adding `download` to that `<a>` does not do
it: on the shared host the href is cross-origin, and browsers ignore `download`
on a cross-origin link and navigate instead. The fix is to ship the document as
a **static asset inside the portal bundle**, served same-origin, and point a
`download` link at that copy.

## Context

- The href comes from `OPENAPI_JSON` (`web/portal/src/landing/links.ts:52`).
  Where `API_ORIGIN` is set — the shared-host build (task 0194) — it resolves to
  `https://prices-api.sorobanscan.rumblefish.dev/api-docs-json`, while the page
  itself is served from `sorobanscan.rumblefish.dev`. Different origin, so
  `download` is inert.
- ⚠️ **In dev it would appear to work.** There the href is `/api/api-docs-json`
  through the Vite proxy — same origin — so a `download` attribute tested
  locally saves a file and then silently stops doing so in production. That is
  the shape of defect to write the test against, not just the behaviour.
- The bundle already carries an artefact generated from this very document:
  `npm run portal:api-types` extracts the spec and writes
  `web/portal/src/api/generated.ts`. A committed `public/openapi.json` is the
  same convention applied to a second consumer, not a new one.
- Task 0269 is the precedent for the delivery mechanism: files in
  `web/portal/public/` ship under `/api/` on the shared host, and the build was
  verified to emit `/api/`-prefixed hrefs. The explorer rewrites only
  *extensionless* `/api/*` paths to `index.html`, so a path ending in `.json` is
  served as a file.
- The document measured from production on 2026-09-07: 34,713 bytes, version
  `0.1.0`, 9 paths, `servers` already stamped with the production base URL.

## Implementation

1. **Generate the file.** `npm run openapi:extract` (`tools/scripts/extract-openapi.sh`)
   emits `target/openapi.json` with `servers` stamped from
   `infra/envs/production.json`'s `apiBaseUrl` — the same document the deployed
   API serves. Place the result at `web/portal/public/openapi.json`.
2. **A NEW constant for the download href.** Add something like
   `OPENAPI_JSON_DOWNLOAD` to `links.ts` pointing at the bundled copy, and give
   the `<a>` a `download` value that names the API
   (`stellar-prices-api-openapi.json`), so the file is identifiable in a
   Downloads folder rather than a generic `openapi.json`.
   ⚠️ **Do not repoint `OPENAPI_JSON` itself.** That constant is also what
   `ApiReference.tsx` **fetches at runtime** to render the reference. Repointing
   it would silently turn the whole rendered reference into a build-time
   snapshot. The fetch keeps the live URL; only the download link changes.
3. **A drift guard.** A committed copy can go stale against a deployed API. CI
   already extracts `target/openapi.json` three times (`openapi:lint`,
   `openapi:verify-routes`, `openapi:verify-servers`), so a guard is one `diff`
   step against the committed file. Decide between *committed file + CI diff*
   and *generated during the portal build*; the first keeps `nx build` free of
   `cargo`, which is why it is the recommendation.

## Acceptance Criteria

- [ ] `GET https://sorobanscan.rumblefish.dev/api/openapi.json` returns the
      document as a file, same-origin with the reference page
      — **pending the next `make -C infra sync-portal-explorer`**; the file is
      in `develop`, nothing else is owed to it
- [x] Clicking "OpenAPI JSON" on `/api/docs` saves a file instead of rendering
      it, under a name that identifies the API
      (`stellar-prices-api-openapi.json`). Verified in a production-shaped
      local build, see Implementation Notes.
- [x] The rendered reference still fetches the **live** document — what
      `ApiReference.tsx` fetches is unchanged
- [x] The bundled copy cannot drift silently: CI fails when it differs from the
      output of `npm run openapi:extract`
      (`npm run openapi:verify-bundled`, step 20 of the Rust job on PR #299)
- [x] A Vitest case asserts the download href is bundle-relative (not
      `API_ORIGIN`-prefixed) and carries `download` — the assertion that would
      have caught the dev-only-works version

## Out of scope

- `Content-Disposition: attachment` on `GET /api-docs-json`. It works
  cross-origin, but it changes the public endpoint for every consumer,
  including a partner who opens the URL in a browser to read it.
- A `?download=1` query parameter on that endpoint. An undeclared query
  parameter under the gateway's 3600 s stage cache is the cross-caller bleed
  measured in task 0118.
- Any change to the API or to `infra/`.

## Design Decisions

### From Plan

1. **D-01 — a static file in the bundle** (Adam's call, 2026-09-07). It works
   with JavaScript disabled, needs no API change, no new gateway resource and no
   infra deploy.
   *Rejected:* a `fetch` + `Blob` + `URL.createObjectURL` handler on the link.
   It avoids the second copy of the document and always serves current bytes,
   but it needs JS and only responds to a real click.
2. **D-02 — the reference page keeps fetching the live document.** The download
   copy being a build-time snapshot is acceptable for a file a reader feeds to a
   generator; the rendered reference showing stale endpoints is not.

### Emerged

3. **The drift guard compares parsed JSON, not bytes.** The task said "one
   `diff` step". A byte diff holds the committed file to the exact
   serialization of whichever `serde_json` produced it, so a formatting change
   no reader can see would fail CI. `tools/scripts/verify-openapi-bundled.mjs`
   sorts keys and compares the structures instead.
4. **The filename is a constant (`OPENAPI_JSON_FILENAME`), not a literal on the
   `<a>`.** It is asserted in `links.spec.ts` alongside the href, so the two
   halves of the affordance are held in one place.
5. **The error state keeps pointing at the live document.** `ApiReference.tsx`
   has a second `OPENAPI_JSON` link — "Open the raw document", shown when the
   fetch fails. Left alone: it exists to show what the API is actually serving
   when the page cannot render it, which the bundled snapshot would not.
6. **The branch was fast-forwarded to `develop` before extracting.** It was cut
   before 0176/0263/0264 merged, so the document it would have committed was
   already behind production (`stalled`, the `current_ledger` rewording). The
   new CI gate would have failed on the first run.

**Broken/modified tests:**
- `ApiReference.spec.tsx` — "fetches the live document…" asserted the
  `OpenAPI JSON` link's href was `OPENAPI_JSON`. Changed to
  `OPENAPI_JSON_DOWNLOAD`. Intentional: that href is exactly what this task
  moves. Not a regression — the live URL is still asserted, two lines above,
  on what the page `fetch`es.

## Issues Encountered

- **`nx build` served a stale bundle when only an env var changed.** Nx does
  not key its cache on `VITE_PORTAL_API_ORIGIN`, so
  `VITE_PORTAL_API_ORIGIN=… npx nx build portal` replayed an earlier build with
  the variable empty, and the reference page rendered "`/api/api-docs-json`
  answered 404". Use `npx vite build` directly, or `--skip-nx-cache`. This will
  bite anyone rehearsing a shared-host build locally, not just this task.
- **`vite preview` does not read `.env.development`.** It runs in production
  mode, so `DEV_API_PROXY_TARGET` is not loaded and there is no dev proxy —
  every `/api/*` path 404s unless the bundle was built with `API_ORIGIN` set.
  The proxy block in `vite.config.mts` is shared by `server` and `preview`, but
  the env file that feeds it is not.
- **A preview build pointed at production shows no "Get API Key".** Production
  `/api/config` sends no `Access-Control-Allow-Origin` for a `localhost` origin,
  so the portal probe fails and `canOfferKey` is false (`app.tsx:4585`) — the
  page renders "Could not reach the portal backend". `/api-docs-json` answers
  `*`, which is why the reference page renders fine in the same build. This is
  correct behaviour, not a fault, but it makes the preview rehearsal look
  broken. For the portal UI use the dev server proxied at a local
  `cargo run --bin serve`, where everything is same-origin.
- **The local `serve` binary needs no ClickHouse for this page.**
  `PORT=8080 cargo run -p prices-api --bin serve --features local-server`
  answers `/api-docs-json` (200, 37 242 bytes) with no Docker and no `PORTAL_*`
  variables. With `PORTAL_ENABLED=true` it also needs `PORTAL_GUILD_ID` and
  `PORTAL_MIN_ACCOUNT_AGE_MINUTES` (or the `_PARAM` forms), which the module
  docs mention but the runbooks did not spell out — without them it panics at
  `serve.rs:81` with `NoSource`. Its document carries **no `servers` block**
  (`API_BASE_URL` is unset locally), so it is not comparable byte-for-byte with
  `public/openapi.json` — that is what `openapi:verify-bundled` is for.

## Implementation Notes

Merged as PR #299 (`241a31e`), one commit, +148/−5 across 6 files plus two new.

- `web/portal/public/openapi.json` — 54 024 bytes, the output of
  `npm run openapi:extract`. Semantically identical to the document production
  serves (compared against a fresh download on 2026-09-09; the two differ only
  in whitespace).
- `web/portal/src/landing/links.ts` — `OPENAPI_JSON_DOWNLOAD`
  (`${ROUTER_BASENAME}/openapi.json`) and `OPENAPI_JSON_FILENAME`.
  `OPENAPI_JSON` untouched (D-02).
- `web/portal/src/docs/ApiReference.tsx` — the lede's link gains `download` and
  the bundle-relative href.
- `tools/scripts/verify-openapi-bundled.mjs` + `npm run openapi:verify-bundled`,
  wired into `.github/workflows/ci.yml` after `openapi:verify-servers`.
- Tests: 211 passing (+5). `links.spec.ts` stubs `VITE_PORTAL_API_ORIGIN` to the
  shared-host origin and asserts `OPENAPI_JSON` goes absolute while
  `OPENAPI_JSON_DOWNLOAD` does not move.

Verified in a production-shaped local build
(`VITE_PORTAL_API_ORIGIN=https://prices-api.sorobanscan.rumblefish.dev npx vite
build` + `npx vite preview`): the page renders from the live cross-origin
document, and the link resolves to `http://localhost:4200/api/openapi.json` —
same-origin, `download="stellar-prices-api-openapi.json"`.

## Future Work

None spawned. The one open thread is the deploy itself: the file reaches
`sorobanscan.rumblefish.dev/api/openapi.json` on the next
`make -C infra sync-portal-explorer`, and the first acceptance criterion is
verified by a `curl` against that URL afterwards. Task 0269's icons are waiting
on the same sync.
