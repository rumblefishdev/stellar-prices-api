---
id: "0141"
title: "make deploy-production-compute ships whatever is in target/lambda/ — no freshness check against the tree"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0072", "0077", "0070", "0132"]
tags: ["priority-high", "effort-small", "deployment", "footgun", "infra"]
links:
  - "../../../docs/runbooks/0072-current-prices-mv-rollout.md"
history:
  - date: 2026-08-03
    status: backlog
    who: okarcz
    note: >
      Hit live during [[0072]] step 6. `make deploy-production-compute` shipped a
      **stale `prices-api` binary** to production — the deploy reported success,
      the CDK diff looked healthy (a clean S3Key change), and the endpoint served
      stubs. Only step 7's response-content check caught it. Rebuilding with
      `cargo lambda build` and redeploying fixed it.
---

# The deploy path ships stale Lambda binaries and reports success

## Summary

```make
deploy-production-compute: build
	npx cdk --app "$(PRODUCTION_APP)" deploy Prices-production-Compute …

build:
	cd .. && npx nx build @rumblefish/stellar-prices-api-aws-cdk
```

`build` compiles the **CDK TypeScript only**. The Lambda code comes from
`Code.fromAsset('../target/lambda/prices-api')` (`compute-stack.ts:77,455`) —
whatever binary happens to be on disk. **Nothing in the deploy path builds the
Rust, and nothing checks the artifact against the tree.**

## Why it is dangerous rather than merely inconvenient

Every signal a careful operator would check says the deploy worked:

- `cdk diff` shows a **clean, plausible S3Key change** — because the stale local
  artifact genuinely differs from what CloudFormation last recorded.
- The deploy reports `✅` and prints outputs.
- `GET /health` passes — it is a **keyless API Gateway mock**, so it passes even
  when the handler is entirely wrong.
- The response **shape** is correct, because both builds serialise the same DTO.

The only thing that caught it was step 7 asserting response *content*
(`sources` populated). Had that gate been the pre-0138 wording — which checked
`change_24h_pct != "0"` — it would have passed on the stub too, and the rollout
would have been declared complete while production served stubs.

## What happened (2026-08-03)

0072 step 6 deployed a `prices-api` artifact predating PR #150's pass-through
handler. `/v1/assets/native/price` returned `sources: {}` with `price_xlm: "0"`
and `change_24h_pct: "0"` — the stub's hardcoded values.

Diagnosis was slower than it should have been because **XLM is a degenerate
probe**: its real CH values are *also* `{}` / `0` / `0`, so both builds emit a
byte-identical response for `native`. Discriminating required an asset with a
populated `sources` (`USDCAllow`).

Fix was `cargo lambda build --release --arm64 --features lambda -p prices-api`
then redeploy — after which `price_xlm` and `change_24h_pct` returned real
values.

⚠️ **The same trap applies to every other Lambda in the stack.** The 0072 deploy
also shipped `prices-ledger-processor` from `target/`; it happened to be a
build made after [[0132]]'s fix merged, so nothing broke — but that was luck,
not process. Had it predated the fix, the deploy would have silently reverted the
99.9% egress reduction behind an equally clean-looking diff.

## Implementation

Options, roughly in order of preference:

- **Make `deploy-*` depend on a Rust build target.** CI already has the correct
  invocation, driven by `tools/scripts/lambda-assets.sh` so the list cannot
  drift (that script exists because of [[0077]], where a hand-maintained list
  caused `CannotFindAsset`). Reuse it:
  `cargo lambda build --release --arm64 --features lambda -p <each>`.
  Note this is a cross-compile on an x86 workstation (zig); CI uses a native ARM
  runner.
- **Or fail fast**: a preflight that compares each `target/lambda/*/bootstrap`
  mtime against the newest source file in its crate, and refuses to deploy on a
  stale artifact. Cheaper, keeps the build explicit, and turns a silent wrong
  deploy into a loud stop.
- **Or deploy only from CI**, which builds and verifies bootstraps
  (`.github/workflows/ci.yml`, "Build Lambda bootstraps" / "Verify Lambda
  artifacts"), and drop local prod deploys entirely.

Whichever is chosen:

- The 0072 runbook's step 6 should say explicitly that the Rust must be built
  first, and the "it also heals the 0132 CFN drift" note should warn that the
  healing is only correct if `target/` is current.
- Keep the step-7 gate on response **content**, never on deploy success.

## Acceptance Criteria

- [ ] A stale `target/lambda/*` artifact cannot reach production silently —
      either it is rebuilt automatically or the deploy refuses.
- [ ] Verified by deliberately staling an artifact and confirming the failure
      mode is loud.
- [ ] The 0072 runbook records the Rust-build prerequisite.
- [ ] The `/health` mock is documented as unusable for post-deploy verification.
