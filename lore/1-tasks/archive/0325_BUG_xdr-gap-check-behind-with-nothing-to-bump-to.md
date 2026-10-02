---
id: "0325"
title: "The XDR gap check reds every PR at BEHIND although protocol 29 changed no XDR and no stellar-xdr 29 exists"
type: BUG
status: completed
related_adr: []
related_tasks: ["0319", "0098", "0277", "0091"]
tags: [ci, resilience, ingestion, priority-high, effort-small]
links:
  - "../../../tools/scripts/verify-xdr-protocol-gap.mjs"
  - "../../../.github/workflows/ci.yml"
  - "../../../.github/workflows/xdr-protocol-watch.yml"
  - "../../../docs/runbooks/xdr-protocol-watch.md"
  - "https://github.com/stellar/stellar-core/compare/v28.0.1...v29.0.0-internal"
  - "https://github.com/rumblefishdev/stellar-prices-api/pull/383"
  - "https://github.com/rumblefishdev/stellar-prices-api/pull/386"
history:
  - date: "2026-10-02"
    status: active
    who: stkrolikiewicz
    note: >
      Created after the protocol 29 vote. The check reports BEHIND (pin 28,
      mainnet 29) and exits 1 on every PR, and the next scheduled watch opens
      an issue saying the decode wall is live. Neither is true: stellar-core
      v29 pins the same XDR commit as v28.0.1, our decoder reads P29 ledgers
      on production, and there is no stellar-xdr 29 to bump to.
  - date: "2026-10-02"
    status: active
    who: stkrolikiewicz
    note: >
      Implemented on fix/0325_xdr-gap-check-behind-with-nothing-to-bump-to.
      BEHIND now asks crates.io first and is WAITING when no crate exists.
      New tier test: 13/13 cases pass; the pre-fix script fails 5 of
      them (exactly the changed ones). Live run against mainnet 29 and
      crates.io 28.0.1: exit 0 and WAITING in PR and --watch mode. No change
      to master's workflow is needed. Waiting for review and merge.
  - date: "2026-10-02"
    status: completed
    who: stkrolikiewicz
    note: >
      PR #383 merged to develop as b6b0732d at 11:30:41 UTC, before the day's
      scheduled watch run (the last one was 2026-10-01 13:16 UTC), with no
      watch issue open. 6 files, +253 −43: 13 new tier cases (the pre-fix
      script fails 5 of them), 56 tests in the infra target, CI 4/4 green.
      Oskar's review: the test reads the pin from [workspace.dependencies]
      like the script (6317af69), the implementation entry is attributed to
      stkrolikiewicz (06b74a28), and the close-comment wording on master is
      PR #386.
  - date: "2026-10-02"
    status: completed
    who: stkrolikiewicz
    note: >
      Verified by a manual xdr-protocol-watch run on master (37002860439,
      11:46 UTC, 20 s): green, report "stellar-xdr 28 is behind mainnet
      protocol 29 — WAITING: no stellar-xdr 29 is published yet", and the
      issue step logged "check passed, no open issue — nothing to do".
---

# The XDR gap check reds every PR at BEHIND with nothing to bump to

## Summary

`verify-xdr-protocol-gap.mjs` treats `pinned < current_protocol_version` as
BEHIND and fails before it asks crates.io. Protocol 29 broke the assumption
behind that: the protocol number moved, the XDR did not. Apply the 0319 rule
to BEHIND too — no published `stellar-xdr` for the mainnet protocol means
nothing to bump to, so report WAITING and exit 0.

## Context

Protocol 29 activated 2026-10-01 17:00:07 UTC at ledger 64,717,645.

- stellar-core `v29.0.0-internal` and `v28.0.1` both pin XDR `9c9c145`
  (`src/protocol-curr/xdr`). P29 changes apply rules only: DEX offer crossing
  accuracy, pool hops exempt from the crossing limit, Wasm cost inputs,
  5 MB message / 4 MB tx-set caps.
- `stellar-xdr =28.0.0` decodes P29 ledgers: `decode_probe` on 64717645,
  64717700 and 64727000 from SDF's public lake, and production
  `ledger-processor` writing P29 ledgers since 07:04 UTC 2026-10-02.
- crates.io max stable is 28.0.1. There is nothing to bump to.
- What actually stopped ingestion was BE's Galexie (captive core 28 cannot
  apply P29), fixed by explorer task 0605. This check cannot see that, by
  design.

Effects today:

- `ci.yml` job `XDR protocol lag` exits 1 on every PR. Not a required check
  (no branch protection), but red on every PR.
- The daily `xdr-protocol-watch` (cron 06:17 UTC, starts ~11–14 UTC) would
  open an issue: "the decode wall is live". False, with no action possible.

## Implementation Plan

1. Script: when `pinned < max(current, supported)`, read crates.io first.
   No stable major reaches the mainnet protocol → WAITING notice, exit 0, in
   both modes. Reuse the `— WAITING: no stellar-xdr` marker so master's
   workflow closes an open issue as "not actionable" with no workflow change.
   A published crate → BEHIND fatal as before. crates.io unreadable → same
   as the LAGGING path (fatal under `--watch`, notice on PRs).
2. Header comment: P29 is the counterexample to "major tracks protocol".
3. Runbooks: tier table in `xdr-protocol-watch.md`, the protocol-lag
   section of `deploy-ledger-processor.md`.
4. A test that runs the tier matrix against a local mock Horizon and
   crates.io, run by CI.

Trade-off, accepted: if a protocol that DOES change XDR is voted before its
crate is published, the watch stays green. Nobody could bump from crates.io
then either, and a real decode stall shows up as frozen candles
(`rollup-freshness-1m` fired 22 min after the P29 stall). Comparing XDR
commits instead of protocol numbers is the precise check; it needs the
GitHub API and core's commit is not always public before release.

## Acceptance Criteria

- [x] Live (mainnet 29, crate 28.0.1): exit 0 in PR and `--watch` mode, report says WAITING
- [x] Mock crate 29.0.0 with mainnet 29: BEHIND, exit 1 in both modes
- [x] 0319's tiers unchanged: WAITING, LAGGING, current, "cannot reach/read"
- [x] A test covers the matrix and runs in CI — the infra project's `test`
      target (`nx run-many -t test` in the `typescript` job, and pre-push)
- [x] Runbooks updated
- [x] Merged to develop before the next scheduled watch run — b6b0732d at
      11:30:41 UTC; the day's scheduled run had not started yet

## Implementation Notes

- `tools/scripts/verify-xdr-protocol-gap.mjs`: BEHIND moved inside the
  crates.io branch. `needed` = mainnet's protocol when BEHIND, core's when
  LAGGING; a crate below `needed` prints WAITING and exits 0.
- `tools/scripts/verify-xdr-protocol-gap.test.mjs` (new, `node:test`):
  mock Horizon + crates.io on `127.0.0.1:0`, 13 cases relative to the
  Cargo.toml pin. Picked up by the infra `test` glob
  `tools/scripts/**/*.test.mjs` (56 tests there now, all pass).
- Runbooks: tier table, the WAITING rule and a "check the candle frontier
  after any vote" line in `xdr-protocol-watch.md`; a protocol 29 caveat in
  `deploy-ledger-processor.md`.
- No existing test was modified; the test file is new.

## Issues Encountered

- **An in-line edit left a 118-character comment line in `ci.yml`.**
  Replacing a phrase mid-line kept the rest of the old line, and prettier
  does not reflow YAML comments, so nothing flagged it. Found on a re-read
  of the diff; reflowed in 21e6a78d.
- **The test read the pin differently from the script** (review). It took
  the first `stellar-xdr` line anywhere in `Cargo.toml`; the script reads
  `[workspace.dependencies]` only. A decoy `[patch]` line at 99 read as 99.
  Fixed in 6317af69 with the script's two steps.
- **Pushes from master-based branches fail the shared pre-push hook**: it
  clippies `comet-extractor`, a crate `master` does not have. PR #386 was
  pushed with `--no-verify`, stated in its commit body.

## Design Decisions

### From Plan

1. **Reuse the `— WAITING: no stellar-xdr` marker.** Master's workflow
   closes an open issue as "not actionable yet" on that text, so the fix
   ships on develop alone.
2. **Protocol numbers, not XDR commits.** Accepted cost in Implementation
   Plan; a `ponytail:` comment in the script names the exact check.

### Emerged

3. **BEHIND needs a crate for mainnet's protocol, not core's.** With pin 28,
   mainnet 29, core 30 and crate 29 published, comparing against core's 30
   would have said WAITING while a real bump existed. Covered by the
   "crate for mainnet but not core" case.
4. **crates.io down while BEHIND is a notice on PRs.** Before, BEHIND failed
   PRs without asking crates.io. Now it follows the script's PR rule: an
   outside outage never fails a PR. Under `--watch` it still fails as a
   check that could not complete.
5. **`// prettier-ignore` on the case table.** Prettier spread 13 rows over
   100 lines; one row per case reads as the decision table it is.
6. **A `*.test.mjs`, not a new CI step.** The first version was a standalone
   script with its own npm script and a step in the `XDR protocol lag` job.
   The repo already runs `tools/scripts/**/*.test.mjs` through the infra
   `test` target, and the `typescript` job triggers on `tools/scripts/**`,
   so the test needs no wiring of its own.
7. **PR #386 also rewrites master's workflow header.** The review nit was
   only the close comment, but the header's WAITING and BEHIND bullets
   would contradict develop's script once #383 merged, so the same
   wording-only PR updates them.

## Future Work

None spawned in this repo.

- **Galexie early warning** — `core_supported_protocol_version` above the
  captive core in BE's Galexie image — belongs to BE, who own the pin and
  the fix (discussed with Oskar and Karol, 2026-10-02). For whoever builds
  it: BE pins a manifest-list digest that Docker Hub's tag list no longer
  maps to `29.0.0`; fetching the manifest by digest and reading
  `STELLAR_CORE_VERSION` from the image config works without tags.
- **PR #386** (master's close-comment wording) is open and needs no task of
  its own.
