---
id: "0239"
title: "A local deploy from macOS hits two undocumented prerequisites — the fd limit and bash 4"
type: DOCS
status: completed
assignee: stkrolikiewicz
related_adr: []
related_tasks: ["0118"]
tags: [layer-infra, priority-medium, effort-small, deploy, tooling, docs]
milestone: 3
links:
  - "../../../docs/runbooks/deploy-ledger-processor.md"
  - "../../../tools/scripts/lambda-assets.sh"
history:
  - date: 2026-08-28
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0118]]'s production deploy, which hit both walls in one
      session. Neither is in a runbook and CI cannot catch either — it builds
      on Ubuntu with bash 5, a high fd limit, and a native ARM runner.
  - date: 2026-09-28
    status: active
    who: stkrolikiewicz
    note: >
      Taken for M3 AC 7: the fresh-account runbook from [[0297]] (PR #357)
      does not pass on a stock macOS and was meant to fold these two
      prerequisites in. okarcz asked for macOS to be documented and
      supported; decided 2026-09-28: document the macOS prerequisites in
      infra/README.md rather than port the scripts to bash 3.2 / BSD.
  - date: 2026-09-29
    status: completed
    who: stkrolikiewicz
    note: >
      PR #360 merged 2026-09-28 (9aedb7c4), CI green: infra/README.md §1
      "On macOS" and a macOS variant of step 4, the zig sentence fixed in
      two runbooks, three scripts refusing with the cause named (one of
      them a silently disabled target-dir guard). 5 files, no test
      changed; tools/scripts tests 41/41. Proven on macOS 26.6: 12
      bootstraps built and verified, credential-free synth. Follow-up
      PR #366 (open): the root README points macOS contributors to §1.
---

# Local deploy from macOS: two undocumented prerequisites

## Summary

`make -C infra deploy-production` does **not** build the Rust Lambda
bootstraps, and the two commands that do are both broken out of the box on a
stock macOS. Both were hit for real during [[0118]]'s deploy, and both cost
time because the failure text names neither cause.

## Context

1. **`ProcessFdQuotaExceeded` during `cargo lambda build`.** zig links ~250
   object files per binary and cargo links several binaries in parallel, so
   macOS's default soft `ulimit -n` is exhausted and the build fails at the
   link step with `failed to open object … ProcessFdQuotaExceeded`. Fix is
   `ulimit -n 61440` (the `kern.maxfilesperproc` ceiling) in the same shell,
   optionally with `-j 4`.
2. **`tools/scripts/lambda-assets.sh` cannot run.** It uses `mapfile`, a bash
   4+ builtin; macOS ships bash 3.2, so the script dies with
   `mapfile: command not found` and the operator has no list of crates to
   pass to `cargo lambda build -p …`.

CI hits neither: Ubuntu has bash 5 and a high fd limit, and the ARM runner
builds natively without zig.

## Implementation

- Add a "local deploy prerequisites" section to
  `docs/runbooks/deploy-ledger-processor.md` (or a new deploy runbook if that
  one is too narrow): the `ulimit` line, the bash requirement, and the fact
  that `deploy-production` does not build the lambdas.
- Make `lambda-assets.sh` portable — replace `mapfile` with a `while read`
  loop — or have it fail with a clear message naming the bash requirement.
  The script already refuses to emit an empty list; the same care should
  cover the interpreter.
- Record the full `cargo lambda build --release --arm64 --features lambda -p …`
  invocation, including why both `--features lambda` and the explicit `-p`
  flags are required (the bins are gated behind `required-features`, so a bare
  build silently produces only the unrelated CLIs).

## Acceptance Criteria

- [x] A runbook states the fd-limit and bash requirements and the lambda build
      command, in the order an operator needs them — `infra/README.md` §1
      Prerequisites, "On macOS", before the steps that need them (4 and 7);
      the build command is `build-lambda-assets.sh`, which step 7 runs and
      `deploy-ledger-processor.md` §1 documents (now pointing to §1)
- [x] `lambda-assets.sh` either runs on bash 3.2 or fails naming the cause —
      it fails naming bash ≥ 4 and the README section; so does
      `verify-lambda-bootstraps.sh`
- [x] ~~The note that `deploy-production` does not build the bootstraps~~ —
      obsolete: since [[0141]] every target that can ship a Lambda runs
      `build-lambdas` first, and `infra/README.md` step 7 says so

## Implementation Notes

Scope moved on 2026-09-28 from "the two walls of [[0118]]" to "the
fresh-account runbook of [[0297]] passes on a Mac" (M3 AC 7). Proven on this
MacBook (macOS 26.6, Apple Silicon), in a fresh worktree off `develop`:

- `make -C infra build-lambdas` with the documented PATH and
  `ulimit -n 61440` set from a 256 soft limit: 12 bootstraps built in
  3m37s from scratch, all verified as distinct aarch64 ELFs. Re-run after
  the script edits: cargo no-op, 12 verified.
- `make -C infra synth-production` with `AWS_CONFIG_FILE` and
  `AWS_SHARED_CREDENTIALS_FILE` set to `/dev/null`: 5 templates.
- Step 4's macOS variant run end to end on dummy certificates: 4 MB RAM disk,
  mode 700, bundle with `ca`/`cert`/`key`, `shred`, detach.
- `/bin/bash` 3.2.57 on `lambda-assets.sh` and `verify-lambda-bootstraps.sh`:
  both exit 1 naming bash ≥ 4. `node --test "tools/scripts/**/*.test.mjs"`:
  41/41.
- Not reproduced: `ProcessFdQuotaExceeded` itself (the build was not run at
  256 to watch it fail); the evidence is [[0118]]'s deploy.

Files: `infra/README.md` (zig row, "On macOS" paragraph, step 4 takes `D`
and has a RAM-disk variant), `docs/runbooks/deploy-ledger-processor.md` (zig
sentence), three guards in `tools/scripts/` (`lambda-assets.sh`,
`verify-lambda-bootstraps.sh`, `build-lambda-assets.sh`).

## Issues Encountered

- **The runbook's zig row was wrong for every Mac.** "Only on x86 machines"
  holds for Linux; macOS on Apple Silicon still cross-compiles darwin → linux
  and needs zig. The same sentence was in `deploy-ledger-processor.md`.
- **BSD `realpath` silently disabled a guard.** `build-lambda-assets.sh`
  compares `realpath -m` of cargo's target dir and `<root>/target`; macOS's
  `realpath` has no `-m`, both sides came out empty, and a mismatched
  `CARGO_TARGET_DIR` passed unchecked (reproduced with the check from
  `develop`: BSD → passes, GNU → refuses). Now refused up front, by name.
- **Step 4 had two more macOS walls:** no `/dev/shm`, no `shred` without
  coreutils.
- **zsh keeps `hdiutil`'s trailing whitespace.** `hdiutil attach` prints the
  device padded with spaces and tabs, and zsh does not word-split an unquoted
  `$RD`, so `diskutil erasevolume … $RD` gets the padding; `awk '{print $1}'`.

**Broken/modified tests:** none. The guards sit in front of the checks the
existing tests exercise; `node --test "tools/scripts/**/*.test.mjs"` passes
41/41 with the documented setup (15/41 with stock macOS tools, measured
2026-09-29).

**Follow-up:** PR #366 adds the macOS pointer to the root `README.md`
"Local development": `npx nx run-many -t test` and the git hooks reach
`infra:test`, which needs the same setup. No backlog task spawned: nothing
is left open.

## Design Decisions

### From Plan

1. **Document, do not port.** Porting the three scripts to bash 3.2 / BSD is
   ~10 spots plus the guard tests, in the CI guards of [[0070]] and [[0141]],
   days before the M3 submission. Homebrew `bash` + `coreutils` + `zig` and
   two shell lines cover all of it (decided 2026-09-28).

### Emerged

2. **Guards that name the cause, in three scripts rather than one.**
   `verify-lambda-bootstraps.sh` has its own bash-4 constructs, and
   `build-lambda-assets.sh`'s GNU dependency is the silent one.
3. **Step 4 takes `D=${D:-/dev/shm/prices-cert}`** so one block serves both
   systems; its closing `rmdir` was dropped (an empty directory in tmpfs is
   harmless, and on macOS the mount point cannot be removed that way).
