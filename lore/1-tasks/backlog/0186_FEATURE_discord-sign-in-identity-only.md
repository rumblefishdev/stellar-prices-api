---
id: "0186"
title: "Sign in with Discord — identity only, scope identify, session cookie"
type: FEATURE
status: backlog
related_adr: ["0007", "0008", "0010"]
related_tasks: ["0183", "0159", "0184", "0185", "0187", "0189", "0194"]
tags: [layer-backend, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, discord, oauth, auth, secrets, slice-3]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0159_FEATURE_discord-oauth-sign-in.md"
  - "../../2-adrs/0010_discord-account-model-and-abuse-barrier.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Third slice, the first half of [[0159]]. Scope is `identify` alone, which
      is what makes this startable: the only manual prerequisite is a registered
      Discord application. No guild, no second account, no measurement of an
      undocumented response shape — all of that is [[0189]].
---

# Discord sign-in — identity only

## Summary

**Story:** *as a visitor, I click "Sign in with Discord", authorise, and the
portal shows my Discord username — so I know the login works.*

The OAuth round-trip, the session, and an ugly page that proves it. No key is
issued, no eligibility is checked, nothing is stored.

## Context

[[0159]] delivered sign-in and entitlement as one task, and so inherited
entitlement's prerequisites: two guilds, two accounts, and five undocumented
Discord behaviours to measure first. None of that is needed to learn a user's
ID. Splitting here is most of what the epic's re-slice buys.

The backend is the **existing `prices-api` axum router** (2026-08-07 meeting,
ADR 0008) — no new crate, no second gateway integration, no second build.

## Implementation

- Routes under **`/api-tokens/api/`**, same origin as [[0185]]'s bundle behind
  [[0184]]'s distribution. `/auth/login`, `/auth/callback`, `/auth/me`,
  `/auth/logout`.
- **Scope: `identify` only.** `guilds.members.read` is added by [[0189]] and is
  a change to the Developer Portal registration as well as to the authorize URL,
  so it is that task's to make. Never `guilds`, never `email` (ADR 0010).
- **Authorization Code flow with PKCE.** Caveat recorded by [[0156]]: Discord's
  OAuth2 topic page never mentions PKCE, and for a confidential server-side
  client with an HTTPS redirect it is not documented as required. Implement it
  anyway; do not expect the docs to describe the server's behaviour.
- **`state` tied to the session and verified on callback.** These are public,
  keyless routes — without it the callback is an open-redirect and login-CSRF
  surface. Give `state` an action slot now even though there is only one action:
  [[0189]] needs it to bind the intended action, and adding it later means
  re-deriving the signing format.
- **Client secret in Secrets Manager**, never an environment variable (ADR 0007
  precedent, audited by Tranche 3 AC 6). Follow `compute-stack.ts`: the env var
  carries the secret *name*, computed by a shared helper alongside
  `mtlsSecretName`, and the value is fetched through the Parameters & Secrets
  extension layer the Lambda already loads.
- **Session: signed, `HttpOnly`, `Secure`, `SameSite=Lax` cookie** carrying the
  Discord user ID and an expiry. `Lax` works because portal and endpoints are
  same-origin ([[0184]]), and it still permits the top-level GET navigation
  Discord uses to return to the callback — which `Strict` silently breaks.
- **Do not persist Discord access or refresh tokens.** Once the ID is read they
  have no use here, and storing them creates a credential to protect and rotate.
- **These routes cannot sit behind `apiKeyRequired`** — a visitor signing in has
  no key by definition — so they need their own method-level throttle, the same
  reasoning [[0124]] applied to `/api-docs-json`.
- **The `methodSettings` entries must be declared outside the `cacheEnabled`
  branch**, the way `apiDocsSettings` already is. `api-gateway-stack.ts` builds
  the full array only inside `if (cacheEnabled)` and its `else` emits just
  `[stageWideThrottle, apiDocsSettings]` — entries added to the `if` arm alone
  vanish wherever `apiGatewayCacheEnabled` is false, leaving anonymous keyless
  routes unthrottled in exactly the configuration where every request is a
  billed Lambda invocation. The existing code comments this trap; inherit the
  requirement rather than rediscover it.
- **CloudFront must forward the session cookie and not cache these paths.** The
  managed default cache policy strips cookies, so with it the session never
  reaches the origin and every request reads as signed-out. This is the first
  slice with a session, so it is the first slice where the policy is writable —
  [[0184]] deliberately left it out.
- Frontend: a button and a line of text saying who you are. Ugly. Signed-out and
  "sign-in cancelled" as plain text, not screens.

**Manual prerequisite, owned by Adam:** register the Discord application, set
the redirect URI to match `/api-tokens/api/auth/callback` on [[0184]]'s
hostname. The docs say registration is required but do **not** state that
matching is character-exact — assume it is, verify once. Re-pointing it when the
custom domain lands ([[0195]], [[0126]]) is his too; record the ordering so
sign-in does not break silently on the cutover.

## Acceptance Criteria

- [ ] **Ships closed.** With `portal-enabled=false` ([[0183]]) sign-in resolves
      the identity and *then* refuses a non-allowlisted user; an allowlisted
      Discord ID completes the round-trip normally
- [ ] A visitor completes the round-trip and the page shows their Discord
      username and ID
- [ ] Client secret is in Secrets Manager; no secret in any env var or in the
      bundle
- [ ] `state` is verified; a mismatched or replayed callback is rejected
- [ ] `state` carries an action slot, signed, even though only one action exists
- [ ] Session cookie is `HttpOnly` + `Secure` + `SameSite=Lax`, scoped and
      expiring; no Discord token is persisted
- [ ] Sign-in routes require no API key and carry their own throttle, declared
      outside the `cacheEnabled` branch
- [ ] A signed-in request reaches the origin still signed in through CloudFront
- [ ] Scope requested is exactly `identify`
- [ ] App registration and redirect-URI ownership written into the deploy-prep
      runbook, including what changes at the domain cutover

## Notes

- Sequencing: needs [[0184]] for the hostname, [[0185]] for somewhere to render.
  Blocks [[0187]] and [[0189]].
- The `methodSettings` array is keyed by `resourcePath + httpMethod` and assigned
  wholesale. Later slices add portal routes to it; this task owns its shape, and
  [[0194]] audits the finished array.
