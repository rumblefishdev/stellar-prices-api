---
id: "0020"
title: "Research SDEX historical backfill options — is a dedicated parser+CH backfill needed?"
type: RESEARCH
status: active
related_adr: ["0001"]
related_tasks: ["0015"]
tags: [priority-high, effort-medium, research, sdex, backfill, stream-2, clickhouse, archive-reads]
links:
  - "../../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md"
  - "../../archive/0015_RESEARCH_redefine-backfill-with-be-clickhouse-events/notes/G-ch-tables-for-price-calculation.md"
  - "../../../../docs/prices-api-general-overview.md"
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

- [ ] Four (or more) Stream 2 options enumerated with cost/scope
      sketches in `notes/I-stream2-options.md`.
- [ ] At least one quantitative measurement of "fraction of ledgers
      with trade-shaped ops" (CH `operations_appearances` against
      a sample ledger range — possible once task 0017's local CH
      lands, or earlier against BE's local backfill).
- [ ] Recommendation captured in `notes/S-sdex-backfill-recommendation.md`.
- [ ] If the recommendation requires BE-side scope expansion (e.g.
      Option C), an inbox message to BE drafted (not necessarily
      sent) capturing the ask.
- [ ] Open questions for human review listed.
- [ ] §5.6 / §11 updates folded into task 0013's scope or spawned
      as new doc-update follow-up.

## Notes

- This research overlaps materially with BE-side considerations
  about extending CH's SDEX coverage. Coordinate with BE (fmazur)
  early — the answer to "would BE accept Option C?" gates the
  attractiveness of Option C.
- Don't pre-commit to a CH-flavoured answer just because Stream 1
  went that way. The SDEX shape (`offersClaimed[]` per
  result, not per appearance) is structurally different from
  Soroban events; the same architectural pattern is not
  guaranteed to fit.
