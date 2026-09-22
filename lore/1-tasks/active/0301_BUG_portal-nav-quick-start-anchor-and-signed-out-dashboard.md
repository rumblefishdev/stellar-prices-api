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

- [ ] On `/api/docs` and `/api/quick-start`, signed out, "Quick Start" in the
      navbar and in the mobile menu opens `/api/quick-start`
- [ ] `/dashboard` without a session, portal open, lands on `/login`; with the
      portal closed it still lands on `/`
- [ ] The signed-out navbar offers "Sign in" whenever it offers "Get API Key",
      and neither when the portal is closed
- [ ] Portal tests, lint and typecheck green

## Notes

- The Figma frame draws three links and one button; "Sign in" is an addition,
  recorded here as the decision. The alternative — relabelling "Get API Key"
  — would break the promise the hero and the frame make to a first-time
  visitor, and the dashboard navbar already has no such button because its
  visitor has a key.
