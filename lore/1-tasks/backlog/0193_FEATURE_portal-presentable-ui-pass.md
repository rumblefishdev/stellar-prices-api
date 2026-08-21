---
id: "0193"
title: "Make the portal presentable — MUI, the landing page, refusal screens, mobile"
type: FEATURE
status: backlog
related_adr: ["0010"]
related_tasks: ["0183", "0162", "0185", "0187", "0188", "0189", "0191", "0192", "0163", "0195"]
tags: [layer-frontend, priority-medium, effort-medium, milestone-M3, epic-self-service-onboarding, ui, dashboard, slice-10]
milestone: 3
links:
  - "../archive/0162_FEATURE_portal-frontend-app.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Tenth slice, the other half of [[0162]]. Every screen already exists and
      works by the time this starts — [[0185]] built them ugly and each backend
      slice added its own. This task changes how they look and how the states
      hang together, and decides nothing about what they say: that wording was
      settled in the slice that owns each behaviour.
---

# Make the portal presentable

## Summary

**Story:** *as a developer who has never heard of us, I land on the portal and
get from the landing page to a working `curl` in under a minute — and it does
not look like a debug harness.*

The reviewer's sign-off wording is *"self-service API key request flow
functional"*, and the demo path is these screens. Correctness is already done by
this point; this slice is about it being legible.

## Context

The rule that keeps this task honest: **it re-decides no copy.** The wording of
the two eligibility refusals is [[0189]]'s, the `delete-key` modal is [[0191]]'s,
the revoke confirmation and its "no replacement is issued" line are [[0191]]'s (0192 merged into it),
the `GetUsage` lag line is [[0188]]'s. If this slice finds one of them wrong, fix
it in the owning task rather than quietly here — otherwise the reason behind the
wording is lost and the next person edits it back.

## Implementation

- **Install MUI 7 + Emotion**, the remaining half of the stack settled on
  2026-08-07 ([[0185]] deliberately shipped without it). Still mirroring
  `soroban-block-explorer`.
- **Two screens, as the epic says — it calls this a *small* portal and that is
  the right size.** Landing, and dashboard.
- **Landing page** explains what the API is, offers "Sign in with Discord", and
  **states both prerequisites before the user authenticates** — Stellar Discord
  membership with the `discord.gg/stellardev` invite, and the minimum account
  age. A developer who learns about the membership requirement only after
  authorising has authorised an app for nothing.
- **Dashboard** — key (masked, reveal toggle, copy button, all carried over from
  [[0187]]), usage against quota with the reset date and the 1 req/s figure as
  **numbers, not prose**, the "last updated" line, and the rework and revoke
  actions.
- **Link out to the quickstart ([[0163]]) and Swagger UI ([[0195]])** from the
  dashboard. A key is only useful next to the thing that shows what to call.
- **Every non-happy state gets a screen, not a blank page:** session expired,
  backend unavailable, Discord sign-in cancelled, not a member, account too
  young, could-not-verify, rework refused, revoked. That list is most of what a
  two-screen app is.
- **"Could not verify membership" must look different from "not a member."**
  Different icon, different action, different tone — the second is an accusation
  the user cannot act on if it is wrong.
- **Works on a phone.** A reviewer will open it on one.
- **Still no third-party scripts** — no analytics, no CDN fonts, no tag
  managers, on a page that renders a credential. Keeps the CSP trivial.

## Acceptance Criteria

- [ ] **Ships closed.** The "portal not yet available" state from [[0183]] is
      one of the states this pass styles, not an afterthought — it may be what a
      visitor sees for weeks
- [ ] Landing page states both prerequisites before the sign-in button, with a
      working invite link
- [ ] First sign-in lands on the dashboard with the key visible and copyable;
      returning shows the same key
- [ ] Dashboard shows used-of-quota, the reset date and 1 req/s
- [ ] Every state in the list above renders something specific — no blank
      screens, no generic "something went wrong"
- [ ] "Could not verify" and "not a member" are visually and textually distinct
- [ ] Usable at 375 px wide
- [ ] No secrets, no AWS calls, no third-party scripts in the bundle
- [ ] No copy owned by another slice was changed here without changing it there
- [ ] Epic AC 2 and AC 4 satisfied from the user's side

## Notes

- Structural precedent from the explorer: it splits `web/` from `libs/ui` and
  `libs/api-types`. Two screens do not justify that here — extract only if a
  second frontend arrives.
- Worth a deliberate clarity pass before Tranche 3, not just a correctness pass.
  This is the screen pair the reviewer looks at.
