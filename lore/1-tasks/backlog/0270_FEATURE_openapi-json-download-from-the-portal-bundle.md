---
id: "0270"
title: "The \"OpenAPI JSON\" link renders the document instead of downloading it — `download` is ignored cross-origin, so ship the spec as a static file in the portal bundle"
type: FEATURE
status: backlog
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
- [ ] Clicking "OpenAPI JSON" on `/api/docs` saves a file instead of rendering
      it, under a name that identifies the API
- [ ] The rendered reference still fetches the **live** document — what
      `ApiReference.tsx` fetches is unchanged
- [ ] The bundled copy cannot drift silently: CI fails when it differs from the
      output of `npm run openapi:extract`
- [ ] A Vitest case asserts the download href is bundle-relative (not
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

1. **D-01 — a static file in the bundle** (Adam's call, 2026-09-07). It works
   with JavaScript disabled, needs no API change, no new gateway resource and no
   infra deploy.
   *Rejected:* a `fetch` + `Blob` + `URL.createObjectURL` handler on the link.
   It avoids the second copy of the document and always serves current bytes,
   but it needs JS and only responds to a real click.
2. **D-02 — the reference page keeps fetching the live document.** The download
   copy being a build-time snapshot is acceptable for a file a reader feeds to a
   generator; the rendered reference showing stale endpoints is not.

## Notes

The document is already downloaded and inspected — see the session of
2026-09-07. Nothing here is blocked; it needs `/promote-task` and a branch.
