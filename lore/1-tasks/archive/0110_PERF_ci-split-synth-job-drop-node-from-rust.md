---
id: "0110"
title: "CI: split synth into its own job so the rust job stops paying for a Node/TS build"
type: PERF
status: done
related_adr: []
related_tasks: ["0077", "0070"]
tags: [layer-ops, ci, cdk, priority-low, effort-small, decision, wont-do]
links: []
history:
  - date: 2026-07-21
    status: backlog
    who: okarcz
    note: >
      Spawned from the code review of 0077 / PR #134 (finding #7). The synth
      guard added there is correct but structurally misplaced: it made the
      rust job install Node and run a full TypeScript build. Recorded as
      cost-not-correctness at the time and deliberately deferred so the
      correctness fixes could land; this task carries it.
  - date: 2026-08-04
    status: active
    who: akot
    note: >
      Picked up. Starting from the measurement the task asks for: establish what
      fraction of the ~6m20s rust job is actually the Node install + TS build
      before choosing between the artifact-split and won't-do options.
  - date: 2026-08-04
    status: done
    who: akot
    note: >
      DECISION: WON'T-DO — closed under acceptance criterion 2's second clause.
      A/B measured on this branch (runs 30904595104 baseline / 30904701513
      experiment): the Node/TS tail is 29s of a 5m08s rust job, not the 2-3
      minutes the task premise assumed — off by ~5x. Only ~20s is removable;
      synth's 9s relocates rather than disappearing, and the new synth job would
      cost ~50-70s serialized AFTER rust, making PR wall-clock and billed minutes
      WORSE. The real cost centre is `Build Lambda bootstraps` at 3m24s = 67% of
      the job. Both measurement commits reverted; ci.yml is byte-identical to its
      pre-0110 state (blob 91e84b7), so the 0077 guards are untouched.
---

# CI: split synth into its own job

## Summary

`make -C infra synth-production` runs at the end of the `rust` job. That target
depends on the Makefile `build` target, which runs
`npx nx build @rumblefish/stellar-prices-api-aws-cdk` — so every `packages/**`
PR installs Node, runs `npm ci`, and builds TypeScript on the ARM runner before
it can synth.

**Resolved WON'T-DO.** The premise was that this costs 2–3 minutes. Measured, it
costs **29 seconds**, and splitting it out would make CI *slower*. See
[Measurement](#measurement).

## Context

The synth step landed in the `rust` job for a real reason: synth needs the nine
Lambda bootstraps to exist, and only the rust job builds them. Splitting the job
means moving artifacts across a job boundary, which is the whole design question
here rather than an implementation detail.

Relevant files:
- `.github/workflows/ci.yml` — `rust` job, "Synth production app" step and the
  `actions/setup-node` + `npm ci` steps immediately before it.
- `infra/Makefile:4,33-34` — `synth-production: build`, `build:` → `npx nx build`.

## Measurement

The task required measuring before choosing. Two commits were pushed to this
branch 90 seconds apart — a no-op comment (baseline) and the same file with the
Node/TS tail deleted (experiment) — so both runs shared runner class and cache
conditions.

### Baseline — run `30904595104`, rust job **5m08s**

| Step | Time |
|------|------|
| checkout + toolchain + rust-cache + cargo-lambda | 29s |
| `cargo fmt` / `check` / `clippy` / `test` | 45s |
| **Build Lambda bootstraps** | **3m24s** |
| Verify Lambda artifacts | 0s |
| `actions/setup-node@v4` | 7s |
| `npm ci` | 13s |
| `make -C infra synth-production` | 9s |

**Node/TS tail = 29s (9.4% of the job).**

### Experiment — run `30904701513`, rust job **4m17s**

Tail removed. `Build Lambda bootstraps` took 3m07s in this run versus 3m24s in
the baseline — a 17s swing on the same commit content, which is why the raw 51s
job delta must not be read as the saving. The attributable figure is the 29s
measured directly from the step timings.

### Corroboration — run `29818521440` (2026-07-21), rust job **6m17s**

setup-node 8s + `npm ci` 19s + synth 10s = **37s tail**. Same order of magnitude,
measured three weeks earlier on the run the original task cited as green.

### Why the split loses

Of the 29s, only ~20s (`setup-node` + `npm ci`) is actually *removed* from the
rust job. Synth's 9s relocates. The proposed `synth` job then pays, serialized
**after** rust because it needs rust's artifact:

    checkout ~2s + download ~110 MB artifact + chmod + re-verify
      + setup-node 7s + npm ci 13s + synth 9s  ≈  50-70s

| | rust job | PR wall-clock | billed minutes |
|---|---|---|---|
| today | 5m08s | 5m08s | 5m08s |
| after split | ~4m48s | **~5m50s** | **~5m48s** |

Both wall-clock and billed minutes get worse. The split also does **not** fix the
"the same TS build runs twice" complaint in the original summary: the
`typescript` job would still build, and the new `synth` job would build again.

`Build Lambda bootstraps` is **3m24s — 67% of the rust job**. This task optimizes
the wrong 10%.

## Acceptance Criteria

- [x] Synth still runs on every PR that could break it, and still fails on a
      missing asset. Unchanged — ci.yml is byte-identical to its pre-0110 state,
      so the 0077 behaviour verified on run 29818521440 stands as-is. No
      re-reproduction of the bad `*_ASSET_DIR` case was needed because nothing
      about the guard moved.
- [x] Closed under the second clause: *"the measured saving is documented as too
      small to justify the split and this task is closed as won't-do with that
      number recorded."* **The number is 29s, of which ~20s is removable, against
      a ~50-70s cost for the new job.** The rust job therefore still installs
      Node — deliberately.
- [x] No regression in the 0077 guards: all 9 assets still built, verified and
      synthesized; `verify-lambda-assets.sh` still runs on infra-only PRs. Byte
      identity with the pre-task blob is the proof.

## Implementation Notes

Nothing shipped to `ci.yml` — that is the outcome, not an omission.

What was actually done:

1. Pushed `5bbfbc8` (baseline marker: a 7-line comment, to force a `rust` run)
   and `a74077b` (experiment: the three tail steps deleted). Both were marked
   *do not merge* in their own body text.
2. Read step-level timings from the Actions API for both runs plus the July
   baseline.
3. Reverted both commits in one revert commit. Verified the restored blob hash
   (`91e84b7`) equals the blob at `5c37ad7` — the commit that activated this
   task — so the file is provably back to where it started.
4. Documented the finding here and in `lore/3-wiki/project/ci-pipeline.md`, and
   left a pointer comment at the `setup-node` step in `ci.yml` itself.

A complete implementation of option 1 (artifact upload + third `synth` job,
~82 lines, including the fix for zip not carrying the Unix executable bit) was
written before the measurement landed and is **not** committed. It was never
CI-tested. It is preserved in a git stash on this branch — if the decision is
ever revisited, recover it rather than rewriting it:

    git stash list | grep lore-0110

## Issues Encountered

- **The premise was wrong by ~5x.** The task summary asserts "roughly 2–3
  minutes of TypeScript work" and it was never measured — it came from a code
  review estimate in 0077. Actual: 29s. This is the whole reason the task asked
  for a measurement first, and the reason that instruction was correct.
- **Run-to-run variance nearly produced a false positive.** The two job totals
  differ by 51s, which is superficially a satisfying result and would have
  "confirmed" the split. But `Build Lambda bootstraps` alone varied 17s between
  the runs. Step-level timings, not job totals, were required to get an honest
  number.
- **Implementation preceded measurement.** The option-1 code was written before
  the A/B ran, which is the wrong order for a task whose central instruction was
  *"measure before choosing."* Caught before it was committed; cost was wasted
  effort only.

## Design Decisions

### From Plan

1. **Measure before choosing.** The task specified this explicitly and it was
   decisive — the split looked obviously correct on the stated premise and is
   obviously wrong on the measured one.

### Emerged

2. **A/B with two throwaway commits rather than reasoning from one run.**
   Comparing against a historical run would have confounded the tail with cache
   state, runner assignment, and three weeks of dependency drift. Two commits 90
   seconds apart on the same branch isolates the variable. Both were reverted.

3. **Reverted rather than force-pushed the branch clean.** The measurement
   commits are evidence for this decision. A reader of the branch history can
   see the experiment and the revert, which is more useful than a branch that
   silently never had them.

4. **Stashed rather than deleted the option-1 implementation.** It is working,
   non-trivial code (the executable-bit restore after artifact download is a
   real trap, correctly handled). If the bootstrap build ever shrinks enough to
   change the arithmetic, this becomes worth revisiting and the code should not
   have to be rewritten.

5. **Left a pointer comment in `ci.yml` at the `setup-node` step.** The three
   tail steps look exactly like something to clean up. Without a note carrying
   the number, this task gets re-opened by the next person who reads the file.
   This is the one deliberate deviation from byte-identity, and it is a comment.

## Future Work

- **The rust job's real cost is `Build Lambda bootstraps` (3m24s, 67%).** Worth
  investigating whether `Swatinem/rust-cache` covers `target/lambda` across runs
  at all, and whether nine `-p` crates in one `cargo lambda build --release`
  parallelize. Not spawned as a task yet — raise with the owner first, since
  0110 is evidence that CI-cost intuitions here need measuring before they earn
  a task.
