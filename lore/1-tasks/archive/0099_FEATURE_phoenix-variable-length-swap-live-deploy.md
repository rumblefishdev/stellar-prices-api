---
id: "0099"
title: "Deploy the Phoenix variable-length swap fix to live"
type: FEATURE
status: completed
related_adr: []
related_tasks: ["0097", "0096", "0101"]
tags: [layer-indexing, priority-high, effort-small, milestone-M1, amm, phoenix, live, deploy]
milestone: 1
links:
  - "../../../packages/phoenix-extractor/src/xyk.rs"
  - "../../../packages/ledger-processor/src/dispatch.rs"
history:
  - date: 2026-07-20
    status: completed
    who: okarcz
    note: >
      COMPLETED. The only open AC — phoenix `latest_candle` advancing past the
      deploy (2026-07-17 11:57:52) under the new variable-length dispatch — is
      confirmed on live prod: phoenix priced candles on 2026-07-20, max
      `latest_candle` 2026-07-20 11:36:00 (per-asset: XLM 06:48, EURC 06:45),
      ~3 days after deploy with no rollback. The new binary prices Phoenix
      7-event (variable-length) swaps live. The live-era reprice was already
      moved to 0101 (M2), so nothing else remains here. Archived.
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      DEPLOYED to prod 2026-07-17 11:57:52 UTC (ComputeStack only,
      `make deploy-production-compute` from develop @ 06376ff; cdk diff showed
      ONLY the Lambda code asset S3Key 33521e28... -> d2e17649..., no IAM/SQS/env
      changes; bootstrap rebuilt first with `cargo lambda build -p
      prices-ledger-processor --release --arm64 --features lambda` — CDK packages
      a PRE-BUILT binary, so skipping the build silently redeploys the old one).
      Lambda config verified: LastModified 11:57:52, arm64, provided.al2023.
      INGESTION HEALTHY: sdex + aquarius both advanced to 12:09 (15s behind) —
      aquarius is an AMM source on the same classify->dispatch path, so the AMM
      chain works under the new code. NOT YET PROVEN LIVE: phoenix's own path —
      its last event (~11:44) and soroswap's (~11:55) both predate the deploy, so
      no phoenix swap has been priced by the new binary yet. Verified it is
      market quiet, NOT a bug: per venue the last candle tracks the last event to
      within ~1-2 min (phoenix last event 63517809 = ~27 min behind tip vs last
      candle ~30 min; soroswap 63517940 = ~16 min vs ~17 min). SPOT-CHECK phoenix
      latest_candle > 11:57:52 later; if phoenix events arrive with no candle,
      roll back by redeploying bootstrap asset 33521e28...
      Remaining: the live-era reprice.
  - date: 2026-07-17
    status: active
    who: okarcz
    note: >
      Promoted to active. 0097 is archived and PR #117 is merged (e55ef7e), so
      the Phoenix variable-length fix is on develop but NOT deployed — live
      Phoenix is still ~2% short. Starting the deploy + live-era reprice.
  - date: 2026-07-17
    status: backlog
    who: okarcz
    note: >
      Spawned from 0097 future work. The Phoenix variable-length swap fix
      (commit f3c677b, PR #117) lands in SHARED live code (ledger-processor +
      phoenix-extractor) on 0097's backfill branch. Live Phoenix pricing has
      been ~2% short since inception; the fix only takes effect on deploy, and
      the live-era gap (post-63352611) needs its own reprice.
---

# Deploy the Phoenix variable-length swap fix to live + reprice the live-era gap

## Summary

Task 0097 found and fixed a **live** Phoenix bug: XYK swap groups are
variable length (Phoenix omits optional fields), but `dispatch_phoenix`
gated on the fully-populated 8-event shape, silently discarding every
7-event swap — **5,175 of them (~2.1%)** over the Soroban era. The fix is
merged in 0097's PR #117, but 0097 only repriced the **historical** range
`[50457424, 63352611]`. Live remains wrong until the fixed
ledger-processor is **deployed**, and the live-era gap needs repricing.

## Context

- Root cause + measurements: 0097 Decision Log, 2026-07-17.
- Only four fields are required (`sell_token`, `offer_amount`, `buy_token`,
  `return_amount`); `sender` is optional. The 7-event groups omit only
  `actual received amount`, which the extractor reads and discards.
- Sibling of the 0096 Soroswap defect — same shape: an extractor assumption
  that didn't match the real event stream, losing volume silently.
- The fix shipped on a *backfill* branch. If PR #117 is split, this task owns
  the live half.

## Implementation

- Deploy the fixed `ledger-processor` to prod (`make deploy-production-compute`;
  see `docs/runbooks/deploy-ledger-processor.md`). **DONE 2026-07-17 11:57:52.**
- Verify live Phoenix prices 7-event groups under the new binary.

**Scope note (2026-07-17):** the live-era **reprice** moved to **0101**
(milestone 2). This task is the DEPLOY only — i.e. live correct going *forward*.
0101 is the backward-looking half: Phoenix's ~2%-short candles from 07-06 and the
Soroswap 07-06 → 07-15 hole.

## Acceptance Criteria

- [x] Fixed ledger-processor deployed to prod; deploy recorded here.
      **2026-07-17 11:57:52 UTC** — cdk diff showed ONLY the Lambda code asset
      (`33521e28…` → `d2e17649…`); LastModified verified; arm64.
- [x] Ingestion healthy under the new binary — sdex + aquarius advanced to 12:09
      (15s behind); aquarius is an AMM source on the same classify→dispatch path.
- [x] **Spot-check:** phoenix `latest_candle` advances past 11:57:52, proving the
      new dispatch prices a phoenix swap live. **Confirmed 2026-07-20** — phoenix
      `max(latest_candle)` = 2026-07-20 11:36:00 (XLM 06:48, EURC 06:45), ~3 days
      after deploy, so the new binary prices Phoenix swaps live. No rollback
      needed.
- [ ] ~~Live-era Phoenix gap repriced + pre-rolled~~ → **moved to 0101**
      (milestone 2).
