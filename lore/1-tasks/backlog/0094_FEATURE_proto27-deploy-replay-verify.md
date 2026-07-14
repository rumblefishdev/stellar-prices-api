---
id: "0094"
title: "Deploy xdr-27 ledger-processor + replay proto27 frozen gap + verify crossing"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0091", "0090"]
tags: ["milestone-M1", "priority-high", "effort-small", "phase-live"]
links:
  - "PR #104 (lore-0091) stellar-xdr 27 migration — merged e17ed03"
history:
  - date: 2026-07-14
    status: backlog
    who: okarcz
    note: >
      Spawned from 0091 future work. 0091 landed the xdr-27 code (PR #104,
      merged to develop) but the RUNNING ledger-processor is still on xdr 26.
      This task is the operational tail: deploy, drain DLQ, replay the frozen
      live gap, and verify the 63,401,875 crossing.
---

# Deploy xdr-27 ledger-processor + replay proto27 frozen gap + verify crossing

## Summary

Task 0091 merged the `stellar-xdr 26→27` migration (PR #104) but only protects
the **running** live processor once **deployed** — the deployed
`prices-production-ledger-processor` is still on xdr 26. This task deploys the
xdr-27 build, drains any proto27 DLQ, replays the frozen live gap, and confirms
live ingestion advances past the Protocol-27 decode wall.

## Context

Live ingestion is stale at ledger **~63,384,067 / 2026-07-08** (all three
sources). The proto27 XDR decode wall is at **63,401,875** (BE #325). The
reconcile cursor reaches it ~**2026-07-14 22:00 UTC** — deploying before then
avoids a stall (recoverable, but avoidable). Decode is BE's `xdr-parser` at
#325, already verified against real proto27 ledgers.

## Implementation

- Deploy the xdr-27 `ledger-processor` (and any other xdr-touching Lambdas) to
  production. **Deploy is approval-gated per session policy — confirm before running.**
- Optionally pre-run `decode_probe` against a real ≥63,401,875 `.xdr.zst` file
  as a local smoke test before/after deploy.
- Watch the 63,401,875 crossing: `price_ohlcv_1m` max ledger should climb past
  it with zero "XDR parse failed" errors.
- Drain the live DLQ; redrive any proto27 parse-fail messages (14d retention).
- Replay/reprocess the frozen gap (63,384,068 → tip) for sdex/aquarius/phoenix.
- Add a version-gap CI guard / renovate policy so a future protocol bump
  surfaces the `stellar-xdr` lag before it freezes prod.

## Acceptance Criteria

- [ ] xdr-27 processor deployed to production.
- [ ] Live ingestion advances past ledger 63,401,875 (max ledger climbs to tip).
- [ ] Live DLQ drained; no residual "XDR parse failed" errors.
- [ ] Frozen gap (63,384,068 → tip) backfilled/replayed for all live sources.
- [ ] Version-gap CI guard in place.
