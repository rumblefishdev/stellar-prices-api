---
id: "0099"
title: "Deploy the Phoenix variable-length swap fix to live + reprice the live-era gap"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0097", "0096"]
tags: [layer-indexing, priority-high, effort-small, milestone-M1, amm, phoenix, live, deploy]
milestone: 1
links:
  - "../../../packages/phoenix-extractor/src/xyk.rs"
  - "../../../packages/ledger-processor/src/dispatch.rs"
history:
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
  see `docs/runbooks/deploy-ledger-processor.md`).
- Verify live Phoenix ticks resume at the corrected rate — the
  `swaps failed dispatch` counter (added in 0097) should sit at ~0 for phoenix.
- Reprice the live-era Phoenix gap `[63352612, live-floor]` with
  `events-backfill` (same CH-to-CH path 0097 built; idempotent), then pre-roll
  per `schema/preroll-amm-reprice.sql`.
- Confirm no same-source minute overlap with live ingestion — keep ranges
  disjoint per [[backfill-live-no-code-coordination]].

## Acceptance Criteria

- [ ] Fixed ledger-processor deployed to prod; deploy recorded here.
- [ ] Live `phoenix` ticks show the ~2% recovery; `swaps failed dispatch` ≈ 0.
- [ ] Live-era Phoenix gap repriced + pre-rolled.
- [ ] 0097's historical range and this range are disjoint and both complete.
