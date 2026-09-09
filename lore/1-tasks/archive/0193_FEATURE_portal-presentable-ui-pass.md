---
id: "0193"
title: "Make the portal presentable — MUI, the landing page, refusal screens, mobile"
type: FEATURE
status: completed
related_adr: ["0010"]
related_tasks: ["0183", "0162", "0185", "0187", "0188", "0189", "0191", "0192", "0163", "0195", "0232", "0233", "0234"]
tags: [layer-frontend, priority-medium, effort-medium, milestone-M3, epic-self-service-onboarding, ui, dashboard, slice-10]
milestone: 3
links:
  - "./0162_FEATURE_portal-frontend-app.md"
history:
  - date: "2026-08-13"
    status: backlog
    who: akot
    note: >
      Tenth slice, the other half of [[0162]]. Every screen already exists and
      works by the time this starts — [[0185]] built them ugly and each backend
      slice added its own. This task changes how they look and how the states
      hang together, and decides nothing about what they say: that wording was
      settled in the slice that owns each behaviour.
  - date: "2026-08-24"
    status: active
    who: akot
    note: >
      Activated by Adam to build the frontend. The portal is served under a
      path prefix on an existing domain — `https://sorobanscan.rumblefish.dev/`
      with the landing page at `/api-key` — so this slice's screens are the
      whole visible surface of the self-service onboarding epic.
  - date: "2026-08-27"
    status: active
    who: akot
    note: >
      Review round on PR #249 (karczuRF, stkrolikiewicz): 25 findings, 22
      confirmed against the code, 2 needing a browser, 1 with a caveat. All
      addressed in six commits except the one that is a measurement —
      `pending_absent` at sign-in, which needs the Discord Developer Portal
      scope and a local run against the real guild (runbook §1 step 3, §5)
      and is Adam's to do before merge. Decisions #3-#9 below emerged from
      it; 0233 spawned for the design-vs-OpenAPI reconciliation; 0191
      amended a second time (the landing restated the superseded model).
      Two of Adam's 2026-08-25 calls reversed on review, both recorded at
      the render site: "Last rotated" → "Last updated", and 0188's lag line
      back under the meter. Portal 156 tests (+4), Rust lib 162 (+1).
  - date: "2026-08-28"
    status: completed
    who: akot
    note: >
      Merged as PR #249 (`d53cfc2`). All ten acceptance criteria met, two with
      a stated limit: 375 px is met by construction and review rather than in a
      browser, and epic AC 2/AC 4 are exercised against a stubbed backend
      because the portal ships closed. Two screens, 21 landing components, 8
      refusal states, MUI 7 + Emotion, no third-party script. Portal 156 tests,
      Rust lib 162. Eleven design decisions, #3-#11 emerged from the review
      round. Spawned [[0232]] (daily chart), [[0233]] (OpenAPI reconciliation)
      and [[0234]] (popup grace). The `pending_absent` measurement ran on
      2026-08-27 and answered `pending: false`, but it closes [[0189]] item 2,
      not anything here.
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
the two eligibility refusals is [[0189]]'s, the `regenerate-key` modal is [[0191]]'s
(phrase amended there on 2026-08-27, decision 41, after this slice changed it),
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

- [x] **Ships closed.** The "portal not yet available" state from [[0183]] is
      one of the states this pass styles, not an afterthought — it may be what a
      visitor sees for weeks. `Hero.tsx` renders the closed hero off
      `PORTAL_ENABLED`; asserted in `main.spec.tsx` and off the stable hook
      `LoginCard.tsx` leaves for it
- [x] Landing page states both prerequisites before the sign-in button, with a
      working invite link. `STELLAR_DISCORD_INVITE` in `landing/links.ts`, the
      registered vanity invite, asserted by href in two tests
- [x] First sign-in lands on the dashboard with the key visible and copyable;
      returning shows the same key
- [x] Dashboard shows used-of-quota, the reset date and 1 req/s
- [x] Every state in the list above renders something specific — no blank
      screens, no generic "something went wrong". All eight have a test that
      names them
- [x] "Could not verify" and "not a member" are visually and textually distinct
      — different icon, action and tone, asserted on both the sign-in and the
      dashboard path
- [x] Usable at 375 px wide — **by construction and by review, not in a
      browser.** The type ramp interpolates from a 375 px `min`
      (`theme/theme.ts`), the signed-in bar was refitted to two rows at that
      width on 2026-08-27, and the review's two browser findings were addressed
      at the render site. Nobody has opened it on a phone
- [x] No secrets, no AWS calls, no third-party scripts in the bundle. Fonts are
      local (`assets/fonts`, `theme/fonts.css`); no CDN, analytics or tag
      manager reference exists in the source
- [x] No copy owned by another slice was changed here without changing it
      there. 0191 amended twice in 0191, 0188's lag line restored as 0188
      decided it, 0189's refusals rendered verbatim
- [x] Epic AC 2 and AC 4 satisfied from the user's side — **at the level this
      slice can reach.** Both are exercised end-to-end in the portal suite
      against a stubbed backend. Neither has been walked against the real
      Discord on the deployed portal, because the portal ships closed; that
      walk belongs to whoever flips `PORTAL_ENABLED`

## Implementation Notes

- **Stack.** MUI 7 + Emotion, the half of the 2026-08-07 decision [[0185]]
  deliberately left out. No third-party script reaches the page.
- **Layout.** `web/portal/src/` — `landing/` (21 components, the hero through
  the FAQ and the login card), `app/` (the dashboard and every refusal state),
  `quickstart/`, `theme/` (tokens, theme, local font faces), `api/portal.ts`.
  Not split into `libs/ui` — see Notes; two screens do not pay for it.
- **Tests.** Portal suite 156 passing across 5 files (147 of them in
  `app.spec.tsx`). Rust lib 162.
- **Review.** PR #249, karczuRF and stkrolikiewicz: 25 findings, 22 confirmed
  against the code, 2 needing a browser, 1 with a caveat. Addressed in six
  commits; decisions #3-#11 below emerged from that round.

## Issues Encountered

- **Two of Adam's 2026-08-25 calls were reversed on review**, both recorded at
  the render site rather than only here: "Last rotated" → "Last updated"
  (decision #6) and 0188's lag line back under the meter (decision #7). The
  first was a contract mismatch — the field is `lastUpdatedDate` and nothing
  rotates; the second belonged to [[0188]], which had decided the wording once.
- **The design's `curl` aimed a real key at `api.soroswap.finance`.** Fixed the
  same day by pointing HOST at our execute-api base (decision #8); the paths
  stay the design's and are [[0233]]'s.
- **The 429 in the frame existed nowhere in the repo.** `RATE_LIMIT_EXCEEDED` +
  `Retry-After: 1` was invented by the design. Replaced with the measured
  response (decision #3); the quota-exhausted body is still not shown because
  it was never measured.
- **The legal footer asked visitors to agree to documents they could not
  open** — two underlined `<span>`s with no href. Not rendered until the URLs
  exist (decision #4).
- **Task ids collided with `develop` twice.** The chart task was renumbered
  0222 → 0226 on 2026-08-27 and then 0226 → 0232 on 2026-08-28, and the
  reconciliation task 0227 → 0233, because `develop` took both numbers while
  this branch was open. The second renumber landed after PR #249 merged and
  was cherry-picked onto `develop` separately. Ids assigned on a long-running
  branch are not reserved anywhere.

## Design Decisions

### From Plan

1. **Re-decide no copy.** Every sentence another slice owns is rendered
   verbatim or amended in the owning task (0191 twice, 0188 honoured).
2. **Two screens, MUI 7 + Emotion, no third-party scripts.** As the epic and
   the 2026-08-07 stack decision say.

### Emerged

3. **The 429 on the quick start is the measured one, not the design's.**
   Measured 2026-08-27 against the production free plan: `429`,
   `x-amzn-errortype: TooManyRequestsException`,
   `{"message":"Too Many Requests"}`, no `Retry-After`. The frame's
   `RATE_LIMIT_EXCEEDED` + `Retry-After: 1` existed nowhere in the repo. The
   quota-exhausted body is not shown because it was not measured.
4. **No legal footer until the documents exist.** "By continuing you agree to
   our Terms of Service and Privacy Policy" is not rendered — two underlined
   `<span>`s asked the visitor to agree to documents they could not open.
   Returns as links when the URLs land in `links.ts`.
5. **Prerequisites are stated before the button again.** On `/login` — the
   only page with the Discord button since the landing lost its card — as one
   line above the control, 0189's words, tertiary type; on the landing page,
   the FAQ row that carries them is open by default, so it states them
   without a click. Restores what 2026-08-26 moved into a collapsed FAQ. The
   acceptance criterion stands as written; the frame draws neither.
6. **"Last updated", not the frame's "Last rotated"** — reverses 2026-08-25.
   The value is `lastUpdatedDate`, nothing rotates (0191), both ends of the
   contract said so.
7. **0188's lag line is back**, restyled small under the meter — reverses
   2026-08-25's removal. 0188 owns the decision and this slice restyles it.
8. **The quick start's HOST is ours** (the execute-api base
   `docs/scf/api-endpoints.md` documents); the design's paths stay and are
   0233's. A page that renders a credential does not aim it at another domain.
9. **`SWAGGER_UI` → `API_REFERENCE`.** The constant names what it opens; 0195
   re-points it.
10. **`.mcp.json` is not committed.** Dev tooling that auto-configures a
    third-party server for everyone; per-developer, gitignored.
11. **A failed session request says which request failed.**
    `SessionState.failed` carries `while: 'checking' | 'signing-out'`; the
    dashboard renders it with a retry instead of redirecting to the landing
    page, and a failed sign-out is never rendered as a successful one.

## Future Work

- 0233 — reconcile the landing page and quick start with the real OpenAPI
  (paths, example fields, `source`, placeholder key, Figma).
- [[0234]] — the popup's 1500 ms grace (`POPUP_MESSAGE_GRACE_MS`) may lose a
  `postMessage` on a cold cache (review, plausible, not reproduced) — see the
  note at `afterGrace` before changing it.

## Notes

- Structural precedent from the explorer: it splits `web/` from `libs/ui` and
  `libs/api-types`. Two screens do not justify that here — extract only if a
  second frontend arrives.
- Worth a deliberate clarity pass before Tranche 3, not just a correctness pass.
  This is the screen pair the reviewer looks at.
