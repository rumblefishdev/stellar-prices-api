---
id: "0303"
title: "Publish the Prices API privacy policy as a portal page and link it from the footer — the footer's 'Privacy policy' is dead text and the corporate policy does not describe the portal"
type: FEATURE
status: completed
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
      `/api/privacy-policy`, because the portal is what collects the data and the
      draft itself says the policy is published "through the portal".
  - date: "2026-09-22"
    status: active
    who: stkrolikiewicz
    note: >
      Activated with the three open points decided the same day: the
      contact address stays `hello@rumblefish.pl` for now, IP addresses are
      recorded in X-Ray for 30 days (measured, sentence to be plain), the
      Payments section stays. Route `/privacy-policy`.
  - date: "2026-09-22"
    status: active
    who: stkrolikiewicz
    note: >
      Implemented on `feat/0303_portal-privacy-policy-page`, stacked on
      `fix/0301_portal-navigation` (both edit the footer and `links.ts`):
      the reviewed document kept as `web/portal/src/privacy/privacy-policy.md`,
      read by a small parser for the constructs it uses, rendered at
      `/privacy-policy` in the doc-page shape, linked from the footer. The
      page carries a version date the draft did not. 216 portal tests,
      typecheck, lint green; production bundle clean. Open: PR, review of
      the text by its owner, deploy.
  - date: "2026-09-22"
    status: completed
    who: stkrolikiewicz
    note: >
      Shipped: PR #338 merged (`a5fa5f98`), bundle synced 14:02 UTC;
      `/api/privacy-policy` is live behind the explorer's basic-auth gate,
      chunk `PrivacyPolicy-DTzXF74o.js`. All 5 criteria met. Not done here:
      the text's owner has not reviewed the rendered page — the text is the
      delivered draft byte for byte, so a later edit is a one-file change
      plus a bump of `POLICY_DATED`.
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

- A route `/privacy-policy` in the portal (served at `/api/privacy-policy`,
  no trailing slash — the portal's routes have none), the same `DocPage`
  shape as the API reference and the quick start, rendering the policy's
  headings and text. The name matches the explorer's footer and the
  corporate site (`/privacy-policy/`), decided 2026-09-22. One source of truth for the text in the repo;
  the draft in `sources/` is the input, not the rendered copy.
- `landing/links.ts`: a `PRIVACY_ROUTE` / `PRIVACY` pair like the quick
  start's; `landing/Chrome.tsx`: the footer's "Privacy policy" becomes a
  router link. Any other place the portal names a privacy policy (the
  sign-in card's legal line, if it returns) points at the same route.
- Before publishing, settle the three things the draft leaves open:
  1. ~~the contact address~~ — **decided 2026-09-22: keep the draft's
     `hello@rumblefish.pl` for now.** rumblefish.dev publishes three
     addresses: `hello@rumblefish.dev` in every header and footer,
     `hello@rumblefish.pl` in the contact form, `hello@rumblefishdev.com`
     only inside the corporate policy's text (the old domain). To revisit
     if the `.pl` and `.dev` boxes turn out not to reach the same people;
  2. ~~whether IP addresses are in fact recorded~~ — **measured 2026-09-22:
     yes, in X-Ray.** Not in the api-handler's CloudWatch logs (a request
     from this machine to `/api/config` and `/api/auth/me` left START/END
     lines and nothing else); the API stage has no access logs and no
     execution logs (`accessLogSettings: null`, `dataTraceEnabled: false`,
     no `API-Gateway-Execution-Logs_*` group); no WAF on the stage or on
     CloudFront; the explorer distribution and the bundle bucket log
     nothing. But `tracingEnabled: true` (M3's X-Ray criterion) puts the
     client address on every sampled request's API Gateway segment —
     this machine's address was there for both calls — and X-Ray keeps
     traces for 30 days (fixed by AWS). Sampling: 1 request/s reservoir
     plus 5 %. So the draft's hedged sentence is true; the page can say it
     plainly: "IP addresses are recorded in request traces (AWS X-Ray) for
     a sample of requests and kept for 30 days";
  3. ~~the "Payments" section~~ — **decided 2026-09-22: it stays**, written
     ahead of a paid model the portal does not have yet ("Contact us for
     commercial plans — no in-app upgrade flow"); the section is hedged with
     "may" throughout, so it is not false today.
- Who approved the text, and when — recorded here before the page ships.

## Acceptance Criteria

- [x] `/api/privacy-policy` renders the approved policy, reachable from the
      footer's "Privacy policy" on every page that has the footer
- [x] The three open points above are decided and the text matches the
      decision (address, IP, payments)
- [x] Every factual statement in the page matches the code: scopes, cookie
      attributes and lifetime, log retention, what revoke does
- [x] No hostname other than ours and no tracking script in the bundle
      (the criterion [[0233]] already holds)
- [x] The explorer's footer keeps the corporate policy; nothing there changes

## Notes

- The draft arrived on 2026-09-22 as a file; its author and review status
  are not recorded — the first step is to find out who owns the text.
- The corporate policy names `www.rumblefishdev.com` as "the Website", so
  the explorer's link to it carries a small scope gap of its own; out of
  scope here, noted for whoever owns that page.

## Implementation Notes (2026-09-22)

Branch `feat/0303_portal-privacy-policy-page`, stacked on [[0301]]'s branch
because both change the footer and `links.ts`; merge #335 first.

- **Text**: `web/portal/src/privacy/privacy-policy.md` is the delivered draft
  byte for byte (also in `sources/` here as the input). It is imported with
  Vite's `?raw` and read by `privacy/policy.ts` — not a Markdown parser, a
  reader for what the document uses: `#`/`##`/`###` headings wrapped in
  `**`, paragraphs, `*` bullets, `**bold**`, `` `code` ``, backslash
  escapes (`1\.`, `\+48`) and the address block's hard line breaks.
  Anything else would render as its literal text, and the spec would show
  it.
- **Page**: `privacy/PrivacyPolicy.tsx`, lazy like the API reference, in the
  `DocPage` shape: the document's two opening paragraphs are the page lede,
  each section's first paragraph is its lede under the heading, the rail
  lists the sixteen sections. `Version of 22 September 2026` above the
  title (`POLICY_DATED`).
- **Route**: `/privacy-policy` in `app.tsx` (`PrivacyPolicyRoute`, same
  chrome rule as `/docs`); `PRIVACY_POLICY_ROUTE` / `PRIVACY_POLICY` in
  `links.ts`; the footer's "Privacy policy" is a router link; the dashboard
  bar accepts `current="privacy-policy"` and underlines nothing there.
- **Tests**: `privacy/PrivacyPolicy.spec.tsx` — sixteen numbered sections in
  order, unique ids, no notation left, every bullet and sub-heading of the
  source kept, the page renders each section as a heading with a rail
  entry and `**`/`` ` `` as `<strong>`/`<code>`; `app.spec.tsx` — the route
  under the landing bar and the footer link. 216 tests, typecheck and lint
  green.
- **Bundle**: fresh production build; the policy is its own chunk; no
  hostname but ours and the footer's, no tracking script.

## Issues Encountered

- **Prettier rewrote the policy on the first commit.** lint-staged formats
  every staged non-Rust file, and Prettier's Markdown style turns `*`
  bullets into `-`. The reader knew only `*`, so the page showed 79
  paragraphs beginning with "- " — and the spec did not notice, because it
  counted bullets off the same reformatted file. Three fixes: the file is
  in `.prettierignore` (reviewed legal text is never reformatted, so it
  stays byte for byte the delivered draft), the reader accepts `-` too, and
  the spec pins the draft's literal counts (79 bullets, 3 sub-headings) and
  asserts no marker survives as text. Found by looking at the page on the
  dev server, not by the tests — the lesson is in the spec's comment.
- **The rail clipped four section titles.** `Toc`'s top-level entries were
  `white-space: nowrap` at desktop width; the reference's and the quick
  start's labels are short, the policy's are not ("Transfers Outside the
  European Economic Area"). Top-level entries wrap now, like nested ones;
  the other two pages look the same because their labels fit.

## Design Decisions

### From Plan

1. **The document stays a `.md` file and the page reads it.** A diff of the
   file is a diff of the policy, which is what a reviewer of legal text
   needs; a retyping into JSX would drift and could not be diffed against
   the delivered version.

### Emerged

2. **A small reader instead of a Markdown dependency.** No Markdown library
   is in the workspace; the document uses six constructs; the reader is
   ~80 lines with a spec that counts the source's bullets and sub-headings.
   A library would also turn any HTML in a future edit into markup on a
   public page; the reader renders unknown notation as text.
3. **A version date on the page.** The draft carries none; a policy that
   cannot say which version a visitor read is a gap of its own.
   `POLICY_DATED` is a constant beside the import, to bump with every edit
   of the file.
4. **The explorer keeps the corporate policy.** This text says the portal
   sets no analytics cookies, which is false of the explorer (HubSpot, GTM);
   one document for the host would need a second part, out of scope here.
