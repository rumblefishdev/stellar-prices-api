---
id: "0183"
title: "Ship-to-production safety — a PORTAL_ENABLED flag, because there is no test environment"
type: FEATURE
status: completed
related_adr: ["0007", "0010"]
related_tasks: ["0157", "0184", "0185", "0186", "0187", "0188", "0189", "0191", "0192", "0194"]
tags: [layer-infra, priority-high, effort-small, milestone-M3, epic-self-service-onboarding, feature-flag, ssm, safety, slice-0]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Added at the re-slice, and it is the thing the original 0158–0162 set
      missed entirely. There is one environment: `envName` is typed
      `'production'` and `infra/envs/` holds only `production.json`, so every
      `cdk deploy` lands on production. Twelve slices that each ship something
      half-built therefore need a way to be invisible until they are finished,
      and a way to be exercised by us in the meantime. First in the order,
      before hosting.
  - date: 2026-08-13
    status: active
    who: akot
    note: >
      Activated as the first slice of the reorganized epic. Nothing blocks it:
      no Discord application, no measurement, no AWS finding. It is activated *before*
      hosting on purpose, because [[0184]] is the first thing that puts a page
      on the production distribution and there is no other distribution.
  - date: 2026-08-13
    status: active
    who: akot
    note: >
      Self-audit after the review fixes, prompted by Adam asking whether they
      were actually complete. They were not. The conditional exemption written
      for review finding #2 caught `/config` in the same net, so with `API_KEYS`
      armed and the portal closed — the configuration production sits in for the
      whole build — the endpoint answered `401` instead of `{"enabled": false}`,
      and [[0185]]'s page would have had no way to learn it was closed. Missed
      because `gate_portal` exempts `CONFIG_PATH` in both directions and
      `is_exempt` was not made to mirror it, and because the new tests covered
      three of the four (portal open/closed) × (keys armed/disarmed) cells.
      `/config` is now unconditionally exempt from the key gate, matching the
      portal gate, and `config_answers_in_all_four_gate_combinations` pins every
      cell. The residual — `/config` is distinguishable on an armed service — is
      accepted and written down: the bundle it serves is public on the CDN from
      [[0184]] onwards, so the portal's existence is not the secret; which
      unbuilt routes sit behind it still is.
  - date: 2026-08-13
    status: active
    who: akot
    note: >
      Code review on PR #207 (karczuRF) found seven issues and all seven were
      real. Two mattered. **The test suite was vacuous** — it stayed green with
      `gate_portal` replaced by `next.run(req).await`, because the only route
      registered under the prefix was `/config`, which the gate skips by
      design, so every "closed" assertion was watching an unrouted 404 rather
      than a refusal. The `[x] asserted, not assumed` above was therefore not
      earned when it was written. Fixed with `PortalGate::new` plus a test that
      layers the gate over a route of its own, and the fix is confirmed the way
      the reviewer found the hole: both mutations now fail the suite.
      **Second**, exempting the portal prefix from `auth::is_exempt`
      unconditionally destroyed the indistinguishability property the moment
      `API_KEYS` is armed — portal paths would answer an empty 404 while every
      other unknown path answered 401, making the prefix uniquely
      fingerprintable. The exemption is now conditional on the portal being
      open, which is stricter than either option the review proposed and keeps
      an open portal anonymous. The remaining five (a test whose name overstated
      it, a missing `is_exempt` test, an unconstructable `pub` state, a wrong
      doc line about the Discord redirect URI, and a forward-looking throttle
      note) are fixed here or, for the last, written into [[0184]].
  - date: 2026-08-13
    status: active
    who: akot
    note: >
      Mechanism changed on Adam's call before any code was written: a plain
      `PORTAL_ENABLED` environment variable, not two operator-seeded SSM
      parameters read at runtime. The point is a boolean that can be flipped and
      exercised on a laptop, and the SSM design bought a fast production flip we
      do not need for a switch thrown once. Three problems dissolved with it —
      no `reqwest` in `prices-api`, no second copy of the extension constants
      `mtls.rs` warns about, and no allowlist keyed on a Discord session that
      [[0186]] has not built yet. The cost is stated rather than buried: flipping
      it in production is a redeploy.
  - date: 2026-08-14
    status: completed
    who: akot
    note: >
      Merged to `develop` as #207 and live: `Prices-production-Compute` carries
      `PORTAL_ENABLED=false`, and `/api-tokens/api/config` answers
      `200 {"enabled":false}` with `no-store` through both CloudFront and
      execute-api. Two criteria stay unticked and are owned elsewhere by
      design — the closed-portal page is [[0185]]'s and the key cleanup is
      [[0194]]'s; this task shipped the `config` endpoint the first reads and
      the flag the second flips. Reached production ahead of its own merge,
      because a `cdk deploy` of the gateway pulls dependency stacks in without
      `--exclusively`.
---

# Ship-to-production safety — the PORTAL_ENABLED flag

## Summary

**Story:** *as the operator, I can land an unfinished slice on production
without a stranger being able to reach it — and still walk the whole flow myself
against the real stack.*

One environment variable and a middleware. Everything else in the epic depends
on it existing first.

## The problem it solves

**There is no staging.** `envName` is typed `'production'`, `infra/envs/` holds
only `production.json`, and the archived [[0159]] already noted in passing that
this means "one parameter whose value is flipped in place, not a per-environment
matrix". Nobody drew the consequence for the portal: **a deploy is a release.**

Concretely, what lands on production without this task:

| Slice | What is publicly reachable the moment it deploys |
| --- | --- |
| [[0184]] / [[0185]] | A half-built page on the URL we are about to put in a Tranche 3 submission |
| [[0186]] | A live OAuth callback on the production hostname |
| **[[0187]]** | **Anyone with a Discord account minting a real key on the real usage plan** — the eligibility gate is three slices later |
| [[0187]] | A reconciler running `DeleteApiKey` against production keys, with the snowflake prefix hazard live |
| [[0191]] / [[0192]] | Endpoints that delete production keys |

The third row is the one that matters. An earlier draft of [[0187]] said the gap
was "fine on the dev distribution" — **there is no dev distribution**, and that
sentence is corrected in that task.

## Implementation

**One environment variable, `PORTAL_ENABLED`, read at cold start.** Same shape
as `CH_ENABLED`, `API_KEYS` and `API_BASE_URL` already in
`packages/prices-api/src/config.rs` — no new mechanism, no new dependency, and
it is togglable on a laptop, which is what makes the slices testable at all.

```bash
PORTAL_ENABLED=true cargo run -p prices-api --features local-server --bin serve
```

**Default `false`.** Note the polarity is the opposite of `ch_enabled`, and that
is deliberate: a missing `CH_ENABLED` should still give the live Lambda its
connection pool, while a missing `PORTAL_ENABLED` must never open a half-built
portal to the internet. Defaults are chosen per flag by what goes wrong when the
variable is forgotten.

In `compute-stack.ts` it is set **explicitly** to `'false'` rather than omitted.
The Rust default covers it either way; spelling it out makes opening the portal
a one-word diff that shows up in a deploy review.

**Behaviour when off:** every `/api-tokens/api/*` path returns a bare `404` with
**no body** — byte-identical to what the router returns for a path that was
never deployed. Not `403`, which confirms the route exists and merely refused;
not `503`, which promises it is coming; and not the `ErrorEnvelope`, which is
what a *real* portal `404` will look like once these routes exist and would
therefore give the gate away. The router has no `fallback`, so axum's own miss
is an empty `404` and the gate matches it exactly.

**Gated by prefix, not by enumeration.** `/api-tokens/api/` covers routes that do
not exist yet — [[0186]]'s `/auth/*`, [[0187]]'s `/key`, [[0188]]'s `/usage` — so
a later slice inherits the gate without editing this module. The bundle at
`/api-tokens/*` is **not** ours: S3 serves it and it never reaches the Lambda.

**One route answers in both states: `GET /api-tokens/api/config`.** It is the
question "is the portal open?", so refusing to answer it while closed would be
circular, and [[0185]]'s bundle would have nothing to render its "not yet
available" page from. Returns `{"enabled": bool}` with `Cache-Control:
no-store` — a stale `enabled: false` at a CDN would keep the portal dark for
that viewer long after it opened, with nothing on screen to explain why.

**Portal routes are exempt from the in-app API-key gate.** A visitor signing in
has no key by definition, so `auth::is_exempt` gains the prefix. Whether they are
served at all is this gate's decision, not that one's.

### What this is not

**It is not an incident kill switch.** An environment variable is set at deploy
time, so flipping it in production takes a redeploy. An earlier draft of this
task specified two operator-seeded SSM parameters read at runtime, on the
argument that "the kill switch has to be faster than a rollback". That argument
was dropped on purpose (2026-08-13): the flag's job is to keep half-built slices
invisible during the build, and it is flipped **once**, by [[0194]]. Paying for a
runtime-config read, an HTTP client and a cache-TTL decision to make a one-time
flip fast is the wrong trade.

If we ever do need a switch that beats a rollback on time, the machinery is
already there — the Parameters and Secrets extension the Lambda loads for mTLS
serves SSM too (`packages/prices-clickhouse/src/mtls.rs` has the client) — and
it is a different task. Do not reach for it inside this one.

**There is no allowlist.** The earlier draft had one so we could walk the flow on
production while it was closed for everyone else. With a local toggle that is
unnecessary, and dropping it removes the awkward part: the allowlist was keyed on
Discord ID, which only exists once [[0186]] ships a session, so it would have
needed a seam through a slice that had not been written.

**Consequence of using the real usage plan** (decided 2026-08-13 — no separate
"incubation" plan): a local run with production AWS credentials creates **real**
keys on the real free-tier plan from [[0187]] onwards. The gate does not stop
that, because the gate is in the Lambda and the laptop is not. Those keys have to
be cleaned up rather than left to be discovered.

**Who turns it on.** Flipping `PORTAL_ENABLED` to `'true'` in `compute-stack.ts`
is an explicit acceptance criterion of [[0194]] (the audit), gated on [[0189]]
(the eligibility gate) having passed. Nobody flips it as a side effect of
finishing their own slice.

**Every slice that deploys inherits one line of acceptance criteria:**

> - [ ] With `PORTAL_ENABLED=false`, this slice's routes return an empty `404`;
>       with it on, they behave normally

## Acceptance Criteria

- [x] `PORTAL_ENABLED` read at cold start, defaulting to `false`
      (`config.rs`), and set explicitly to `'false'` on the Lambda
      (`compute-stack.ts`)
- [x] With the flag off, every `/api-tokens/api/*` path returns `404` with an
      empty body, `GET .../config` excepted
- [x] That `404` is **byte-identical** to a path that was never deployed —
      asserted, not assumed, and the assertion is itself verified by deleting
      the gate and watching it fail (`tests/portal.rs`)
- [x] The same holds with `API_KEYS` **armed**, which is the configuration
      `config.rs` documents as the end state: a closed portal path answers `401`
      like every other unknown path rather than an empty `404`. An unconditional
      exemption in `auth::is_exempt` made the prefix the only unauthenticated
      surface on the service, and so uniquely fingerprintable
- [x] At least one test drives the gate over a route that **really exists**
      under the prefix, so the suite cannot pass with the gate removed
- [x] The gate matches by prefix, so routes added by later slices are covered
      without touching this module
- [x] The gate does not reach `/api-tokens/*`, which S3 serves
- [x] `GET /api-tokens/api/config` answers in both states, reports `enabled`,
      and is never cached
- [x] Portal routes are exempt from the in-app `X-API-Key` gate
- [x] Data routes are unaffected in both states
- [x] Turning the flag on locally serves the portal routes — the whole point of
      the flag being an env var
- [ ] The static page renders "not yet available" and no sign-in button while
      the portal is closed — **[[0185]]**, which is the first slice with a page
      to render it on; this task ships the `config` endpoint it reads
- [ ] A procedure exists, and is run once, for enumerating and deleting keys
      created against the real plan during the build — **[[0194]]**, which is
      when there is something to clean up
- [x] The single-environment fact is written into the epic doc, so the next
      person does not assume a staging deploy exists

## Notes

- API Gateway had no resource for `/api-tokens/api/*` when this task shipped, so
  the `config` route was unreachable in production regardless of the flag. That
  was the correct order — the gate exists before the door — and [[0184]] added
  the proxy behind it. **Resolved 2026-08-14:** the route answers, and the flag
  is what decides it.
- The rollback story for every later slice: if a portal slice misbehaves in
  production, set `PORTAL_ENABLED: 'false'` and deploy, rather than reverting a
  stack. Slower than an SSM flip and still much faster than untangling a revert.
- Worth revisiting once, later: if the flag is still off when [[0195]] lands the
  custom domain, the Discord redirect URI has been pointed at a closed portal for
  weeks. Harmless, but confirm sign-in on the new hostname before advertising it.
