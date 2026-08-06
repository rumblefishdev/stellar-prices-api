---
id: "0124"
title: "Expose the OpenAPI spec through API Gateway — /api-docs-json is unroutable in production"
type: FEATURE
status: active
related_adr: ["0008"]
related_tasks: ["0119", "0120", "0128"]
tags: [layer-infra, layer-backend, priority-medium, effort-small, milestone-M2, api-gateway, openapi, documentation]
milestone: 2
links:
  - "../../../packages/prices-api/src/lib.rs"
  - "../../../infra/src/lib/stacks/api-gateway-stack.ts"
  - "../../../docs/scf/milestone-1-evidence.md"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Authored as part of the M2 task set ([[0117]]). Closes the "Swagger"
      row of `milestone-1-evidence.md` Table 4, which states the axum router
      defines `/api-docs-json` but the API Gateway does not map it. Scoped to
      the **spec**; the Swagger **UI** and the onboarding portal stay
      Tranche 3 per overview §9.
  - date: 2026-08-04
    status: active
    who: akot
    note: >
      Promoted to active, picked up by Adam. Scope unchanged: the spec
      document over the deployed API, not the Swagger UI. First open
      question is the auth posture — the task recommends anonymous to
      match `/health` (the keyless-mock block in `api-gateway-stack.ts`);
      the decision needs
      recording either way before implementation.
  - date: 2026-08-06
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
      passes Redocly's recommended ruleset with 0 errors / 0 warnings
- [x] `servers` resolves to a URL that actually serves the API, stage path
      included — stamped from config, invariant enforced at synth; the fetch
      needs a deploy
- [x] Route coverage matches the deployed router exactly, both directions —
      enforced twice: a fast in-process test, and a CI check deriving both sides
      from the synthesized template and the extracted document
- [x] Security scheme (`x-api-key`) declared for the key-gated routes
- [x] Response cached with a TTL appropriate to a per-deployment-static document
      — 3600 s, gateway + handler agreeing
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
- `lib.rs` — the spec response carries `Cache-Control: public, max-age=3600`
  (new `cache_control::DEPLOY_STATIC` tier).
- `bin/extract_openapi.rs` — reads `AppConfig::from_env()` and stamps `servers`,
  so the linted document is the served document rather than a variant of it.
- `tests/openapi.rs` — 7 new tests (route coverage both ways, security scheme,
  per-route auth posture, `servers` stamp incl. stage path, reachable with the
  gate armed, cache header, OpenAPI 3.x).

**Infra**

- `api-gateway-stack.ts` — `GET /api-docs-json` as a keyless Lambda proxy
  (`apiKeyRequired: false`), 3600 s stage-cache TTL via `CACHE_TTL.apiDocs`.
- `types.ts` / `envs/production.json` / `compute-stack.ts` — new `apiBaseUrl`
  config → `API_BASE_URL` on the api-handler, validated at synth (https, no
  trailing slash, stage path present for execute-api hosts).

**Lint gate**

- `redocly.yaml`, `.redocly.lint-ignore.yaml`, `tools/scripts/extract-openapi.sh`,
  `npm run openapi:{extract,lint}`, and a CI step in the `rust` job (the only
  job with both cargo and node).

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

`npx ibm-openapi-validator target/openapi.json --errors-only` — **13 errors,
now 5**. Kept here rather than in a spawned task: the fixable half was fixed,
and the rest are decisions with reasons, not open work.

### Fixed (8)

| Error | Count | Why it was real |
| ----- | ----- | --------------- |
| `ibm-integer-attributes` on 5 ledger fields | 5 | A Stellar ledger sequence is `uint32` in the protocol's `LedgerHeader`. The DTOs carry `u64` because ClickHouse returns `UInt64`, so the document promised a range 4 billion times wider than reality. `maximum: 4294967295` is a domain fact, not a limit we impose. |
| `ibm-integer-attributes` on the `limit` param | 1 | The 1..=200 bound was **already enforced** (`limit == 0 \|\| limit > MAX_LIMIT` → 400) and entirely invisible to clients — a caller sending `limit=500` got a 400 the document did not explain. Now `minimum: 1, maximum: 200`. [[0119]] owns extending this to the remaining params. |
| `ibm-operation-summary-length` on `/health` | 1 | utoipa publishes the whole rustdoc as `summary` — 223 characters of maintainer-facing prose where a one-line label belongs. Split into `summary` + `description`. |
| `ibm-integer-attributes` on `Candle.trade_count` | 1 | Reversed on review — see below. `maximum: 9007199254740991` (`2^53 - 1`). |

### Left, deliberately (5)

| Error | Count | Why not |
| ----- | ----- | ------- |
| `ibm-schema-type-format` — "invalid type" | 2 | `Option<T>` renders as `oneOf: [{type: null}, …]`. `"type": "null"` is valid OpenAPI 3.1 / JSON Schema 2020-12; the validator is applying 3.0's type list. |
| `$ref` must not sit beside other properties | 2 | Same construct: `{$ref, description}`. Legal in 3.1, illegal in 3.0. Removing it would mean dropping the field descriptions. |
| `ibm-path-segment-casing-convention` | 1 | `/api-docs-json` is not snake_case. Kept — see below. |

### The `trade_count` reversal

This was originally left as "no truthful maximum exists", reasoning that a trade
count has no protocol bound, `u64::MAX` exceeds JSON's safe-integer range, and
anything smaller would be invented. The first two halves of that are right and
the conclusion still didn't follow: **the ceiling is the safe-integer range
itself.**

`2^53 - 1` is the largest integer an IEEE 754 double represents exactly, and
JSON has no integer type — so above it a client's parser silently rounds
(`JSON.parse("9007199254740993")` yields `…992`). Publishing `maximum:
9007199254740991` therefore states a fact about the wire format, not a limit we
impose: values above it cannot be delivered correctly whatever the database
holds. That is the same *kind* of claim as the ledger bound, sourced one layer
down — the ledger ceiling is a protocol fact, this one is a transport fact.

Stellar's real volumes sit ~10 orders of magnitude below it, so it never binds
in practice and cannot make a future response contradict the document — which
was the actual worry behind the original decision. Same caveat as the ledger
fields, noted in review: it is a published ceiling, not a runtime clamp.

The four 3.1 entries are the same root cause and the concrete form of design
decision #4: they are not quality signals, they are a tool that predates 3.1.
If the project ever concludes it must satisfy a 3.0-era validator, that is a
dependency decision (downgrade utoipa) with a cost far beyond this task.

**Decided: stay at 5.** Reaching 0 is available and was measured, not assumed —
`ibm-openapi-validator -r <ruleset>` accepts a Spectral ruleset, and switching
off `ibm-schema-type-format`, `no-$ref-siblings` and
`ibm-path-segment-casing-convention` produces "passed the validator". It is not
taken, for two reasons.

The document is already right: `type: "null"` and `{$ref, description}` are
valid 3.1, so removing the findings would mean either disabling rules globally
(broader than the two narrow path entries in `.redocly.lint-ignore.yaml`) or
down-converting to 3.0 before linting — and the latter breaks decision #9 by
making the linted document stop being the served one.

Second, and the part that was not known when the six were first accounted for:
**errors are not where IBM's ruleset stops.** At warning level it also reports
`ibm-error-response-schemas` against `ErrorEnvelope`, demanding a `trace` string
and an `errors` array — IBM's error-container shape, not ours. No ruleset toggle
removes that honestly, and satisfying it means redesigning the error body on
every endpoint, breaking every client. So "adopt IBM's validator" is not a lint
cleanup; it is an API redesign plus a utoipa downgrade. That materially raises
the cost of a "yes, IBM's specifically" answer to the open question for Oskar,
and should be said out loud when asking it.

### The path question — decided: keep `/api-docs-json`

The AC allowed "or the agreed public path", and this PR is the last cheap moment
to rename, so it was weighed rather than defaulted:

- It already exists in the axum router and predates this task.
- `milestone-1-evidence.md` documents it as the path the router defines.
- Hyphenated path segments are ordinary REST; snake_case is IBM house style, not
  a spec requirement. `/openapi.json`, the industry convention, fails the same
  rule.

Renaming buys one lint error against churn in a submitted document and a broken
path for anyone already reading it.

## Verification

- `cargo test --workspace` — 223 passed, 0 failed
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

After the PR #169 review fixes:

- `cargo test -p prices-api --test openapi` — 8 passed (was 7), including the
  new ledger-ceiling test; mutation-checked as described above
- `cargo clippy -p prices-api --all-targets`, `npx prettier --check` — clean
- `npm run openapi:lint` — still valid, 0 errors / 0 warnings, 2 ignored
- `npm run openapi:verify-routes` — still 9/9; the rewritten `fullPath()`
  confirmed to exit 1 on both an unresolved parent and a resource cycle
- `cdk synth` **not** re-run for the review fixes: the only infra edit is a
  comment, so the synthesized template is unchanged (verify-routes ran against
  it and still agrees on all 9 routes)

**Not verified — needs a deploy.** Two ACs are about the *deployed* API: the
live `GET …/production/api-docs-json` fetch and confirming the advertised
`servers` URL serves a route. Both are one `make -C infra deploy-production`
away, and `docs/scf/api-endpoints.md` carries the curl to confirm.

## PR #169 review (okarcz, 2026-08-05)

All seven points taken; nothing declined. Two of them were about guards that
looked like guards, which is the same failure this task exists to fix.

1. **`0144` collided (blocking).** PR #168 had already claimed it for the
   BE-0199 USD read-surface defects, unmerged when this branch was cut, so the
   ID never showed in the tree. That side is cited by 0145–0151 and 0154 plus
   the phase plan; this side moved. Renumbered to **0155** — 0152 (#172), 0153
   and 0154 are all taken, and 0153's own renumber note reserving 0152 for this
   task has been overtaken. Five sites, not four: the reviewer's list plus
   `redocly.yaml:26`, where the `info-license-strict: off` comment points here.

2. **The route-drift gate never ran on the PRs most likely to trip it.**
   `openapi:verify-routes` lives in the `rust` job and `infra/**` was not in its
   paths filter, so an infra-only PR adding a gateway route skipped the only
   check that sees the gateway→spec direction — and adding a gateway route *is*
   a pure-infra edit, while adding an axum route touches `packages/**`. So the
   uncovered direction was the more likely one. Fixed by listing
   `infra/src/lib/stacks/api-gateway-stack.ts` specifically, not `infra/**`, so
   unrelated CDK edits do not pay for an ARM Rust build. Same hole class this PR
   already closed twice (`package.json`, `tools/scripts/**`); the general form
   is [[0153]].

3. **`LEDGER_SEQ_MAX` asserted a tautology.**
   `assert!(LEDGER_SEQ_MAX == 4_294_967_295)` only restated
   `u32::MAX as u64 == 4_294_967_295`; it tied the five `#[schema(maximum = …)]`
   literals to nothing. Const and assert deleted, replaced with
   `every_ledger_field_publishes_the_uint32_ceiling` in `tests/openapi.rs`,
   which reads the bounds back out of the served document — the artifact-derived
   form the rest of this PR argues for. Its field set is derived from the
   document (any `*_ledger` / `ledgers_remaining` property) rather than listed,
   so a ledger field added later without the attribute fails as a missing
   `maximum` instead of passing unnoticed; a count assertion stops a rename from
   emptying the filter and passing vacuously. Mutation-checked: setting one
   literal to `4_294_967_296` leaves the old assert green and fails the new test
   with the field named.

4. **`/health` is the precedent for the posture, not the cost profile.**
   Correct — `/health` is a `MockIntegration` and can never invoke anything;
   `/api-docs-json` is `proxy([])`, so a cache miss reaches the Lambda, and it
   sits outside the usage plan with only the stage-wide throttle. The
   mitigations the reviewer names are real and already in place (3600s TTL with
   no cache-key parameters, so all callers collapse onto one entry; API
   Gateway's default `requireAuthorizationForCacheControl: true` blocks
   anonymous cache-busting), so the residual stays small and the posture stands.
   Written into the stack comment so it stops reading as "same cost profile",
   with the lever named for anyone who needs a harder bound: a method-level
   throttle, not a key requirement.

5. **Method sets disagreed between the two guards.** `spec_routes()` matched
   `head`/`options`; `verify-openapi-routes.mjs` drops both from both sides so
   0126's `addCorsPreflight` does not read as drift. Rust filter aligned to
   `HTTP_METHODS`, with the reason stated in both files.

6. **`fullPath()` truncated silently.** Both exits returned a *partial* path
   despite the comment promising to fail loudly — which surfaces as drift on a
   path that looks almost right, not as the parse failure it is. Both now throw,
   caught at the call site and printed as `error:`. Verified against a mutated
   template in each direction: an unresolved parent and a resource cycle both
   exit 1 with the resource named (the unresolved-parent case previously
   reported `/assets` as undocumented drift). The root-method check moved above
   the `ANY` check so the `ANY` message always has a path to name.

7. **`extract-openapi.sh` used `node -p "require('…json')"`.** Switched to
   `JSON.parse(readFileSync(…))`. Worth noting the stated hazard does not
   reproduce: `node -p` is still evaluated as CommonJS under
   `"type": "module"` (measured on v26), and the root `package.json` has no
   `type` anyway. The change stands because the old form depended on both of
   those staying true and the new one depends on neither.

**Double extraction** (`openapi:lint` and `openapi:verify-routes` each chaining
`openapi:extract`) left as-is, per the reviewer — the chaining is what makes
each script correct standalone, and the second `cargo run` is a no-op rebuild.

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

3. **3600 s TTL**, gateway and handler agreeing, per the `CACHE_TTL` /
   `cache_control.rs` single-source-of-truth rule.

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
