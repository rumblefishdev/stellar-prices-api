---
id: "0348"
title: "Answer the API hub's dashboard contract: account, reveal, revoke, issue"
type: FEATURE
status: completed
related_adr: ["0010"]
related_tasks: ["0340", "0186", "0187", "0188", "0189", "0349"]
tags: ["portal", "hub", "priority-high", "effort-medium"]
links:
  - https://github.com/rumblefishdev/sorobanscan-api-portal/blob/main/docs/superpowers/specs/2026-11-17-shared-sign-in-design.md
  - https://github.com/rumblefishdev/sorobanscan-api-portal/blob/main/docs/superpowers/plans/2026-11-18-prices-hub-contract-plan.md
history:
  - date: 2026-11-18
    status: active
    who: akot
    note: >
      Task created and started. Stage 2 of the shared sign-in: the hub's
      dashboard at sorobanscan.com/api/dashboard shows the Prices tab through
      the hub's backend, which calls these four routes server-to-server.
  - date: 2026-11-19
    status: active
    who: akot
    note: >
      In review as #422 (refactor) and #423 (contract); Oskar's review of
      both answered on the branches. The task's record moves to develop
      (lore/1-tasks/CLAUDE.md), with commit subjects in place of the local
      branch's hashes.
  - date: "2026-11-23"
    status: completed
    who: akot
    note: >
      #422 (2a21582d) and #423 (21b938fa) merged into develop on 2026-11-19
      on akot's word, with no approving review; production's api-handler
      redeployed from 21b938fa the same day. Follow-up 0349 merged as #424.
      Closed by akot with two items left in this file, no task for either:
      the Cloudflare rules (spec decision 16) and `agrees_with_listing`
      (Future Work).
---

# Answer the API hub's dashboard contract: account, reveal, revoke, issue

## Summary

`https://sorobanscan.com/api` becomes one hub for every API, with one Discord
sign-in and one dashboard with a tab per product (`rumblefishdev/sorobanscan-api-portal`).
The hub's backend (BFF) holds the session and calls each product server-to-server
with a 60-second HMAC-signed token. Prices answers four calls under
`/api/hub/v1/`: `GET account`, `POST key/reveal`, `POST key/revoke`,
`POST key/issue`, reusing its own key, cap and usage logic. Today's portal routes
keep working unchanged until the hub takes over (cutover step 3 of the design).

## Status: Completed

**Closed 2026-11-23.** #422 (the services pulled out of the handlers, no
behaviour change) and #423 (the contract) merged into `develop` on 2026-11-19,
with no approving review: Oskar's review of 2026-11-18 is answered on the
branches (decisions 26–30). Production's api-handler has answered the contract
since 2026-11-18 12:25 UTC, and was redeployed from `develop` (21b938fa) with
`make -C infra deploy-production-compute` on 2026-11-19. Not done, and left
without a task: agree the Cloudflare rules with 0340's owner (spec decision
16), and `agrees_with_listing` (Future Work).

## Context

- Design: `docs/superpowers/specs/2026-11-17-shared-sign-in-design.md` in
  `sorobanscan-api-portal` ("The dashboard contract", decisions 11–16; decision
  15 is the token format with a test vector both sides pin).
- Plan: `docs/superpowers/plans/2026-11-18-prices-hub-contract-plan.md` there.
- A change from today's portal (akot, 2026-11-17, design decision 4): no key is
  issued on sign-in. An account without a key gets one only from "Get API Key"
  in the hub's Prices tab, which reaches `key/issue` with `act=issue`.

## Implementation Plan

1. Extract the portal's key and usage services from the handlers, no behaviour
   change (PR 1).
2. Verify the hub's token (`v1.<payload>.<mac>`, HMAC-SHA256, `current` or
   `previous` secret from Secrets Manager `sorobanscan-hub/production/prices-token`).
3. The four routes, inside the `PORTAL_ENABLED` gate, outside the portal's CORS.
4. Infra (`PORTAL_HUB_SECRET_NAME`, read on that one secret), runbook, ADR 0010
   amendment (PR 2). Deploy is akot's.

## Acceptance Criteria

- [x] Given the design's test vector, when Prices verifies it, then it is accepted
      inside its lifetime and refused after it (`hub::token` unit tests)
- [x] Given a token signed with another secret, malformed, expired or living
      longer than 60 s, when any route is called, then 401 and no AWS call
      ("no control-plane call", decision 11; `tests/portal_hub.rs`)
- [x] Given a `read` token on `key/revoke` or `key/issue`, then 403 and no AWS call
- [x] Given no or a bad hub secret, then the contract answers 503
      `hub_unconfigured` and the portal and `/api/config` still work
- [x] Given an account with no key, when `key/issue` is called with an `issue`
      token, then it gets a key under Prices' rules (one active, month cap after
      a revoke, account-age backstop)
- [x] Given today's portal routes, then their behaviour and tests are unchanged
      (`git diff f7ce49de -- packages/prices-api/tests/`, the two new files
      aside, only adds lines: one assertion in `portal_issue.rs`, the portal
      route's `IssuedBy` (Oskar's review of #422), and the mock recording a
      created key's description; no assertion was removed or loosened. The
      unit tests in `src/` that #423 edits were added by #422's refactor)

Not criteria, still to do: the merge (Status).

## Implementation Notes

All in `packages/prices-api` unless named. #422, then #423 on top of it:

- **Services out of the handlers, no behaviour change** (#422, "take the
  IssuedBy tag as Gateway::create's argument" to "extract the usage answer
  into UsageState::answer_for"):
  `Gateway::create(name, issued_by)`; `keys::issue_for(…, issued_by)`;
  `keys::key_state` (the reveal); `keys::revoke_for` (the revoke);
  `UsageState::answer_for` (the usage). Unit tests run against the integration
  tests' loopback mock (`portal::test_harness`).
- **The token and the secret** (#423, "verify the hub's signed tokens"): `portal/hub/token.rs`
  (`verify`, the spec's test vector, `HubCaller` with a redacted `Debug`),
  `portal/hub/secret.rs` (`{current, previous?}`, 32 bytes of padded base64
  each), `config::hub_secret_from_env`, and `sources::HubSource`, read on the
  first hub call and retried after a failed read.
- **The routes** (#423, "answer the hub's dashboard contract"):
  `portal/hub/{mod,auth,account,key}.rs`, mounted in `portal::apply_with`
  beside the portal's group; `key_id` on the hub's issue and revoke lines; the
  usage fallback held to the listing (Issues); two contract tests for a revoke
  in an earlier period. Its helpers come in #422: `keys::key_unavailable`,
  `keys::key_status`, `UsageState::begin` / `answer_from_listing`,
  `eligibility::account_age`, with no caller there yet.
- **Infra** (#423, "read the hub token secret in the api-handler"):
  `hubTokenSecretName` in `infra/src/lib/mtls.ts`;
  `compute-stack.ts` sets `PORTAL_HUB_SECRET_NAME` and grants
  `PortalReadHubTokenSecret`; `verify-openapi-routes.mjs` check 7b pins both.
- **Docs** (#423, "add the hub contract's runbook", "amend ADR 0010 for keys
  the hub issues", "name the hub lines the runbook left out"):
  `docs/runbooks/hub-contract.md`, the README's env table, ADR 0010 §8a.
- **Tests** (2026-11-19, after Oskar's review, rustc 1.97.1 as in CI):
  `cargo test -p prices-api` 683 passed, 0 failed, 83 ignored (the
  ClickHouse ITs); `tests/portal_hub.rs` 37, `tests/portal_hub_logs.rs` 1;
  clippy (`--all-targets`, and `--features lambda`) and fmt clean. The tests
  the review added fail with what they guard taken out (the reveal's
  eviction, the description). Infra, 2026-11-18: `npm run
  infra:synth:production` (no AWS credentials), check 7b with six mutations
  caught, `infra:verify-dashboard`, `openapi:verify-servers`,
  `openapi:verify-bundled`, 75 script tests. 2026-11-19: check 7b's role rule
  on that synth passes, and fails with the grant on another role or shared
  with one; the review changed no CDK code (a comment in `mtls.ts`).

## Issues Encountered

- **A failed hub-secret read was kept for the environment's life** in the first
  design (task 2): one throttle and the contract stayed closed until a
  recycle, what 0311 removed from the portal. Fixed before the PRs
  (`HubSource`, decision 9).
- **A contradicted "no key" could come back through the usage fallback.** A
  "no key" cached before another execution environment issued the key failed
  `agrees_with_listing`, but the throttle and deadline arms of `finish` served
  it anyway and stored it again: `usage: null` beside `active`, and
  `/api/usage` held at `404 no_key` for another TTL. Fixed in #423
  (`UsageState::fallback`); #422 alone still has it, in code with no caller
  until #423, hence the merge straight after.
- **`aws_sigv4` at `RUST_LOG=trace` prints `name=discord-<id>-key`** in every
  signed request (portal routes too), and likely the role's session token in
  the canonical request. Production runs at `info`. Spawned 0349.
- **The synthesized grant's ARN** is an `Fn::Join` around `AWS::AccountId`
  without `CDK_DEFAULT_ACCOUNT` (CI) and a literal with it; check 7b reads both.
- **`secret.rs` described changing the secret as a one-step swap**, which
  leaves a gap: Prices keeps the secret it read for an environment's life.
  Corrected to the runbook's four steps (decision 23).
- **`cargo doc` with warnings denied fails on this crate**, with 82 warnings
  that predate the task (35 public docs linking a private item, 47 unresolved
  links); none is on a line this task wrote (`git blame` of each). CI does not run `cargo doc`, so
  nothing gates on it; left alone.

## Design Decisions

### From Plan

1. **A shared HMAC secret, not KMS** (spec decision 12): one Secrets Manager
   secret both sides read; the hub signs with `current`, Prices accepts
   `current` or `previous`. Token format and test vector: spec decision 15.
2. **Four routes under `/api/hub/v1/`**, mounted through `portal::apply_with`
   inside the `PORTAL_ENABLED` gate, outside the portal's CORS, not in the
   OpenAPI document; `Cache-Control: no-store` on every answer.
3. **Services extracted with no behaviour change**, so the contract runs
   Prices' own key, cap and usage logic and the portal's tests stay untouched.
4. **The secret by name, through the extension** (`PORTAL_HUB_SECRET_NAME`);
   the inline `PORTAL_HUB_SECRET` is compiled out of the Lambda. A missing or
   bad secret closes only the contract (`503 hub_unconfigured`).
5. **Prices' rules unchanged; hub keys tagged `IssuedBy=hub-contract`.**
6. **Membership and screening are the hub's proof; age is re-checked here**
   (ADR 0010 §8a).
7. **Logs name a call by `jti`, `act`, outcome and `key_id`**; never the token,
   a key's value, the Discord id or the username (`tests/portal_hub_logs.rs`).
8. **One IAM statement on one secret**, `PortalReadHubTokenSecret`.

### Emerged

9. **The hub secret retries after a failed read, as 0311's portal load does**
   (controller's ruling on task 2): a success is kept for the environment's
   life, a failure for a 2 s cooldown.
10. **The portal's load and the secret's read are awaited with `tokio::join!`**
    (ruling): in sequence a cold call pays both budgets; a `select!` that
    dropped the portal's load would count as a failed load and close
    `/api/config` for the cooldown.
11. **"No AWS call" before a 401 or 403 means no control-plane call** (ruling):
    a cold call with a forged token may still trigger the portal's own cached
    SSM and Secrets Manager reads, never an API Gateway call.
12. **`account` reads no key value** (`act=read`): one listing with
    `includeValues=false` serves the key's state and its usage; `reveal`
    reads the value.
13. **A hub issue or revoke evicts the account's usage entry** in this
    execution environment (Review Focus 5). An issue evicts the whole entry
    (`invalidate`), not only a "no key": a replacement key has a fresh counter.
14. **A non-canonical snowflake in `sub` is refused** (`0`, a leading `0`), so
    one account has one spelling.
15. **`account` answers `200` with `usage: null` when only the usage read
    failed**; the listing failing is `502`/`503 key_unavailable`.
16. **A revoke from an earlier period is `revoked` with `can_issue: true`**,
    not `none`; the reveal still says `no_key` for it.
17. **`too_young` carries `next_eligible_at`**, when the account clears
    Prices' threshold.
18. **Cached usage stands beside a listing only when the two agree**, the
    fallbacks included.
19. **`key_id` on the hub's issue and revoke lines**: `IssueOutcome::Issued`
    carries the id created or adopted; `RevokeOutcome::Revoked` the account's
    current key (`current_key`). The account line writes `key_id=<id>` like
    the others and leaves it out for no key or a revoked one.
20. **One call budget, 14 s** (`LOAD_BUDGET` + `RECONCILE_DEADLINE`); issue's
    minimum-age read comes out of the control plane's share.
21. **No alarm for the contract.** `invalid_token` WARNs come from every
    scanner on a public URL, and an alarm would need an Observability deploy
    outside this task's (`deploy-production-compute`). The runbook (§5) says
    what to watch and that a later filter must leave that line out. The one
    to alarm on would be the ERROR `the hub contract's secret did not load`:
    akot's call.
22. **The secret's name is the hub's, not under `prices/`**
    (`hubTokenSecretName`), and CI pins it with the grant (check 7b): a
    drifted name would close the hub's Prices tab with nothing failing here.
23. **Changing the secret takes four steps with two recycles**, documented, not
    coded: new key as `previous`, recycle, swap, drop the old one and recycle.
    A TTL on Prices' copy (as the hub's 5 minutes) would make it two steps;
    a code change after review, left to akot.
24. **Follow-up task 0349 is on `develop`** (the repo's rule for task
    documents), since 2026-11-19; its fix is #424.
25. **One branch, refactor commits first; two PRs at the end with akot**
    (ruling), the secret's retry squashed into the token's commit at PR time.

### From Oskar's review of #422 and #423 (2026-11-18, answered 2026-11-19)

26. **A token is good for its action until it expires, not for one call.**
    `jti` is logged, never checked, so a captured token can be replayed until
    `exp` + 5 s. The docs said "one token, one call"; they now say this, and
    ADR 0010 §8a records it as accepted. A `jti` cache would hold per
    execution environment only.
27. **A rotation step run out of turn refuses.** Step 1 refuses while the
    secret holds a `previous`; step 3 refuses unless the stored secret is
    still the version step 1 wrote (its `VersionId`); the rollback finds its
    two versions itself. No step puts a key on a command line. The runbook
    also gains the path for a leaked secret: write the new key alone, recycle
    at once, and trace what forged tokens did (`jti`, `key_id`, the key's
    name).
28. **The secret's load logs one INFO with how many keys it holds**
    (`the hub contract's secret loaded`, `keys`), and the runbook checks it
    between steps 2 and 3.
29. **`keys::key_state`'s failure line is `portal key lookup failed`**, not
    "portal key reveal failed": the hub's reveal shares the lookup.
30. **A key's description names what issued it**: the hub contract's keys read
    "Self-service key issued through the API hub (task 0348)", the portal's
    keep theirs.

## Future Work

- `agrees_with_listing` compares a key's presence, not its id (Oskar's review
  of #423, not blocking): after a revoke and a new key in another execution
  environment inside this one's TTL, the old key's cached usage "agrees" with
  the new key. `/api/usage` has the same staleness today. Store the key id in
  `CachedAnswer::Usage` and compare it.
- [[0349]]: hold `aws_sigv4` to `info` in `telemetry::CREDENTIAL_BEARING_TARGETS`;
  at `RUST_LOG=trace` it names `discord-<id>-key` on the portal's and the
  hub's routes, and likely prints the role's session token.
