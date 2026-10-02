---
id: "0297"
title: "Tranche 3 AC 7 says `cdk deploy` from the README works in a fresh AWS account — nobody has ever tried it"
type: TEST
status: completed
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
  - date: 2026-09-25
    status: completed
    who: okarcz
    note: >
      Closed by documentation, the way the explorer claimed its own Tranche 3
      AC 2: public repo + a fresh-account runbook with the Hetzner side and a
      manual-by-design list (operator decision 2026-09-25; not a deviation).
      PR #357 merged (23f457a2): new root README.md (the repo had none),
      infra/README.md rewritten as the runbook and corrected against the CDK
      code, `make bootstrap` fixed for a fresh clone. 3 files, +548/-169.
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

Reworded 2026-09-25 to the operator's decision: AC 7 is claimed on the runbook,
as the explorer claimed its AC 2 — the sandbox criteria no longer apply.

- [x] The root README leads a stranger to a fresh-account deployment runbook
      (`README.md` → `infra/README.md` "Fresh-account deployment", steps 1–8
      plus tear-down)
- [x] "Works" is defined for the inputs a fresh account cannot have, and the
      README documents how to supply each — the platform (explorer AWS stacks +
      Hetzner ClickHouse via Ansible), the tenant certs and CN map, the schema,
      three secrets, four SSM seeds, the domain config
- [x] Every README defect found is fixed, or listed with a reason it stays
      (see Issues Encountered)
- [x] The result is stated for the evidence package: the AC 7 paragraph in the
      explorer's shape, handed over for [[0294]] (Implementation Notes)
- [x] ~~The sandbox is torn down~~ — not applicable: no sandbox was used. The
      runbook carries a tear-down section for whoever deploys it

## Notes

- If no sandbox account can be had, the honest alternative is a recorded reason
  and a deviation — not a claim that it works.

## Implementation Notes

- **PR #357**, merged 2026-09-25 as `23f457a2`: `README.md` (new),
  `infra/README.md` (rewritten), `infra/Makefile` (`bootstrap`).
- Modelled on `soroban-block-explorer/docs/scf/milestone-3-evidence.md` §AC2,
  `infra/README.md`, `docs/deployment.md` and `infra-hetzner/README.md`.
- **AC 7 paragraph for [[0294]]** (its owner places it): _Public repository:
  github.com/rumblefishdev/stellar-prices-api. Fresh-account deployment
  runbook: `infra/README.md` → prerequisites → the platform (Soroban Block
  Explorer AWS stacks + Hetzner Ansible) → prices tenant (certs, CN map,
  schema) → secrets and seeds → `npm run infra:bootstrap` →
  `npm run infra:deploy:production`. A few steps are manual by design: ordering
  the Hetzner server and Storage Box, issuing client certificates from the
  platform CA, registering the Discord application, and the Route 53 hosted
  zone._

## Issues Encountered

- **No root README existed** — the criterion says "from README".
- **`infra/README.md` had drifted from the code:** the SSM input table named
  `ch-endpoint`/`ch-database`/`ch-user`/`stellar-ledger-data-sns-arn` (the code
  reads `ch-domain`, `ledger-events-topic-arn`, the two bucket keys and
  `stellar-network-passphrase`); the output table named secret ARNs and a
  `ledger-processor-lambda-arn` the code never publishes; the mTLS section
  described two CDK-created cert/key secrets (the stack creates none — one
  `{cert,key,ca}` bundle per CN, operator-created); it pointed at a
  `deploy.yml` that does not exist; the four operator SSM seeds were
  unmentioned; the build pins (rustc 1.97.1, cargo-lambda 1.9.1, zig) lived
  only in `ci.yml`; the schema apply path for a remote server was undocumented.
  All fixed.
- **`make bootstrap` failed on a fresh clone:** a bare `cdk bootstrap`
  synthesizes the app in `cdk.json` first — proven with a fake profile and a
  sentinel `--app` — and the production app cannot synth without `dist/` and
  the Lambda assets. Fixed (Emerged, below).
- **Stays, with a reason:** the GitHub OIDC provider in `Prices-Cicd` is a
  per-account singleton and collides in an account that already has one — the
  stack is optional and nothing deploys through it, so documented, not changed;
  the secrets-extension layer is mapped only for `eu-central-1`/`us-east-1` —
  documented as the region constraint.

## Design Decisions

### From Plan

1. **Claim the runbook, as the explorer did.** Operator decision 2026-09-25:
   the explorer's AC 2 claimed a public repo and a fresh-account runbook with
   its manual steps named; AC 7 is claimed the same way and is not a deviation.

### Emerged

2. **The project is framed as a tenant of the explorer platform.** It owns no
   ledger bucket, topic or database server — five `/platform/*` parameters and
   the explorer's `users.d` already define everything it reads — so the runbook
   deploys the platform first by linking the explorer's own runbook instead of
   duplicating it.
3. **`make bootstrap` names the environment and passes a no-op app**
   (`aws://<caller account>/<production.json region>`), so bootstrapping never
   needs a built app. Tested only as far as the credential call, with a fake
   profile.
4. **Schema apply is documented over an SSH tunnel as `default`**, not through
   the mTLS proxy: the scoped users lack `DROP VIEW`, which `views.sql` needs.

