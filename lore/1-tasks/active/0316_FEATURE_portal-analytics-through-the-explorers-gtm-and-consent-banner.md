---
id: "0316"
title: "Add Google Analytics to the portal through the explorer's GTM container and HubSpot consent banner — the privacy policy changes in the same deploy"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0303", "0193", "0162", "0305", "0194"]
tags: [layer-frontend, portal, legal, analytics, epic-self-service-onboarding, priority-medium, effort-small]
links:
  - "../../../web/portal/index.html"
  - "../../../web/portal/src/privacy/privacy-policy.md"
  - "../../../web/portal/src/privacy/PrivacyPolicy.tsx"
history:
  - date: "2026-09-28"
    status: active
    who: stkrolikiewicz
    note: >
      Decided today: the portal gets analytics, and the privacy policy will be
      changed to say so. The portal uses the explorer's mechanism as-is (GTM
      `GTM-TBF2GP5S` and the HubSpot 8102665 consent banner) and no second
      stack. The new policy text comes from its owner and lands byte for byte,
      as in [[0303]]. Merge and deploy wait for that text. This reverses the
      "no third-party scripts, ever" rule from [[0162]] and [[0193]].
  - date: "2026-09-28"
    status: active
    who: stkrolikiewicz
    note: >
      Snippet, footer "Cookie settings" link and corrected comments committed
      on `feat/0316_…` (portal 266 tests, typecheck, lint and build green; not
      pushed). Measured: the explorer's GTM fires GA and sets `_ga` before any
      consent, with no Consent Mode signal (Context). Open decision: gate
      consent in the portal's own page (a consent default of denied plus a
      HubSpot listener that updates it), in the GTM container and HubSpot
      settings for the whole host, or both. The explorer has the same defect
      today.
---

# Add Google Analytics to the portal through the explorer's GTM container and HubSpot consent banner

## Summary

The portal (`/api/*` on `sorobanscan.rumblefish.dev`) loads no analytics.
Its visits are counted only in the explorer distribution's CloudFront logs
(explorer 0576), and those logs miss every SPA navigation. The explorer root
already runs GTM → GA4 `G-DFMXSJQ9DR` behind HubSpot's consent banner. The
portal copies that setup, so one consent choice and one GA property cover the
whole host.

## Context

Verified on 2026-09-28:

- **The RFP and the design system say nothing about analytics or cookies.**
  The only limits were our own. The first is the rule from [[0162]] and
  [[0193]] (`web/portal/index.html:30`). The second is the current privacy
  policy: §5 says there are no analytics cookies, and §7 names only AWS and
  Discord as recipients.
- **No new party gets access to the key page.** API Gateway allows
  credentialed calls to the portal routes from exactly one origin,
  `https://sorobanscan.rumblefish.dev` (`infra/envs/production.json:23`). The
  explorer's GTM and HubSpot already run on that origin, so any tag in that
  container can already call the portal routes with a signed-in visitor's
  cookie. Loading the same container on the portal gives nobody access they
  do not already have.
- **Nothing enforces a CSP.** The explorer distribution `EA2TLS5SS5M87`
  deliberately sets none (explorer `infra/src/lib/stacks/delivery-stack.ts:229`).
  The portal comments that cite `default-src 'self'`
  (`theme/fonts.css`, `landing/DiscordIcon.tsx`) state an intent, not the live
  state.
- **The OAuth `code` and `state` never reach an analytics page.** Discord
  returns to `/api/auth/callback`, which is handled by the backend and answers
  303 to `/api/`. The SPA's own query parameters (`?issue=`, `?signin=`) carry
  nothing secret.
- **The explorer's GTM does not wait for consent** (measured 2026-09-28 in a
  browser in Poland). The setup: cookies cleared, `sorobanscan.rumblefish.dev/`
  reloaded, HubSpot banner visible and untouched. The page still sent a GA4
  `page_view` (`region1.google-analytics.com/g/collect`,
  `tid=G-DFMXSJQ9DR`), set `_ga` and `_ga_DFMXSJQ9DR`, and HubSpot sent
  `track.hubspot.com/__ptq.gif`. The dataLayer holds no `consent default`,
  and `gcd=13l3l3l2l1l1` means no Consent Mode signal at all. The explorer's
  own policy says analytics are "activated only after you provide consent",
  so it is already out of step with it. Copying the container 1:1 would give
  the portal the same defect.

## Implementation Plan

1. `web/portal/index.html`: add the GTM head snippet, the noscript iframe and
   the HubSpot loader verbatim from explorer `web/index.html`. Replace the
   "No third-party scripts, ever" comment with this decision.
2. Portal footer: add a "Cookie Settings" link that pushes
   `['showBanner']` onto `window._hsp`, copied from explorer
   `libs/ui/src/layout/Footer.tsx`. It is the only way to withdraw consent.
3. Correct the CSP and third-party claims in `theme/fonts.css` and
   `landing/DiscordIcon.tsx`. The fonts stay self-hosted.
4. Replace `privacy/privacy-policy.md` with the new text byte for byte, keep
   the draft under `sources/`, and bump `POLICY_DATED`.
5. Deploy with `make -C infra sync-portal-explorer`, with an explicit yes, and
   only together with step 4.

## Acceptance Criteria

- [ ] `/api/` loads `GTM-TBF2GP5S` and HubSpot 8102665 with the same snippet
      as the explorer root
- [ ] In a fresh browser profile, before consent there is no
      `google-analytics.com/g/collect` request and no `_ga` cookie. After
      accepting, a page_view for an `/api/…` path is sent.
- [ ] Consent given on the explorer root also applies on `/api/`, and the
      reverse
- [ ] "Cookie Settings" in the portal footer re-opens the banner, with a test
- [ ] No GA hit carries `code=` or `state=` in `page_location`
- [ ] The new policy text is live in the same deploy as the snippet, and
      `POLICY_DATED` is bumped
- [ ] Comments that claim no third-party scripts or an enforced CSP are
      corrected
- [ ] Portal tests, typecheck and lint pass

## Notes

- `vite dev` on localhost would also load the production container. The
  explorer accepts that. Filter `localhost` in the GA property if it turns out
  to be noise.
- After deploy, update the "portal traffic only in CloudFront logs" knowledge:
  the logs stay, but they are no longer the only record.
