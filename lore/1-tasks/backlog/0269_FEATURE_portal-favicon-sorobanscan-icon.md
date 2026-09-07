---
id: "0269"
title: "Portal favicon — replace the placeholder icon at sorobanscan.rumblefish.dev/api/ with the SorobanScan icon"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0185", "0194", "0195"]
tags: [portal, frontend, branding, priority-low, effort-small]
links:
  - https://sorobanscan.rumblefish.dev/api/
history:
  - date: "2026-09-07"
    status: backlog
    who: adam-kot
    note: "Task created"
---

# Portal favicon — replace the placeholder icon at sorobanscan.rumblefish.dev/api/ with the SorobanScan icon

## Summary

The portal at `https://sorobanscan.rumblefish.dev/api/` still ships the
favicon that [[0185]] dropped into `web/portal/public/favicon.ico` when the
page was "ugly but real". The rest of the page has since been rebranded
(SorobanScan icon and wordmark in `web/portal/src/assets/`), so the browser
tab is the one thing left that does not match the product it sits inside.
Replace it with the SorobanScan icon.

## Context

- The portal is a static SPA in the block explorer's bucket, under `/api/`
  ([[0194]], [[0195]]). It is built and shipped by
  `make -C infra sync-portal-explorer`; there is no CDK stack behind it.
- `web/portal/index.html` links `/favicon.ico` root-relative on purpose:
  Vite's `base` (`/api/`) rewrites absolute public-dir URLs, so the built
  page requests `/api/favicon.ico`. A relative href would break on
  sub-routes such as `/api/docs`. Keep that shape — see the comment block
  in `index.html` and `vite.config.mts`.
- The site root `/favicon.ico` belongs to the explorer, not to us. Whatever
  we ship must live under `/api/` and must not assume the explorer's icon.
- Source of truth for the mark: `web/portal/src/assets/sorobanscan-icon.svg`
  (already used by the landing page). If the explorer repo carries a
  ready-made `.ico`/PNG set for the same mark, reuse it rather than
  re-rasterising — the two tabs should look identical.

## Implementation Plan

### Step 1: Produce the icon files

From `sorobanscan-icon.svg` (or the explorer's existing favicon set)
generate `favicon.ico` (16/32/48 px) and, optionally, `favicon.svg` +
`apple-touch-icon.png` (180 px). Put them in `web/portal/public/`. Move the
old `favicon.ico` to `.trash/`, never `rm`.

### Step 2: Wire them in `index.html`

Keep the root-relative `href="/favicon.ico"`. If an SVG icon is added, add
`<link rel="icon" type="image/svg+xml" href="/favicon.svg">` before the
`.ico` line so modern browsers prefer it. No other head changes.

### Step 3: Verify locally and in production

- `pnpm nx build portal`, then `vite preview` (the base-prefixed way of
  running the built bundle, see `vite.config.mts`) and confirm the tab icon
  on `/api/` and on a sub-route.
- Ship with `make -C infra sync-portal-explorer`. Confirm
  `https://sorobanscan.rumblefish.dev/api/favicon.ico` returns the new file
  and the tab shows the SorobanScan mark (CloudFront may cache the old one
  — invalidate or wait for TTL).

## Acceptance Criteria

- [ ] `https://sorobanscan.rumblefish.dev/api/favicon.ico` serves the
      SorobanScan icon; the browser tab on `/api/` and on a sub-route shows it
- [ ] `web/portal/public/favicon.ico` replaced; old file in `.trash/`
- [ ] `index.html` keeps the root-relative href and its explanatory comment
- [ ] Icon matches the mark used by the explorer's own tab

## Notes

Cosmetic; no API change. Cheapest path if the explorer repo already has an
`.ico`: copy it byte-for-byte.
