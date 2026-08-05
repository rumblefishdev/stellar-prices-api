---
id: "0145"
title: "CI: cdk synth never runs on infra-only PRs — the 0070 guard has a hole"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0110", "0077", "0070"]
tags: [layer-ops, ci, cdk, synth, paths-filter, priority-medium, effort-small]
links: []
history:
  - date: 2026-08-05
    status: backlog
    who: akot
    note: >
      Spawned from 0110 future work, itself found by the PR #165 review of that
      task. 0110 ticked an acceptance criterion asserting "synth still runs on
      every PR that could break it"; the reviewer checked and it does not. The
      overclaim has been withdrawn in 0110; the gap is this task.
---

# CI: `cdk synth` never runs on infra-only PRs

## Summary

`make -C infra synth-production` runs in the `rust` job. That job's paths filter
is `packages/**`, `Cargo.{toml,lock}`, `.github/workflows/ci.yml`,
`tools/scripts/**`. The CDK source directory is **not in it** — `infra/**`
appears only in the `typescript` filter, and the `typescript` job never invokes
synth.

So a PR that changes only CDK source gets linted, built and typechecked, and
never synthesized. A stack-level construct error goes green and surfaces at
deploy — which is exactly how task 0070's `CannotFindAsset` reached production
in the first place.

## Context

Task 0077 added the synth step to close 0070. It works, for the PRs it runs on.
Task 0110 then measured whether synth should move to its own job, concluded
won't-do, and in the process ticked *"Synth still runs on every PR that could
break it."* The PR #165 reviewer tested that claim against the paths filters and
it is false in the one direction that matters: the PRs most likely to break
synth are the ones that edit the CDK app, and those are precisely the PRs that
do not run it.

`verify-lambda-assets.sh` (which *does* run on infra-only PRs) closes only the
asset-name-mapping subset — it asserts every `Code.fromAsset` path maps to a
buildable crate. It compiles nothing and instantiates no stacks, so it cannot
catch a construct error, a bad prop, or a malformed CFN resource.

Related: `synth-cicd` is never run by CI at all.

Relevant files:
- `.github/workflows/ci.yml` — the `changes` job's two filters, and the `rust`
  job's `if:` guard.
- `lore/3-wiki/project/ci-pipeline.md` — the job/filter table.
- `infra/Makefile` — `synth-production`, `synth-cicd`.

## Implementation

The obvious fix — adding `infra/**` to the `rust` filter — is the expensive one:
it makes every infra-only PR build nine Lambda bootstraps, which 0110 measured
at **3m24s, 67% of that job**, to run a 9s synth. Weigh at least these:

1. **Add the CDK source directory to the `rust` filter** (`infra/**`). One line,
   correct, and turns a ~45s infra PR into a ~5m one.
2. **Synth in the `typescript` job against stub assets.** Synth needs the
   bootstrap *files* to exist, not to be real binaries — `Code.fromAsset` hashes
   whatever is there. Touching nine empty files would let synth run on a
   Node-only runner in seconds. Trades away the "the assets are real" half of
   what the rust-job synth proves, which `verify-lambda-assets.sh` partly
   covers already. **Check first whether an empty file actually satisfies the
   asset bundling** — if CDK rejects it, this option dies.
3. **A third job**, synth-only, on the `infra/**` filter with stub assets.
   Same trade as 2 with clearer separation and one more job's fixed cost.
4. **Accept and document.** Legitimate if the deploy pipeline synths before it
   deploys — worth confirming whether `make deploy-production` already does,
   since that would make this a "fails later, not never" bug rather than a
   silent one.

Do not repeat 0110's mistake: **measure the added CI cost before choosing.**
0110's own premise was wrong by 5x for want of one measurement.

## Acceptance Criteria

- [ ] An infra-only PR introducing a stack-level construct error fails CI.
      Demonstrate with a deliberately broken construct on a scratch branch —
      red before the fix, green after reverting the break.
- [ ] The added cost to an infra-only PR is measured and recorded, not
      estimated.
- [ ] `packages/**` PRs still synth with the real bootstraps — whatever changes
      here must not weaken the guard 0077 added for the case it already covers.
- [ ] `lore/3-wiki/project/ci-pipeline.md` job table and "Known gaps" updated to
      match whatever ships.
- [ ] A decision recorded on `synth-cicd` — either wire it into CI or note why
      it exists unrun.
