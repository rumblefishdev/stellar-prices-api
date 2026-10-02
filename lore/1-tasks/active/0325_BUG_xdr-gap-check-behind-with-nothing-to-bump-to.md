---
id: "0325"
title: "The XDR gap check reds every PR at BEHIND although protocol 29 changed no XDR and no stellar-xdr 29 exists"
type: BUG
status: active
related_adr: []
related_tasks: ["0319", "0098", "0277", "0091"]
tags: [ci, resilience, ingestion, priority-high, effort-small]
links:
  - "../../../tools/scripts/verify-xdr-protocol-gap.mjs"
  - "../../../.github/workflows/ci.yml"
  - "../../../.github/workflows/xdr-protocol-watch.yml"
  - "../../../docs/runbooks/xdr-protocol-watch.md"
  - "https://github.com/stellar/stellar-core/compare/v28.0.1...v29.0.0-internal"
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
    who: claude
    note: >
      Implemented on fix/0325_xdr-gap-check-behind-with-nothing-to-bump-to.
      BEHIND now asks crates.io first and is WAITING when no crate exists.
      New tier test: 13/13 cases pass; the pre-fix script fails 5 of
      them (exactly the changed ones). Live run against mainnet 29 and
      crates.io 28.0.1: exit 0 and WAITING in PR and --watch mode. No change
      to master's workflow is needed. Waiting for review and merge.
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
- [ ] Merged to develop before the next scheduled watch run

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
