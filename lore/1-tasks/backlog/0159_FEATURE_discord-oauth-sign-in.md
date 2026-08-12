---
id: "0159"
title: "Discord OAuth sign-in for the onboarding portal"
type: FEATURE
status: backlog
related_adr: ["0007", "0010"]
related_tasks: ["0156", "0158", "0160", "0161", "0162", "0170", "0171"]
tags: [layer-backend, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, discord, oauth, auth, secrets]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../packages/prices-api"
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
  - date: 2026-08-07
    status: backlog
    who: akot
    note: >
      Rescoped after the 2026-08-07 meeting: the routes are added to the
      **existing** `prices-api` axum Lambda, not a new one. That removes the
      runtime choice, the second gateway integration and the route-placement
      question, and drops the estimate from large to medium.
  - date: 2026-08-10
    status: backlog
    who: akot
    note: >
      [[0156]] / ADR 0010 settles the two open items here. Scope is
      `identify` + `guilds.members.read` (never `guilds`), plus a snowflake
      account-age minimum. Adam owns app registration and the `stellar_test`
      guild — the "someone" placeholder is gone. Guild ID becomes
      per-environment SSM config. Adds a membership check whose error shape is
      undocumented ([[0171]]).
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
directly", so a backend is required. The same backend terminates the OAuth flow,
because exchanging an authorization code requires the Discord client secret,
which a browser must never see.

**That backend is `prices-api` (2026-08-07 meeting)** — the routes are added to
the existing axum router (ADR 0008, single Lambda serving all route groups),
not to a new function. Practical effect: no new crate, no second gateway
integration, no second build, and the `methodSettings` entries for these routes
have a single owner (this task) rather than being split with [[0160]].

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

- **Scope — settled by [[0156]] / ADR 0010: `identify` + `guilds.members.read`.
  Never `guilds`.** The membership check *is* what the abuse story depends on,
  because under `identify` alone the flow observes nothing but the existence of
  a Discord account (`verified` requires the `email` scope; there is no phone
  field on the OAuth user object at all). `guilds` is rejected on both privacy
  and utility grounds: it returns every server the user belongs to, and its
  partial guild objects carry neither `pending` nor `joined_at`. Ask for nothing
  else — `email` in particular buys us data we have decided not to hold.
- **Authorization Code flow with PKCE.** Caveat from [[0156]]: Discord's OAuth2
  topic page does not mention PKCE at all. It is documented only in the Social
  SDK mobile guide, where it is mandatory for deep-link redirects. For a
  confidential server-side client with an HTTPS redirect it is **not documented
  as required** — implement it anyway, but do not expect the docs to describe
  the server's behaviour.
- **Eligibility is proved per action, not carried in the session (ADR 0010 §8).**
  This task owns the OAuth round-trip; it does **not** own issuance. Concretely:
  `state` carries the intended action, signed, alongside its CSRF role. The
  callback exchanges the code for a **fresh** token, performs the checks below,
  and only then hands off to [[0160]]'s issue or rework logic. Nothing about
  eligibility is stored in the session cookie — a signed "eligible" claim would
  date the verdict to sign-in time and would not survive to a rework weeks later.
  Discord does not re-prompt for consent on repeat authorization of the same
  scopes, so the second and later round-trips are a redirect, not a consent
  screen.

  | Path | Re-auth | Checks |
  | --- | --- | --- |
  | Sign in | — | identity only |
  | Issue a key | **yes** | membership (`pending === false`) + account age |
  | Reveal / usage | no | session only |
  | Rework | **yes** | membership only — age is not re-checked |

  Account age is deliberately **not** re-checked on rework: an account old enough
  once is old enough forever.
- **Membership check.** `GET /users/@me/guilds/{guild.id}/member`, guild ID from
  SSM — **per-environment, not a constant**: `stellar_test` in dev,
  `897514728459468821` in production once [[0170]] lands. The not-a-member
  response shape is **undocumented** (only a generic `404` plus error codes
  `10004`/`10007` exist), so treat only an explicit `10007`/`10004`-style 404 as
  "not a member" and treat 401/403/429/5xx as "unknown, do not deny". Measure it
  first — [[0171]] #1.
- **`pending` is optional (`pending?`).** The docs' presence guarantee is written
  about gateway events, not this route. Handle `undefined` as a third state;
  never read absent as "cleared". Also note `BYPASSES_VERIFICATION` means
  `pending === false` can mean "an admin waved them through" — [[0171]] #2.
- **Minimum account age from the snowflake**, per ADR 0010:
  `(BigInt(id) >> 22n) + 1420070400000n`. Costs no extra scope and no extra
  consent line — the `id` is already in the `identify` response. Use `BigInt`;
  `Number` loses precision above 2^53.
  **Threshold: 5 minutes**, matching Stellar's own `verification_level: 2`.
  **In SSM, not a literal** — the value is expected to be raised if churn is
  ever observed, and that must not need a deploy. Store it as a duration
  (minutes), not a day count: the initial value is minutes and a
  `minAccountAgeDays` parameter would make 5 minutes unrepresentable.
- **This task owns the two new SSM parameters** — their names, their read path
  and their seeding step. ADR 0010 says the guild ID and the age threshold are
  configuration; nobody else in the epic declares where they come from, and left
  unassigned they will be hard-coded — the one thing the ADR forbids. Follow the
  existing key contract (`/prices/{env}/*`, alongside `api-gateway-id` and the
  `mtls-*-secret-name` entries):

  | Parameter | Value |
  | --- | --- |
  | `/prices/{env}/discord-guild-id` | `stellar_test` while building, `897514728459468821` after [[0170]]'s flip |
  | `/prices/{env}/min-account-age-minutes` | `5` |

  **Operator-seeded, not CDK-created — corrected 2026-08-10 after audit.** Do
  **not** write `new ssm.StringParameter` for these. The repo's precedent for
  operator-owned values is explicit: `SecretsStack` *"deliberately does NOT
  create the secrets"* and publishes names only, and `compute-stack.ts` **reads**
  `/prices/{env}/ledger-processor/initial-cursor` because the operator seeds it
  "at deploy prep, like the mTLS secrets… rather than committed config, so it is
  never a stale magic number".

  This is not style. A CloudFormation-managed parameter is CDK-owned, so **the
  next `cdk deploy` would silently restore the committed value** — un-flipping
  production back to the test guild after launch ([[0170]] step 4) and reverting
  any temporary threshold change ([[0164]] check 10). Note also that `envName` is
  typed `'production'` and `infra/envs/` holds only `production.json`: there is
  one parameter whose value is flipped in place, not a per-environment matrix.

  **Read them at runtime via the SSM SDK, not `valueForStringParameter`** — the
  latter resolves through a CFN parameter at deploy, which would make the
  threshold un-tunable without a redeploy and defeat the point of it being a
  parameter. No IAM work is required: `lambda-baseline.ts` already grants
  `ssm:GetParameter*` on `arn:…:parameter/prices/{env}/*` to every prices Lambda.

  What this task owns is therefore the **read path, the documented parameter
  names, and the seeding step in the deploy-prep runbook** — not a CDK resource.
- **Never re-check on session refresh or on the dashboard** (ADR 0010 §8). Reveal
  and usage are session-only and must keep working indefinitely, including for a
  user who has left the guild — a key, once issued, never expires. The two
  exceptions are issue and rework, which carry their own round-trip per the table
  above; rework re-checks membership only, never age.
- **Distinguish "not a member" from "could not tell".** The not-a-member result
  is inferred from an undocumented error shape ([[0171]] #1), so the handler
  must return three outcomes, not two: eligible, ineligible, and unknown. Only
  an explicit `10007`/`10004`-style 404 is ineligible; 401/403/429/5xx is
  unknown and must not issue a key **and** must not tell the user they are not a
  member. [[0162]] renders those differently.
- **Client secret in Secrets Manager**, never an environment variable. ADR 0007
  set that precedent for the mTLS material and Tranche 3 AC 6 audits it
  explicitly ("no secrets in env vars"). Follow the shape already in
  `compute-stack.ts`: the env var carries the **secret name**, computed by a
  shared helper alongside `mtlsSecretName` so it cannot drift, and the value is
  fetched at runtime through the Parameters & Secrets extension layer the Lambda
  already loads.
- **A `state` parameter tied to the session**, verified on callback. Without it
  the callback is an open redirect / login-CSRF surface — and this is a public
  route with no key in front of it.
- **Session as a signed, `HttpOnly`, `Secure`, `SameSite=Lax` cookie** carrying
  the Discord user ID and an expiry. `Lax` is available because the portal and
  its endpoints are same-origin behind one CloudFront distribution ([[0161]]);
  it also still permits the top-level GET navigation Discord uses to return to
  the callback, which `Strict` would silently break. Do not persist Discord
  access or refresh tokens: once the identity is read they have no further use
  here, and storing them creates a credential we would have to protect and
  rotate.
- **Routes are anonymous** — a visitor signing in has no API key by definition,
  so these cannot sit behind `apiKeyRequired`. That means they need their own
  method-level throttle, the same reasoning [[0124]] applied to
  `/api-docs-json`.
- **Route placement — settled 2026-08-07:** same API Gateway, same Lambda,
  mounted under **`/api-tokens/api/`**, and fronted by the same CloudFront
  distribution as the portal bundle ([[0161]]). The browser therefore makes
  same-origin requests: CORS ([[0126]]) never enters the picture for portal
  traffic and the session cookie is not cross-site.

  The prefix follows [[0161]]'s convention — `<app>/*` is an app's bundle,
  `<app>/api/*` is its backend — so a second frontend adds two rows and invents
  nothing. Both halves of the string are load-bearing: the OAuth redirect URI
  registered with Discord must match it exactly, and [[0161]]'s distribution
  must order this behaviour before `/api-tokens/*` or every call here is served
  the SPA bundle instead.
- **The Discord application is a manual prerequisite, owned by Adam Kot**
  (settled by [[0156]] / ADR 0010 — this replaces the "someone" that stood
  here). He registers it in the Discord Developer Portal, configures the
  redirect URI, and owns re-pointing it when the custom domain lands ([[0126]]).
  Record the ordering so sign-in does not break silently on the domain cutover.
  Note the docs say registration is required but **do not state that matching is
  character-exact** — assume it is, verify it once.
  Scopes "must be declared in the Developer Portal", so the scope decision above
  is part of registration, not just of the authorize URL.
- **Adam also owns the `stellar_test` guild** used for development and testing.
  It must have Community enabled and Rules Screening on, and
  `verification_level: 2`, or `pending` will not exercise the real code path.
  Production integration with the Stellar guild is [[0170]].

## Acceptance Criteria

- [ ] A visitor completes Discord sign-in and the backend resolves their Discord
      user ID
- [ ] Client secret lives in Secrets Manager; no secret in any env var or in the
      static site bundle
- [ ] `state` verified on callback; a mismatched or replayed callback is
      rejected
- [ ] Session cookie is `HttpOnly` + `Secure` + `SameSite=Lax`, scoped and
      expiring; Discord tokens are not persisted
- [ ] Sign-in routes require no API key and carry their own throttle
- [ ] This task owns the `methodSettings` entries for **all** portal routes —
      throttle here and `cachingEnabled: false` from [[0160]] declared together
      per route, since the array is keyed by `resourcePath + httpMethod` and
      assigned wholesale (`api-gateway-stack.ts`)
- [ ] Those entries are declared **outside** the `cacheEnabled` branch, the way
      `apiDocsSettings` already is. The stack builds the full array only inside
      `if (cacheEnabled)` and its `else` emits just
      `[stageWideThrottle, apiDocsSettings]`, so entries added to the `if` arm
      alone vanish wherever `apiGatewayCacheEnabled` is false — leaving the
      anonymous, keyless sign-in routes unthrottled in exactly the configuration
      where every request is a billed Lambda invocation. The existing code
      comments this trap; inherit the requirement rather than rediscover it
- [ ] Discord app registration and redirect-URI ownership documented, including
      what changes when the custom domain lands
- [ ] Scope set is exactly `identify` + `guilds.members.read` (ADR 0010); no
      `guilds`, no `email`
- [ ] `/prices/{env}/discord-guild-id` and
      `/prices/{env}/min-account-age-minutes` are **operator-seeded and read at
      runtime**, never `new ssm.StringParameter` — so [[0170]]'s guild flip
      survives the next `cdk deploy`
- [ ] The parameter names and the seeding step are written into the deploy-prep
      runbook, alongside the mTLS material
- [ ] Changing `min-account-age-minutes` in SSM takes effect **without a
      redeploy** — otherwise it is a constant with extra steps
- [ ] Guild ID is read from SSM per environment, never a constant
- [ ] A non-member is refused, and the refusal distinguishes "not a member" from
      "Discord unavailable" — a 429 or 5xx must not read as "not a member"
- [ ] `pending === undefined` is handled explicitly and does not silently pass
- [ ] Account age is derived with `BigInt` and compared against an SSM
      threshold expressed in **minutes**, defaulting to 5; a below-threshold
      account is refused with the time remaining, and no key is issued
- [ ] Issue and rework each carry their own OAuth round-trip; neither trusts the
      session cookie for eligibility
- [ ] Reveal and usage require **no** re-auth and keep working indefinitely for a
      user who has left the guild
- [ ] `state` binds the intended action as well as the CSRF nonce, and a callback
      cannot be replayed to perform a different action than the one confirmed

## Notes

- The epic's non-goal is worth restating in code comments: a user who later
  leaves the Discord server keeps their key. Sign-in proves identity at the
  moment of issuance and nothing afterwards.
- Sequencing: [[0158]] before this (the callback writes the registry record),
  [[0156]] before the scope decision, [[0161]] alongside it for the redirect
  URI.
