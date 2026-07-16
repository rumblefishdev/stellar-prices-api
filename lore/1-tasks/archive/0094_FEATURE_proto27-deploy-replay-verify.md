---
id: "0094"
title: "Deploy xdr-27 ledger-processor + replay proto27 frozen gap + verify crossing"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0091", "0090"]
tags: ["milestone-M1", "priority-high", "effort-small", "phase-live"]
links:
  - "PR #104 (lore-0091) stellar-xdr 27 migration — merged e17ed03"
  - "docs/runbooks/deploy-ledger-processor.md — deploy procedure"
history:
  - date: 2026-07-14
    status: backlog
    who: okarcz
    note: >
      Spawned from 0091 future work. 0091 landed the xdr-27 code (PR #104,
      merged to develop) but the RUNNING ledger-processor is still on xdr 26.
      This task is the operational tail: deploy, drain DLQ, replay the frozen
      live gap, and verify the 63,401,875 crossing.
  - date: 2026-07-14
    status: active
    who: okarcz
    note: >
      MILESTONE — xdr-27 ledger-processor DEPLOYED to production (AC #1 met).
      Built with `--features lambda`, cargo-lambda ARM64; `make diff-production`
      showed only the ComputeStack LedgerProcessorFunction Code.S3Key change
      (d774…f9e9 → 27c5…88b6, no IAM/SQS/env drift); deployed ComputeStack-only
      at 18:11:33 UTC. Verified live: LastModified 18:11:33Z, provided.al2023,
      arm64, CodeSha256 xUZuFtbpszD6+RJExhZcDfCJUEThXLOX44TvfjvRodY=. Decode
      wall is no longer a code risk — zero "XDR parse failed", DLQ=0, cursor
      persisting every invoke. AC #2–#5 remain (crossing verify, DLQ drain,
      gap replay, CI guard). Followed docs/runbooks/deploy-ledger-processor.md.
  - date: 2026-07-16
    status: completed
    who: okarcz
    note: >
      DONE — proto27 crossing VERIFIED, freeze lifted, archived. Prod CH
      2026-07-16 ~07:57 UTC: reconcile cursor 63,500,065 climbed well past the
      63,401,875 decode wall on xdr-27 code (AC #2); frontier caught up to real
      time, behind_sec ≈ 18 s across all four sources (AC #4, frozen gap
      replayed). DLQ confirmed drained (AC #3 — `prices-ingest-dlq-production`
      0 visible / 0 in-flight via SQS, 2026-07-16). AC #5 (version-gap CI guard)
      DEFERRED to spawned backlog task 0098. This closes the live-ingestion
      freeze jointly with 0064 (durable CH cursor) — xdr-27 removed the decode
      wall, the cursor removed the ephemeral-reset rewind; the frontier only
      unfroze once BOTH shipped. Archived together.
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

- [x] xdr-27 processor deployed to production. *(2026-07-14 18:11 UTC — verified live)*
- [x] Live ingestion advances past ledger 63,401,875. *(2026-07-16 — cursor
      63,500,065 ≫ wall, monotonic; crossing achieved on xdr-27 code, never
      stalled on old code — preventive deploy.)*
- [x] Live DLQ drained; no residual "XDR parse failed". *(2026-07-16 — confirmed
      `prices-ingest-dlq-production` = 0 visible / 0 in-flight via SQS
      `get-queue-attributes`; consistent with DLQ 0 at deploy + zero parse-fails
      on xdr-27 code.)*
- [x] Frozen gap (63,384,068 → tip) replayed for all live sources. *(2026-07-16 —
      frontier caught up to real time, behind_sec ≈ 18 s; all four sources
      aquarius/sdex/phoenix/soroswap current within ~150 s.)*
- [ ] Version-gap CI guard in place. **(deferred to 0098)** — spawned as backlog
      task 0098 (renovate/CI check surfacing a `stellar-xdr` protocol lag before
      it can freeze prod); forward-looking, not a blocker for closing the freeze.

## Implementation Notes

### Deploy (2026-07-14, AC #1)

- Built `prices-ledger-processor` bootstrap with `cargo lambda build -p
  prices-ledger-processor --release --arm64 --features lambda` off `develop`
  at `e17ed03` (the xdr-27 fix, PR #104). Artifact: 13.4 MB, ELF ARM aarch64,
  stripped.
- `make diff-production` preview was clean and scoped: **only**
  `Prices-production-Compute` changed, and only the `LedgerProcessorFunction`
  `Code.S3Key` (`d774…f9e9` → `27c5…88b6`). All other stacks (Secrets,
  ApiGateway, EventBridge, Observability) reported no differences. No IAM/SQS/
  env-var/other-stack drift.
- Deployed ComputeStack-only via `make deploy-production-compute` (22s).
  Verified live: `LastModified 2026-07-14T18:11:33Z`, runtime
  `provided.al2023`, arch `arm64`, `CodeSha256
  xUZuFtbpszD6+RJExhZcDfCJUEThXLOX44TvfjvRodY=`.
- Post-deploy health: zero `XDR parse failed`, DLQ depth 0, `persisted:16`
  every invoke, `rows` climbing (~1000–1442/invoke). Decode wall is no longer
  a **code** risk; remaining work is grind + verification.

### Live state at deploy time

- Candle frontier still pinned at **2026-07-08 12:29–12:31** (all three
  sources), `behind_sec` ~5.4×10⁵ (~6.2 days). Frontier will only advance once
  the reconcile cursor climbs past the July-8 high-water ledger (~63,384,067);
  until then writes land **behind** the frontier so `max(timestamp)` stays flat
  by design.
- Reconcile cursor was `~63,371,203` and climbing at ~7–11k ledgers/hr
  pre-deploy; the wall at **63,401,875** was still a couple hours out — so this
  was a **preventive** deploy (crossed the wall on new code, never stalled on
  old code).

## Design Decisions

### Emerged

1. **Promoted backlog → active on first deploy, not held to completion.**
   The task carries AC #2–#5 (crossing verify, DLQ drain, gap replay, CI
   guard) that resolve over hours-to-days, so it moves to `active` and stays
   there rather than being archived at the deploy milestone.

2. **Post-deploy reconcile-cursor restart-lower is expected, not a fault.**
   After the ComputeStack code swap the reconcile cursor resumed from an
   **earlier checkpoint** (`~63,354,755`, down from `~63,371,203` pre-deploy)
   rather than continuing from the pre-deploy high-water mark, then climbed
   monotonically again. No data loss — candle writes are idempotent on
   `(timestamp, asset_id, quote_asset_id, source)`. Net effect: a few extra
   hours of grind before re-reaching the wall. Documented so a future session
   doesn't misread the backward jump as a regression.
