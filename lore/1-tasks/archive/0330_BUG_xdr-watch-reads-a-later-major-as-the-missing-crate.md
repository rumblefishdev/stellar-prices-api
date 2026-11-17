---
id: "0330"
title: "The XDR watch reports BEHIND because stellar-xdr 30 counts as a crate for protocol 29"
type: BUG
status: completed
related_adr: []
related_tasks: ["0325", "0319", "0098"]
tags: [ci, resilience, ingestion, priority-medium, effort-small]
links:
  - "../../../tools/scripts/verify-xdr-protocol-gap.mjs"
  - "../../../tools/scripts/verify-xdr-protocol-gap.test.mjs"
  - "../../../docs/runbooks/xdr-protocol-watch.md"
  - "https://github.com/rumblefishdev/stellar-prices-api/issues/398"
  - "https://github.com/stellar/rs-stellar-xdr/releases/tag/v30.0.0"
  - "https://github.com/stellar/stellar-core/pull/5478"
  - "https://github.com/rumblefishdev/stellar-prices-api/pull/399"
  - "https://github.com/rumblefishdev/stellar-prices-api/actions/runs/38240193493"
history:
  - date: "2026-10-12"
    status: active
    who: stkrolikiewicz
    note: >
      Created from issue #398. The scheduled watch has been red since
      2026-10-09 and reports BEHIND (pin 28, mainnet 29, newest crate 30.0.0).
      Nothing is broken: protocol 29 kept the XDR of 28 (0325), and every
      production rollup-freshness and ledger-processor alarm is OK.
  - date: "2026-10-12"
    status: active
    who: stkrolikiewicz
    note: >
      Implemented on fix/0330_xdr-watch-reads-a-later-major-as-the-missing-crate.
      A bump target for protocol N is now the newest stable, unyanked version
      with pin < major ≤ N, asked for mainnet and core separately. Tier test:
      18/18 pass. The pre-fix script fails 5 of them, exactly the new cases.
      Live run against mainnet 29 and crates.io 30.0.0 exits 0 with
      "WAITING: no stellar-xdr 29" in both --watch and PR mode.
  - date: "2026-10-12"
    status: completed
    who: stkrolikiewicz
    note: >
      PR #399 merged to develop as 78765f9e at 11:02:55 UTC. 4 files,
      +82 −23. 18 tier cases, 5 of them new. CI 4/4 green. A manual
      xdr-protocol-watch run on master (38240193493, 11:03 UTC) was green and
      reported "WAITING: no stellar-xdr 29 is published yet". It closed #398
      with the "Closed as not actionable yet" comment.
---

# The XDR watch reports BEHIND because stellar-xdr 30 counts as a crate for protocol 29

## Summary

`verify-xdr-protocol-gap.mjs` asks crates.io whether a crate for the needed
protocol exists by comparing `max_stable_version`'s major with it
(`crate.major < needed`). Upstream skipped 29: `stellar-xdr` went from 28.0.1
to 30.0.0. Since 30 ≥ 29, the script treats 30.0.0 as the crate for protocol
29 and fails BEHIND. A bump target must have a major above our pin and **not
above** the protocol it is for. Read every published version, not only the
newest.

## Context

- `stellar-xdr` 30.0.0 was published 2026-10-08. It carries protocol 30 XDR
  (CAP-0084 and CAP-0087 ungated). No `stellar-xdr` 29 exists.
- Horizon mainnet and testnet both report current 29 and core-supported 29
  (stellar-core 29.0.0). There is no stellar-core v30 release, and no
  testnet or mainnet date for protocol 30. stellar-core merged its "Bump to
  protocol 30" PR (#5478) to master on 2026-10-09.
- The watch has been red every day since 2026-10-09 (runs on 10-09, 10-10 and
  10-11). Issue #398 is open at tier BEHIND.
- Production is healthy. `stellar-xdr =28.0.0` decodes P29 (0325), and all
  `prices-production-rollup-freshness-*`, `ledger-processor-lag` and
  `ledger-processor-no-invocations` alarms are OK.
- BE's `soroban-block-explorer` develop still pins `stellar-xdr = "28"`.

## Implementation Plan

### Step 1: Read the published versions

`publishedCrate()` also returns the stable, unyanked versions from the
`versions` array of the same crates.io document (no second request). A
missing `versions` array is an unreadable document, which is the
`cannot read` path.

### Step 2: Choose the bump target per protocol

`bumpTarget(protocol)` = the newest stable version with
`pin < major ≤ protocol`.

- BEHIND: `pin < current` and `bumpTarget(current)` exists.
- LAGGING: otherwise, `pin < supported` and `bumpTarget(supported)` exists.
  This keeps the lead time when mainnet sits on a crate-less protocol (29)
  while core already supports the next (30).
- WAITING: neither. The `— WAITING: no stellar-xdr` wording stays, because
  master's workflow matches it to close the issue.

### Step 3: Tests and runbook

New tier cases: today's state (pin, current pin+1, crate pin+2 only →
WAITING) and the next one (current pin+1, core pin+2, crate pin+2 →
LAGGING). The crates.io mock serves `versions`. Update the runbook's
"published crate" reading and the header comment.

## Acceptance Criteria

- [x] Live run against mainnet 29 and crates.io 30.0.0 exits 0 with
      `— WAITING: no stellar-xdr 29`, in PR and `--watch` mode.
- [x] Current pin+1, core pin+2, crate pin+2 → LAGGING (exit 1 under `--watch`).
- [x] All existing tier cases still pass, and the pre-fix script fails the new
      WAITING case.
- [x] After merge to `develop`, a manual watch run on `master` is green and
      closes #398 as "not actionable yet".

## Implementation Notes

- `publishedCrate()` returns `releases`: the stable, unyanked versions from
  the `versions` array of the crate document. The `major` field is gone.
- `bumpTarget(crate, protocol)` returns the newest release with
  `pin < major ≤ protocol`.
- The tier check asks `bumpTarget` for mainnet (BEHIND) and then for core
  (LAGGING). WAITING when both are empty.
- The BEHIND report names the target (`bump to …`). The LAGGING report names
  the target instead of `max_stable_version`.
- No workflow change on `master` was needed: the workflow checks out
  `develop` for the script.

**Modified tests:** the `published()` mock now also serves `versions`. No
existing case changed.

## Design Decisions

### From Plan

1. **Ask mainnet and core separately.** Mainnet can sit on a crate-less
   protocol (29) while core supports the next one, which has a crate (30).
   That reading is LAGGING, not WAITING.
2. **Keep the WAITING wording.** `— WAITING: no stellar-xdr` is what
   master's workflow matches to close the issue.

### Emerged

3. **A range, not an exact major.** The target is `pin < major ≤ N`, not
   `major == N`. In chat I first described it as "exactly 29". The range also
   covers a pin two protocols behind (pin 27, mainnet 29, only 28 published:
   BEHIND, bump to 28). An exact check would call that WAITING.
4. **Same request, `versions` from the crate document.** No call to the
   paginated `/versions` endpoint. A document without `versions` is
   unreadable, which is the `cannot read` path (red run, no issue).
5. **Yanked versions do not count.** No test covers this.
6. **The WAITING report explains a later major.** When the newest crate is
   above the needed protocol, the report says it carries a later protocol's
   XDR and is not a bump target.

## Issues Encountered

- **`lore_set-task` could not find 0330** (TASK_NOT_FOUND) from the
  `issue-398-review` worktree. The MCP server seems to resolve tasks in the
  main checkout. The task was never set as the session's current task.
- **The pre-push hook took 3 of 4 nx tasks from cache.** It is unclear
  whether the infra `test` target re-ran the changed test file locally. The
  test was run directly (18/18), and CI ran it on the PR.
- **Prettier re-padded the whole tier table** in the runbook, which made
  the runbook diff larger than the change.
