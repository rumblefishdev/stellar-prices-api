---
id: "0319"
title: "The XDR protocol watch fires before any stellar-xdr crate exists to bump to — say when the bump is actually possible"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0098", "0277", "0091"]
tags: [layer-backend, ci, resilience, ingestion, priority-medium, effort-small]
links:
  - "https://github.com/rumblefishdev/stellar-prices-api/issues/336"
  - "../../../tools/scripts/verify-xdr-protocol-gap.mjs"
  - "../../../.github/workflows/xdr-protocol-watch.yml"
history:
  - date: "2026-09-29"
    status: active
    who: okarcz
    note: >
      Filed from issue #336. The watch ([[0098]]) opened it on 2026-09-22:
      mainnet Horizon reports `core_supported_protocol_version = 29` while we
      pin `stellar-xdr` 28. The reading is correct, but nothing can be done
      with it: the newest `stellar-xdr` on crates.io is 28.0.1 (published
      2026-09-29), so there is no crate to bump to.
  - date: "2026-09-29"
    status: active
    who: okarcz
    note: >
      PR #369. Decided the same day: the watch alerts ONLY when something can
      be done. WAITING (no crate published) is a green run with no issue and
      no email; a WAITING run closes an open issue as "not actionable yet". A
      check that cannot complete fails the run but leaves the issue alone.
      Rollout is master first (workflow-only PR, as #308), then #369 to
      develop, or master's old workflow closes #336 as "Resolved".
  - date: "2026-09-29"
    status: active
    who: okarcz
    note: >
      Rolled out: #336 closed by hand, #370 merged to master, #369 merged to
      develop, manual run 36561221555 green with WAITING and no issue opened.
      Every acceptance criterion met; the task can be archived.
---

# The XDR protocol watch fires before any stellar-xdr crate exists to bump to

## Summary

`xdr-protocol-watch` (task [[0098]]) reports LAGGING as soon as core supports
a protocol newer than our `stellar-xdr` pin. That early warning is the point of
the watch: `core_supported_protocol_version` rises weeks before the vote. But
LAGGING also fires when no `stellar-xdr` release for that protocol exists yet,
so the issue asks for a bump that nobody can make. The watch should tell the two
apart and notify on the day the bump becomes possible.

## Context

Measured 2026-09-29:

- `https://horizon.stellar.org/`: `core_version` `stellar-core 29.0.0
  (4eb83337…)`, `current_protocol_version` 28, `core_supported_protocol_version`
  29.
- stellar-core `master` (`73f2bdd1`) has `CURRENT_LEDGER_PROTOCOL_VERSION = 29`
  in `src/main/Config.cpp:34`, with XDR `stellar/stellar-xdr@ee040cd6`. There is
  no `v29.0.0` tag, and `4eb83337` is not in the public repo.
- crates.io `stellar-xdr`: max stable **28.0.1**. rs-stellar-xdr `main`
  (`a7018f66`) already pins XDR `ee040cd6` (protocol 29) but is labelled
  `28.0.0`. Core and rs-soroban-env take it by git, not from crates.io, which is
  how SDF runs protocol 29 before the crate is published.

## Implementation

- `verify-xdr-protocol-gap.mjs`: when LAGGING, read `max_stable_version` from
  crates.io. No published stable major reaching the supported protocol →
  **WAITING**, a notice with exit 0 even under `--watch`. Published → LAGGING
  (fatal under `--watch`). crates.io unreadable → fatal under `--watch` as a
  check that could not complete.
- Workflow: a green WAITING run closes an open issue as "not actionable yet";
  a check that could not complete leaves the issue untouched; LAGGING/BEHIND
  open or update it as before, commenting only on LAGGING → BEHIND.
- Runbook `docs/runbooks/xdr-protocol-watch.md`, incl. the master-first
  rollout.

## Acceptance Criteria

- [x] With crates.io at 28.x and core at 29, the watch run is green, the report
      says WAITING and names the newest crate version (live, 2026-09-29).
- [x] With a 29.x published, the run fails, the report says LAGGING and asks
      for the bump (mock).
- [x] crates.io or Horizon unreachable → red run, tracking issue untouched
      (mock + fake `gh`).
- [x] BEHIND is unchanged; LAGGING → BEHIND comments once (fake `gh`).
- [x] A WAITING run closes an open issue as "not actionable yet" (fake `gh`).
- [x] Runbook explains the check, the tiers and the rollout to `master`.
- [x] Workflow-only PR to `master` merged (#370, `531c8a6d`), then #369 merged
      to `develop` (`0f7d0571`); the two workflow files are identical.
- [x] Manual run on `master` (run 36561221555, 2026-09-29) is green with
      `WAITING … newest on crates.io 28.0.1`; the issue step logged "check
      passed, no open issue — nothing to do". #336 had been closed by hand
      the same day with an explanatory comment.
