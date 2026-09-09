---
id: "0239"
title: "A local deploy from macOS hits two undocumented prerequisites — the fd limit and bash 4"
type: DOCS
status: backlog
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

- [ ] A runbook states the fd-limit and bash requirements and the lambda build
      command, in the order an operator needs them
- [ ] `lambda-assets.sh` either runs on bash 3.2 or fails naming the cause
- [ ] The note that `deploy-production` does not build the bootstraps is
      written where someone about to deploy will read it
