---
id: "0308"
title: "The landing's 'Built by Rumble Fish' mark is not a link, while the footer's is"
type: BUG
status: completed
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
  - date: "2026-09-23"
    status: active
    who: stkrolikiewicz
    note: "Activated; the fix goes on its own branch."
  - date: "2026-09-23"
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped: PR #342 merged 09:41 UTC (`e0bed6da`), bundle synced 09:46 UTC
      (`index-CllFuKsq.js`, `/api/*` invalidation completed), checked live.
      1 criterion met; the band's mark is the footer's `RumbleFishMark`, held
      by a landing test that fails without it.
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

- [x] The band's Rumble Fish mark links to `https://rumblefish.dev`, asserted
      in `app.spec.tsx`

## Implementation Notes

- `landing/Hero.tsx` renders `RumbleFishMark` from `landing/Chrome.tsx` in
  place of the bare image (`09a5df02`); same 32 px, now a link with a pointer
  cursor. 2 files, +24 / −10.
- New test in `app.spec.tsx`, "links both Rumble Fish marks on the landing to
  the company": every Rumble Fish link on `/` goes to `https://rumblefish.dev`,
  and there are two. It fails with the band put back to a bare image — one
  link found. Portal suite 250 passed, 4 skipped.
- **Shipped**: PR #342 merged 09:41 UTC (`e0bed6da`); `make -C infra
  sync-portal-explorer`, `api/index.html` written 09:46 UTC (`index-CllFuKsq.js`),
  invalidation `I6EVU7HDAFC8YOHTN4EXNZP9NO` completed in ~20 s. Checked live:
  both marks on the landing link to `https://rumblefish.dev/`.

## Design Decisions

### From Plan

1. **Reuse the footer's `RumbleFishMark`** rather than wrap the band's image
   in a second link: one component, one href.

### Emerged

2. **The band's `alt` became the footer's**, "Rumble Fish — software
   development" (it was "Rumble Fish"), because the component carries it. The
   band still reads as "Built by Rumble Fish…" to a screen reader.
3. **The quick-start logo test kept its single-link assertion.** It renders
   `/quick-start`, which has only the footer's mark; the two-mark check is a
   separate test on `/`.

## Issues Encountered

- **pre-push failed on stock macOS**: `@rumblefish/stellar-prices-api-aws-cdk:test`
  needs GNU `realpath -m`, and `nx affected` pulled it in for a portal-only
  branch. Pushed with Homebrew coreutils first on `PATH`, hooks intact.
