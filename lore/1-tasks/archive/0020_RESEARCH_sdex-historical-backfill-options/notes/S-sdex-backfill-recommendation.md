---
title: "Synthesis — SDEX historical backfill recommendation"
type: synthesis
status: mature
spawned_from: ../README.md
spawns: []
tags: [sdex, backfill, synthesis, recommendation, stream-2]
links:
  - "./R-sdex-operation-xdr-shape.md"
  - "./G-sdex-trade-extraction-design.md"
  - "./I-stream2-options.md"
history:
  - date: 2026-05-12
    status: mature
    who: okarcz
    note: "Final recommendation; open questions and spawned follow-ups listed."
---

# Synthesis — SDEX historical backfill recommendation

## TL;DR recommendation

**Adopt Option A (prices-api-owned archive-reader Fargate task)
as the baseline, with Option B (CH pre-filter via task 0017's
local CH instance) layered on top if the measured trim ratio
justifies the plumbing.** Send a low-cost inbox message to BE
about Option C (BE-side `sdex_trades` table) — if BE has appetite,
revisit later; do not gate prices-api on it.

This preserves §5.6 Stream 2 substantially as-written: the
prices-api Fargate task does the archive walks. The G-note's
extraction algorithm replaces what would otherwise have been
hand-rolled at implementation time. The CH pre-filter is a free
optimisation IF it's worth the plumbing; that's a measured
question, not a guess.

## Why not the CH-shaped answer (Option B/C as primary)

Stream 1 (Soroban AMM) won the CH-shaped answer because the
**price-relevant payload is in CH** (`soroban_events.topics_xdr`
+ `data_xdr`). The archive read is fully replaced.

Stream 2 (SDEX) doesn't have the same structural fit:

- CH `operations_appearances` carries operation *appearance*
  metadata (type, source, destination, asset code, amount), NOT
  the per-trade `ClaimAtom` list that's the price-relevant
  payload (per R-note §2).
- The protocol stores `ClaimAtom`s inside `TransactionResult`,
  which is in `LedgerCloseMeta`, which lives in archive.
- Replacing the archive walk requires either (a) BE adding an
  `sdex_trades` table to CH (Option C, cross-team), or (b)
  prices-api parsing the same archive bytes anyway just to write
  them to CH first (Option D, strictly worse than A).

So the architectural fit that made Stream 1's CH-sourced answer
right does not apply to Stream 2. The optimisation budget for
Stream 2 is bounded to a **pre-filter** of which ledgers to read.

## What's in scope for Tranche 2/3

Build the **prices-api SDEX backfill Fargate task** with the
extractor designed in G-note §"Extraction algorithm":

1. ECS Fargate task definition (2 vCPU / 4 GB RAM per §5.6).
2. Rust binary using `stellar-xdr` for `LedgerCloseMeta` decoding
   and the typed walk to `ClaimAtom`s.
3. `TradeTick` emission into a staging table (matches the same
   tick shape as Stream 1's Soroban-AMM ticks, so OHLCV
   aggregation is uniform across both streams).
4. `backfill_progress` checkpointing per §3.5 of the design doc.
5. CloudWatch alarm on stalled heartbeat per §5.6.

This is the scope task 0012 (existing backlog: design
Prices-owned Fargate backfill) was already going to absorb. 0012
now has the G-note as its concrete spec.

**Optional add-on (Option B):** plumb the CH pre-filter as a
two-step pipeline:

```text
local CH `operations_appearances` query
  → sorted Vec<i64> of ledgers with trade-shaped ops
    → uploaded to S3 as a Bloom filter or sorted list
      → consumed by the Fargate task on startup
        → archive reads filtered to the list
```

Whether to build this is gated on the trim ratio
measurement — task 0021.

## Open questions for human review

1. **Run the trim-ratio measurement?** Cheap once task 0017's
   local CH lands. Decides Option B's go/no-go. Recommend yes:
   one CH query against the populated instance + a `pandas`-ish
   bucket-count. Task 0021 carries this.
2. **Send the BE inquiry about Option C now or after we have
   measurements?** Pro of now: parallel scheduling. Pro of later:
   you bring actual numbers to the conversation ("if BE built an
   `sdex_trades` table, prices-api would save N CPU-days") rather
   than a hypothetical. Recommend now, with explicit framing as
   "no urgency, exploring options".
3. **Multi-task fan-out parallelism for Option A?** §5.6's 16-day
   single-task estimate may be acceptable, or may not. Multi-task
   parallelism (the extractor is trivially parallelisable across
   disjoint ledger ranges) is a separate design call. Recommend
   defer to implementation time — measure single-task throughput
   on a 1-day sample first.
4. **Should the SDEX backfill task share a binary with the Stream 1
   consumer?** Both decode XDR with the same crate and emit
   `TradeTick`s of the same shape. One binary with `--stream sdex`
   / `--stream soroban-amm` flags vs two binaries is a packaging
   question. Recommend one binary — shared `TradeTick` emit path,
   shared checkpointing, shared CloudWatch wiring.
5. **Classic-LP swaps from path payments — include in SDEX OHLCV
   or split?** R-note §3.3 and G-note §3.3 both flag the
   distinction. For the prices-api's `/ohlcv` endpoint, the
   conservative answer is "include everything classic" — SDEX
   order-book + classic LP swaps both contribute to the same
   classic-Stellar liquidity view. Soroban AMMs (Soroswap etc.)
   stream separately because they're contract-level, not protocol.

## Spawned follow-up tasks

| Slot | Title | State |
|------|-------|-------|
| [0012](../../../backlog/0012_FEATURE_design-prices-owned-backfill-fargate.md) | Design Prices-owned Fargate backfill | existing backlog — G-note becomes its concrete extractor spec |
| [0013](../../../backlog/0013_DOCS_update-design-doc-to-match-be-reality.md) | Update §5.6 / §11 of design doc | existing backlog — 0020's recommendation feeds §5.6 Stream 2 rewrite |
| 0021 (new) | Measure SDEX trade-shaped op density in CH `operations_appearances` | new backlog (spawn) — decides Option B's go/no-go |

## Folded-in resolutions from prior work

- **Task 0015 §5.6 Stream 2** stayed unchanged ("CH does not save
  us from archive reads for SDEX, but can pre-filter modestly").
  This task confirms it formally: pre-filter is the only CH lever
  for Stream 2, and whether to pull that lever is a measurement
  decision. No reversal of any prior decision.
- **ADR 0001 scope.** ADR 0001 covers Stream 1 only. No new ADR
  needed for Stream 2 — Option A is the existing §5.6 plan, and
  Option B is a refinement, not a reversal. If Option B is
  greenlit by 0021's measurement, document it as a §5.6 §"CH
  pre-filter" subsection inline; doesn't warrant its own ADR.
