# CI pipeline

Current state of `.github/workflows/ci.yml`. Measured 2026-08-04; the ClickHouse
integration-test steps were added by task 0275 (2026-09-18).

## Shape

Three jobs. `changes` fans out to two workers via `dorny/paths-filter`, so a PR
pays only for the stacks it touches.

```
changes (ubuntu-latest, ~7s)
├── typescript   if paths-filter `typescript`   ubuntu-latest      ~45s
└── rust         if paths-filter `rust`         ubuntu-24.04-arm   ~5m
```

Both workers also run unconditionally on `push` to `master`.

| Job          | Runs when                                                                                            | Does                                                                                                                     |
| ------------ | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------ |
| `changes`    | always                                                                                               | paths-filter → `rust` / `typescript` booleans                                                                            |
| `typescript` | `libs/**`, `infra/**`, `package*.json`, `tsconfig*.json`, `nx.json`, `ci.yml`, `tools/scripts/**`    | `nx format:check`, `nx run-many -t lint build typecheck`, the `#[ignore]` guard's tests, `verify-lambda-assets.sh`       |
| `rust`       | `packages/**`, `Cargo.{toml,lock}`, `ci.yml`, `tools/scripts/**`, `docker-compose.yml`, `scripts/**` | `cargo fmt/check/clippy/test`, the 229 ClickHouse integration tests, build 9 Lambda bootstraps, verify them, `cdk synth` |

`ci.yml` and `tools/scripts/**` appear in **both** filters deliberately — the
Lambda asset guards live in those scripts, and without the entry a PR touching
only a guard would run no job and CI would never exercise the one file both jobs
depend on.

`docker-compose.yml` and `scripts/**` are in the `rust` filter because the
integration tests run against the ClickHouse the compose file pins and one of
them needs the proxy in `scripts/` — a PR that bumps the pin or breaks the proxy
must run the job that depends on them.

## The ClickHouse integration tests

Owned by task 0275. Until then every ClickHouse integration test sat behind
`#[ignore]` and ran only by hand, so a guard like 0215's pivot-set test could not
fail the build. Now all of them run in the `rust` job on every Rust PR.

**The inventory is derived, never listed.** `tools/scripts/ignored-tests.sh`
classifies every `#[ignore]` under `packages/` by its reason — a closed
vocabulary of three prefixes: `requires ClickHouse` (run), `requires public
network` and `requires production` (recorded, never run: third-party uptime and
production state must not gate a PR). A bare or unknown reason, an `#[ignore]`
outside `packages/*/tests/*_it.rs`, a test target holding two classes, a
ClickHouse target sharing its name with another `_it` target, or an empty
inventory fails the build. At 0275: 229 ClickHouse tests in 31 targets across 12
crates; 5 network, 5 production.

The steps, in order, and what each one guards:

| step                                             | guards                                                                                                                                                                                                          |
| ------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Start ClickHouse` (right after checkout)        | `docker compose up -d clickhouse` early, so the container's startup overlaps the compile (the pull happens inside the step). The pin is `docker-compose.yml`'s — one copy.                                      |
| `Classify #[ignore]d tests`                      | `ignored-tests.sh check` — database-free, seconds; a vocabulary violation fails before anything expensive runs                                                                                                  |
| `Wait for ClickHouse and assert its version`     | `up --wait --wait-timeout 120`, then `preflight`: a host-side retry (the in-container healthcheck is satisfied by the image's temporary initdb server), then `version()` equals the pin and `timezone()` is UTC |
| `Apply the ClickHouse schema`                    | `prices-clickhouse-init -- --rollups` — the mounted `init.sql` creates the database but not the rollup MVs                                                                                                      |
| `Start the ClickHouse reverse proxy`             | `scripts/ch-proxy-0281.sh up` (`caddy:2.11.4`) — `execution_bound_error_it` is vacuous without a proxy in the path (task 0281)                                                                                  |
| `ClickHouse integration tests`                   | `ignored-tests.sh run`: ONE `cargo test --workspace --test …` invocation, `--no-fail-fast -- --ignored --test-threads=1`; red unless passed == the derived count AND every target printed a summary             |
| `ClickHouse and proxy logs` (`if: failure()`)    | dumps the container logs — the image's entrypoint echoes every decision it makes                                                                                                                                |
| `Stop ClickHouse and the proxy` (`if: always()`) | frees the 4 CPU / 8 GB runner before the release Lambda build                                                                                                                                                   |

Two further guards sit outside the table. The unit step `cargo test --workspace`
runs with `CLICKHOUSE_URL=http://127.0.0.1:9`: ClickHouse is already listening on
the tests' default URL by then, so without it a ClickHouse test that lost its
`#[ignore]` would pass there quietly and shrink the inventory instead of failing
as it did before 0275. And the ClickHouse steps carry `timeout-minutes` (5 / 10 /
3 / 20): the tests set no request timeout and the proxy allows 7200 s, so one hung
query would otherwise hold the runner for the 360-minute job default — and a
cancelled job skips the `failure()` log dump.

The two count assertions are deliberate: the summed `passed` catches a test that
stopped running, and the number of `test result:` lines catches a test binary
killed by a signal or the runner's OOM ceiling, which prints no summary at all.

`--test-threads=1` is also deliberate (decided with Adam, 2026-09-18) — several
targets write the shared `prices` database, and two of them were measured flaky
in parallel. It costs **~100 s of test time serial vs ~50 s parallel**, measured
on the workstation (three runs at 99.8–101.2 s). The first CI runs are the
number to record here next to `Build Lambda bootstraps`' 3m24s.

The guard's own tests (`npm run ignored-tests:verify-guard`, `node:test` over
fixture trees) run in the `typescript` job, which pins Node from `.nvmrc`. When
the infra Nx `test` target from PR #325 lands, they belong there — a one-line
follow-up, not a dependency.

**Two traps a future editor must not undo:**

- **Never cache or restore the `clickhouse-data` volume.** The image runs
  initdb only on an empty data dir; a warm volume skips `init.sql` and
  `CREATE DATABASE prices` while printing `Skipping initialization`.
- **`CLICKHOUSE_DEFAULT_ACCESS_MANAGEMENT: 1` in `docker-compose.yml` is
  load-bearing.** With `CLICKHOUSE_USER=default` and no password, it is the only
  clause in the image's entrypoint that opens the `default` user to `::/0`.
  Delete it and every test fails at connect time.

Run them locally exactly as CI does: `scripts/ch-proxy-0281.sh up`, then
`CLICKHOUSE_PROXY_URL=http://localhost:8124 tools/scripts/ignored-tests.sh`.
Never two runs against one server at once.

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
| ClickHouse integration tests (task 0275)¹             | —         | —       |
| **Build Lambda bootstraps**                           | **3m24s** | **67%** |
| Verify Lambda artifacts                               | 0s        | —       |
| `actions/setup-node` + `npm ci` + `cdk synth`         | 29s       | 9%      |
| **total**                                             | **5m08s** |         |

¹ Added after this measurement; ~100 s of serial test time on the workstation
plus schema bootstrap and image pull, not yet measured in CI. See
[The ClickHouse integration tests](#the-clickhouse-integration-tests).

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
