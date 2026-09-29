---
id: "0319"
title: "The XDR protocol watch fires before any stellar-xdr crate exists to bump to — say when the bump is actually possible"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0098", "0277", "0091"]
tags: [layer-backend, ci, resilience, ingestion, priority-medium, effort-small]
links:
  - "https://github.com/rumblefishdev/stellar-prices-api/issues/336"
  - "../../../tools/scripts/verify-xdr-protocol-gap.mjs"
  - "../../../.github/workflows/xdr-protocol-watch.yml"
  - "../../../docs/runbooks/xdr-protocol-watch.md"
  - "https://github.com/rumblefishdev/stellar-prices-api/pull/369"
  - "https://github.com/rumblefishdev/stellar-prices-api/pull/370"
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
  - date: "2026-09-29"
    status: completed
    who: okarcz
    note: >
      Closed. #369 merged to develop as 0f7d0571 (3 commits: WAITING tier,
      runbook, green-while-waiting rework); #370 merged to master as 531c8a6d
      (workflow file only). The watch now fails, emails and opens an issue
      only when a stellar-xdr bump is possible (LAGGING) or mainnet has voted
      (BEHIND). Verified by a manual run on master (36561221555): green,
      WAITING, newest crate 28.0.1, no issue. 8/8 acceptance criteria. No
      automated test suite exists for the script; covered by mocked runs.
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

## Implementation Notes

- `tools/scripts/verify-xdr-protocol-gap.mjs` — `publishedCrate()` reads
  `max_stable_version` / `max_version` from crates.io (User-Agent required;
  `CRATES_URL` overrides, like `HORIZON_URL`). The LAGGING branch now splits
  into WAITING (notice, exit 0 in both modes), LAGGING (fatal under
  `--watch`) and "cannot read crates.io" (fatal under `--watch`).
- `.github/workflows/xdr-protocol-watch.yml` — a green run closes an open
  issue ("not actionable yet" for WAITING, "Resolved" otherwise); a report
  starting `error: cannot reach` / `error: cannot read` leaves the issue
  untouched; LAGGING/BEHIND open or update it, commenting only on
  LAGGING → BEHIND.
- `docs/runbooks/xdr-protocol-watch.md` — new: the check, the tiers, what to
  do per tier, running by hand, which branch runs what, the master-first
  rollout of a workflow change.
- Rollout: #336 closed by hand with an explanation, #370 (workflow only) to
  `master`, then #369 to `develop`, then `gh workflow run … --ref master`.

## Issues Encountered

- **The schedule runs `master`'s workflow, the script comes from `develop`.**
  `master` is updated only by workflow-only PRs (729 commits behind
  `develop` on 2026-09-29). A workflow change merged to `develop` changes
  nothing that runs; it needed its own PR to `master` (#370, like #308).
- **Merge order was load-bearing.** `master`'s old workflow reads any green
  run as "resolved". With the new script first it would have closed the
  tracking issue with a false "level with mainnet again". Merged `master`
  first; the new workflow with the old script is harmless (still LAGGING).
- **No test harness exists for the script or the workflow.** Verified with a
  local mock of crates.io (`python3 -m http.server`) and the issue step's
  shell extracted from the YAML and run against a fake `gh`, across every
  tier transition.

## Design Decisions

### From Plan

1. **Ask crates.io only when the pin lags `core_supported`.** No extra
   network call on a healthy run.
2. **A pre-release does not count as published.** `29.0.0-rc.1` is named in
   the report, the tier stays WAITING.

### Emerged

3. **WAITING is a green run with no issue and no email** (decided by the
   operator after the first version). The first version kept WAITING red so
   the tracking issue stayed open through the wait, with a one-time "Now
   actionable" comment on the crate's release. Reversed: an alert is only for
   something someone can act on. Cost: no early "a protocol is coming"
   notification; the run summary still shows WAITING.
4. **A check that could not complete is red but leaves the issue alone.**
   Horizon or crates.io unreachable means nothing was measured: the failure
   email is the signal, and there is no bump to ask for. Replaces the first
   version's "crates.io unknown → LAGGING", which would have opened an issue
   asking for a bump on a crates.io outage.
5. **A WAITING run closes an open issue as "not actionable yet"**, rather
   than leaving a stale LAGGING issue open (the #336 case).
6. **The issue's "what to do" names the XDR commit check**, because
   rs-stellar-xdr `main` carried protocol-29 XDR under the label `28.0.0`:
   the crate's version number alone is not proof of its protocol.

## Future Work

None. The protocol 29 bump itself is not this task: it becomes actionable
when the watch opens its issue (a stable `stellar-xdr` 29 on crates.io), and
is filed then, like [[0277]] was for 28.
