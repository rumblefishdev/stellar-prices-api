---
id: "0308"
title: "The landing's 'Built by Rumble Fish' mark is not a link, while the footer's is"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0301", "0305", "0307"]
tags: [layer-frontend, portal, priority-medium, effort-small]
links:
  - "../../../web/portal/src/landing/Hero.tsx"
  - "../../../web/portal/src/landing/Chrome.tsx"
history:
  - date: "2026-09-23"
    status: backlog
    who: stkrolikiewicz
    note: >
      Reported by Stanisław on the public landing, after 0305 shipped: the
      trust band under the hero shows the Rumble Fish mark as a bare image.
---

# The landing's "Built by Rumble Fish" mark is not a link

## Summary

The trust band under the landing hero reads "Built by" and the Rumble Fish
mark, drawn as a plain `<img>` in `landing/Hero.tsx`. The same mark in the
footer is `RumbleFishMark` (`landing/Chrome.tsx`), a link to
`https://rumblefish.dev` since [[0301]]. The band's mark should be that link
too.

## Implementation

- Render `RumbleFishMark` in the band instead of the bare image.
- `app.spec.tsx` looks the Rumble Fish link up with `getByRole`, which throws
  once there are two; assert that every Rumble Fish link on the landing goes
  to `https://rumblefish.dev`, and that the band's is one of them.

The sign-in card's own "Built by Rumble Fish" lockup (`LoginCard.tsx`) is not
part of this: it is one `role="img"` over two images, on a card whose job is
the sign-in.

## Acceptance Criteria

- [ ] The band's Rumble Fish mark links to `https://rumblefish.dev`, asserted
      in `app.spec.tsx`
