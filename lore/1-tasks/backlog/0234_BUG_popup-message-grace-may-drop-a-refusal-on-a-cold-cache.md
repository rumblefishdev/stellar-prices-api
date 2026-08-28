---
id: "0234"
title: "The popup's 1500 ms grace may drop a refusal query on a cold cache — a refused visitor sees a plain dashboard"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0193", "0159", "0189"]
tags: [layer-frontend, priority-low, effort-small, milestone-M3, epic-self-service-onboarding, oauth, portal]
milestone: 3
links:
  - "../archive/0193_FEATURE_portal-presentable-ui-pass.md"
history:
  - date: "2026-08-28"
    status: backlog
    who: akot
    note: >
      Spawned from [[0193]]'s Future Work. Raised in PR #249's review as
      plausible and **not reproduced** — the grace window was the fix for the
      opposite bug (poll and close ending the wait before the message landed),
      so it is a bound that was chosen, not one that was measured.
---

# The popup grace is a fixed 1500 ms, and a cold cache may outlast it

## Summary

`POPUP_MESSAGE_GRACE_MS` (`web/portal/src/app/app.tsx:1904`) holds a slower
signal's verdict open for 1500 ms so the popup's `postMessage` can overtake it.
On a cold cache the popup has to download the bundle before `bridgeOAuthPopup`
posts at all. If that exceeds the grace, the poll's or the close's verdict wins
and the query the message carried — `?issue=too_young`, `?signin=not_member` —
is lost. The visitor is refused and shown a plain dashboard or the generic
failure card.

## Context

⚠️ **Read the comment at `afterGrace` before changing this.** The grace exists
because ending the wait on the spot was the original defect: the callback's 303
sets the cookie and lands the popup on its query, but the popup still has to
load and post, and it closes in the same breath — so `closed` can be true while
the message is still queued. Shortening or removing the window reopens exactly
the bug it fixed.

The number was never measured against a cold cache. That is the whole of this
task.

## Implementation

- Measure the real interval from popup navigation to `postMessage` on a cold
  cache, on a throttled connection, against the deployed bundle. That number is
  the input every option below needs.
- Then choose, and record why:
  - raise the constant to the measured p99, or
  - have the popup post before it renders — the message needs no React, only
    `location.search`, so it could go in a tiny inline head script or the entry
    module's first statement, or
  - drop the grace in favour of an explicit handshake: the opener acknowledges,
    the popup closes only after that.
- Whichever wins, keep an assertion for the original bug — a message arriving
  after the poll and after `closed` must still be the verdict that renders.

## Acceptance Criteria

- [ ] The cold-cache popup-to-`postMessage` interval is measured and recorded
- [ ] A refusal query survives a cold-cache sign-in, demonstrated rather than
      assumed
- [ ] The existing tests that cover the overtaking cases still pass, including
      "lets a late popup message overtake a poll that already saw the session"
      and "lets a popup message overtake the closed-window watch"
- [ ] The chosen bound has a stated reason at the constant, not a bare number
