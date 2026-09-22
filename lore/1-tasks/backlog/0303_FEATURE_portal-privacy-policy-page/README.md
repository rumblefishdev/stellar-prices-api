---
id: "0303"
title: "Publish the Prices API privacy policy as a portal page and link it from the footer — the footer's 'Privacy policy' is dead text and the corporate policy does not describe the portal"
type: FEATURE
status: backlog
related_adr: ["0010"]
related_tasks: ["0193", "0301", "0159", "0187", "0189", "0157"]
tags: [layer-frontend, priority-high, effort-small, epic-self-service-onboarding, portal, legal]
links:
  - "sources/privacy-policy-draft-2026-09-22.md"
  - "../../../web/portal/src/landing/Chrome.tsx"
  - "../../../web/portal/src/landing/links.ts"
history:
  - date: "2026-09-22"
    status: backlog
    who: stkrolikiewicz
    note: >
      Filed from the footer walk in [[0301]]. A dedicated privacy policy for
      the Prices API arrived today (`sources/`); the corporate policy on
      rumblefish.dev describes the marketing site (HubSpot, GA, pixel,
      recruitment) and none of what the portal processes (Discord id and
      username, API key, usage, logs). The page is the portal's, at
      `/api/privacy`, because the portal is what collects the data and the
      draft itself says the policy is published "through the portal".
---

# Publish the Prices API privacy policy as a portal page

## Summary

The portal's footer renders "Privacy policy" as plain text ([[0193]]: "a
footer link to a 404 is worse than one that is plainly not wired"), and the
only policy the company publishes — rumblefish.dev's — is about the marketing
website. A policy written for the Prices API now exists
(`sources/privacy-policy-draft-2026-09-22.md`). Publish it as a page of the
portal and link it wherever the portal names it.

## Context

What the portal processes, verified against the code on 2026-09-22 and
matching the draft: Discord OAuth with scopes `identify guilds.members.read`;
the Discord account id stored as the API key's name in API Gateway (the key
registry); the username in the signed session cookie only (`HttpOnly`,
`SameSite=Lax`, 24 h — `SESSION_TTL_SECS`); membership and account age
checked live and not stored (ADR 0010); per-key status and monthly usage;
api-handler logs in CloudWatch with a 30-day retention
(`PRICES_LAMBDA_LOG_RETENTION`); revoke is `UpdateApiKey(enabled=false)`, the
record stays. No analytics or tracking script in the bundle, so the session
cookie needs no consent banner. The explorer is a different case: it loads
HubSpot and GTM, has no accounts, and the corporate policy covers it.

## Implementation

- A route `/privacy` in the portal (served at `/api/privacy`), the same
  `DocPage` shape as the API reference and the quick start, rendering the
  policy's headings and text. One source of truth for the text in the repo;
  the draft in `sources/` is the input, not the rendered copy.
- `landing/links.ts`: a `PRIVACY_ROUTE` / `PRIVACY` pair like the quick
  start's; `landing/Chrome.tsx`: the footer's "Privacy policy" becomes a
  router link. Any other place the portal names a privacy policy (the
  sign-in card's legal line, if it returns) points at the same route.
- Before publishing, settle the three things the draft leaves open:
  1. the contact address — the draft says `hello@rumblefish.pl`, the
     corporate policy `hello@rumblefishdev.com`; one of them is right;
  2. whether IP addresses are in fact recorded (no access logs are
     configured on the API; the Lambda's execution logs are what exists);
  3. the "Payments" section describes a paid model the portal does not have
     ("Contact us for commercial plans — no in-app upgrade flow"); keep it
     only if it is wanted ahead of time.
- Who approved the text, and when — recorded here before the page ships.

## Acceptance Criteria

- [ ] `/api/privacy` renders the approved policy, reachable from the
      footer's "Privacy policy" on every page that has the footer
- [ ] The three open points above are decided and the text matches the
      decision (address, IP, payments)
- [ ] Every factual statement in the page matches the code: scopes, cookie
      attributes and lifetime, log retention, what revoke does
- [ ] No hostname other than ours and no tracking script in the bundle
      (the criterion [[0233]] already holds)
- [ ] The explorer's footer keeps the corporate policy; nothing there changes

## Notes

- The draft arrived on 2026-09-22 as a file; its author and review status
  are not recorded — the first step is to find out who owns the text.
- The corporate policy names `www.rumblefishdev.com` as "the Website", so
  the explorer's link to it carries a small scope gap of its own; out of
  scope here, noted for whoever owns that page.
