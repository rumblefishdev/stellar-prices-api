---
id: "0162"
title: "Portal frontend — sign-in, key on screen, usage-against-quota dashboard"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0159", "0160", "0161", "0163", "0164"]
tags: [layer-frontend, priority-high, effort-large, milestone-M3, epic-self-service-onboarding, ui, dashboard, discord]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      The visible half of the epic — the "small portal" of its summary line.
      Consumes [[0159]] for sign-in and all four [[0160]] endpoints; deploys
      into [[0161]]'s distribution.
---

# Portal frontend

## Summary

The portal itself: a landing page that explains what this API is and offers
"Sign in with Discord", and a dashboard that shows the user their key and their
usage against quota. Two screens — the epic calls it a *small* portal and that
is the right size.

## Context

Every hard part lives elsewhere: identity in [[0159]], AWS operations in
[[0160]], hosting in [[0161]]. What remains here is making the flow legible —
a developer who has never heard of us should get from the landing page to a
working `curl` in under a minute, which is the whole point of self-service.

The bundle is static and ships to a CDN, so it can hold no secret and make no
AWS call. It talks only to the backend endpoints, carrying the session cookie.

## Implementation

**From the epic**

- Sign in with Discord.
- Key shown on screen immediately after sign-in **and** viewable again later on
  the dashboard — this is not a one-time reveal.
- Dashboard shows usage against the rate limit and quota, sourced from
  `GetUsage` via the backend.
- A "generate new key" action, with the once-a-month rule surfaced *(subject to
  [[0160]] Open #1 — the team has not settled whether rotation ships in
  Tranche 3)*.

**Follows from the epic, but not stated in it**

- **Framework decision, recorded.** This is the first frontend in the repo, so
  the choice sets a precedent — pick with the workspace's Nx conventions rather
  than in isolation, and prefer the smallest thing that builds to static files.
  Two screens do not need a router-heavy SPA.
- **Mask the key by default with a reveal toggle and a copy button.** The epic's
  point is that the key is *retrievable*, not that it should sit on screen
  during a screen-share. Copy-to-clipboard is what people actually use.
- **No third-party scripts.** No analytics, no fonts from a CDN, no tag
  managers, on a page that renders a credential. It also keeps the CSP simple.
- **Show the limits as numbers, not prose**: 1 req/s, and used-of-quota for the
  current period, with the reset date. A developer hitting `429` should be able
  to self-diagnose here rather than email us.
- **Render the `GetUsage` lag honestly** — a "last updated" line, using the
  wording [[0160]] settles, so the dashboard does not look broken when it
  trails a live test by a few minutes.
- **Rotation needs a confirmation step** that says plainly that the old key
  stops working immediately, and the refusal path needs to render the next
  eligible date rather than a generic error.
- **Link out to the quickstart ([[0163]]) and Swagger UI ([[0161]])** from the
  dashboard — the key is only useful next to the thing that shows what to call.
- **Signed-out and error states**: session expired, backend unavailable, Discord
  sign-in cancelled. These are most of the states a two-screen app has.

## Acceptance Criteria

- [ ] Landing page explains the API and offers Discord sign-in
- [ ] First sign-in lands on the dashboard with the key visible and copyable
- [ ] Returning sign-in shows the same key again
- [ ] Dashboard shows requests used against quota for the current period, the
      reset date, and the 1 req/s rate limit
- [ ] Rotation flow warns before acting and renders the refusal with a date
      *(pending [[0160]] Open #1)*
- [ ] No secrets, no AWS calls and no third-party scripts in the bundle
- [ ] Session expiry and backend-error states handled, not blank screens
- [ ] Works on a phone — a reviewer will open it on one
- [ ] Epic AC 2 and AC 4 satisfied from the user's side

## Notes

- The reviewer's sign-off wording is *"self-service API key request flow
  functional"* — the demo path is this screen pair, so it is worth a pass for
  clarity before Tranche 3, not just correctness.
- Keep the response shapes agreed with [[0160]] stable; this is the only
  consumer, but it is the one a reviewer looks at.
