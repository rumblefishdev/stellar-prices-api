---
id: "0162"
title: "Portal frontend — sign-in, key on screen, usage-against-quota dashboard"
type: FEATURE
status: backlog
related_adr: ["0010"]
related_tasks: ["0156", "0159", "0160", "0161", "0163", "0164", "0169", "0170"]
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
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Meeting outcome: served from the `/api-tokens/` prefix, so the build needs
      a base path from day one. Rework confirmation is a modal gated on typing
      `delete-key`. Rework is in Tranche 3 scope — the conditional wording is
      removed.
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      ADR 0010 adds two eligibility refusals this task did not have — sign-in
      can now succeed while issuance is refused (not a guild member, or account
      below the minimum age). Both need actionable screens, the landing page
      must state the prerequisites before authenticating, and "could not
      verify" must render differently from "not a member".
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
AWS call. It talks only to the backend endpoints — mounted at
**`/api-tokens/api/`** ([[0159]], [[0161]]), same origin as the bundle itself —
carrying the session cookie.

## Implementation

**From the epic**

- Sign in with Discord.
- Key shown on screen immediately after sign-in **and** viewable again later on
  the dashboard — this is not a one-time reveal.
- Dashboard shows usage against the rate limit and quota, sourced from
  `GetUsage` via the backend.
- A rework action ("generate new key"), with the once-a-quota-period rule
  surfaced. In Tranche 3 scope as of 2026-08-07.

**Follows from the epic, but not stated in it**

- **Framework — settled 2026-08-07: mirror `soroban-block-explorer`.** That is
  the sibling repo in the same org (we already consume its `xdr-parser` crate),
  so the stack is proven, reviewed and familiar to the team rather than newly
  chosen here:

  | Concern | Choice |
  | --- | --- |
  | Framework | React 19 |
  | Build | Vite 7 + `@vitejs/plugin-react` |
  | Language | TypeScript 5.9 |
  | Routing | `react-router-dom` 7 |
  | UI / styling | MUI 7 + Emotion |
  | Server state | TanStack Query 5 |
  | Tests | Vitest 4 + Testing Library |
  | Orchestration | Nx — `@nx/react`, `@nx/vite`, `@nx/vitest` |

  **This validates [[0161]]'s hosting rather than straining it.** The explorer's
  `web/` is a plain Vite SPA — `index.html` plus `vite.config.ts`, no Next.js and
  no SSR — so it builds to static files, which is exactly what S3 + CloudFront
  serves. Had the reference been an SSR framework, 0161 would have needed
  reopening.

  Structural precedent worth copying: the explorer splits `web/` (the app) from
  `libs/ui` (shared components) and `libs/api-types` (shared types). Two screens
  do not justify all three here — start with the app alone and extract only if a
  second frontend arrives.

  One deliberate divergence: the explorer hand-maintains `libs/api-types`, but
  our API publishes its own OpenAPI document at `/api-docs-json` ([[0124]]), so
  generate the types from the spec instead. Hand-written copies of a published
  contract are the drift [[0124]] spent a task preventing.

  Note Nx is 22.7.0 here against 22.6.1 there — close enough to share config
  shapes, worth a glance if a generator behaves unexpectedly.

- **Copy the explorer's dev-proxy pattern.** Its `vite.config.ts` proxies the
  API paths through the Vite dev server so the browser only ever talks to
  `localhost` — same-origin locally, no CORS — and injects the dev `x-api-key`
  **server-side in the Node config**, never into the client bundle. That is the
  same-origin model [[0161]] gives us in production and the same discipline
  [[0163]] teaches partners, so local dev and production agree by construction.
- **The build must know it is served from `/api-tokens/`.** Concretely, with the
  stack above: `base: '/api-tokens/'` in `vite.config.ts` and
  `basename="/api-tokens"` on the router. From the first commit — without it
  every asset and route URL points at the domain root and the app breaks the
  moment it is not served from `/`. This is a build setting, not a CloudFront
  setting; [[0161]] cannot fix it after the fact.
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
- **Rework confirmation — specified 2026-08-07.** The action opens a modal that
  states plainly that the current key is deleted and stops working immediately,
  so anything using it breaks the moment the user confirms. The confirm button
  stays **disabled until the user types `delete-key`**. Disable it again on
  submit so a double-click cannot fire two reworks. The refusal path renders
  `next_eligible_at` from [[0160]]'s `409`, not a generic error — for a key
  reworked on 3 August that reads "1 September".
- **Link out to the quickstart ([[0163]]) and Swagger UI ([[0161]])** from the
  dashboard — the key is only useful next to the thing that shows what to call.
- **Signed-out and error states**: session expired, backend unavailable, Discord
  sign-in cancelled. These are most of the states a two-screen app has.
- **Two eligibility refusals — added 2026-08-10 by ADR 0010.** Sign-in can now
  succeed while key issuance is still refused, which is a state this task did
  not previously have. Both must be specific and actionable, not a generic
  error, because the user *can* fix both and will abandon otherwise:
  - **Not a member of the Stellar Discord.** Say which server, and link the
    invite so joining is one click from here. Then let them retry without
    signing in again — they were authenticated, just not eligible.
    **Use `discord.gg/stellardev`**, the registered vanity code; the other
    invites SDF publishes are personal invites belonging to individual accounts
    and one of them is already dead ([[0169]]).
  - **Discord account below the minimum age.** The threshold is **5 minutes**
    (ADR 0010), so this is a *wait*, not a rejection — render the time
    remaining and let them retry, e.g. "your Discord account is very new; try
    again in about 4 minutes". **Do not render a calendar date** the way
    [[0160]]'s `409` + `next_eligible_at` does: that pattern is right for the
    rework cap, where the wait is weeks, and absurd here.
    Do not hard-code "5 minutes" in the copy — the threshold is an SSM value
    expected to be raised, so drive the wording from what the backend returns.
- **A Discord outage must not render as "you are not a member."** The membership
  check infers non-membership from an undocumented error shape ([[0170]] #1), so
  the backend distinguishes "not a member" from "could not tell" and this app
  must render them differently — the second is "try again shortly", not an
  accusation the user cannot act on.
- **The landing page states both prerequisites before sign-in.** A developer who
  learns about the membership requirement only after authenticating has been
  made to authorise an app for nothing. Say it on the landing page, alongside
  the invite link.

## Acceptance Criteria

- [ ] Landing page explains the API and offers Discord sign-in, and states the
      two prerequisites (Stellar Discord membership, minimum account age)
      **before** the user authenticates
- [ ] A non-member is told which server to join, with a working invite link, and
      can retry without re-authenticating
- [ ] A below-threshold account is shown how long to wait and can retry in
      place — not a calendar date, and not a generic refusal
- [ ] Neither eligibility check runs after issuance: the dashboard, key reveal
      and rework work for a user who has since left the Discord server
- [ ] "Could not verify membership" renders differently from "not a member"
- [ ] First sign-in lands on the dashboard with the key visible and copyable
- [ ] Returning sign-in shows the same key again
- [ ] Dashboard shows requests used against quota for the current period, the
      reset date, and the 1 req/s rate limit
- [ ] App works when served from `/api-tokens/` — assets and routes resolve,
      and a refresh on a sub-page returns to the same screen
- [ ] Rework modal states that the old key dies immediately, and confirm is
      disabled until `delete-key` is typed
- [ ] Refused rework renders the next eligible date, not a generic error
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
