---
id: "0297"
title: "Tranche 3 AC 7 says `cdk deploy` from the README works in a fresh AWS account — nobody has ever tried it"
type: TEST
status: active
related_adr: []
related_tasks: ["0294", "0239", "0141"]
tags: [layer-infra, priority-medium, effort-medium, milestone-M3, deploy, docs, scf]
milestone: 3
assignee: okarcz
links:
  - "../../../README.md"
  - "../../../infra/README.md"
history:
  - date: 2026-09-18
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from [[0294]]. The public-repo half of AC 7 is met (checked
      2026-09-16). The fresh-account half has no owner — [[0239]] was linked to
      it by mistake; it covers two macOS prerequisites of a local deploy, which
      is adjacent but not this.
  - date: 2026-09-25
    status: active
    who: okarcz
    note: >
      Activated and taken by the operator for M3 AC 7 (fresh-account
      `cdk deploy` from the README). Implementation instructions to follow.
---

# `cdk deploy` from the README in a fresh AWS account

## Summary

Tranche 3 AC 7: _"GitHub repository public; `cdk deploy` from README works in a
fresh AWS account."_ Every deploy so far has gone to the one production account,
by people who already know its history. Rehearse the README as a stranger would,
in an account with nothing in it, and fix what breaks — or state precisely what
a fresh account cannot have and declare that.

## Context

- The README documents prerequisites, `npm run infra:bootstrap`, a manual
  CicdStack step and `make deploy-production` with per-stack variants. None of
  it has been followed end to end from zero.
- **A fresh account cannot be fully self-contained.** The stack depends on
  things that live outside AWS or are created out of band: the mTLS client
  certificate and key in Secrets Manager (the README notes later deploys do not
  overwrite the uploaded secret), the ClickHouse endpoint on the shared Hetzner
  box, the Discord OAuth bundle, DNS for the custom domain. "Works" needs a
  definition: stacks synthesise and deploy with documented placeholder inputs,
  versus a serving API.
- Known traps a stranger would hit: the two macOS prerequisites in [[0239]]
  (fd limit, bash 4), Lambda assets that must be built from the commit being
  deployed ([[0141]]), and `--require-approval broadening` needing a TTY.
- The account id comes from the environment (`CDK_DEFAULT_ACCOUNT`), so nothing
  pins the deploy to production — good for this test, and the reason to be
  careful about which profile is active while running it.

## Implementation

- Get an empty AWS account (a sandbox in the organisation) and confirm the
  active profile before every command.
- Follow the README literally, from clone to deployed stacks, writing down each
  place where it is wrong, silent, or assumes knowledge.
- Decide and record what "works" means for the out-of-band inputs, and make the
  README say how to supply each (or a placeholder that lets the deploy finish).
- Fix the README; fold [[0239]]'s two prerequisites in if they are still
  missing.
- Tear the sandbox down and record that too.

## Acceptance Criteria

- [ ] The README was followed from zero in an account that had never seen this
      project, and the transcript of what broke is in this task
- [ ] "Works" is defined for the inputs a fresh account cannot have, and the
      README documents how to supply each
- [ ] Every README defect found is fixed, or listed with a reason it stays
- [ ] The result is stated for the evidence package: works as written, or works
      with the declared prerequisites — and the latter is a deviation in [[0294]]
- [ ] The sandbox is torn down

## Notes

- If no sandbox account can be had, the honest alternative is a recorded reason
  and a deviation — not a claim that it works.
