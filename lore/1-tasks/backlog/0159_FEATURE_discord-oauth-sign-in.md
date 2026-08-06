---
id: "0159"
title: "Discord OAuth sign-in for the onboarding portal"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0156", "0158", "0160", "0161", "0162"]
tags: [layer-backend, priority-high, effort-large, milestone-M3, epic-self-service-onboarding, discord, oauth, auth, secrets]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../infra/src/lib/stacks/compute-stack.ts"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-08-06
    status: backlog
    who: akot
    note: >
      The authentication half of the epic. Splits from [[0160]] deliberately:
      this task establishes who the caller is, [[0160]] does things on their
      behalf. Blocked in practice by [[0156]], which decides whether guild
      membership is checked.
---

# Discord OAuth sign-in

## Summary

A visitor signs in with Discord and the portal knows who they are. That
identity — the Discord user ID — *is* the account, per the epic: there is no
separate registration, no email, no password, and no manual approval.

This task delivers the sign-in itself and the session that later calls carry.
Issuing keys, revealing them, showing usage and rotating them are [[0160]].

## Context

The portal is a static site on S3 + CloudFront, so nothing in it can hold a
secret or call AWS. The epic already recognises this for the dashboard: the
`GetUsage` call needs IAM credentials and "cannot be called from the browser
directly", so a small backend Lambda is required — one the component list did
not previously have. The same backend terminates the OAuth flow, because
exchanging an authorization code requires the Discord client secret, which a
browser must never see.

Discord OAuth authenticates a *Discord account*. Whether it also proves anything
about Stellar Discord membership is exactly the question [[0156]] answers, and
it decides whether this flow requests the `guilds` scope or only `identify`.

## Implementation

**From the epic**

- Discord OAuth as the only sign-in mechanism. No email, no captcha, no
  separate account record.
- The Discord user ID is the account key, matching the registry in [[0158]].
- No manual approval anywhere in the flow.

**Follows from the epic, but not stated in it**

- **Authorization Code flow with PKCE**, `identify` scope only — request
  `guilds` solely if [[0156]] concludes the membership check is what the abuse
  story depends on. Ask for nothing else: `email` in particular buys us data we
  have decided not to hold.
- **Client secret in Secrets Manager**, never an environment variable. ADR 0007
  set that precedent for the mTLS material and Tranche 3 AC 6 audits it
  explicitly ("no secrets in env vars").
- **A `state` parameter tied to the session**, verified on callback. Without it
  the callback is an open redirect / login-CSRF surface — and this is a public
  route with no key in front of it.
- **Session as a signed, `HttpOnly`, `Secure`, `SameSite` cookie** carrying the
  Discord user ID and an expiry. Do not persist Discord access or refresh
  tokens: once the identity is read, the OAuth tokens have no further use here,
  and storing them creates a credential we would have to protect and rotate.
- **Routes are anonymous** — a visitor signing in has no API key by definition,
  so these cannot sit behind `apiKeyRequired`. That means they need their own
  method-level throttle, the same reasoning [[0124]] applied to
  `/api-docs-json`.
- **Where the routes live is a decision to record**: same API Gateway under a
  distinct path prefix, or a separate API. Same gateway keeps one hostname,
  one deploy and one place to reason about throttling; a separate API keeps the
  partner-facing surface free of portal routes. Pick one and write down why —
  it affects [[0161]]'s CloudFront path layout and whether CORS ([[0126]]) is
  needed at all.
- **The Discord application is a manual prerequisite.** Someone registers it in
  the Discord Developer Portal and configures the redirect URI, which must match
  exactly and therefore changes when the custom domain lands ([[0126]]). Record
  the ordering so sign-in does not break silently on the domain cutover.

## Acceptance Criteria

- [ ] A visitor completes Discord sign-in and the backend resolves their Discord
      user ID
- [ ] Client secret lives in Secrets Manager; no secret in any env var or in the
      static site bundle
- [ ] `state` verified on callback; a mismatched or replayed callback is
      rejected
- [ ] Session cookie is `HttpOnly` + `Secure`, scoped and expiring; Discord
      tokens are not persisted
- [ ] Sign-in routes require no API key and carry their own throttle
- [ ] Route placement (same gateway vs separate API) decided and recorded
- [ ] Discord app registration and redirect-URI ownership documented, including
      what changes when the custom domain lands
- [ ] Scope set matches [[0156]]'s conclusion

## Notes

- The epic's non-goal is worth restating in code comments: a user who later
  leaves the Discord server keeps their key. Sign-in proves identity at the
  moment of issuance and nothing afterwards.
- Sequencing: [[0158]] before this (the callback writes the registry record),
  [[0156]] before the scope decision, [[0161]] alongside it for the redirect
  URI.
