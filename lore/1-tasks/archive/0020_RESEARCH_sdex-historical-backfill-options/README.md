---
id: "0020"
title: "Research SDEX historical backfill options — is a dedicated parser+CH backfill needed?"
type: RESEARCH
status: completed
related_adr: ["0001"]
related_tasks: ["0012", "0013", "0015", "0021"]
tags: [priority-high, effort-medium, research, sdex, backfill, stream-2, clickhouse, archive-reads]
links:
  - "../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/G-ch-tables-for-price-calculation.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-12
    status: backlog
    who: okarcz
    note: >
      Spawned from task 0015 open-question Q3 resolution. With Stream 1
      reframed onto BE's local CH (ADR 0001), the same question applies
      to Stream 2 (SDEX): does a similar dedicated-backfill-with-parser-
      and-CH approach pay off, or is the existing archive-read Fargate
      task pattern still the right shape? Open question — research
      first, then decide.
  - date: 2026-05-12
    status: active
    who: okarcz
    note: >
      Promoted to active. Scope widened by user directive to also
      answer: (a) what does an SDEX operation look like after parsing
      from XDR, and (b) what's the best way to extract data needed
      for token price calculation. These shape the Option B / Option D
      cost-benefit analysis directly.
  - date: 2026-05-12
    status: completed
    who: okarcz
    note: >
      Research complete. 4 notes produced (R/G/I/S). Recommendation:
      Option A baseline (prices-api archive-reader Fargate task), with
      Option B (CH pre-filter via task 0017's local CH) layered on if
      task 0021's trim-ratio measurement justifies the plumbing.
      Stream 2's payload (`ClaimAtom` lists in TransactionResult) is
      not in CH, so the Stream-1 CH-sourced architecture doesn't
      transfer. R-note walks the full XDR shape; G-note specifies the
      extractor algorithm + TradeTick output shape; both answer the
      user's two questions directly. Spawned task 0021 (measurement);
      task 0012 (existing backlog) absorbs G-note as the extractor spec.
---

# Research SDEX historical backfill options — is a dedicated parser+CH backfill needed?

## Summary

The existing §5.6 design for Stream 2 (SDEX trades) is an ECS
Fargate task reading `LedgerCloseMeta` from public archives and
extracting `offersClaimed[]` from `OperationResult` XDR
(~57M ledgers, ~16 days of pure compute). This task ships with
Tranche 2/3.

With Stream 1 now sourced from BE's local CH (ADR 0001), it is
worth asking whether Stream 2 could benefit from a similar shape —
i.e. running BE's `backfill-runner` against archive data to
populate CH `operations_appearances` (and possibly extended SDEX-
specific tables), then querying CH instead of doing the
archive-read in prices-api code.

This task is the research that decides which Stream 2 shape ships.

## Status: Completed

**Outcome:** Recommendation = Option A baseline + Option B as a
free optimisation gated on task 0021's trim-ratio measurement.
User's two scoped questions (XDR shape after parse; best
extraction approach for price calc) answered directly in R-note
and G-note.

## Research Plan

### Step 1 — Catalogue Stream 2 options

| Option | Sketch |
|--------|--------|
| **A** | Status quo: prices-api owns a Fargate Rust task that does archive reads + offersClaimed extraction directly, writes OHLCV to PG. (Today's §5.6 plan.) |
| **B** | Run BE's `backfill-runner --target=clickhouse` over the full archive range; query CH `operations_appearances` and the related `transactions` / `ledgers` tables for trade-shaped ops; do a smaller archive-read step only for the `offersClaimed[]` payload (which lives in `OperationResult` and is not unfolded in CH). Hybrid. |
| **C** | Push BE to add an `sdex_trades` table to CH (the trade-equivalent of `soroban_events`'s full-content unfold for Soroban events). Read-only consumption analogous to Stream 1. Requires BE-side scope expansion. |
| **D** | CH pre-filter only: use CH `operations_appearances` to compute the set of ledgers containing trade-shaped ops, then run the existing Fargate archive-read task against only that subset. Trim factor TBD. |

### Step 2 — Quantify each option

- **A:** Already estimated in §5.6: ~16 days of pure compute. No new dependencies.
- **B:** How much of the 57M-ledger window has at least one
  `MANAGE_*_OFFER` / `PATH_PAYMENT_*` op? (CH query against
  `operations_appearances` once BE's full mainnet backfill is run.)
  The archive-read step still has to happen for every trade-bearing
  ledger because `offersClaimed[]` lives in `OperationResult` and
  the CH `operations_appearances` table does not carry it (the
  `amount` column is the operation's nominal amount, not the
  per-tier matching ladder). So Option B's gain over A is mostly in
  ledger-skip ratio, not in payload-fetch saving.
- **C:** Requires BE to commit to a new fact table. Out of
  prices-api's unilateral control.
- **D:** Strict subset of B's gain (same ledger-skip ratio). Avoids
  introducing CH writes for prices-api; uses BE's already-done backfill.

### Step 3 — Make the call

Recommend one option with reasoning. Spawn follow-up tasks
analogous to 0017–0018 if a non-A option wins.

## Acceptance Criteria

- [x] Four Stream 2 options enumerated with cost/scope sketches in
      [`notes/I-stream2-options.md`](./notes/I-stream2-options.md).
- [ ] At least one quantitative measurement of "fraction of ledgers
      with trade-shaped ops" — **deferred to spawned task 0021**.
      Cannot run today because task 0017's local CH has not been
      populated yet; once it lands, 0021 is ≤ 1 hour of work.
- [x] Recommendation captured in [`notes/S-sdex-backfill-recommendation.md`](./notes/S-sdex-backfill-recommendation.md).
- [x] BE-side ask captured as Open Question 2 in the S-note (light-
      weight inbox to fmazur framed as "no urgency, exploring
      options"). Drafting the actual inbox message is a separate
      light-weight follow-up if/when the user wants to send it.
- [x] Open questions for human review listed in the S-note (5 items).
- [x] §5.6 / §11 updates: folded into the scope of existing backlog
      task 0013 (already amended during 0015 closure to cover
      Stream 1; this S-note adds the Stream 2 picture).
- [x] **User question 1 answered** in [`notes/R-sdex-operation-xdr-shape.md`](./notes/R-sdex-operation-xdr-shape.md)
      — full XDR walk from `LedgerCloseMeta` to `ClaimAtom`.
- [x] **User question 2 answered** in [`notes/G-sdex-trade-extraction-design.md`](./notes/G-sdex-trade-extraction-design.md)
      — extractor algorithm, `TradeTick` shape, pair canonicalisation,
      CH pre-filter optimisation, and why this is the best approach
      vs Horizon REST / Captive Core / BE pipeline reuse.

## Notes layout

```
notes/
├── R-sdex-operation-xdr-shape.md       — XDR shape walk (user Q1)
├── G-sdex-trade-extraction-design.md   — extractor algorithm (user Q2)
├── I-stream2-options.md                — four options (A/B/C/D)
└── S-sdex-backfill-recommendation.md   — recommendation + open Qs
```

## Implementation Notes

- Inputs catalogued: Stellar protocol XDR spec (current at protocol
  22) — `Stellar-transaction.x` (`ManageSellOfferResult`,
  `ManageBuyOfferResult`, `PathPaymentStrict*Result`, `ClaimAtom`,
  `Asset`) and `Stellar-ledger.x` (`LedgerCloseMeta`,
  `TransactionResultMeta`); 0015 G-note for the CH-table shape;
  0015 I-note's option-template; prices-api design doc §5.6 / §10
  estimates.
- Three structurally hard answers produced:
  1. **SDEX trade-relevant ops are five.** Types 2, 3, 4, 12, 13.
     All five funnel through `ClaimAtom`.
  2. **`ClaimAtom` is the price-tick unit.** Three variants
     (V0 legacy, ORDER_BOOK modern, LIQUIDITY_POOL via path
     payment) — all carry the same trade-shaped fields
     `(asset_sold, amount_sold, asset_bought, amount_bought)`.
  3. **CH is a pre-filter for SDEX, not a payload replacement.**
     Unlike Soroban events, the SDEX price payload (`ClaimAtom`s
     in `TransactionResult`) is not unfolded into any CH table.
     This is the architectural reason Stream 2's recommendation
     diverges from Stream 1's.

## Design Decisions

### From Plan

1. **Four options enumerated, then quantitatively
   discriminated.** The original task framing (per S-note of
   0015) was "is dedicated parser+CH needed?" Translating that
   into A/B/C/D comparable on the same axes gave a defensible
   discriminator (trim ratio) for the close call between A and B.

2. **Spawn the measurement as its own task (0021).** Acceptance
   criterion "at least one quantitative measurement" cannot be
   met without task 0017's local CH being populated. Per
   `/lore-framework-tasks` "Deferred items: Mark [ ] with note
   '(deferred to NNNN)'", the cleanest answer is the deferred
   spawn — not blocking 0020 closure on infrastructure that does
   not exist yet.

### Emerged

3. **User-scoped questions answered as discrete R- and G- notes
   rather than woven into S-note.** The user's "how does SDEX op
   look after XDR parse?" and "best way to extract data?" are
   structurally separate from the four-option comparison — the
   XDR shape is true regardless of which option ships, and the
   extractor algorithm applies to A and B alike. Putting them in
   their own notes keeps the comparison clean and gives the
   answers a stable home that any follow-up (0012, 0021, future
   readers) can link to.

4. **G-note's "why this is the best way" section comparing to
   Horizon REST / Captive Core / BE pipeline reuse.** Not asked
   for in the original task scope, but the user's question
   "what's the BEST way" invites a comparison. Added because the
   answer requires the reader to know why the obvious alternates
   are worse.

5. **No new ADR.** Option A is the existing §5.6 plan (no
   reversal); Option B is a refinement (not a reversal). The
   ADR-trigger threshold is "overturns a design-doc
   commitment" — neither does. Recommendation captured in S-note
   and routed through doc-update task 0013.

## Issues Encountered

- No actual data accessible today (task 0017 hasn't run). All
  reasoning is from spec walk-through and informed estimation.
  The "trim ratio" measurement that picks between A and B is
  explicitly deferred to 0021 rather than guessed at.

## Future Work

All future work is captured as spawned/amended backlog tasks:

- Task 0012 (existing backlog) — Fargate backfill design now has
  the G-note as its extractor spec.
- Task 0013 (existing backlog) — §5.6 / §11 doc update covers
  Stream 2's recommendation in addition to Stream 1's.
- Task 0021 (new backlog) — trim-ratio measurement that picks
  between Option A and Option B.
