---
id: "0124"
title: "Expose the OpenAPI spec through API Gateway — /api-docs-json is unroutable in production"
type: FEATURE
status: completed
related_adr: ["0008"]
related_tasks: ["0119", "0120", "0128"]
tags: [layer-infra, layer-backend, priority-medium, effort-small, milestone-M2, api-gateway, openapi, documentation]
milestone: 2
links:
  - "../../../packages/prices-api/src/lib.rs"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../docs/scf/milestone-1-evidence.md"
history:
  - date: "2026-07-23"
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Closes the "Swagger"
      row of `milestone-1-evidence.md` Table 4, which states the axum router
      defines `/api-docs-json` but the API Gateway does not map it. Scoped to
      the **spec**; the Swagger **UI** and the onboarding portal stay
      Tranche 3 per overview §9.
  - date: "2026-08-04"
    status: active
    who: akot
    note: >
      Promoted to active, picked up by Adam. Scope unchanged: the spec
      document over the deployed API, not the Swagger UI. First open
      question is the auth posture — the task recommends anonymous to
      match `/health` (the keyless-mock block in `api-gateway-stack.ts`);
      the decision needs
      recording either way before implementation.
  - date: "2026-08-06"
    status: active
    who: akot
    note: >
      PR #169 review (okarcz) addressed — all seven points taken. Blocking
      ID collision fixed (spawned license task renumbered 0144 → [[0155]],
      five reference sites). `openapi:verify-routes` now runs on infra-only
      PRs that touch the gateway stack, closing the gateway→spec direction
      it could not previously see. The tautological `LEDGER_SEQ_MAX` assert
      replaced with a document-derived test. Full accounting in the
      "PR #169 review" section.
  - date: "2026-08-06"
    status: active
    who: akot
    note: >
      Self-review round applied (e18b936, 79bd33a, 07cbc39, 479548c). Its
      central finding: three of the four fixes made for the #169 review were
      incomplete in the same way the originals were — a check that reads as
      covering something it does not. Also documents the 429 both anonymous
      routes can return (lint-ignore now empty), splits the spec cache by who
      can invalidate it (3600 s gateway + flush on deploy, 300 s client),
      stops serving `{}` as 200, and throttles the anonymous route. The
      throttle goes beyond what #169 accepted, so it is isolated in 479548c
      and awaits okarcz's call. Full accounting in the "Self-review round"
      section.
  - date: "2026-08-06"
    status: completed
    who: akot
    note: >
      PR #169 merged to develop (squash `dabdd15`) after okarcz's approval;
      three CI jobs green. Shipped: `GET /api-docs-json` mapped as a keyless
      cached proxy route, `servers` stamped from `apiBaseUrl`, `ErrorEnvelope`
      published, a Redocly `recommended-strict` gate, and three artifact-derived
      guards (routes both directions, `servers` vs the synthesized handler
      config, ledger ceilings read back out of the document). 225 workspace
      tests, openapi suite 7 → 9 across two review rounds; lint 0 errors /
      0 warnings / 0 ignored. Converted to a directory at archive time as
      planned, with the three heavy sections split into `notes/S-*`.
      **Two ACs stay deployment-verified rather than verified** — the live
      `/api-docs-json` fetch and the `servers` URL serving a route both need
      `make -C infra deploy-production`; `docs/scf/api-endpoints.md` carries
      the curl.
---

# Expose the OpenAPI spec through API Gateway

## Summary

`packages/prices-api/src/lib.rs` builds the OpenAPI spec at startup and serves
it at `GET /api-docs-json` — but API Gateway never maps that path, so in
production the spec is reachable only by running
`cargo run -p prices-api --bin extract_openapi` locally. The M1 submission said
so explicitly and deferred the fix to Tranche 2.

Scope here is the **specification document**, served over the deployed API. The
Swagger **UI** and the self-service onboarding portal (S3 + CloudFront) are
Tranche 3 per §9 and are out of scope.

## Context

The spec is generated from the axum routes via `utoipa`, so it cannot drift from
the implementation — which is exactly why it is worth exposing rather than
hand-maintaining a parallel document. It is also the input [[0120]] validates
responses against, and the natural home for the enumerations and ranges
[[0119]] adds.

Design decision needed: **should the spec require an API key?** Recommendation
is **no** — an API description is public documentation, and gating it behind the
key a developer does not yet have is a self-service dead end. `/health` is
already anonymous (the keyless-mock block in `api-gateway-stack.ts` — this
Context originally cited line 238, which this task's own edits have since
shifted; line citations rot, so it is named rather than numbered), so the
precedent for an
unauthenticated route exists. Record the choice either way.

## Implementation

- Map the route through API Gateway to the existing axum handler. Prefer the
  existing proxy integration over a second mechanism so there is one source of
  truth.
- Decide and apply the auth posture (recommended: anonymous, matching
  `/health`).
- Cache it — the spec is static per deployment, so a long TTL is free and keeps
  it off the Lambda. Note that `apiKeyRequired: false` + caching means it must
  contain nothing key-specific (it does not).
- Stamp `servers` correctly. `lib.rs` already sets `spec.servers` from
  `config.base_url`; confirm the deployed value is the real invoke URL,
  including the stage path. **Watch the stage-prefix trap** — the same class of
  bug that required `AWS_LAMBDA_HTTP_IGNORE_STAGE_IN_PATH=true` for the `/v1`
  routes will make the advertised `servers` URL wrong if `base_url` omits the
  stage.
- Sanity-check the emitted document: valid OpenAPI 3.0, every deployed route
  present, no route present that is not deployed, and the `x-api-key` security
  scheme declared so a reader knows what auth the *data* routes need.
- Once [[0126]] lands a custom domain, `servers` must follow it — note the
  ordering.

## Acceptance Criteria

- [x] `GET <base>/api-docs-json` (or the agreed public path) returns the spec
      from the deployed production API — **route implemented and synth-verified;
      the live fetch needs a deploy** (see Verification below)
- [x] Auth posture decided, applied, and recorded — anonymous
- [x] Document is valid OpenAPI (3.1.0, not 3.0 — see Design Decisions) and
      passes Redocly's `recommended-strict` ruleset with 0 errors / 0 warnings /
      **0 ignored** — the two `operation-4xx-response` exceptions were deleted
      rather than kept, see the self-review round
- [x] `servers` resolves to a URL that actually serves the API, stage path
      included — stamped from config, invariant enforced at synth; the fetch
      needs a deploy
- [x] Route coverage matches the deployed router exactly, both directions —
      enforced twice: a fast in-process test, and a CI check deriving both sides
      from the synthesized template and the extracted document
- [x] Security scheme (`x-api-key`) declared for the key-gated routes
- [x] Response cached with a TTL appropriate to a per-deployment-static document
      — 3600 s at the gateway (flushed on deploy), 300 s at the client. The two
      deliberately disagree; the self-review round explains why "static for the
      life of a deployment" does not justify a cache that outlives it
- [x] Path recorded in `docs/scf/` so [[0128]] can cite it —
      `docs/scf/api-endpoints.md`

## Implementation Notes

**Rust (`packages/prices-api`)**

- `openapi/mod.rs` — `SecurityAddon` declares the `x-api-key` scheme; a
  document-wide `security(("api_key" = []))` requirement makes key-gating the
  default; `/health` and `/api-docs-json` opt out with `security(())`, mirroring
  the two exemptions already in `auth::is_exempt`. Added a documentation-only
  `#[utoipa::path]` stub for `/api-docs-json` (the real route is wired after
  `split_for_parts()`, since it serves the document that split produces) so the
  route list is complete. New `stamp_servers()` shared by `app()` and the
  extract bin.
- `common/errors.rs` — `ErrorEnvelope` is now `ToSchema` and is the documented
  body of every 4xx/5xx. Seven key-gated operations gained `401`/`403`; the
  ohlcv route gained its real `503 quote_unavailable`.
- `backfill/dto.rs`, `assets/handlers.rs`, `ops/mod.rs` — published bounds that
  already existed in the code but not in the document: `maximum` on the five
  ledger-sequence fields, `minimum`/`maximum` on the `limit` param, and a
  one-line `summary` for `/health`. See "The `openapi-validator` result".
- `lib.rs` — the spec response carries `Cache-Control: public, max-age=300`
  (new `cache_control::DEPLOY_STATIC` tier; started at 3600 s, shortened in the
  self-review round). The document is serialized once at startup and `.expect`ed
  — an earlier revision fell back to `{}`.
- `bin/extract_openapi.rs` — reads `AppConfig::from_env()` and stamps `servers`,
  so the linted document is the served document rather than a variant of it.
- `tests/openapi.rs` — 9 tests (route coverage both ways, security scheme,
  per-route auth posture, `servers` stamp incl. stage path, reachable with the
  gate armed, cache header, OpenAPI 3.x, the ledger-ceiling rules, and the
  no-documented-OPTIONS guard — the last two added in the review rounds).

**Infra**

- `api-gateway-stack.ts` — `GET /api-docs-json` as a keyless Lambda proxy
  (`apiKeyRequired: false`), 3600 s stage-cache TTL via `CACHE_TTL.apiDocs`, and
  a method-level 10 req/s throttle (self-review round; isolated in `479548c`).
- `types.ts` / `envs/production.json` / `compute-stack.ts` — new `apiBaseUrl`
  config → `API_BASE_URL` on the api-handler, validated at synth (https, no
  trailing slash, stage path present for execute-api hosts).
- `infra/Makefile` — `flush-production-cache`, run after `deploy-production` and
  `deploy-production-compute`. The 3600 s TTL is only defensible with it.

**Lint gate**

- `redocly.yaml`, `.redocly.lint-ignore.yaml`, `tools/scripts/extract-openapi.sh`,
  `npm run openapi:{extract,lint}`, and a CI step in the `rust` job (the only
  job with both cargo and node). The lint-ignore file is **empty** — it stays in
  the tree carrying the reason the two exceptions were deleted.

**Artifact-derived guards** (both in the `rust` job, after synth)

- `tools/scripts/verify-openapi-routes.mjs` — routes, both directions.
- `tools/scripts/verify-openapi-servers.mjs` — the advertised `servers` URL
  against the api-handler's synthesized `API_BASE_URL` (self-review round).

## Issues Encountered

Both found by reviewing the first commit, not by a failing test — worth
recording because both were guards that looked like they worked.

- **The lint gate did not fail on warnings.** `redocly lint` exits 0 with
  warnings, and under the `recommended` ruleset most checks — including
  `operation-4xx-response`, the rule that found seven undocumented 401/403 —
  report as warnings. So CI would have accepted the exact regression the step
  was added to catch. Fixed by extending `recommended-strict`, which promotes
  them to errors, and pinning `operation-4xx-response: error` explicitly so a
  future switch back to `recommended` cannot silently demote it. Verified by
  stripping the 4xx responses off a route and confirming exit 1.

- **"Both directions" was only one and a half directions.** The Rust test
  compares the spec against a hand-written `EXPECTED_ROUTES` mirroring the CDK
  source. It catches a gateway route the spec omits and a spec path the gateway
  does not map — but *not* a route added to axum with a plain `.route()` call
  and never documented or mapped, which is precisely how `/api-docs-json` went
  unroutable for months. Added `tools/scripts/verify-openapi-routes.mjs`, which
  derives both sides from artifacts (the synthesized CloudFormation template and
  the extracted document) and runs in CI after synth. Same reasoning as
  `lambda-assets.sh` / task 0077: this repo has been bitten three times by
  hand-maintained mirrors. Verified failing in both directions before wiring in.

- **Three of the four gates had holes, found by the PR #169 review.** Same
  theme as the two above — guards that looked like they worked:

  1. `npm run openapi:verify-routes` did not chain `openapi:extract`, unlike
     `openapi:lint` which did. So it compared the synthesized template against
     whatever `target/openapi.json` happened to hold — in this working tree,
     a file from another branch. A stale spec reads as a pass, or as drift
     nobody can reproduce. Now chained, and the script header says what
     invoking the file directly still skips.
  2. The `rust` paths filter omitted `package.json` / `package-lock.json`,
     while `@redocly/cli` is a devDependency and this is the only job that
     runs it. A PR dropping or bumping it merged green; the failure landed on
     the next author to touch `packages/**`. Both files added to the filter.
  3. `HTTP_METHODS` included `options` and `head`, so [[0126]]'s
     `addCorsPreflight` — which emits an OPTIONS method on every resource —
     would have failed this gate with the remedy "add a `#[utoipa::path]` for
     each", for methods OpenAPI does not conventionally describe. Both are now
     excluded from *both* sides (excluding one side only manufactures drift).
     `ANY` is rejected loudly rather than skipped: it can never match an
     operation key, and skipping it silently would hide a mapped route from
     the check.

  The fourth was not a gate: seven key-gated operations documented 401/403 but
  not 429 or 500 — the two statuses a partner is most likely to meet, since the
  usage plan throttles and every one of the seven reaches `errors::db_error`.
  A generated client fell into its "unexpected response" branch for both. The
  403 description was also wrong ("Rejected by the API Gateway usage plan"):
  403 is the key being missing or unauthorized, 429 is the usage plan. Fixed
  together.

  Verified by injection rather than by reading: an OPTIONS method added to the
  synthesized template is ignored, an ANY method exits 1, and deleting
  `/v1/prices/batch` from the document still reports it undocumented — so the
  exclusion did not cost the drift detection it sits next to.

- **A second validator disagreed, and most of it was worth listening to.** The
  Notes justify the lint gate as de-risking Tranche 3 AC 2, which names
  `openapi-validator`. The shipped gate is Redocly and the document passes it
  cleanly, so the AC as written ("passes **a** linter cleanly") is met. But
  IBM's `ibm-openapi-validator` — the tool that owns that name on npm — reported
  **13 errors** on the same bytes. Eight were fixed, five are deliberate; see
  "The `openapi-validator` result" below for the full accounting.

  The AC's source text is in this repo, at
  `docs/prices-api-general-overview.md:1332` — our own document, reviewed by
  SCF. Since the wording is ours, `openapi-validator` is almost certainly
  generic rather than a procurement of IBM's package. Worth one confirming
  question to Oskar, who wrote the overview. **Do not edit that AC's wording**:
  it is a submitted, reviewed document, and rewriting an acceptance criterion
  after the fact reads as moving goalposts even when the intent is
  clarification. Record the interpretation in the [[0128]] evidence instead.

## The `openapi-validator` result

Moved to [`notes/S-openapi-validator-result.md`](notes/S-openapi-validator-result.md) — 13 errors down to 5, which five remain and why reaching 0 was
measured and rejected.

## Verification

- `cargo test --workspace` — 223 passed, 0 failed (225 after the review rounds)
- `cargo fmt --check`, `cargo clippy -p prices-api --all-targets` — clean
- `npm run openapi:lint` — "Your API description is valid", 0 errors, 0 warnings
  (`recommended-strict`; regression case confirmed to exit 1)
- `npm run openapi:verify-routes` — gateway and document agree on all 9 routes;
  confirmed to fail in both drift directions, and re-confirmed after the
  OPTIONS/HEAD exclusion landed
- `cdk synth Prices-production-ApiGateway` — `/api-docs-json` resource present,
  `ApiKeyRequired: false`, `CacheTtlInSeconds: 3600`; the 9 mapped routes match
  the 9 documented paths exactly
- `cdk synth Prices-production-Compute` — `API_BASE_URL` on the api-handler

After the PR #169 review fixes (`e40da29`, `cb646e0`, `d809a46`):

- `cargo test -p prices-api --test openapi` — 8 passed (was 7), including the
  new ledger-ceiling test; mutation-checked as described above
- `cargo clippy -p prices-api --all-targets`, `npx prettier --check` — clean
- `npm run openapi:lint` — still valid, 0 errors / 0 warnings, 2 ignored
- `npm run openapi:verify-routes` — still 9/9; the rewritten `fullPath()`
  confirmed to exit 1 on both an unresolved parent and a resource cycle
- `cdk synth` **not** re-run at this point: the only infra edit was a comment

After the self-review round (`e18b936`, `79bd33a`, `07cbc39`, `479548c`):

- `cargo test --workspace` — 225 passed, 0 failed; the openapi suite is 9 (7 → 8
  → 9 across the two rounds)
- `cargo fmt --check`, `cargo clippy -p prices-api --all-targets`, `eslint`,
  `tsc`, `prettier` — clean
- `npm run openapi:lint` — valid, 0 errors / 0 warnings / **0 ignored**. The
  count dropped from "2 explicitly ignored" because the exceptions were deleted,
  not because the rule was weakened
- `npm run openapi:verify-routes` — 9/9
- `npm run openapi:verify-servers` — advertised URL matches the synthesized
  api-handler config
- `cdk synth` — ran this time, and it needed a workaround worth recording: the
  Lambda bootstrap assets are not built in a plain dev checkout, so synth fails
  with `CannotFindAsset` before rendering anything. Placeholder directories
  under `target/lambda/` let it through (they only need to exist), and were
  moved to `.trash/` afterwards. The rendered template carries **one**
  `/api-docs-json` `GET` method setting holding both the 3600 s TTL and the
  10/20 throttle — method settings are keyed by `resourcePath`+`httpMethod`, so
  two entries would have collided — and the stage-wide `/*` `*` entry survives.

The mutation checks — each confirming a guard now fails where it previously
passed — are in [`notes/S-self-review-round.md`](notes/S-self-review-round.md).

**Not verified — needs a deploy.** Two ACs are about the *deployed* API: the
live `GET …/production/api-docs-json` fetch and confirming the advertised
`servers` URL serves a route. Both are one `make -C infra deploy-production`
away, and `docs/scf/api-endpoints.md` carries the curl to confirm.

## Review rounds

Two rounds, both recorded in full:

- [`notes/S-pr-169-review-response.md`](notes/S-pr-169-review-response.md) — okarcz's seven points (2026-08-05), all taken.
- [`notes/S-self-review-round.md`](notes/S-self-review-round.md) — the multi-agent round that followed, whose central finding is that three of
  the four fixes made for #169 were incomplete in the same way the
  originals were.

## Design Decisions

### From Plan

1. **Anonymous.** As the task recommended. An API description is public
   documentation; gating it behind a key the reader does not yet have is a
   self-service dead end, and `/health` already established the anonymous-route
   precedent (`api-gateway-stack.ts`). Applied at both layers — the gateway
   (`apiKeyRequired: false`) and the in-app gate, which already exempted the
   path. Safe to cache for all callers because the document holds nothing
   key-specific.

2. **Existing proxy integration, not a second mechanism.** The route proxies to
   the same axum handler as the data routes, so there is one place the document
   comes from.

3. ~~**3600 s TTL**, gateway and handler agreeing, per the `CACHE_TTL` /
   `cache_control.rs` single-source-of-truth rule.~~ **Superseded** by the
   self-review round: 3600 s at the gateway (flushed on deploy), 300 s at the
   client. The single-source-of-truth rule still holds for every other route;
   this one is the documented exception, because the two caches differ in who
   can invalidate them. Kept struck through rather than rewritten — the original
   decision was made, shipped, and then found wrong, and that is the part worth
   remembering.

### Emerged

4. **The document is OpenAPI 3.1.0, not 3.0.** utoipa 5 has no 3.0 emit mode
   (`OpenApiVersion` has exactly one variant). Reaching 3.0 would mean
   downgrading utoipa or post-processing the document — both worse than the
   deviation. 3.1 is a valid major release and reads natively in modern tooling
   including Swagger UI 4+, which Tranche 3 will use. The AC wording was taken
   as "valid OpenAPI, lints clean".

5. **`apiBaseUrl` is configured, not derived.** ComputeStack owns the Lambda's
   environment but ApiGatewayStack owns the URL, and Compute is already a
   *dependency* of ApiGateway — reading `api.url` in Compute closes a cycle and
   fails synth. SSM would work but breaks on a first-ever deploy (Compute runs
   before Gateway, so the parameter does not exist yet). One config value, with
   synth-time validation for the stage-prefix trap, is the honest option. Task
   [[0126]] changes exactly this one value.

6. **The linter found real gaps, so the fix widened.** Redocly's recommended
   ruleset flagged an empty `servers` (only because `extract_openapi` did not
   stamp it — fixed by sharing `stamp_servers`), and seven key-gated operations
   documenting no `401`/`403` at all. Documenting them meant making
   `ErrorEnvelope` a published schema. All of this is inside "passes a linter
   cleanly", but it is more than a route mapping.

7. **`operation-4xx-response` kept on, with two path exceptions.** `/health` and
   `/api-docs-json` genuinely have no 4xx. Weakening the rule globally would
   have thrown away the check that caught #6, so the two paths are excepted in
   `.redocly.lint-ignore.yaml` with a note to delete the entry if either route
   ever gains an error path.

8. **`info.license` left empty; spawned [[0155]].** utoipa emits `{"name": ""}`
   from an unset `CARGO_PKG_LICENSE`. The repo has no `LICENSE` file and no
   Cargo `license` field; the only "MIT" is in a `private: true` root
   `package.json` and reads as generator boilerplate. Declaring a license in a
   public API document on that basis would be inventing a legal position, so the
   rule is off with a pointer to the follow-up. **Adam: this one needs your
   call.**

9. **`servers` stamping moved into a shared helper** so the CI-linted document
   and the served document cannot disagree. Previously `extract_openapi` emitted
   a `servers`-less variant — linting that would have blessed a document no
   reader ever receives.

10. **`docs/scf/api-endpoints.md` is a new running record**, not a line appended
    to `milestone-1-evidence.md`. M1 is submitted; its Table 4 correctly
    describes the state at submission time and rewriting it would falsify the
    record. [[0128]] cites the new file.

## Future Work

**At archive time — convert this task to a directory.** It is ~680 lines against
the ~150-line threshold in `lore/1-tasks/CLAUDE.md`, and larger than any existing
task README in the repo. Deferred deliberately (Adam, 2026-08-06), not
overlooked: the archive move is a `git mv` anyway, so doing it then is one
operation instead of two, and restructuring mid-review would have handed the PR
reviewer a large rename on the file he was reading. Proposed split — all three
are `S-` (conclusions and decisions, not research):

| Note | Content to move |
| --- | --- |
| `notes/S-openapi-validator-result.md` | "The `openapi-validator` result" (13 → 5, and why 5 stays) |
| `notes/S-pr-169-review-response.md` | "PR #169 review (okarcz, 2026-08-05)" |
| `notes/S-self-review-round.md` | "Self-review round (post-#169)" + the mutation table |

Leaves a README of ~420 lines, in line with 0088 and 0044. Also repoint
`lore/0-session/current-task.{md,json}` and check the inbound links from [[0128]]
and [[0155]].

- [[0155]] — decide and declare the API license (spawned; renumbered from 0144,
  which PR #168 had already claimed).
- Ask Oskar whether Tranche 3 AC 2's `openapi-validator` names a specific tool.
  Deliberately **not** a task: the measurement and the reasoning are recorded
  above, so it is one question, not a work item. Ask it with the cost attached —
  "IBM's specifically" now means a utoipa downgrade **and** an `ErrorEnvelope`
  redesign, not a lint pass. If the answer is "IBM's
  specifically", that is when it becomes a task — and its first line is the
  utoipa downgrade.
- [[0119]] — extend the declared ranges to the params `limit` now models
  (`type`/`sort`/`order` enums, `search` length, date ranges).
- [[0126]] — when the custom domain lands, `apiBaseUrl` and the base URL in
  `docs/scf/api-endpoints.md` change together.
- Spec-diff check in CI (fail a PR that changes the published surface without
  saying so) — `extract_openapi` makes it cheap, but it is not this task.

## Notes

- The `extract_openapi` binary stays useful for CI (spec-diff checks, client
  generation); this task adds a runtime surface, it does not replace the binary.
  It now also feeds the lint gate.
- Tranche 3 AC 2 requires *"OpenAPI spec passes `openapi-validator` lint with no
  errors; Swagger UI deployed"*. Doing the lint half now is cheap and de-risks
  M3 — hence its inclusion above. Done, with Redocly as the linter.
