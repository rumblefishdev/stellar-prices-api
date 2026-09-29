---
id: "0316"
title: "Add Google Analytics to the portal through the explorer's GTM container and HubSpot consent banner — the privacy policy changes in the same deploy"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0303", "0193", "0162", "0305", "0194"]
tags: [layer-frontend, portal, legal, analytics, epic-self-service-onboarding, priority-medium, effort-small]
links:
  - "sources/privacy-policy-cookies-and-analytics-2026-09-29.md"
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
  - date: "2026-09-28"
    status: active
    who: stkrolikiewicz
    note: >
      Decided: consent gating is fixed in soroban-block-explorer, opened there
      as sbe 0589 (backlog). The portal repeats the explorer's configuration
      1:1, including today's ungated GTM (sbe 0451, decision 7), and repeats
      0589's change once it lands. Whoever writes the new portal policy text
      must be told that GA currently fires before consent.
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
  so it is already out of step with it. Copying the container 1:1 gives the
  portal the same defect. By decision the fix is made in the explorer first
  (sbe 0589) and then repeated here. **Since fixed:** sbe 0589 went live on
  2026-09-28 at 18:58 UTC, and this task repeats it (see Notes).

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
5. When sbe 0589 lands, repeat its `index.html` change here verbatim.
6. Deploy with `make -C infra sync-portal-explorer`, with an explicit yes, and
   only together with step 4.

## Acceptance Criteria

- [ ] `/api/` loads `GTM-TBF2GP5S` and HubSpot 8102665 with the same snippet
      as the explorer root
- [ ] Consent behaviour on `/api/` matches the explorer root, measured in a
      fresh browser profile the same way on both. Once sbe 0589 lands, that
      means no `_ga` before consent.
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

- Consent Mode wiring from explorer 0589 (`web/index.html`, deployed
  2026-09-28) is repeated in `web/portal/index.html`. Everything is denied
  before GTM loads, and a HubSpot `addPrivacyConsentListener` forwards the
  banner's categories. `consent-mode.spec.ts` runs those inline scripts. On a
  `vite preview` build, a fresh visitor gets no cookies at all, and "Accept
  All" grants consent and sets `_ga`. GA still sends cookieless `gcs=G100`
  pings before consent, so the criterion "no `g/collect` request" needs the
  GTM-side setting (explorer 0589, option 2).
- Before GTM loads, the same script pushes
  `{'gtm.blocklist': ['customScripts']}` (explorer task 0593). GTM then runs
  no Custom HTML tag and no Custom JavaScript variable, whatever the container
  holds. A tag published there can therefore not read the key this page
  renders. The container's only tag today is the Google tag for GA4, and on a
  `vite preview` build it still sends hits after consent.
- **Policy text (2026-09-29).** The owner sent one section, "Cookies and
  Analytics", instead of a whole new document. It is kept verbatim in
  `sources/privacy-policy-cookies-and-analytics-2026-09-29.md`. It went into
  §5, which is renamed "Cookies and Analytics". The session-cookie
  paragraphs stay as they were, and the new text follows them word for word.
  One sentence was removed because it is no longer true: "…portal does not
  use cookies or similar technologies for advertising, behavioral tracking or
  analytics purposes." The rest of the policy is unchanged, as the owner
  confirmed. `POLICY_DATED` is now 29 September 2026. The new text promises
  consent before GA cookies, and that matches what the page does.
- `vite dev` on localhost would also load the production container. The
  explorer accepts that. Filter `localhost` in the GA property if it turns out
  to be noise.
- After deploy, update the "portal traffic only in CloudFront logs" knowledge:
  the logs stay, but they are no longer the only record.
