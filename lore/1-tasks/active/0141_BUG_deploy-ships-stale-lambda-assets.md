---
id: "0141"
title: "make deploy-production-compute ships whatever is in target/lambda/ — no freshness check against the tree"
type: BUG
status: active
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
  - date: 2026-08-12
    status: backlog
    who: akot
    note: >
      Two findings from [[0157]]'s production deploy that widen this beyond
      what the original report covers, both caught by reading `cdk diff`
      rather than by any guard. First, the delivery vector is not only
      `deploy-production-compute`: no per-stack target carries
      `--exclusively`, so `make deploy-production-apigateway` announced
      `Including dependency stacks: Prices-production-Compute` and would have
      deployed Compute as a side effect of a deploy that has nothing to do
      with it. Second, "stale" understates what sits on a developer machine —
      all eleven bootstraps under `target/lambda/` were byte-identical 10-byte
      files containing `#!/bin/sh`, so the ledger processor and the api
      handler resolved to a single asset hash. That deploy would have replaced
      every Lambda in production with a shell stub, not with an old binary.
      Worked around by hand with `--exclusively`; nothing in the repo stops
      the next person from missing it.
  - date: "2026-09-18"
    status: active
    who: akot
    note: >
      Activated ahead of [[0286]]'s rollout, which is a series of Compute
      deploys. Approach chosen: the deploy **builds** the Lambdas rather than
      judging their freshness — cargo is the freshness check (a no-op when the
      tree is unchanged), followed by a per-bootstrap ELF/aarch64 verification
      that catches the 10-byte `#!/bin/sh` stubs on its own. A hand-rolled
      staleness detector was rejected: the Rust sources `include_str!` two
      files from `docs/runbooks/`, so "the crate's sources" is not the input
      set. Scope also takes in `deploy-production-eventbridge`
      (`eventbridge-stack.ts` references `target/lambda` too) and
      `--exclusively` on every per-stack target.
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

- [x] A stale `target/lambda/*` artifact cannot reach production silently —
      **rebuilt automatically**: `deploy-production`, `-compute` and
      `-eventbridge` depend on `make build-lambdas`.
- [x] Verified by deliberately staling an artifact and confirming the failure
      mode is loud — four runs on 2026-09-18, see "Proof" below.
- [x] The 0072 runbook records the Rust-build prerequisite (step 6), and the
      0132-heal note now says the healing is only correct from a current tree.
- [x] The `/health` mock is documented as unusable for post-deploy verification
      — 0072 runbook step 7, `infra/README.md`, and the `build-lambdas` comment
      in `infra/Makefile`.

## Implementation Notes

- `tools/scripts/build-lambda-assets.sh` — the one `cargo lambda build
  --release --arm64 --features lambda -p …` invocation, list from
  `lambda-assets.sh`; prints branch and `git describe --dirty`; ends by running
  the verifier. CI's two inline steps ("Build Lambda bootstraps", "Verify Lambda
  artifacts") are replaced by it, so CI and the workstation share one copy.
- `tools/scripts/verify-lambda-bootstraps.sh` — refuses a bootstrap that is
  missing, not executable, not an ELF, not aarch64 (`e_machine` at offset 18),
  or byte-identical to another asset's.
- `infra/Makefile` — `build-lambdas`; the three targets above depend on it;
  all five per-stack `deploy-production-*` targets pass `--exclusively`.
- `tools/scripts/lambda-deploy-guard.test.mjs` — 8 `node:test` tests
  (`npm run test:scripts`, new step in CI's `typescript` job). The Makefile
  half reads the deploy targets from the Makefile and the Lambda-packaging
  stacks from the CDK source, so neither list is hand-maintained. Mutation
  check: dropping `build-lambdas` from `deploy-production-eventbridge` turns it
  red by name.
- Docs: `deploy-ledger-processor.md` steps 1, 2, 4 and the EventBridge snippet
  now go through `make build-lambdas`; `compute-stack.ts` header.

### Proof (2026-09-18, x86 workstation, rustc 1.97.1 / cargo-lambda 1.9.1, `CARGO_BUILD_JOBS=4`)

| start state | result | wall |
|---|---|---|
| artifacts from 2026-09-14 (pre-[[0286]]), `prices-api` overwritten with the 10-byte `#!/bin/sh` | verifier alone: exit 1, `prices-api: bootstrap is not an ELF binary (10 bytes)`. `make build-lambdas`: all rebuilt, 11 verified | 7m42s |
| nothing changed | cargo `Finished` in 0.24 s, **sha256 of all 11 identical to the previous run** | 1.0 s |
| stub written over a current `prices-api` | cargo-lambda re-places the bootstrap even though cargo compiled nothing; 11 verified | 1.0 s |
| `touch packages/prices-api/src/main.rs` | only `prices-api` recompiled; 11 verified | 43 s |

The 2026-09-14 `prices-api` (`d5686b5d…`) differed from the build of today's
tree (`a0a53fcf…`): a deploy from this machine this morning would have shipped
pre-0286 code. `make synth-production` passes against the new artifacts.

**Not verified:** a real `cdk deploy --exclusively` against the account — the
agent cannot run production deploys. The flag exists in the pinned CLI (2.1124.1)
and was used by hand on 2026-08-12. The first operator deploy is the check. The
changed CI steps have not run yet either; the PR's own run is that check.

## Design Decisions

### From Plan

1. **Build, do not detect staleness.** Decided with Adam before starting. Cargo
   owns the input set; a detector would have had to re-derive it (path deps,
   `Cargo.lock`, two `include_str!`s into `docs/runbooks/`). Measured cost of
   being wrong about "expensive": 1.0 s when current.
2. **`--exclusively` on every per-stack target**, not only the ones without a
   build. A target named after a stack deploys that stack. A dependency that is
   genuinely missing fails in CloudFormation, loudly; `deploy-production` is the
   target for "everything".

### Emerged

3. **`deploy-production-eventbridge` is in scope.** The task named Compute only;
   `eventbridge-stack.ts` packages `asset-discovery` and the scheduled workers
   from `target/lambda` too.
4. **CI now runs the same script.** Not asked for. Two copies of the build
   invocation is the 0077 failure, and the verifier is strictly stronger than the
   inline `-x` check it replaces.
5. **Duplicate-binary check.** The 2026-08-12 symptom was "one asset hash for
   every Lambda". ELF magic already catches the stubs; this catches one *real*
   binary copied over the rest, which ELF magic cannot.
6. **A dirty tree is reported, not refused.** Whether to ship uncommitted code
   is the operator's call; the build line makes it impossible to do unknowingly.
7. **`diff-production` was left alone.** A diff taken before `build-lambdas`
   shows the old artifacts' hashes; the runbooks now say build → diff → deploy
   instead. The ledger-processor runbook diffs with a raw `npx cdk diff
   <stack>` anyway, which a Makefile dependency would not reach.
8. **Tests are `node:test`, no new dependency.** `tools/scripts/` had no tests
   and `infra/` has no test runner.

## Issues Encountered

- **Group build vs single-crate build.** The runbooks told operators to build
  one crate (`-p prices-ledger-processor`); `build-lambdas` builds all eleven in
  one invocation, as CI always has. `deploy-ledger-processor.md` already warns
  that cargo feature unification can make those hash differently. Consequence:
  **the first `deploy-production-compute` after this lands may show a new S3Key
  for every Lambda in the stack**, not only the one that changed. After that the
  build mode is uniform and the hashes are stable (measured above).
- **The activation commit carried only the move** (`b3afad8`): `git add` was
  given the old backlog path too, failed on it, and the error was discarded.
  Fixed forward on `develop` in `1f4d54b`.
- **Known limit of the Makefile test:** "which stacks package a Lambda" is read
  as "which `*-stack.ts` names `target/lambda`". Moving the asset-dir literals
  into a shared module would blind it; the sanity test only demands one match.
