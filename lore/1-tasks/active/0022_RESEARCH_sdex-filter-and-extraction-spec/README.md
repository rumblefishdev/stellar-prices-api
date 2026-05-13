---
id: "0022"
title: "SDEX filter predicates and extraction spec for the dedicated archive-read backfill"
type: RESEARCH
status: active
related_adr: ["0002"]
related_tasks: ["0012", "0013", "0020"]
tags: [priority-high, effort-medium, research, sdex, backfill, stream-2, xdr, parser]
links:
  - "../../2-adrs/0002_stream2-sdex-archive-backfill-independent-of-be.md"
  - "../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/R-sdex-operation-xdr-shape.md"
  - "../archive/0020_RESEARCH_sdex-historical-backfill-options/notes/G-sdex-trade-extraction-design.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-13
    status: backlog
    who: okarcz
    note: >
      Spawned by ADR 0002 to consolidate the SDEX filter + decode
      contract for the dedicated, BE-independent Stream 2 archive
      backfill. Builds on archived task 0020's R-note (XDR shape
      walk) and G-note (extractor algorithm + TradeTick shape).
      Output is the consumer-ready specification that task 0012's
      Rust implementation module consumes.
  - date: 2026-05-13
    status: active
    who: okarcz
    note: >
      Promoted to active. Use the XDR sample files in `/.temp` as
      example Stellar ledger data for the filter-profile run and
      worked examples. The `xdr-parser` crate from
      `../soroban-block-explorer` is available as a reference
      implementation for XDR decoding if the stellar-xdr crate's
      surface needs adapting.
---

# SDEX filter predicates and extraction spec for the dedicated archive-read backfill

## Summary

Produce the consolidated specification that the Stream 2 SDEX backfill
(ADR 0002 / task 0012) implements. Output is one or two notes in this
task's `notes/` directory: a precise filter contract (which ledgers
contain trade-bearing operations and how to detect them without parsing
every op fully), and a precise decode + bucket contract (how each
matching ledger turns into `TradeTick`s and how those bucket into
`price_ohlcv` rows under ON CONFLICT semantics).

This task takes the **research** outputs from archived task 0020 (R-note
on XDR shape, G-note on extraction design) and produces the
**implementation contract** for task 0012 to code against. No re-research
of the XDR layout — that's settled.

## Context

ADR 0002 commits Stream 2 to archive reads from ledger 1 to tip, with
the BE-authored `stellar-xdr` parser crate as a library dependency. The
trade-shaped op types are the five identified in 0020/R-note (types
2, 3, 4, 12, 13). The price-tick unit is `ClaimAtom` (V0 / ORDER_BOOK /
LIQUIDITY_POOL). The G-note in 0020 sketches the extractor algorithm
and `TradeTick` shape.

What's still open and this task closes:

1. **Filter strategy** — how to cheaply identify trade-bearing ledgers
   without fully parsing every operation in every transaction. Options
   include: parse `TransactionResultMeta` only and short-circuit on
   absence of `ClaimAtom` arrays; pre-filter on `operation.body.type`
   discriminant before touching results; ledger-level early-out via
   counts in `LedgerHeader`. Pick one with measured CPU cost.

2. **Pair canonicalisation** — `ClaimAtom` records `asset_sold` /
   `asset_bought`. Token A / Token B canonical ordering for
   `price_ohlcv` is asset-id-comparison-based (already implied in
   G-note) but the SDEX-specific edge cases (native XLM as one side,
   classic vs SAC for the same asset, liquidity-pool leg attribution)
   need a single source-of-truth table.

3. **`ClaimAtom` variant handling** — V0 (legacy, ≤ protocol 17),
   ORDER_BOOK (modern), LIQUIDITY_POOL (path-payment-through-AMM).
   Each carries the same trade-shaped fields but the parent
   operation context differs; spec the per-variant decode.

4. **`price` computation per `ClaimAtom`** — `amount_bought /
   amount_sold` is the spot price in (bought-units-per-sold-unit).
   Spec the precision strategy (NUMERIC(28,14)), the asset-side
   decimals normalization (stellar classic = 7 stroops; SAC = ?), and
   the failure mode for divide-by-zero or extreme-precision losses.

5. **Bucket + UPSERT contract under ON CONFLICT** — §5.2's UPSERT
   write semantics apply, but the backfill is processing ledgers in
   bulk (potentially out of order if implementation chooses chunked
   scan). Spec the (`asset_id`, `timestamp`, `granularity='1m'`)
   merge rules for backfill writers: preserve `open` (lowest
   ledger-time wins within the minute), overwrite `close` (highest
   ledger-time wins), `GREATEST(high)` / `LEAST(low)`, sum volumes
   and `trade_count`, recompute `vwap`. Match the live-ingestion
   contract exactly so backfill and live converge on the same row
   shape if they touch the same minute.

6. **Resumability checkpoint contract** — what gets written to
   `backfill_progress.current_ledger` and when. Per-ledger?
   Per-chunk? Spec atomicity guarantee (a crash mid-ledger must not
   double-count on resume).

7. **Asset discovery side-effects** — when a `TradeTick`'s asset is
   not in the `assets` table yet, what does the backfill do? Insert
   on the fly? Skip and let the Asset Discovery Lambda backfill the
   gap? Spec the decision.

## Implementation

Run as a pure spec task — no code lands. Produce:

- `notes/G-sdex-filter-strategy.md` — answers 1, 6, 7. Includes the
  measured CPU cost of the chosen filter (parse one sample ledger,
  profile).
- `notes/G-sdex-decode-and-bucket-spec.md` — answers 2, 3, 4, 5.
  Includes one worked example per `ClaimAtom` variant (V0 +
  ORDER_BOOK + LIQUIDITY_POOL), with input XDR bytes → output
  `TradeTick` → resulting `price_ohlcv` row delta.

Both notes are written so that task 0012's Rust implementation can be
graded against them clause-by-clause.

## Acceptance Criteria

- [ ] `notes/G-sdex-filter-strategy.md` covers (1) filter strategy,
      (6) checkpoint contract, (7) asset-discovery side-effects.
      Filter strategy is recommended with a CPU-cost note (rough
      profile, not full benchmark).
- [ ] `notes/G-sdex-decode-and-bucket-spec.md` covers (2) pair
      canonicalisation, (3) per-`ClaimAtom`-variant decode, (4) price
      computation + precision, (5) UPSERT bucket merge rules.
- [ ] One worked example per `ClaimAtom` variant in the decode spec.
- [ ] Cross-links to 0020's R-note (XDR shape) and G-note (algorithm)
      — no duplication, just extension.
- [ ] Task 0012's implementation acceptance criterion ("spec from
      task 0022 is folded into the Rust implementation module")
      is satisfiable: the spec is sectioned so it maps 1:1 onto
      `filter` / `decode` / `bucket` Rust modules.

## Notes

- This task does **not** re-litigate Option A vs Option B from task
  0020. ADR 0002 already settled that. The filter strategy explored
  here is in-binary filtering on XDR-decoded ops, not BE-CH-sourced
  pre-filtering.
- This task does **not** explore parallelisation / sharding strategies
  for the backfill — that's a task 0012 implementation concern
  (single Fargate task with chunked-ledger-range scan is the default
  per §5.6).
- Estimated effort: 2-3 days of spec writing + one profile run on a
  sample ledger range.
