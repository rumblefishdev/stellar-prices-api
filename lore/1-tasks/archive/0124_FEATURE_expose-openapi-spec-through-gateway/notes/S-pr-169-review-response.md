---
title: "PR #169 review response (okarcz, 2026-08-05)"
type: synthesis
status: mature
spawns: []
tags: [code-review, ci-gates, openapi]
links: []
history:
  - date: 2026-08-06
    status: mature
    who: akot
    note: "Split out of the 0124 README at archive time, per its Future Work plan."
---

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
