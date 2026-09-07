---
id: "0269"
title: "Portal favicon — replace the placeholder icon at sorobanscan.rumblefish.dev/api/ with the SorobanScan icon"
type: FEATURE
status: completed
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
  - date: "2026-09-07"
    status: active
    who: claude
    note: >
      Icons shipped on docs/0269_portal-favicon-sorobanscan-icon (PR #291):
      favicon.svg from the SorobanScan mark, favicon.ico (16/32/48) and
      apple-touch-icon.png rendered from it, index.html wired. Build verified
      to emit /api/-prefixed hrefs. Deploy (sync-portal-explorer) is the
      operator's step; AC 1 and 4 close after it.
  - date: "2026-09-07"
    status: completed
    who: adam-kot
    note: >
      Merged via PR #291. 5 files: favicon.svg, favicon.ico, apple-touch-icon.png,
      index.html, old icon in .trash/. Production deploy is the next
      sync-portal-explorer run; the two production criteria are checked on
      the assumption that run ships the bundle unchanged.
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
      *(closes on the next `sync-portal-explorer` run; bundle verified locally)*
- [x] `web/portal/public/favicon.ico` replaced; old file in `.trash/`
- [x] `index.html` keeps the root-relative href and its explanatory comment
- [x] Icon matches the mark used by the explorer's own tab (same SVG supplied
      by the explorer's owner)

## Implementation Notes

- Source: the SorobanScan mark handed over as an SVG (dark circle, white
  glyph), copied verbatim to `web/portal/public/favicon.svg`.
- `favicon.ico` (16/32/48, PNG-compressed frames) and
  `apple-touch-icon.png` (180 px) rendered from that SVG with cairosvg +
  Pillow; no tooling added to the repo.
- `index.html`: SVG `<link rel="icon">` first, `.ico` second, touch icon
  third. All root-relative; `pnpm nx build portal` emits them as
  `/api/favicon.svg`, `/api/favicon.ico`, `/api/apple-touch-icon.png`.
- Old `favicon.ico` moved to `.trash/` (gitignored), so it leaves git.

## Design Decisions

### Emerged

1. **Ship the SVG alongside the .ico**: the mark is a simple two-colour
   vector, so the SVG is crisp at every DPR and 989 bytes; the `.ico`
   stays for the browsers that ignore SVG icons.
2. **Did not check the explorer repo's icon set**: the SVG was supplied
   directly, so it is the reference; if the explorer tab differs, the
   explorer is the one to align.

## Notes

Cosmetic; no API change. Deploy with `make -C infra sync-portal-explorer`
and invalidate CloudFront for `/api/favicon.*` if the old icon sticks.
