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
  crates.io. If no published major reaches the protocol core supports, report
  **WAITING** (nothing to do yet, with the newest version named); otherwise
  report **LAGGING** (bump now). An unreadable crates.io falls back to LAGGING,
  never to silence.
- The workflow tells three tiers apart (WAITING / LAGGING / BEHIND) and
  comments on a change of tier, so WAITING → LAGGING ("the crate is
  published") notifies once.
- WAITING still fails the run under `--watch`, so `master`'s current workflow
  (which closes the issue on success) keeps the issue open until the new
  workflow reaches `master`.

## Acceptance Criteria

- [ ] With crates.io at 28.x and core at 29, the report says WAITING, names the
      newest crate version, and says there is nothing to bump to yet.
- [ ] With a 29.x published, the report says LAGGING and asks for the bump
      (and BE's `xdr-parser` first).
- [ ] crates.io unreachable → LAGGING, with the reason in the report.
- [ ] BEHIND is unchanged.
- [ ] The workflow comments once on WAITING → LAGGING and on → BEHIND, and stays
      silent while the tier is unchanged.
- [ ] Issue #336's body reads WAITING after the next scheduled run.
