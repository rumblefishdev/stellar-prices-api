---
id: "0187"
title: "Issue the simplest possible key and show it — AWS is the source of truth, no database"
type: FEATURE
status: completed
related_adr: ["0008", "0010"]
related_tasks: ["0183", "0157", "0158", "0160", "0186", "0188", "0190", "0194"]
tags: [layer-backend, priority-high, effort-medium, milestone-M3, epic-self-service-onboarding, api-gateway, usage-plan, iam, slice-4]
milestone: 3
links:
  - "../../../docs/epics/self-service-onboarding.md"
  - "../archive/0160_FEATURE_onboarding-backend-endpoints.md"
  - "../archive/0158_FEATURE_discord-key-registry-table.md"
history:
  - date: 2026-08-13
    status: backlog
    who: akot
    note: >
      Fourth slice and the first one a stranger would call the product. Two of
      [[0160]]'s four operations, chosen together because reveal is the same
      lookup as issue. Deliberately built with **no registry table** — [[0158]]'s
      own argument is that API Gateway is the arbiter, and nothing in this slice
      needs a row.
  - date: "2026-08-18"
    status: active
    who: akot
    note: >
      Activated on top of [[0185]] (#218) and [[0186]] (#220), both merged to
      `develop`. The signed session cookie [[0186]] issues is exactly what this
      slice reads to know whose key to issue, so the prerequisite is in place and
      the round-trip is verified locally. Carry [[0186]]'s one open criterion
      forward: the session **through CloudFront** is unverified until [[0205]]'s
      deploy, and these routes sit under the same depth-3 prefix — so a depth-3
      `403` against the deployed gateway belongs to [[0205]], not here.
  - date: "2026-08-19"
    status: completed
    who: akot
    note: >
      Shipped in PR #224 (branch `feat/0187_...`, two commits). 46 Rust tests
      for this slice (27 over HTTP against a mock control plane that paginates
      and matches `nameQuery` by prefix, 17 unit, 2 alone in binaries of their
      own), 9 frontend tests, 3 CI assertions. Ten of twelve acceptance criteria
      closed; the live `curl` waits on [[0205]]'s deploy and is [[0164]]'s to
      record.

      Three defects were found AFTER the implementation looked finished, and all
      three had a test or a comment asserting the property they broke. A key
      value reached the logs through the AWS SDK at `trace` while the test
      meant to prove otherwise passed vacuously; the usage-plan attach ran only
      on the create path, so a create whose attach failed left a key that every
      later request handed out and `/v1/` refused, permanently; and the IAM
      comment claimed `DELETE /apikeys/*` was already the narrowest form the
      service offers, which [[0194]] would have read as settled. A review from
      karczuRF then found four more, two of them in the fixes themselves —
      including a guard that a single trailing colon walked past. The log filter
      is an allowlist now, the reconciliation has a wall-clock deadline, and
      `NotFoundException` from the attach is disambiguated rather than assumed.

      Carried out: an adopted key is never checked for `enabled` ([[0192]]
      decides what "revoked" means), a key created just before a failed attach
      is left in the account (inert, self-healing, swept by [[0194]]), and
      [[0194]]'s cleanup check should be re-scoped from a date to a property.
---

# Issue a key, and show it

## Summary

**Story:** *as a signed-in developer, I press one button, get an API key on
screen, and it works against `/v1/` on the first `curl` — and when I come back
tomorrow the same key is still there.*

This is the slice the epic exists for. After it, self-service is functional for
anyone we let sign in.

## Context

[[0160]] kept issue, reveal, usage and rework together "because they share a
Lambda, an IAM policy and the registry record". They do — but they are four
stories, and bundling them is why nothing was demonstrable until all four
existed. This slice takes the two that answer "where is my key".

**No ClickHouse table.** [[0158]] designed one, and its own "Issue flow" section
explains why it is not needed first: key names are `discord-<userId>-key`, and
**API Gateway, not our store, is the source of truth for whether a key exists**.
The registry buys a hot-path read and a history; neither is required here. It
returns as [[0190]], which has to justify itself.

## Implementation

**Issue** — `POST /api-tokens/api/key`

1. `GetApiKeys(nameQuery = "discord-<userId>-key")`, **page to exhaustion**, then
   **filter to exact name equality in the client**.
2. Nothing found → `CreateApiKey(name = "discord-<userId>-key")` + tag it +
   `CreateUsagePlanKey` onto the free plan.
3. More than one survives the filter → keep the earliest `createdDate`,
   `DeleteApiKey` the rest. This is the reconciler, and it is deterministic:
   both sides of a double-submit read the same list and compute the same winner.

**Reveal** — `GET /api-tokens/api/key` → `GetApiKey(includeValue=true)`. If it
404s, re-enter the issue flow from step 1 and adopt or re-create: a key deleted
by hand in the console otherwise leaves the user with a dead id forever
([[0160]] "Settled 2026-08-07" #4).

**Three things measured on 2026-08-12 that this code must respect** (evidence:
archived `0180/notes/R-apigw-namequery-quota-and-disable.md`):

- `nameQuery` is a **case-sensitive prefix match**, not exact. So the
  client-side exact filter is load-bearing, not defence in depth. Comment it as
  such — "AWS returns prefixes and never promised not to" — so it is not later
  simplified away.
- The prefix hazard is therefore **real**, not hypothetical: Discord snowflakes
  are 17–19 digits, so a shorter id prefixes a longer one, and step 3 would
  delete a stranger's key. The `-key` suffix is what prevents it. Keep it.
- A `nameQuery` result **still paginates** — it comes back with a `position`
  token. Ranking by earliest `createdDate` off page one can pick a winner from a
  partial list and delete a key it never saw.

**Other requirements**

- **`Cache-Control: no-store` on the reveal response, and `cachingEnabled: false`
  on these methods.** Not deferrable to [[0194]]: `deployOptions.cachingEnabled`
  is on in this stack and the gateway cache has no cache-key parameters, so every
  caller collapses onto one entry — a cached reveal hands one user another user's
  key. The CloudFront behaviour must not cache these paths either.
- **IAM, narrow:** `apigateway:POST` on `/apikeys` and
  `/usageplans/{freePlanId}/keys`; `GET` on `/apikeys` (the **collection** — the
  reconciler lists it, and without this every path here fails at runtime with
  `AccessDenied`), on `/apikeys/{id}`; `DELETE` on `/apikeys/{id}`. No wildcards.
  `POST /apikeys` cannot be narrowed further — there is no ARN for "keys this
  function created" — so record that as a consciously accepted limit, mitigated
  by tagging and by attaching only to the self-service plan. [[0194]] audits it.
- **Read the plan id from SSM** ([[0157]]), never hard-coded and never a
  cross-stack reference: `ComputeStack` is a dependency of `ApiGatewayStack` and
  cannot import from it.
- **Never log a key value**, including error paths and X-Ray annotations.
- Handlers in their own module under their own path prefix inside the existing
  `prices-api` router, so the IAM additions are obviously attributable to them.
- Frontend: a button, and the key masked by default with a reveal toggle and a
  copy button. The masking and the copy button are the only two UI niceties that
  earn their place before [[0193]] — one because this renders during
  screen-shares, the other because it is what people actually use.

**Not in this slice:** eligibility ([[0189]]), usage ([[0188]]), rework
([[0191]]), revocation ([[0192]]).

**And the gap that leaves is the reason [[0183]] exists.** Until [[0189]] lands,
this code issues a real key on the real usage plan to anyone who can sign in —
and **this deploy goes to production**, because `envName` is typed `'production'`
and `infra/envs/` holds only `production.json`. There is no dev distribution to
be relaxed on. The only thing standing between a stranger and a production key
for the three slices between here and the gate is `PORTAL_ENABLED=false`, so
treat that flag as part of this slice's correctness, not as ops hygiene.

The same applies to the reconciler above: it calls `DeleteApiKey` against
production keys, with the snowflake prefix hazard live. Exercise it from a local
run against keys this task created, and nothing else — and remember the flag
lives in the Lambda, so it does not protect a laptop holding production
credentials.

## Acceptance Criteria

- [x] **Ships closed — this is the slice the flag exists for.** With
      `PORTAL_ENABLED=false` ([[0183]]) the issue route is unreachable in
      production — asserted for both verbs **with a valid session presented**,
      byte-identical to an unrouted path under the same prefix, and with the
      frontend asserted to render no control at all
      (`both_key_routes_are_an_empty_404_while_the_portal_is_closed`). The
      client is not merely unused while the portal is closed: it is never
      constructed, so no code path in the process can reach the control plane.
      The flag still does **not** protect a local run holding production
      credentials — written into the runbook §7 and the README as a blockquote
- [x] First press issues a key attached to the free plan; the value is shown —
      one key, enabled, tagged `ManagedBy=prices-portal`, attached to the plan
      id read from SSM, and its value in the response body
- [ ] **(deferred to [[0205]] + [[0164]])** That key returns `200` from a `/v1/`
      route on the first try — **cannot be closed from a keyboard.** Nothing in CI can mint a real key or call the
      real gateway. What is asserted is the two properties that make it true
      (the key is `enabled`, and it is attached to the free usage plan); the
      `curl` itself is step 3 of the README's local procedure and is Adam's,
      like [[0186]]'s round-trip
- [x] A second press returns the **same** key, not a new one — same id, same
      value, `created: false`, and exactly one `CreateApiKey` across both
- [x] Signing out and back in still shows the same key — a fresh session cookie
      for the same Discord id resolves to the same key, which is the criterion
      that fails if anything about the key is kept in the session
- [x] Two concurrent first presses converge on one key, and the loser is deleted
      — both as a seeded duplicate set (the deterministic rule) and as two
      genuinely concurrent requests, after which exactly one key exists and it
      is the one the next reveal hands out
- [x] A key deleted by hand in the console is adopted or re-created on the next
      reveal, not returned as a dead id — two tests: the console deletion, and
      the narrower race where the key is listed and gone before its value is
      read
- [x] The reconciler pages `GetApiKeys` to exhaustion before ranking — five
      duplicates at a page size of two, with the **earliest deliberately on the
      last page**, so stopping early both returns the wrong key and deletes the
      right one. The mock paginates like the service does
- [x] A user id that is a prefix of another user's id cannot see or delete that
      other user's key — and the mock matches `nameQuery` by **prefix**, like
      the service, so the test fails if either the `-key` suffix or the
      client-side exact filter is removed
- [x] Reveal is not cached at either layer, verified against the synthesized
      template rather than assumed — the gateway's `CachingEnabled: false` on
      the portal `GET`/`POST` entries and CloudFront's `CachingDisabled` are
      both asserted by `verify-openapi-routes.mjs`, each proven non-vacuous by
      flipping it and watching CI fail
- [x] IAM policy names specific resources; the un-narrowable `POST /apikeys` is
      documented as an accepted limit — and so are the two the first pass got
      wrong. `GET /apikeys` is equally un-narrowable and worse (a collection has
      one ARN, and `includeValues=true` reads every key value in the account),
      while `DELETE /apikeys/*` **can** be scoped with an
      `aws:ResourceTag/ManagedBy` condition and deliberately is not yet — the
      original comment claimed it was already the narrowest form the service
      offers, which was false and would have been read as settled by [[0194]]'s
      audit. All three are now written out in `compute-stack.ts`, the runbook
      and `infra/README.md`
- [x] No key value appears in any log or trace — and this one was **false when
      first ticked as covered**. `KeyValue` stops our own code, not the SDK's:
      `aws-sdk-apigateway` marks nothing `@sensitive`, so at `trace` the
      orchestrator prints the whole `GetApiKeyOutput`, value included. The log
      filter (`src/telemetry.rs`) is what closes it, `tests/telemetry_filter.rs`
      and `tests/portal_keys_logs.rs` are what prove it, and the latter runs
      alone in its own binary because the first version of it passed **vacuously**
      — thread-local subscriber against a global callsite cache

## Notes

- Epic AC 2 is satisfiable at the end of this slice for anyone with a Discord
  account. AC 2 *as the reviewer means it* needs [[0189]] as well.
- These are control-plane calls, throttled far harder than the data plane and
  metered per account — the same budget our CDK deploys draw on. Backoff belongs
  here; the `GetUsage` in-process cache belongs to [[0188]].


## Implementation Notes

Backend in the **existing** `prices-api` axum router (ADR 0008) — no new crate,
no new gateway integration, no database. Both routes live under [[0183]]'s gated
prefix, so that task's middleware covers them without knowing they exist, and
under a module of their own so the IAM additions are attributable.

**New — `packages/prices-api/src/portal/keys/`** (three modules, ~1,050 lines
with docs):

- `mod.rs` — the two routes, `KeysState`, and the reconciler (`ensure_key` →
  `reconcile` → `attempt`). `KEY_PATH`.
- `naming.rs` — **pure**: `key_name`, `exact_matches`, `choose_winner`,
  `losers`. No AWS, no I/O, no clock. This is where both ways the slice could
  harm somebody live — picking the wrong key to hand out, and picking the wrong
  key to delete — so both are decidable by a test on a list typed by hand.
- `gateway.rs` — the five control-plane calls, `KeyValue`, pagination to
  exhaustion, and the timeouts.

**Changed:**

- `packages/prices-clickhouse/src/mtls.rs` — added `fetch_parameter_string`
  beside `fetch_secret_string`. Same listener, same token header, same timeouts;
  the SSM path rather than the Secrets Manager one.
- `packages/prices-api/src/config.rs` — `portal_keys` field, the async
  `load_portal_keys`, `PortalKeysError`, and the plan-id resolution.
- `packages/prices-api/src/portal/auth/mod.rs` — extracted `current_session` so
  the key routes read a session the same way `/auth/me` does; `me` now calls it.
- `packages/prices-api/src/common/errors.rs` — `unauthorized_with`, so a portal
  refusal says `not_signed_in` rather than the partner API's `unauthorized`.
- `packages/prices-api/src/portal/mod.rs`, `src/main.rs`, `src/bin/serve.rs`.
- `packages/prices-api/Cargo.toml`, root `Cargo.toml` — `aws-config`,
  `aws-sdk-apigateway`. Unconditional; see the manifest comment.
- Eleven existing test files gained `portal_keys: None`. Mechanical; no
  assertion changed.

**Infra:**

- `compute-stack.ts` — `portalFreePlanParameterName`, four `apigateway:` grants,
  an `ssm:GetParameter`, and `PORTAL_FREE_PLAN_PARAM` on the api-handler.
- `api-gateway-stack.ts` — a standalone `iam.Policy` granting `POST` on the free
  plan's `/keys`, plus `apiHandlerRole` on the props.
- `app.ts` — passes the role through.
- `tools/scripts/verify-openapi-routes.mjs` — three new CI assertions
  (decision 9).

**Frontend** — `web/portal/src/api/portal.ts` (`issueKey`, `PortalKey`) and an
`ApiKey` component in `src/app/app.tsx`, inside the authenticated branch.

**Docs** — `docs/runbooks/portal-oauth-deploy-prep.md` §7,
`packages/prices-api/README.md` (local procedure + env table), `infra/README.md`.

**Tests: 43 covering this task, 34 of them new on the Rust side.**

| where | count | covers |
| --- | --- | --- |
| `portal/keys/naming.rs` | 9 | the name, the prefix hazard, the exact filter, the winner rule |
| `portal/keys/gateway.rs` | 3 | `KeyValue` redaction, `Gateway` `Debug` |
| `portal/keys/mod.rs` | 2 | the route sits under the gated prefix, at depth 3 |
| `tests/portal_keys.rs` | 20 | both routes over HTTP against a mock control plane |
| `web/portal/src/app/app.spec.tsx` | 9 new (30 total) | issue-on-press, masking, reveal, copy, failure, both closed states |
| `verify-openapi-routes.mjs` | 3 new assertions | the SSM handshake, IAM scope, gateway caching |

Verified: `cargo fmt --all --check`, `cargo check --workspace`, `cargo clippy -p
prices-api --all-targets -D warnings`, `cargo test --workspace` (0 failures),
`cargo check --features lambda` (the Lambda bin builds), `nx run-many -t lint
typecheck build test`, `nx format:check --all`, `make -C infra synth-production`,
`npm run openapi:lint`, `openapi:verify-routes`, `openapi:verify-servers`.

### Added after the first pass (log filter + two review rounds)

**New — `packages/prices-api/src/telemetry.rs`.** The log filter both binaries
build their subscriber from. An **allowlist**: a bare level, or a target inside
`OUR_TARGETS` (`prices_api`, `prices_clickhouse`), survives; everything else is
dropped, and the credential-bearing SDK crates are then pinned at `info`.

**Changed:** `src/main.rs` and `src/bin/serve.rs` use it instead of
`EnvFilter::try_from_default_env`. `keys/gateway.rs` gained `Attachment`,
`exists`, `GatewayError::PlanNotFound` and an idempotent
`attach_to_free_plan`; `keys/mod.rs` gained `RECONCILE_DEADLINE`, the
`KeysState::with_deadline` test seam (compiled out of the Lambda), the union of
a just-created key into the candidate list, and non-fatal loser deletion.
`KeyResponse` lost its `Debug`.

**Tests restructured:** the mock control plane moved to
`tests/portal_keys/harness.rs`, shared by three binaries — `portal_keys.rs`
(27), `portal_keys_logs.rs` (1) and `telemetry_filter.rs` (1). The two
single-test binaries are single on purpose; see Issues Encountered.

**Totals for the task: 46 Rust tests** — 27 over HTTP against the mock, 17 unit
(`naming` 9, `telemetry` 6, `gateway` 3, `keys/mod` 2 — 20, of which the unit
count includes the two route-shape tests), 2 in binaries of their own — plus 9
frontend tests and 3 CI assertions.

## Issues Encountered

- **The SDK retries a 500, so a one-shot failure is not observable.** The first
  version of the "control plane is down" test made `GetApiKeys` fail once; the
  retry succeeded and the handler answered `200`. Only a control plane that
  *stays* down produces the `502` this slice promises. The mock's knob is sticky
  now, and the comment says why — a one-shot failure knob would have made the
  test look like it covered an outage while covering nothing.

- **The bounded-retry branch is not reachable by deleting keys.** Written first
  as "something deletes the key on every read", which returned `200`: an attempt
  that *creates* the winner already holds its value and never reads it back, so
  the next attempt simply creates one and succeeds. The branch guards a listing
  and a reader that **disagree** — a stale list — which is what the test now
  drives, and what the mock's knob is named for. The first version would have
  passed as a `200` assertion and left the `503` path untested.

- **The narrow `ssm:GetParameter` grant is redundant today.** The baseline role
  already carries `ReadSsmNamespaces` over the whole `/prices/{env}/*`
  namespace, so the statement this task adds grants nothing new. Kept, with the
  comment rewritten to say so: an IAM statement that *looks* like the reason
  something works, while something broader is the actual reason, is worse than
  no statement — and it means narrowing the baseline (which [[0194]] may want)
  cannot silently break key issuance.

- **Two identical handlers claimed a distinction that does not exist.** `issue`
  and `reveal` started as separate functions with identical bodies. Collapsed
  into one: without a registry they *are* one operation, and two functions would
  have invited someone to invent a difference between them.

- **The "never log a key value" type stops our code, not the SDK's.** `KeyValue`
  has no `Display` and no `Serialize`, which is airtight for every format string
  in this workspace and irrelevant to `aws_smithy_runtime`, which prints
  `GetApiKeyOutput { value: Some(…) }` at `trace` and the raw response body next
  to it. Smithy has a redaction branch; it is conditional on the operation being
  `@sensitive` in the model, and `aws-sdk-apigateway` marks nothing sensitive.
  `RUST_LOG` is one `UpdateFunctionConfiguration` away from `trace`, and the
  local procedure in the README runs against production credentials.

- **The test that was supposed to catch that passed vacuously.**
  `tracing::subscriber::set_default` is thread-local; the callsite `Interest`
  cache is global. In a binary running tests in parallel, another thread reached
  the SDK's `trace!` callsites with no subscriber installed, they were cached as
  `Interest::never`, and the one test driving a `TRACE` subscriber then captured
  nothing and asserted nothing. Run alone it failed and printed a live key value.
  Hence one test per binary for that assertion, and a `trace!` probe of its own
  inside it so "captured nothing" can never again read as "leaked nothing".

- **The guard was a blocklist and was walked past twice.** First by
  `RUST_LOG='[try_op]=trace'` — a directive with no target, which `EnvFilter`
  scopes by span and therefore ranks above a target pin. Then, after that was
  closed, by `RUST_LOG='aws_smithy_runtime:=trace'` — **one trailing colon**:
  targets match by `starts_with`, the parser accepts any character in a target,
  and `StaticDirective` orders by target length, so 19 characters outranked the
  18-character pin. Both were closed by naming one more shape. The third
  version inverts the rule instead, because a blocklist here has to anticipate
  every way of spelling a target that prefix-matches something dangerous.

- **Attaching to the usage plan only on the create path left a permanent dead
  end.** A `CreateApiKey` that succeeded followed by a `CreateUsagePlanKey` that
  failed — a throttle, a timeout — left a key that every later request adopted
  and handed out with `200`, while it answered `403` from `/v1/`. No retry could
  fix it: every retry took the same branch. The attach now runs on whatever key
  is about to be handed out, idempotently (`409 ConflictException` is the
  desired state).

- **`NotFoundException` from `CreateUsagePlanKey` means two things.** "The key is
  gone" and "the usage plan is gone" are the same error. Reading it as the first
  turned a stale or mistyped plan id into `Ok(None)`, retried, exhausted, and
  reported to every user as *"try again"* — forever. Resolved by asking whether
  the key still exists before deciding which happened.

- **The per-call timeouts never composed into a request-level bound**, and the
  module docs claimed they did. One attempt makes up to six calls plus a delete
  per duplicate, `list_named` alone can walk `MAX_PAGES` pages each with its own
  budget, and the reconciler runs twice — minutes, against a 15s function. A
  wall-clock deadline is the only shape that bounds work whose call count
  depends on how many pages and duplicates the account holds.

- **`GetApiKeys` is eventually consistent in both directions.** The empty
  re-list was handled from the start; the non-empty-but-missing-our-key case
  fell through silently and orphaned a live, enabled key carrying the user's
  exact name. The created record is now unioned into the candidates so it is
  ranked and, if it loses, deleted like any duplicate.

## Review Findings

A final pre-commit pass over the whole diff. **Seven fixed**, none of them
behavioural regressions — six were defects of consistency, documentation or
blast radius, and one was a latent panic.

| # | severity | what | where | why it mattered |
| --- | --- | --- | --- | --- |
| R1 | Medium | **A panic path in a public handler.** `choose_winner(...).expect("candidates is non-empty here")` is unreachable — the branch above returns early — but [[0186]]'s F7 established what being wrong about that costs: a panic is not a `500`, it is a dropped connection with no response (`curl` reports `000`) and an invocation error on the Lambda's `Errors` metric. Now a `let ... else` that falls through to the retry and answers `503`. | `keys/mod.rs`, `attempt` | the last panic on the route |
| R2 | Medium | **`api/portal.ts` documented a `401` branch the page did not have.** The comment said the status is carried through "so the caller can say so"; nothing read it, so an expired session rendered "answered 401". Added `describeFailure`, plus a test. | `api/portal.ts`, `app.tsx` | a comment that promised behaviour |
| R3 | Low | **`signOut`'s doc comment was orphaned.** The new `PortalKey`/`issueKey` block landed between the comment and the function it documented, leaving `signOut` undocumented and the comment attached to an unrelated interface. | `api/portal.ts` | pure insertion damage |
| R4 | Low | **`ApiKey` had no unmount guard** while `SignIn` beside it keeps one. No supersede case (the button disables itself), but a response landing after sign-out would write a credential into a departed component's state — and the inconsistency itself invites the next reader to decide one of the two is wrong. | `app.tsx` | consistency, [[0186]] F5's lesson |
| R5 | Low | **The usage-plan id was not trimmed.** It goes straight into an ARN path segment, so a trailing newline — what an operator gets from `echo <id> \| aws ssm put-parameter` — would produce a malformed request reported as a control-plane failure rather than as the typo it is. | `config.rs` | a misdiagnosed failure |
| R6 | — | **A stale comment in `portal/mod.rs`** listed `/auth/*` and `/key` as "routes that do not exist yet". Both now exist, and neither touched that function — which is the prefix gate working, and now says so. | `portal/mod.rs` | |
| R7 | — | **Two navigation defects.** The CI script's sections read 1, 2, 4, 5, 6, 3; and the IAM comment said revocation "needs none of these" while `DELETE` — which [[0192]] does need — is granted here for the reconciler. | `verify-openapi-routes.mjs`, `compute-stack.ts` | |

Two tests were added for gaps the pass exposed:

- **`a_read_that_fails_for_any_other_reason_is_a_502_and_creates_nothing`.** The
  whole not-found handling rests on one line of `Gateway::value_of`: a `404`
  becomes `Ok(None)` and re-enters the flow, everything else is an error. Wrong
  in the permissive direction — any failure treated as "gone" — and an
  AccessDenied or a throttle would make the handler create a **second** key for
  a user who already has one, on every request. Only the `404` half was covered.
- **The `401` frontend branch** added by R2.

### Checked and found sound

- **`SdkError::into_service_error` does not panic** on a non-service error
  (a timeout, a connection failure); it produces an `Unhandled` variant, so
  `is_not_found_exception()` is `false` and the call lands in the error arm.
  This was the pass's top hypothesis — the method's name reads like it asserts
  — and the new test above pins the behaviour rather than the reading.
- **The extension's SSM endpoint and response shape.**
  `/systemsmanager/parameters/get?name=…`, `{"Parameter":{"Value":…}}`, and
  `reqwest`'s query serializer percent-encodes the leading `/` of a hierarchical
  name, which the extension requires.
- **No scope belonging to a later slice.** No `GetUsage` ([[0188]]), no
  eligibility or guild scope ([[0189]]), no rework cap ([[0191]]), no revocation
  route ([[0192]]). `DELETE` on `/apikeys/*` is granted for the reconciler, which
  is this task's own requirement.
- **The in-app API-key gate.** `auth::is_exempt` covers the whole portal prefix
  while the portal is open, so the route needing no `X-API-Key` is not a special
  case this task added.

## Design Decisions

### From Plan

1. **No database.** API Gateway is the source of truth; every request is a
   reconciliation (list → filter → rank → converge), not a lookup of something
   we wrote down. [[0158]]'s table returns as [[0190]] if it can justify itself.
2. **`GetApiKeys` pages to exhaustion, then an exact client-side filter.** Both
   halves load-bearing, both argued for in `naming.rs`'s module docs, both with
   a test that fails if either is removed.
3. **The `-key` suffix stays.** It is what makes a shorter snowflake unable to
   prefix a longer one, and it is asserted rather than commented.
4. **Earliest `createdDate` wins; the rest are deleted.**
5. **Plan id from SSM**, by name, never hard-coded and never a cross-stack
   reference.
6. **Never log a key value** — enforced by a type, not by review.
7. **Handlers in their own module under their own prefix**, so the IAM
   additions are attributable to them.
8. **Frontend: a button, masked by default, reveal toggle, copy button.** The
   two niceties the task allows ahead of [[0193]], and no more.

### Emerged

9. **The three properties infra can get wrong are CI assertions, not comments.**
   The SSM parameter name is written by `ApiGatewayStack` and read by
   `ComputeStack` as two hand-typed strings in two files that cannot reference
   each other; a drift fails **Lambda init**, which takes `/v1` down, on the
   deploy that opens the portal. `verify-openapi-routes.mjs` now compares them
   across the two synthesized templates, refuses `apigateway:*` and
   `Resource: "*"` on any control-plane statement, and asserts the portal's
   gateway methods are uncached. Each proven non-vacuous by breaking it.

10. **The usage-plan grant is declared in `ApiGatewayStack`, as a standalone
    `iam.Policy`.** The other four grants need no id from that stack and live in
    `ComputeStack`. This one cannot: `addToPrincipalPolicy` would append to the
    role's default policy — a **ComputeStack** resource — so the plan id would
    travel as an export of ApiGatewayStack imported by ComputeStack, while
    ApiGatewayStack already imports the Lambda from ComputeStack. That is the
    cycle, written differently. A standalone `Policy` is a resource of the
    gateway stack that names the role, so the reference runs the one way that
    works.

11. **The control-plane client is built only when the portal is open**, mirroring
    decision 8 of [[0186]] and adding a second reason: with the portal closed
    there is no client in the process, so no code path — not a bug, not a stray
    handler — can create or delete a production API key. A closed portal also
    pays nothing at cold start for it.

12. **`PORTAL_FREE_PLAN_ID` is compiled out of the Lambda**, like
    `PORTAL_OAUTH_SECRET_FILE` and the Discord endpoint overrides ([[0186]] F7).
    Same permission, `lambda:UpdateFunctionConfiguration`, and here it would
    move every newly issued key onto a usage plan of somebody else's choosing —
    a different rate limit, a different quota, or a plan on a stage we do not
    control.

13. **A `GET` may create, and the reasoning is restated rather than inherited.**
    The task requires a reveal to adopt-or-create so a hand-deleted key is not a
    dead id, and without a registry "deleted by hand" and "never issued" are the
    same observation. `auth/mod.rs` explicitly warns the next route on this
    prefix not to inherit its CSRF reasoning, so it was re-derived: the exposure
    is a third-party page causing a top-level `GET` navigation, which
    `SameSite=Lax` does send the cookie on — and it buys nothing, because the
    flow is idempotent (no forged navigation produces a second key), the
    response is not readable cross-origin, and `POST` is not reachable
    cross-site under `Lax` at all. The worst outcome is a visitor holding the
    key they could have pressed a button for.

14. **The page therefore fetches nothing on mount.** The backend cannot
    distinguish the verbs, so the *page* is what keeps the visitor's intent
    explicit: the only call is behind a press. A page that asked on load would
    issue a real production key to anyone who merely opened it.

15. **Tested against a mock control plane over real HTTP, driving the real
    SDK** — [[0186]]'s decision 20, and stronger here. A trait fake would be
    tempted to implement `nameQuery` as equality, which is the exact bug the
    client-side filter exists to survive; it could not have a pagination bug at
    all; and `include_values` is only observable in what was *sent*. The mock
    matches by prefix, paginates, and records every parameter.

16. **The winner rule tie-breaks on id.** API Gateway reports `createdDate` in
    whole seconds, so two keys created in the same second tie — and a tie broken
    by list order is broken by whatever order the service happened to return,
    which means the two sides of a double-submit can each delete the other's
    key. Tested by ranking the same pair in both orders.

17. **The name is re-checked immediately before every `DeleteApiKey`.**
    `exact_matches` has already run; this is the guard that survives somebody
    simplifying that away, and it logs at `error` rather than proceeding.

18. **A `502` for the control plane, a `503` for a lost race, a `401` for no
    session.** Distinct because they point at different people: an AWS incident,
    a transient conflict the caller should retry, and a visitor who needs to
    sign in. The retry is bounded at two attempts so the answer is an answer
    rather than a Lambda timeout.

19. **`aws-config` and `aws-sdk-apigateway` are unconditional dependencies**, as
    [[0186]]'s `reqwest` is. Gating them behind `lambda` would make the
    reconciler — the pagination, the filter, the winner rule, the deletes — the
    one part CI never compiles. The cost is binary size, not cold start: the
    client is only ever constructed when the portal is open.

20. **The log filter is an allowlist, not a longer blocklist.** Two bypasses got
    in through the blocklist version (Issues Encountered), each closed by naming
    one more shape. The third version changes the failure mode instead: only a
    bare level or one of our own crates survives, so a form nobody anticipated is
    **refused** rather than honoured. The cost is accepted and written down —
    raising a third-party crate's log level now needs a code change. Matching is
    case-sensitive, like `EnvFilter`'s own.

21. **The attach runs on every key handed out, not only on created ones**, and is
    idempotent through `409 ConflictException`. Verifying membership instead
    would cost a call and an IAM grant to learn what this call can simply
    assert. The page fetches nothing on mount (decision 14), so the extra call
    per press is a human pressing a button.

22. **A `404` from the attach is disambiguated with `GetApiKey`, not assumed.**
    One extra call, only on that error path, on a grant already held. The
    alternative considered and rejected was validating the plan at cold start:
    it needs a new grant and would still miss a warm container holding a plan id
    that was correct when it booted. Recorded for [[0188]], which opens that
    policy anyway.

23. **A wall-clock deadline bounds the reconciliation; the per-call timeouts are
    deliberately not tightened to match.** A shorter attempt timeout would start
    cutting off cold TLS handshakes and turn a healthy first invocation into a
    retry. The deadline guarantees an answer; the per-call values only decide how
    many slow calls fit inside it before that answer becomes a `503`.

24. **A failed `DeleteApiKey` on a loser is logged and stepped over.** By then the
    winner is created, attached and ready; propagating would answer `502` and
    withhold a key that demonstrably works. It is also not hypothetical: if
    [[0194]] puts an `aws:ResourceTag` condition on `DELETE`, an exact-name
    duplicate created by hand in the console carries no tag and would fail on
    every request.

25. **One test per binary where the assertion depends on a global cache.**
    `portal_keys_logs.rs` and `telemetry_filter.rs` each hold exactly one test,
    because `tracing`'s callsite interest is process-global while the subscriber
    is thread-local — the property that made the first version of the log test
    pass while the leak was live.

## Future Work

Nothing new spawned — every follow-up already has a task:

- The live `curl` against `/v1/` with an issued key, and the through-CloudFront
  check → precondition is [[0205]]'s deploy; the end-to-end check belongs to
  [[0164]] and [[0194]].
- Eligibility, so this stops issuing keys to anyone who can sign in → [[0189]].
  Until it lands, `PORTAL_ENABLED=false` is the control.
- Per-key usage → [[0188]]. It needs its own `GetUsage` grant on the plan;
  this slice deliberately grants only `/keys`.
- Rework and the once-per-period cap → [[0191]]. Revocation → [[0192]].
- Whether a registry row is needed at all → [[0190]].
- Cleanup of keys created during local verification, and the IAM audit →
  [[0194]]. **Two things to carry into it, found here:** (a) its cleanup check is
  scoped by date ("keys created while the flag was off"), which stops meaning
  anything the day the flag goes to `true` — the better check is a property,
  "every key tagged `ManagedBy=prices-portal` that belongs to no usage plan",
  and it also catches the orphan below; (b) `DELETE /apikeys/*` can be scoped
  with an `aws:ResourceTag/ManagedBy` condition, which the code comment used to
  deny.
- **A key created immediately before a failed attach is left in the account.**
  Inert — a key on no usage plan cannot be used against `/v1/` — and
  self-healing, because once the plan id is right the next request from that
  user adopts and attaches it. Deliberately not rolled back: adding a
  destructive call to an error path to tidy something that costs nothing is the
  wrong trade in the one module whose most guarded operation is `DeleteApiKey`.
  Swept by [[0194]]'s property check above.
- **An adopted key is never checked for `enabled`.** A key disabled by hand is
  attached, returned with `200` and answers `403`. Not fixed here on purpose:
  `PATCH` is deliberately ungranted, and re-enabling a disabled key would undo a
  revocation — so what "disabled" should mean to this flow is [[0192]]'s to
  decide.
- **No global deadline existed until review; the per-call timeouts still do not
  compose.** The deadline is in place, but the cost side — control-plane call
  volume per press and per dashboard load, and backoff under throttling — is
  [[0194]]'s, which already owns costing it.
- Styling → [[0193]]. The page is deliberately unstyled.
