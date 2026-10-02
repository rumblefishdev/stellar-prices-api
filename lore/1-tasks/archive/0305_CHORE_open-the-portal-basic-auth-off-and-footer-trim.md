---
id: "0305"
title: "Open the portal to the public — explorer basic auth off /api/*, footer trimmed to live links"
type: CHORE
status: completed
related_adr: []
related_tasks: ["0194", "0195", "0301", "0303", "0306", "0307"]
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
  - date: "2026-09-23"
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped: PR #341 merged 09:10 UTC (`4c9b70e2`), bundle synced 09:16 UTC
      (`index-DIgi8Afa.js`, `/api/*` invalidation completed), checked live.
      All 9 criteria met; the portal is public since the explorer's
      basic auth came off `/api/*`. Scope grew by three items found on the
      way: hash scrolling and the loader, the Quick Start error section, the
      sign-in card's status page. Spawned [[0306]] (docs drift) and [[0307]]
      (revoked-key contact). ⚠️ The explorer's `master` still reads `true`.
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
- [x] `docs/scf/api-endpoints.md` no longer describes the portal as gated
- [x] A `#hash` URL lands on its target: from another page's bar, pasted, on
      a lazy page, and on back/forward; an in-page link keeps its smooth
      scroll; a pushed page opens at the top
- [x] The lazy pages' loader keeps the footer below the fold
- [x] The Quick Start's 429 card shows what its Copy button writes, and the
      error lede no longer promises a `code` on the gateway's 403 and 429
- [x] No page names a status page: the sign-in refusal card ends at "contact
      support"

## Implementation Notes

- **Footer and logo** (`93738a9d`): `Footer` in `landing/Chrome.tsx` lists
  Documentation, Dashboard (when a key can be offered), Contact and Privacy
  policy; the Rumble Fish mark is its only `rumblefish.dev` link.
  `app.spec.tsx` asserts the wordmark's `href` on every route with a bar,
  signed in and out.
- **Scroll and loader** (`259fbbbc`): `useScrollOnNavigate` in `app.tsx`
  (Step 5) and `PageLoading`, a viewport tall less the bar (Step 6), with
  tests for a hash on load, on a lazy page, on back, and push-to-top.
- **Quick Start error section** (`e464bdb8`): the 429 card is `RATE_LIMIT`,
  a `Snippet` in `SNIPPET_TABLES`, so the existing drift spec compares what it
  renders with what it copies — and fails on the old card, checked. New lede.
- **Sign-in card** (`0800ab31`): `KeepsHappening` ends at "contact support".
- **Docs** (`9b0c1c7d`): `docs/scf/api-endpoints.md` describes the portal as
  public, with the explorer-`master` caveat below.
- **Shipped**: PR #341 merged 09:10 UTC (`4c9b70e2`); `make -C infra
  sync-portal-explorer` from that commit, `api/index.html` written 09:16 UTC,
  invalidation `I8D8ALLIBV53W9F3ALN12OSDB9` completed in ~20 s. Checked live
  in a browser: entry chunk `index-DIgi8Afa.js`, both footers, the 429 card
  and lede, the refusal card, `/api/docs` (9 operations, 20 schemas).
- 6 files outside `lore/`, +291 / −114. Portal suite 249 passed, 4 skipped;
  typecheck and lint clean.

**Modified test:** the footer assertion in `app.spec.tsx` went from "no
`Status` link" to "no `Status` and no `rumblefish.dev` text inside the footer
navigation" — stricter, because `Status` used to render as plain text.

## Design Decisions

### From Plan

1. **`Status` and the `rumblefish.dev` text out of the footer**; the Rumble
   Fish mark keeps the company link.
2. **Back/forward to a hashed entry shows its target** even if the reader had
   scrolled away before leaving (Step 5's ⚠️).

### Emerged

3. **The 429 card became a `Snippet`** rather than a second hand-kept copy:
   the drift spec already renders every `SNIPPET_TABLES` entry against its
   Copy text, so one table entry is the whole test.
4. **The error lede was rewritten, not dropped**, and says only what the table
   under it shows: the API's own errors carry a `code` and a `message`, the
   gateway's 403 and 429 a `message` only.
5. **The sign-in card's "status page" was cut too** (Stanisław, 2026-09-23),
   for the reason the footer's `Status` was.
6. **The basic-auth criteria were ticked from measurement** (Stanisław,
   2026-09-23), with the explorer's `master` caveat recorded rather than
   waited on.
7. **The docs-vs-production comparison became its own task** ([[0306]]);
   only the two statements that contradicted the public page itself were
   fixed here.

## Issues Encountered

- **An earlier fix reached half a card.** 0193's measured 429 body went into
  the Copy text (`RATE_LIMIT_BODY`); the rendered card kept the Figma frame's
  invented body, and nothing compared the two because the card was not in
  `SNIPPET_TABLES`.
- **The explorer's `master` still reads `enableApiSpaBasicAuth: true`.**
  Production is open, but no `deploy-production` run followed 2026-09-21, so
  the change reached it outside the tag pipeline; a `-all` or `-Delivery`
  release tagged from `master` before their `develop` merges closes the gate
  again.
- **`lore-framework_validate` rejects `type: CHORE`** (its enum is BUG,
  FEATURE, RESEARCH, REFACTOR, DOCS). Left as created; the repo uses CHORE
  elsewhere too.

## Future Work

- [[0306]] — the rest of the docs-vs-production comparison.
- [[0307]] — the revoked-key card's contact button and "Contact support."
  lead nowhere.
- Explorer `develop` → `master`, so the flag stays `false` — their lore-0519,
  not a task in this repo.
