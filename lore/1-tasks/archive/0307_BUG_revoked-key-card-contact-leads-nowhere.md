---
id: "0307"
title: "The revoked-key card's contact button and 'Contact support.' lead nowhere, though a contact URL has existed since 0301"
type: BUG
status: completed
related_adr: []
related_tasks: ["0305", "0301"]
tags: [layer-frontend, portal, priority-low, effort-small]
links:
  - "../../../web/portal/src/app/app.tsx"
history:
  - date: "2026-11-25"
    status: completed
    who: okarcz
    note: >
      Closed as obsolete, no code change. The Prices portal moved to
      sorobanscan-api-portal and production now serves it from there, so
      web/portal here no longer reaches anyone. The bug moved with it
      (src/prices/app/app.tsx there); passed to Adam, who owns that repo.
  - date: "2026-09-23"
    status: backlog
    who: stkrolikiewicz
    note: "Spawned from 0305 future work."
---

# The revoked-key card's contact affordances lead nowhere

## Summary

The dashboard's revoked-key card offers "Contact us about this decision" (a
filled button, `data-testid="revoked-contact"`) and "Questions? Contact
support." (underlined text). Neither goes anywhere. Their comments say the
portal has no support address, but `RUMBLEFISH_CONTACT`
(`https://www.rumblefish.dev/contact/`) has been in `landing/links.ts` since
[[0301]], and the sign-in card's "contact support" already links to it.

## Context

Found while [[0305]] cut the sign-in card's "status page" (2026-09-23). The
same underline style (`UNDERLINED` in `web/portal/src/app/app.tsx`) marks
these two and the sign-in card's link; only the last one goes anywhere.
"Contact us for higher limits." on the rate-limit strip is a different case —
there is no commercial-plans destination — and is not part of this.

## Implementation

- The `revoked-contact` button becomes `component="a"` with
  `href={RUMBLEFISH_CONTACT}`, or gets a destination chosen for appeals if the
  contact page is not the right place for them.
- "Contact support." becomes an `<a href={RUMBLEFISH_CONTACT}>`.
- The comments that say no support address exists are corrected.

## Acceptance Criteria

- [ ] Both lead somewhere a person answers, asserted in `app.spec.tsx` (obsolete here — see Outcome)
- [ ] No comment in `app.tsx` still says the portal has no support address (obsolete here — see Outcome)

## Outcome (2026-11-25): closed as obsolete

- **The portal moved.** Adam moved the Prices portal and its key dashboard to
  `sorobanscan-api-portal` (first commit 2026-11-17). Its README says
  `/api/prices-api/` is served from this repo's `web/portal` only until the
  hub's launch, and the hub is live: on 2026-11-25 `https://sorobanscan.com/api/`
  was titled "Soroban Scan APIs" and `/api/hub/auth/login` answered `303`. A fix
  in `web/portal/src/app/app.tsx` here would reach no one.
- **The bug moved with it.** In `sorobanscan-api-portal`, `src/prices/app/app.tsx`
  (around lines 1700–1772 at `15b5070`) still has the `revoked-contact` button
  and "Contact support." wired to nothing, with the comment saying no support
  address exists, while `RUMBLEFISH_CONTACT` is in `src/prices/landing/links.ts`.
  `/api/prices-api/dashboard` still answered `200`.
- **The shared dashboard does not repeat it.** Its revoked-key card
  (`src/account/prices/KeyCard.tsx`) has no contact controls; per that README it
  replaces the Prices portal's own dashboard.
- Passed to Adam, who owns that repo. No backlog task spawned here, because the
  code no longer lives in this repo.
