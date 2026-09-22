---
id: "0301"
title: "Portal navigation: the navbar's Quick Start goes to a landing anchor, and a key holder without a session has no visible way to the dashboard"
type: BUG
status: active
related_adr: []
related_tasks: ["0193", "0195", "0163", "0233"]
tags: [layer-frontend, priority-medium, effort-small, epic-self-service-onboarding, portal]
links:
  - "../../../web/portal/src/landing/Chrome.tsx"
  - "../../../web/portal/src/app/app.tsx"
history:
  - date: "2026-09-22"
    status: active
    who: stkrolikiewicz
    note: >
      Filed and activated from a walk through the deployed portal while
      finishing [[0163]] and [[0233]]: "Quick Start" in the signed-out navbar
      on `/api/docs` lands on `/api/#get-started`, and the footer's
      "Dashboard" bounces a visitor without a session back to the landing
      page.
  - date: "2026-09-22"
    status: active
    who: stkrolikiewicz
    note: >
      Implemented on `fix/0301_portal-navigation`: "Quick Start" is the
      route in the bar and the phone's menu, "Sign in" sits beside "Get API
      Key", `/dashboard` without a session goes to `/login`. One emerged
      decision: a mount that had a session and lost it still goes to `/`, so
      signing out ends on the landing page as before. 212 portal tests,
      typecheck, lint green. Open: PR and deploy.
---

# Portal navigation: Quick Start goes to a landing anchor, and a key holder without a session has no way to the dashboard

## Summary

Two navigation defects in the portal's signed-out chrome, both left over from
before the quick start page ([[0193]]) and the API reference route ([[0195]])
existed. The navbar's "Quick Start" is a landing-page anchor, so on
`/api/docs` and `/api/quick-start` it leads to `/api/#get-started` — the
four-step marketing section — rather than to the page that carries its name.
And a visitor who already holds a key but has no session cookie sees only
"Get API Key"; the footer's "Dashboard" sends them to `/dashboard`, which
sends them straight back to `/` with no explanation.

## Context

`landing/Chrome.tsx` defines the three navbar links as in-page anchors
(`#features`, `#get-started`, `#faq`) and, off the landing page, prefixes
them with `LANDING` so they lead back to the sections. That was right when
"Quick Start" could only mean the four-step section. `app.tsx`'s
`DashboardRoute` answers `!gate.open || !gate.authenticated` with
`<Navigate to="/" />`, which is right for a closed portal and wrong for a
signed-out visitor — the landing already forwards `?signin` / `?issue`
arrivals to `/login`, and `/login` forwards an authenticated visitor to
`/dashboard`, so the loop is safe.

## Implementation

- `landing/Chrome.tsx`: "Quick Start" becomes a router link to
  `QUICKSTART_ROUTE` in the navbar and the mobile menu; the two anchors stay.
  A quiet "Sign in" link beside "Get API Key" (same condition: the portal is
  open), in both places.
- `app/app.tsx`, `DashboardRoute`: a closed portal still goes to `/`; a
  signed-out visitor goes to `/login` with the query carried along.
- `app/app.spec.tsx`: the `/dashboard` redirect cases split by cause; the
  navbar test on the quick start reads the new href; a case for "Sign in".

## Acceptance Criteria

- [x] On `/api/docs` and `/api/quick-start`, signed out, "Quick Start" in the
      navbar and in the mobile menu opens `/api/quick-start`
- [x] `/dashboard` without a session, portal open, lands on `/login`; with the
      portal closed it still lands on `/`
- [x] The signed-out navbar offers "Sign in" whenever it offers "Get API Key",
      and neither when the portal is closed
- [x] Portal tests, lint and typecheck green

## Notes

- The Figma frame draws three links and one button; "Sign in" is an addition,
  recorded here as the decision. The alternative — relabelling "Get API Key"
  — would break the promise the hero and the frame make to a first-time
  visitor, and the dashboard navbar already has no such button because its
  visitor has a key.

## Implementation Notes (2026-09-22)

- `landing/Chrome.tsx`: `NAV` is a union of anchor and route entries. "Quick
  Start" is `QUICKSTART_ROUTE` through `RouterLink` in the bar and in the
  phone's drawer; the two anchors keep the `LANDING` prefix off the landing
  page. "Sign in" (`LOGIN_ROUTE`) sits beside "Get API Key" in both places,
  under the same `canOfferKey`. The link styles moved to `navLinkSx` /
  `menuLinkSx` so the two branches share them.
- `app/app.tsx`, `DashboardRoute`: a closed portal still goes to `/`; no
  session goes to `/login` with the query; a mount that has seen a session
  and lost it goes to `/` (see Emerged 1).
- `app/app.spec.tsx`: `/dashboard` without a session lands on `/login` with
  the login card; with the portal closed it lands on `/` and offers no
  "Sign in"; the quick start's navbar renders "Quick Start" as
  `/quick-start` and "Sign in" as `/login`; the closed landing offers no
  "Sign in". 212 tests, typecheck and lint green.
- Not pinned by a test: the phone's drawer (not mounted while closed). It
  maps the same `NAV` and the same `canOfferKey`. The API reference route
  is not exercised separately either: it renders the same
  `<Navbar inPage={false}>` as the quick start.

## Design Decisions

### Emerged

1. **Sign-out is told apart by a ref, not by a session flag or a navigate
   call.** The sign-out test's contract predates this task: signing out ends
   on the landing page "with its way back in, not the 'you are not signed
   in' line". A `hadSession` ref in `DashboardRoute` keeps that in three
   lines; a promise-returning `onSignOut` that navigates on success would
   have touched the `Gate` type and four call sites for the same outcome.
2. **"Sign in" is added, "Get API Key" is not relabelled** — the frame's
   promise to a first-time visitor stays, and the returning visitor gets a
   word for what they want (also in Notes above).
