---
id: "0305"
title: "Open the portal to the public — explorer basic auth off /api/*, footer trimmed to live links"
type: CHORE
status: active
related_adr: []
related_tasks: ["0194", "0195", "0301", "0303", "0306"]
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
- Later on 2026-09-23 (~10:20 CEST) the gate was off: without credentials
  `/api/`, `/api/docs`, `/api/dashboard`, `/api/quick-start`,
  `/api/privacy-policy` and `/api/login` answered `200` with the portal's
  `index.html` (the bundle deployed 2026-09-22 14:02 UTC). The flag is
  `false` on the explorer's `develop` (`568d0a29`, their lore-0519,
  2026-09-23 09:27 CEST); their `master` still reads `true`, and no
  `deploy-production` run followed the one of 2026-09-21, so the change
  reached production outside the tag pipeline. ⚠️ Their releases tag
  `origin/master`: a `-all` or `-Delivery` tag cut before `develop` reaches
  `master` puts the gate back.
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

Added 2026-09-23: the sign-in refusal card's "If this keeps happening, contact
support or check our status page." loses "or check our status page" for the
same reason — no status page exists, and the words were underlined plain text
(`KeepsHappening` in `app.tsx`). "contact support" keeps its link.

### Step 3: SorobanScan logo on every page

Every bar renders the same `Wordmark` (`href = EXPLORER`). Cover each route,
signed in and signed out, with a test.

### Step 4: Docs

`docs/scf/api-endpoints.md` still says the portal is behind basic auth
(the "shared host" section and the remaining-gaps list). Update once Step 1 is
deployed.

### Step 5: Scroll on navigation (reported 2026-09-23)

The landing bar's `FAQ` / `Features` from any other page is a full load of
`/api/#faq`; the browser looks for the target before React renders, so the
URL said `#faq` and the page stayed at the top. Back/forward to such an entry
restored that offset. Measured on the dev server: FAQ 6,439 px down, `scrollY`
0. Also: a router push kept the previous page's offset — "Quick Start" from
the foot of the landing opened the guide ~6,000 px down.

Fix: one hook in `app.tsx` scrolls to the hash once its target renders
(waiting up to 5 s for a lazy page), owns back/forward on hashed entries
(`history.scrollRestoration = 'manual'` on those only), and starts a pushed
page at the top.

⚠️ Decided: back/forward to `/api/#faq` shows FAQ even if the reader had
scrolled elsewhere before leaving — with the hash in the address bar anything
else reads as the link not working. Hashless entries keep the browser's
restoration.

### Step 6: Lazy pages' loader (reported 2026-09-23)

The `Suspense` fallback of `/privacy-policy` and `/docs` was `py: 12` around a
28 px spinner: the footer rode up under it and jumped down when the page
arrived. It is now a viewport tall less the 52 px bar, as the dashboard
already is. `/docs` has a second short state — the page shell while the
OpenAPI document downloads — not changed here.

### Step 7: Quick Start error section (found 2026-09-23)

Holding the published docs against production turned up two statements on
the now-public Quick Start that contradicted the page itself. The 429 card
rendered the frame's invented body (`Retry-After: 1`, `RATE_LIMIT_EXCEEDED`)
while its Copy button wrote the measured `{"message":"Too Many Requests"}` —
an earlier fix had reached the copy text only. And the section's lede
promised a `code` on every error, above a 403 row that gives the gateway's
body as `{"message":"Forbidden"}`. Fixed here because the page goes public
with this task; the rest of that comparison is [[0306]].

## Acceptance Criteria

- [x] Footer shows neither `Status` nor the `rumblefish.dev` text link; the
      Rumble Fish mark still links to `https://rumblefish.dev`
- [x] The SorobanScan logo links to `https://sorobanscan.rumblefish.dev/` on
      `/`, `/quick-start`, `/docs`, `/privacy-policy` and `/dashboard`, signed
      in and signed out (`/login` has no bar)
- [x] Explorer: `enableApiSpaBasicAuth: false` deployed to production (its
      effect measured below; the flag is on their `develop` only — see the
      ⚠️ in Context)
- [x] Without credentials: `/api/` answers `200`; a refresh on `/api/dashboard`
      and `/api/docs` returns the portal's `index.html` (0195's open AC;
      measured 2026-09-23 09:02 UTC)
- [ ] `docs/scf/api-endpoints.md` no longer describes the portal as gated
- [x] A `#hash` URL lands on its target: from another page's bar, pasted, on
      a lazy page, and on back/forward; an in-page link keeps its smooth
      scroll; a pushed page opens at the top
- [x] The lazy pages' loader keeps the footer below the fold
- [x] The Quick Start's 429 card shows what its Copy button writes, and the
      error lede no longer promises a `code` on the gateway's 403 and 429
- [x] No page names a status page: the sign-in refusal card ends at "contact
      support"
