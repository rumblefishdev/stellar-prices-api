---
id: "0098"
title: "Version-gap CI guard — surface stellar-xdr protocol lag before it freezes prod"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0094", "0091"]
tags: ["milestone-M1", "priority-medium", "effort-small", "phase-live", "ci", "resilience"]
links:
  - "../active/0094_FEATURE_proto27-deploy-replay-verify.md — parent (AC #5)"
history:
  - date: 2026-07-16
    status: backlog
    who: okarcz
    note: >
      Spawned from 0094 future work (AC #5). Proto27 (Zipper) froze LIVE
      ingestion because the deployed ledger-processor was on stellar-xdr 26 while
      mainnet advanced to protocol 27 — the decode wall at ledger 63,401,875
      stalled the reconcile loop and was only caught reactively (0091 bumped
      xdr→27, 0094 deployed + verified the crossing). This task makes the lag
      PROACTIVE so the next protocol bump surfaces before it can freeze prod.
---

# Version-gap CI guard — stellar-xdr protocol lag

## Summary

Add a guard that surfaces when our pinned `stellar-xdr` (and `xdr-parser`) lags
the live Stellar mainnet protocol version, so a future protocol bump is caught
*before* it stalls live ingestion at a decode wall — not after (as proto27 was).

## Context

Proto27 froze the live processor: it was running `stellar-xdr 26` while mainnet
crossed to protocol 27, hitting an XDR decode wall at ledger `63,401,875`. The
freeze was diagnosed and fixed reactively across tasks 0091 (xdr→27, exact-pin
`=27.0.0`, PR #104) and 0094 (deploy + crossing verify). Note `xdr-parser` is
deliberately kept on `branch="develop"` while `stellar-xdr` is exact-pinned (see
[[xdr-parser-develop-branch-intentional]]) — the guard must account for that.

## Implementation

- **CI job**: compare our pinned `stellar-xdr` protocol version against the
  current mainnet protocol (Horizon `/` `core_supported_protocol_version` or an
  RPC `getNetwork`/ledger header) and **warn/fail** when ours < network.
- Optionally a **Renovate rule** tracking `stellar-xdr` releases so a new
  protocol version opens a PR automatically.
- Wire a heads-up (existing Slack alarm channel `#stellar-prices-api-bot`) or a
  scheduled check, so lag surfaces even without a dependency PR.
- Document the check + response in `docs/runbooks/deploy-ledger-processor.md`.

## Acceptance Criteria

- [ ] Guard exists (CI job and/or Renovate rule) tracking `stellar-xdr` vs the
      live mainnet protocol version.
- [ ] It warns/fails when the pinned protocol lags the network protocol.
- [ ] Behaviour documented in the deploy runbook; `xdr-parser` develop-pin caveat
      noted so the guard doesn't false-alarm on it.
