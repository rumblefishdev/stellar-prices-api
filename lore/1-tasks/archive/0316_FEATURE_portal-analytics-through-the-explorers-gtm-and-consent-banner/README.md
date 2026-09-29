---
id: "0316"
title: "Add Google Analytics to the portal through the explorer's GTM container and HubSpot consent banner — the privacy policy changes in the same deploy"
type: FEATURE
status: completed
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
  - date: "2026-09-29"
    status: completed
    who: stkrolikiewicz
    note: >
      Merged in #362 (01f1ef24) and deployed with `make -C infra
      sync-portal-explorer` at 10:49 UTC (invalidation
      ISCA1BYSUT18XLC5J4SSBKPED). Production serves the build of 01f1ef24
      (`index-BXqdF69r.js`). 7 of 8 criteria met, measured live. The OAuth
      `code`/`state` check needs a Discord sign-in and was not run. Beyond the
      plan: sbe 0589's Consent Mode wiring and sbe 0593's GTM blocklist were
      repeated here, and the owner's "Cookies and Analytics" section went into
      §5. Portal 269 tests (+3).
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

- [x] `/api/` loads `GTM-TBF2GP5S` and HubSpot 8102665 with the same snippet
      as the explorer root. Live HTML checked after deploy.
- [x] Consent behaviour on `/api/` matches the explorer root, measured in a
      fresh browser profile the same way on both. Once sbe 0589 lands, that
      means no `_ga` before consent. Live on 2026-09-29 with the banner
      untouched: no cookies at all, `consent default denied`, and one
      `page_view` with `gcs=G100`, which is a cookieless ping.
- [x] Consent given on the explorer root also applies on `/api/`, and the
      reverse. Measured live in one direction: "Decline All" on `/api/`, then
      the root showed no banner and sent `update denied` on load. The other
      direction uses the same host cookie (`__hs_cookie_cat_pref`). "Accept"
      was not clicked on production.
- [x] "Cookie Settings" in the portal footer re-opens the banner, with a test
      (`app.spec.tsx`). The link is live.
- [ ] No GA hit carries `code=` or `state=` in `page_location`. Not measured:
      it needs a Discord sign-in. By design the callback is answered by the
      backend with a 303 to `/api/`, so no page carrying them loads GTM.
- [x] The new policy text is live in the same deploy as the snippet, and
      `POLICY_DATED` is bumped. Live bundle: "Cookies and Analytics",
      "29 September 2026", and the removed sentence is gone.
- [x] Comments that claim no third-party scripts or an enforced CSP are
      corrected (`fonts.css`, `DiscordIcon.tsx`, `main.tsx`,
      `vite.config.mts`)
- [x] Portal tests, typecheck and lint pass (269 tests)

## Implementation Notes

- `web/portal/index.html`: GTM head snippet, the noscript iframe and the
  HubSpot loader, verbatim from the explorer. Before GTM: Consent Mode
  denied by default and `gtm.blocklist: ['customScripts']`. After HubSpot: an
  `addPrivacyConsentListener` that forwards the banner's categories.
- `landing/Chrome.tsx`: "Cookie settings" in the footer, pushing
  `['showBanner']` onto `window._hsp`. Tested in `app.spec.tsx`.
- `src/consent-mode.spec.ts` (new): runs the inline scripts. It checks that the
  denied default and the blocklist come before GTM, and that categories map to
  flags. Mutation-checked for both the default and the blocklist.
- `privacy/privacy-policy.md` §5 and `POLICY_DATED`: see Notes.
- Deploy on 2026-09-29, 10:49 UTC, from 01f1ef24 with `soroban-admin`. The
  Discord guild check passed (897514728459468821). S3 sync and `/api/*`
  invalidation followed. The live `index-BXqdF69r.js` matches the local build
  hash.
- The deploy also shipped three changes that had merged since the last portal
  deploy. 0309: the quick start and `openapi.json` describe the
  `404 not_found` for an unknown route, which production already returns.
  0311: the usage request timeout goes from 15 s to 20 s.

## Design Decisions

### From Plan

1. **The explorer's mechanism, no second stack.** One GTM container, one GA
   property and one consent choice for the whole host.
2. **Policy text in the same deploy as the snippet.** The page never claimed
   "no analytics" while loading GA.

### Emerged

3. **Consent gating instead of a 1:1 copy of the ungated setup.** The plan
   was to copy the explorer's configuration, defect included, and fix it
   later. sbe 0589 landed first (2026-09-28), so its Consent Mode wiring went
   in before the first deploy.
4. **GTM blocklist (sbe 0593).** Loading GTM on the page that renders the key
   is safe only if the container cannot run arbitrary code there. The
   explorer's copy is in sbe #537.
5. **The policy amendment went into §5, not over the whole document.** The
   owner sent one section. It follows the still-true session-cookie
   paragraphs, and only the sentence it contradicts was removed. The owner
   confirmed that the rest of the policy is unchanged.
6. **The task became a directory** to keep the owner's text verbatim under
   `sources/`, as in [[0303]].

## Issues Encountered

- **The live portal was not built from develop.** The build of 395c03ba (the
  0311 branch on 2026-09-24) reproduces the live asset hashes byte for byte,
  so that is what production served before this deploy. The deploy's real
  diff was therefore 395c03ba → 01f1ef24, which is why 0309 and 0311 changes
  shipped with it.
- **GitHub marked #362 as conflicting while a local merge was clean.** The
  task file had been moved to a directory on the branch and edited on
  develop. Fixed with a plain merge of develop into the branch.
- **The pre-push hook needs GNU `realpath`.** The lambda-deploy-guard test
  refuses BSD `realpath`. Pushed with Homebrew coreutils' `gnubin` first on
  `PATH` (infra/README.md, Prerequisites), with the hooks still running.

## Future Work

- Cookieless `gcs=G100` pings before consent: only a GTM-side "require
  `analytics_storage`" on the GA tag stops them. That is shared with the
  explorer (sbe 0589, option 2) and is not a portal change.
- The `code=`/`state=` criterion above is worth measuring on the next
  Discord sign-in against production.

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
  the logs stay, but they are no longer the only record. That knowledge lives
  in sbe `docs/architecture/infrastructure/infrastructure-overview.md` (the
  task 0576 paragraph). Corrected on sbe develop in bc0c1815.
