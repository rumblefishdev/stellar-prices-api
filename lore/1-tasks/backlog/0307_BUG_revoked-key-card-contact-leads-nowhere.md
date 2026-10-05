---
id: "0307"
title: "The revoked-key card's contact button and 'Contact support.' lead nowhere, though a contact URL has existed since 0301"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0305", "0301"]
tags: [layer-frontend, portal, priority-low, effort-small]
links:
  - "../../../web/portal/src/app/app.tsx"
history:
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

- [ ] Both lead somewhere a person answers, asserted in `app.spec.tsx`
- [ ] No comment in `app.tsx` still says the portal has no support address
