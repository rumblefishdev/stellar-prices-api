---
id: "0186"
title: "Sign in with Discord — identity only, scope identify, session cookie"
type: FEATURE
status: active
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
  - date: "2026-08-17"
    status: active
    who: akot
    note: >
      Activated and implemented in one session, on top of [[0185]] (#218, merged
      to `develop`). Backend, frontend, infra and runbook. 87 automated tests
      cover it — 64 new (41 unit across five new modules, 23 integration against
      a mock Discord bound to loopback) plus 9 new frontend tests. Two of the ten
      acceptance criteria cannot be closed from a keyboard: the live round-trip
      needs a registered Discord application (Adam's, per ADR 0010 §6) and the
      through-CloudFront check needs [[0205]]'s deploy, because the sign-in
      routes sit at depth 3 and the currently-deployed gateway maps only depth
      1-2. Two criteria turned out to be already satisfied by [[0184]]'s code and
      were verified by measurement rather than re-implemented — see decisions 12
      and 13.
  - date: "2026-08-17"
    status: active
    who: akot
    note: >
      **Adam verified the live Discord round-trip** against a Discord
      application he registered himself, running locally with the portal flag
      on: sign-in completes and the page renders his Discord username and ID.
      That closes the two criteria that could not be closed from a keyboard —
      the local round-trip against a `localhost` redirect URI, and the
      username+ID display — and settles the open question [[0156]] recorded:
      redirect-URI matching behaved as the runbook assumes it does. What is
      still open is only the deployed path: the session **through CloudFront**
      needs [[0205]]'s gateway deploy, since the four sign-in routes sit at
      depth 3 and the mapping currently in production covers depth 1-2.
  - date: "2026-08-17"
    status: active
    who: claude
    note: >
      Adversarial review pass over the whole diff, on top of the round-trip
      above. No High or Medium defect found in the code; the top hypothesis —
      that API Gateway REST v1 would drop the second of the callback's two
      `Set-Cookie` headers and silently lose the session — was checked against
      `lambda_http` 1.2.1 and does not hold (it sends everything through
      `multiValueHeaders`). Two genuine coverage gaps were found and closed,
      each proved non-vacuous by reintroducing the exact defect: the PKCE
      verifier's VALUE reaching the authorize URL (the existing assertion
      checked the parameter NAME, so swapping the two signed tokens would have
      passed), and the `methodSettings` trap this task names in its own
      criteria, which had **no** automated guard at all — a synth cannot see it,
      because production runs `cacheEnabled: true` and only ever exercises one
      arm. +1 unit test, +3 CI assertions. Eight Low findings recorded below.
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

- [x] **Ships closed.** With `PORTAL_ENABLED=false` ([[0183]]) the sign-in
      routes return an empty `404` — asserted for all four routes, both verbs,
      byte-identical to an unrouted path, cookies included
      (`every_sign_in_route_is_an_empty_404_while_the_portal_is_closed`), and
      **the local round-trip was run by Adam** against a Discord application he
      registered, with the flag on and the redirect URI pointed at localhost —
      per the runbook's §4
- [x] A visitor completes the round-trip and the page shows their Discord
      username and ID — **verified live by Adam** against his own Discord
      application, locally, with `PORTAL_ENABLED=true`. Also covered against a
      mock in CI (`me_reports_the_username_and_id_after_a_round_trip`, `shows
      the username and the Discord ID once signed in`). The live run is what
      settles [[0156]]'s open question about redirect-URI matching
- [x] Client secret is in Secrets Manager; no secret in any env var or in the
      bundle — read off the synthesized template: the api-handler carries
      `PORTAL_OAUTH_SECRET_NAME=prices/production/portal-discord-oauth` and no
      value; the IAM grant is scoped to that one secret and to that one role
- [x] `state` is verified; a mismatched or replayed callback is rejected — five
      distinct refusals, each its own test, at both the unit and HTTP layer.
      Replay is defended by clearing the cookie *before* the outcome is known
- [x] `state` carries an action slot, signed, even though only one action exists
      — signed into **both** halves and compared; an unknown action is refused at
      `/auth/login` rather than defaulted
- [x] Session cookie is `HttpOnly` + `Secure` + `SameSite=Lax`, scoped and
      expiring; no Discord token is persisted — asserted on the whole
      `Set-Cookie` string, on the 24h expiry being enforced server-side, and on
      the serialized session carrying exactly three fields
- [x] Sign-in routes require no API key and carry their own throttle, declared
      outside the `cacheEnabled` branch — **already true from [[0184]]**;
      verified by synthesizing with `apiGatewayCacheEnabled` flipped to `false`
      and diffing the arrays (decision 12), not by reading the code
- [ ] ~~A signed-in request reaches the origin still signed in through
      CloudFront~~ — **BLOCKED on [[0205]]'s deploy.** The two settings it
      depends on are correct in the committed template and now asserted in CI
      (decision 13), but the property itself is only observable in a browser
      against a deployed distribution
- [x] Scope requested is exactly `identify` — sent in the authorize URL and
      **verified on the token response**, so a Developer Portal registration that
      drifts wider fails closed
- [x] App registration and redirect-URI ownership written into the deploy-prep
      runbook, including what changes at the domain cutover —
      `docs/runbooks/portal-oauth-deploy-prep.md`, whose §6 is a five-step
      ordering with the failure mode of inverting it spelled out

## Notes

- Sequencing: needs [[0184]] for the hostname, [[0185]] for somewhere to render.
  Blocks [[0187]] and [[0189]].
- The `methodSettings` array is keyed by `resourcePath + httpMethod` and assigned
  wholesale. Later slices add portal routes to it; this task owns its shape, and
  [[0194]] audits the finished array.

## Implementation Notes

Backend in the **existing** `prices-api` axum router (ADR 0008) — no new crate,
no second gateway integration, no second build, as the 2026-08-07 meeting
settled. All four routes live under [[0183]]'s gated prefix, so that task's
middleware covers them without knowing they exist.

**New — `packages/prices-api/src/portal/auth/`** (six modules, ~1,450 lines
with docs):

- `mod.rs` — the four routes, `AuthState`, the authorize-URL builder, and the
  response helpers. `LOGIN_PATH` / `CALLBACK_PATH` / `ME_PATH` / `LOGOUT_PATH`.
- `crypto.rs` — HMAC-SHA256 sign/verify with **domain separation**, base64url,
  OS-entropy tokens, the PKCE S256 derivation.
- `state_token.rs` — the `state`/cookie pair, the `Action` slot, and `accept()`,
  which is where mismatch and replay are refused.
- `session.rs` — the signed session cookie: `{sub, name, exp}` and nothing else.
- `cookies.rs` — reading and building `Set-Cookie`; all four attributes in one
  place.
- `discord.rs` — the two upstream calls, the scope check, `AccessToken` with a
  redacting `Debug` and consume-by-value semantics.
- `secret.rs` — the Secrets Manager bundle, its validation, and the local-file
  alternative.

**Changed:**

- `packages/prices-clickhouse/src/mtls.rs` — extracted `fetch_secret_string`
  from `fetch_bundle_from_extension`, which now calls it. One extension client,
  two callers.
- `packages/prices-api/src/config.rs` — `portal_oauth` field plus the async
  `load_portal_oauth`, conditional on `portal_enabled` (decision 8).
- `packages/prices-api/src/portal/mod.rs`, `src/main.rs`, `src/bin/serve.rs`,
  `src/auth/mod.rs` (`ct_eq` → `pub(crate)`).
- `packages/prices-api/Cargo.toml` — `hmac`, `sha2`, `getrandom`,
  `form_urlencoded`, `reqwest` (rustls, webpki roots). All unconditional; see
  the manifest comment for why gating them would gut the test suite.
- Nine existing test files gained `portal_oauth: None` in their `AppConfig`
  literals. Mechanical; no assertion changed.

**Infra:**

- `infra/src/lib/mtls.ts` — `portalOauthSecretName()`, beside `mtlsSecretName`.
- `infra/src/lib/stacks/compute-stack.ts` — `PORTAL_OAUTH_SECRET_NAME` on the
  api-handler and a `secretsmanager:GetSecretValue` grant scoped to that one
  secret, on that one role.
- `infra/src/lib/stacks/secrets-stack.ts` — publishes the name to
  `/prices/{env}/portal-oauth-secret-name`. Names only, as with the mTLS pair.
- `infra/src/lib/stacks/portal-hosting-stack.ts` — **comments only.** The two
  settings this task needed were already correct (decision 13).
- `infra/src/lib/stacks/api-gateway-stack.ts` — **comments only**, plus the
  depth-3 warning about the currently-deployed mapping.
- `tools/scripts/verify-openapi-routes.mjs` — three new CI assertions on the
  synthesized CloudFront template (decision 13).

**Frontend** — `web/portal/src/api/portal.ts` (`fetchSession`, `signInUrl`,
`signOut`) and `src/app/app.tsx` (a `SignIn` component behind the open flag).

**Docs** — `docs/runbooks/portal-oauth-deploy-prep.md` (new), `infra/README.md`,
`.gitignore`, `web/portal/vite.config.mts`.

**Tests: 87 covering this task, 64 of them new on the Rust side.**

| where | count | covers |
| --- | --- | --- |
| `portal/auth/crypto.rs` | 7 | domain separation, tamper, the RFC 7636 vector |
| `portal/auth/state_token.rs` | 9 | the five refusals, PKCE derivation, the action slot |
| `portal/auth/session.rs` | 7 | forgery, expiry, "no token is representable" |
| `portal/auth/cookies.rs` | 5 | the four attributes, path scoping, clearing |
| `portal/auth/secret.rs` | 7 | validation, redaction, the redirect-URI check |
| `portal/auth/mod.rs` | 5 | the authorize URL, encoding, redirect literals |
| `tests/portal_auth.rs` | 23 | the four routes over HTTP against a mock Discord |
| `web/portal/src/app/app.spec.tsx` | 9 new (18 total) | link-not-fetch, username+ID, cancelled, sign-out |

Verified: `cargo fmt --all --check`, `cargo check --workspace`, `cargo clippy -p
prices-api --all-targets -D warnings`, `cargo test --workspace` (0 failures),
`cargo build --features lambda` (links), `nx run-many -t lint typecheck build
test`, `nx format:check --all`, `make -C infra synth-production`,
`npm run openapi:lint`, `openapi:verify-routes`, `openapi:verify-servers`.

## Issues Encountered

- **`reqwest`'s rustls feature pulls `ring`, while `prices-clickhouse` uses
  `aws-lc-rs`.** Two crypto providers now compile into the Lambda. Not a runtime
  conflict — `reqwest` selects `ring` explicitly under `__rustls-ring` rather
  than reading the process default, and `install_default_crypto_provider` is
  unaffected — but it is a few hundred KB of binary and worth knowing before
  someone "unifies" it. The alternative (`-no-provider` + relying on
  `prices-clickhouse` having installed a default) makes the Discord client's TLS
  depend on the ClickHouse client having been built first, which is a worse
  failure mode than a larger binary.

- **Nine existing test files broke on the new `AppConfig` field.** Expected, and
  each spells out every field by existing convention, so the fix was mechanical.
  No assertion in any of them changed. Not a regression.

- **`app.spec.tsx`'s "renders the open state" test asserted on [[0185]]'s
  placeholder sentence** ("Sign-in arrives with the next slice"), which this
  slice deletes. Rewritten to assert on the sign-in link instead. Intentional:
  this task *is* the next slice.

- **A single blanket `fetch` stub could not express the open state**, because
  the page now makes two calls. A route-keyed stub replaced it for the new
  tests; the original `stubFetch` stays for the closed-portal and error cases,
  where there is only one call.

- **Clippy's `await_holding_lock` caught a real hazard** in the first version of
  `tests/portal_auth.rs`, which held a `std::sync::Mutex` guard across the whole
  test body to serialize an env var. Safe under the current-thread test runtime,
  a deadlock under a multi-threaded one. The guard is now released as soon as
  the router is built, which is sound because `Endpoints::from_env` runs once at
  construction.

## Review Findings (adversarial pass, 2026-08-17)

No High or Medium defect in the code. The findings below are Low and are
recorded rather than fixed, because each is a judgement call rather than a bug
and fixing them blind would widen this slice.

1. **`/auth/me` does not clear an invalid or expired session cookie.** The
   browser keeps re-sending it until it expires on its own. Cosmetic; the
   request is cheap and the answer is correct.
2. **`secret.rs` validates the `redirect_uri` suffix but not its scheme or
   host.** A relative or otherwise malformed URI ending in `CALLBACK_PATH`
   passes the load-time check and fails at Discord instead — which is the
   failure mode the check exists to avoid, for a narrower class of typo.
3. **Two tabs signing in at once**: the second `/auth/login` overwrites the
   pending cookie, so the first tab's callback fails `invalid_state`. Standard
   for a single-cookie design, but user-visible.
4. **Timeout budget is tight.** The api-handler's Lambda timeout is 15s and the
   callback makes two Discord calls at 5s each. A slow Discord plus a cold start
   could reach it.
5. **`mtlsSecretArnFromParts` now builds an ARN for a secret that has nothing to
   do with mTLS.** The helper is correct; its name is not.
6. **Signing-key strength cannot be validated beyond length** — 32 identical
   characters pass. The runbook prescribes `openssl rand -hex 32`.
7. **The frontend offers no sign-in control when `/auth/me` fails.** The visitor
   sees an error and cannot act on it; arguably the button should still render.
8. **`signing_key` is not zeroized on drop.** It lives for the process lifetime
   regardless.

**Checked and found sound**, recorded because none of it was previously
written down:

- **The callback's two `Set-Cookie` headers survive API Gateway REST v1.**
  `lambda_http` 1.2.1 sends everything through `multiValueHeaders` and leaves
  `headers` deliberately empty (`response.rs:85`). Had it used `headers`, the
  session cookie would have been dropped and sign-in would have failed only in
  production. This was the review's top hypothesis.
- **Path traversal cannot bypass the API-key gate.** `auth::is_exempt` and the
  axum router read the same `req.uri().path()`, so they cannot disagree about
  whether a request is under the portal prefix.
- **`Session::issue` has exactly one production call site**, in the callback,
  after `current_user` succeeds.
- **No secret reaches the logs at any level.** Probed with canary values at
  `RUST_LOG=info`, `debug` and `trace` — including `reqwest`/`hyper` internals
  at trace, which is the level that would log a request body if anything did.

## Design Decisions

### From Plan

1. **Routes on the existing axum router**, under `/api-tokens/api/auth/*`.
   Nothing new to deploy, and [[0183]]'s prefix gate covers them for free.
2. **Authorization Code + PKCE (S256)**, sent whether or not Discord enforces
   it. [[0156]]'s caveat stands: the docs do not describe the server's
   behaviour, so the test asserts our half of the loop rather than theirs.
3. **Scope exactly `identify`.** ADR 0010. Never `guilds`, never `email`.
4. **`state` signed and bound to the session**, with an action slot.
5. **Client secret in Secrets Manager**, name in the env var, value through the
   Parameters & Secrets extension — `compute-stack.ts`'s existing shape, with
   the helper beside `mtlsSecretName` exactly as the task specified.
6. **Session cookie signed, `HttpOnly`, `Secure`, `SameSite=Lax`**, carrying the
   Discord user ID and an expiry, with no Discord token persisted.
7. **Frontend is a link and a line of text.** Ugly, per [[0193]]'s claim on the
   styling.

### Emerged

8. **The OAuth secret is read only when `PORTAL_ENABLED` is true.** The obvious
   implementation reads it at cold start unconditionally — which, on a
   deployment where nobody has created it yet, fails Lambda init. That is not
   confined to the portal: one router serves every route group (ADR 0008), so it
   would take `/v1` down to protect four routes that answer an empty `404`
   either way. Conditional, and *fatal when the portal is open*, is the pairing
   that fails at the right moment. This is the single most load-bearing decision
   in the task and it is invisible until the deploy that would have broken.

9. **One secret with four fields, three of which are not secret.**
   `client_id`, `client_secret` and `redirect_uri` are one Discord application
   registration, owned by one person, and they change together at the cutover.
   Splitting them across a secret and `production.json` would put `redirect_uri`
   under CloudFormation — which the repo has already learned means the next
   `cdk deploy` silently restores the committed value (`SecretsStack`'s own
   comment; `compute-stack.ts`'s ledger cursor). `session_signing_key` rides
   along because it is provisioned in the same step by the same person.

10. **Domain-separated HMAC: one key, three token kinds.** The operator
    provisions one signing key. Without a context string in the MAC input, the
    `state` parameter — which the holder reads out of their own address bar —
    would verify as a session cookie, and `/auth/login` would be a session
    vending machine. Tested directly.

11. **The username is in the signed session, as a display-only field.** The ADR
    says the cookie carries the user ID; the acceptance criterion says the page
    shows the username. With no registry ([[0190]]) and no stored token, the
    cookie is the only place it can survive the redirect. It is signed, it
    authorizes nothing, and it refreshes at each sign-in. Called out because it
    is a deviation from ADR 0010's literal wording, though not from its intent
    (the ADR's concern is an *eligibility* claim dating the verdict).

12. **The `methodSettings` criterion was already met — verified, not
    rebuilt.** [[0184]] declared `portalSettings` outside the `cacheEnabled`
    branch and spread it into both arms, citing this task. Rather than take that
    on trust, `apiGatewayCacheEnabled` was flipped to `false` in a throwaway
    synth and the arrays diffed: the three portal entries survive, with
    `rate=10, burst=40, cachingEnabled=false`. Zero lines changed here; the
    criterion is closed on measurement.

13. **CloudFront cookie forwarding was also already correct — so the work became
    a CI guard.** `ALL_VIEWER_EXCEPT_HOST_HEADER` was chosen by [[0184]] for
    `Host`, and it happens to forward cookies too; `CACHING_DISABLED` was chosen
    for `x-api-key`, and it happens to satisfy "do not cache auth paths". Both
    are load-bearing for this task by coincidence, which is exactly the kind of
    thing a later tidy-up breaks. `verify-openapi-routes.mjs` now asserts both
    managed-policy IDs and `IncludeCookies: false` against the synthesized
    template. Proven non-vacuous by flipping the policy to `Managed-CORS-S3Origin`
    in a copy and watching it fail.

14. **The granted scope is checked on the token response, not just requested.**
    Scopes are declared in the Developer Portal *and* in the authorize URL, so
    the two can disagree and only the response shows the real grant. Compared as
    a whole string, not `contains`, so `identify guilds` is refused.

15. **The pending cookie is cleared on every path out of the callback, before
    the outcome is known.** That is what makes a callback single-use without a
    server-side nonce table. Discord's codes are single-use as well, but that is
    Discord's guarantee to keep.

16. **`/auth/me` answers `200 {authenticated: false}`, not `401`.** Being signed
    out is an answer to "am I signed in?", not a refusal — the same circularity
    [[0183]]'s `/config` avoids. It lets the page render plain text rather than
    an error.

17. **`/auth/logout` is `POST`-only** and the frontend uses `fetch`, while
    sign-in is an `<a href>`. A `GET` sign-out is triggerable by any third-party
    `<img>`, which `SameSite=Lax` permits; a `fetch`ed sign-*in* cannot perform
    the top-level navigation the flow needs.

18. **`303 See Other` for every redirect**, and the redirect target is a
    literal in all four branches. No `redirect_to` parameter, ever — this is the
    origin that terminates OAuth, and the same open-redirect reasoning
    [[0184]]'s `DirectoryIndexFn` records.

19. **Session TTL 24h, pending TTL 10 minutes.** Neither is in the task. 24h
    because from [[0187]] this cookie authorizes revealing an API key and
    re-signing in costs a redirect with no consent screen; 10 minutes because
    that is a consent screen plus a login plus 2FA with room to spare.

20. **Tested against a mock Discord on loopback rather than behind a trait.** A
    `DiscordClient` trait would let the tests skip the HTTP layer — which is
    where three of the requirements actually live (secret in the body not the
    URL, the verifier matching its challenge, the scope check). The mock records
    what it received and the assertions are against that.

21. **`Endpoints` are overridable from the environment** (`DISCORD_API_BASE`,
    `DISCORD_AUTHORIZE_URL`) purely as a test and local-dev seam.
    `compute-stack.ts` does not set them, so production always takes the
    hard-coded Discord endpoints.

22. **The local secret source is a FILE, not an env var holding JSON.** So that
    no code path anywhere reads a client secret out of the environment. A path
    that exists "only for local" is a path production can be misconfigured onto.

23. **The PKCE assertion checks the verifier's VALUE, not the parameter name.**
    The original test asserted `!url.contains("code_verifier")`, which is the
    NAME — so swapping `state_param` for `pending_cookie` in
    `state_token::start` would have put the verifier in the query string under
    `state` and left every assertion passing. Reintroducing exactly that defect
    now fails seven tests.

24. **The `methodSettings` trap is guarded by a SOURCE scan, not a template
    check.** It cannot be checked from the synthesized template: production runs
    `apiGatewayCacheEnabled: true`, so a synth exercises one arm, and moving
    `...portalSettings` into that arm alone produces a byte-identical template —
    measured, not assumed. The property is about where a line sits in a
    conditional, which synthesis erases. `verify-openapi-routes.mjs` therefore
    reads `api-gateway-stack.ts` and asserts both entries appear in both
    assignments, plus that there are exactly two, so the check cannot go vacuous
    after a refactor.

## Future Work

Nothing new spawned — every follow-up this task touches already has a task:

- Live verification of the round-trip and of the session through CloudFront →
  precondition is [[0205]]'s deploy plus Adam's Discord registration; the
  end-to-end check itself belongs to [[0164]] and [[0194]].
- `guilds.members.read`, the membership call, `pending`, account age → [[0189]],
  which amends this task's runbook §1 step 3 and `discord::SCOPE` together.
- Issuing and revealing a key → [[0187]]; the rework cap → [[0191]].
- Whether a registry row is needed at all → [[0190]].
- Styling → [[0193]]. The page is deliberately unstyled.
