# CI pipeline

Current state of `.github/workflows/ci.yml`. Measured 2026-08-04.

## Shape

Three jobs. `changes` fans out to two workers via `dorny/paths-filter`, so a PR
pays only for the stacks it touches.

```
changes (ubuntu-latest, ~7s)
├── typescript   if paths-filter `typescript`   ubuntu-latest      ~45s
└── rust         if paths-filter `rust`         ubuntu-24.04-arm   ~5m
```

Both workers also run unconditionally on `push` to `master`.

| Job          | Runs when                                                                                         | Does                                                                                |
| ------------ | ------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------- |
| `changes`    | always                                                                                            | paths-filter → `rust` / `typescript` booleans                                       |
| `typescript` | `libs/**`, `infra/**`, `package*.json`, `tsconfig*.json`, `nx.json`, `ci.yml`, `tools/scripts/**` | `nx format:check`, `nx run-many -t lint build typecheck`, `verify-lambda-assets.sh` |
| `rust`       | `packages/**`, `Cargo.{toml,lock}`, `ci.yml`, `tools/scripts/**`                                  | `cargo fmt/check/clippy/test`, build 9 Lambda bootstraps, verify them, `cdk synth`  |

`ci.yml` and `tools/scripts/**` appear in **both** filters deliberately — the
Lambda asset guards live in those scripts, and without the entry a PR touching
only a guard would run no job and CI would never exercise the one file both jobs
depend on.

## The Lambda asset guards

Owned by task 0077, after task 0070 hit `CannotFindAsset` at deploy time. The
set of Lambda assets is **derived from the CDK source** by
`tools/scripts/lambda-assets.sh`, not hand-listed — three hand-maintained copies
had already drifted twice.

Two tiers, split by cost:

- **`typescript` job — `verify-lambda-assets.sh`.** Asserts every
  `Code.fromAsset` path maps to a buildable crate, via `cargo metadata
--no-deps --offline`. Compiles nothing; seconds on a Node-only runner. This
  is what catches an infra-only PR that renames an asset.
- **`rust` job — build, verify, synth.** Builds all 9 bootstraps
  (`--features lambda`, explicit `-p` per crate — a bare `cargo lambda build`
  silently skips them), asserts each expected path exists and is executable,
  then runs `make -C infra synth-production` — proof that the app the operator
  deploys actually synthesizes **against the assets this job just built**.

  Read that scope literally: synth runs in the `rust` job, so it only runs on
  PRs matching the `rust` filter. **An infra-only PR is never synthesized** —
  see [Known gaps](#known-gaps).

Both loops fail explicitly on an empty asset list rather than passing
vacuously. Synth is credential-free: every SSM read is
`valueForStringParameter` (a deploy-time CloudFormation dynamic reference), and
there are no `fromLookup` context lookups, which are what would need real AWS
credentials.

## Cost profile — `rust` job

Step timings from run `30904595104` (2026-08-04). This is the job worth knowing
the shape of; `typescript` is ~45s end to end and not worth optimizing.

| Step                                                  | Time      | Share   |
| ----------------------------------------------------- | --------- | ------- |
| setup (checkout, toolchain, rust-cache, cargo-lambda) | 29s       | 9%      |
| `cargo fmt` / `check` / `clippy` / `test`             | 45s       | 15%     |
| **Build Lambda bootstraps**                           | **3m24s** | **67%** |
| Verify Lambda artifacts                               | 0s        | —       |
| `actions/setup-node` + `npm ci` + `cdk synth`         | 29s       | 9%      |
| **total**                                             | **5m08s** |         |

**If you want to make CI faster, `Build Lambda bootstraps` is the only step that
matters.** It is two thirds of the job. Everything else is rounding error.

Note that `Build Lambda bootstraps` varies ~17s run to run on identical commit
content — compare step timings, never job totals, when measuring a change here.

## Why the `rust` job installs Node (do not "clean this up")

The three steps at the tail of the `rust` job — `actions/setup-node`, `npm ci`,
`make -C infra synth-production` — look misplaced on a Rust runner. They are
deliberate.

Synth needs the nine Lambda bootstraps, and only the `rust` job builds them.
Moving synth to its own job means shipping ~110 MB of binaries across a job
boundary via `actions/upload-artifact`, and the new job would be serialized
after `rust` because it depends on that artifact.

Task 0110 measured the trade in both directions:

|                             | rust job | PR wall-clock  |
| --------------------------- | -------- | -------------- |
| today                       | 5m08s    | 5m08s          |
| with a separate `synth` job | ~4m39s   | ~5m29s – 5m49s |

The tail costs **29s**, all of which leaves the `rust` job — but only ~20s
disappears, because synth's own 9s reappears in the new job. That job costs
~50–70s (checkout + artifact download + `chmod` + re-verify + `setup-node` +
`npm ci` + synth), serialized after `rust` because it needs its artifact. The
split also adds an `actions/upload-artifact` step for ~110 MB that was never
measured, so the low end of that range is optimistic.

Splitting makes wall-clock **worse**, and does not even remove the duplicate
TypeScript build, since `typescript` and `synth` would each run one.

These are elapsed times, not invoice figures — GitHub bills per job rounded up
to the whole minute at per-runner-class rates. Today that is 6 ARM minutes;
after a split, 5 ARM minutes plus 1–2 on the `synth` runner. The conclusion
holds either way.

Closed won't-do. Full numbers and method:
[0110](../../1-tasks/archive/0110_PERF_ci-split-synth-job-drop-node-from-rust/README.md).
A complete but never-CI-tested implementation of the split is committed at
[`notes/G-option-1-synth-split.md`](../../1-tasks/archive/0110_PERF_ci-split-synth-job-drop-node-from-rust/notes/G-option-1-synth-split.md)
if the arithmetic ever changes — which it would only do if the bootstrap build
shrank by an order of magnitude.

## Known gaps

### `cdk synth` never runs on infra-only PRs

The `rust` job owns synth, and its filter is `packages/**`, `Cargo.{toml,lock}`,
`ci.yml`, `tools/scripts/**`. The CDK source directory is **not among them**. A
PR touching only `infra/` therefore runs the `typescript` job, which lints,
builds and typechecks but never synthesizes. A stack-level construct error goes
green and surfaces at deploy, which is how 0070's `CannotFindAsset` happened.

`verify-lambda-assets.sh` does run on those PRs, but it only asserts that each
`Code.fromAsset` path maps to a buildable crate — it instantiates no stacks.

This is the larger of the two gaps and the one most likely to bite. Owned by
[0153](../../1-tasks/backlog/0153_BUG_synth-not-run-on-infra-only-prs.md). Note
also that `synth-cicd` is never run by CI at all.

### `develop` pushes run no CI

`on: push` covers `master` only, while PRs target `develop`. A direct push to
`develop` therefore runs nothing. Largely defused in practice because the paths
filters catch the relevant PRs, but it is a real hole if branch protection ever
comes to depend on these checks. Flagged as out of scope in 0110 — it is a
trigger-policy decision, not a job-cost one, and needs its own task.
