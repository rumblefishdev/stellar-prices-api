---
id: "0286"
title: "Candles take every fill at equal weight, including stroop-dust, in the wrong intra-ledger order — rebuild OHLC from price-forming fills with a windowed close, then re-ingest the history"
type: BUG
status: active
related_adr: ["0287"]
related_tasks: ["0278", "0276", "0266", "0228", "0146", "0142", "0137", "0200", "0088", "0282"]
tags: [layer-backend, priority-high, effort-large, ohlcv, ingest, enrichment, clickhouse, data-correctness, api-contract]
links:
  - "../../2-adrs/0287_candle-prices-come-from-price-forming-fills-and-a-windowed-close.md"
  - "../archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/README.md"
  - "../archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/notes/S-close-estimator-and-pivot-reference.md"
  - "../archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/notes/R-measurements-2026-09-15.md"
  - "../../../docs/ohlcv-outlier-prints-analysis.md"
  - "../../../packages/prices-ingest-core/src/filter.rs"
  - "../../../packages/prices-ingest-core/src/tick.rs"
  - "../../../packages/prices-ingest-core/src/bucket.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
  - "../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: "2026-09-15"
    status: backlog
    who: akot
    note: >
      Spawned from [[0278]] as ONE task for the whole candle fix, by Adam's
      decision (not one per phase). Carries decisions D1–D8 and D10 of 0278
      with their measured basis; D9 (cross-source median on the read path) is
      deliberately out of scope. Phases below are sequencing inside this
      task, not separate deliverables.
  - date: "2026-09-15"
    status: active
    who: akot
    note: >
      Activated by Adam; branch feat/0286 cut from develop. Phase 1 first
      (price-forming fills, pf_* columns, windowed close in the rollups, ADR
      0287 already accepted).
---

# Candles are built from dust fills in the wrong order

## Summary

A candle's `open`/`high`/`low`/`close` are single fills at equal weight, and a
fill's price is the ratio of two integer stroop amounts. A fill of a few
stroops therefore prints an exact small fraction (1/17, 5/34) up to hundreds of
percent off the market; when it lands last in the day it is the close, and
through the pivot tier the USD reference for every XLM-quoted asset (7 328
daily candles ~26 % low on 2023-03-11). The order key
`(ledger, operation_index, claim_index)` has no transaction index, so the "last"
fill is also the wrong one on 54/60 sampled days. Full analysis:
`docs/ohlcv-outlier-prints-analysis.md`; decisions and measurements: [[0278]].

This task implements 0278's decisions D1–D8 and D10 as one change in three
phases, and re-ingests the history in place so old and new candles mean the
same thing.

## What changes (0278's decisions, in implementation order)

### Phase 1 — price-forming fills, windowed close, one method per tier

- **D3/D4 in ingest** (`bucket.rs`, `tick.rs`): a fill is *price-forming* iff
  its price does not come from its own rounded amounts, or its rounding bound
  `1/amount_sold + 1/amount_bought ≤ 0.001` holds. Until phase 2 every fill is
  ratio-priced, so the bound applies to all. Every fill still counts in
  `volume_*`, `vwap` and `trade_count`.
- **New 1m columns** (`init.sql`): `pf_trade_count`, `pf_volume` (Σ vᵢ),
  `pf_price_volume` (Σ pᵢ·vᵢ) over price-forming fills, with pᵢ the fill's
  price. ⚠️ Close must be computed from these, never from
  `volume_quote / volume_base` — that ratio *is* the amount-derived price
  (0278 refinement 1). A minute with `pf_trade_count = 0` keeps its volume
  and carries the price fields forward.
- **D5/D6/D7 — one definition on every tier**:
  - `close` = `Σ pf_price_volume / Σ pf_volume` over the bucket's last minutes
    until `Σ pf_trade_count ≥ 3`, capped at 60 minutes and at the bucket;
    fewer if that is all there is. `open` symmetric from the bucket start.
    The 1m candle's close is its own price-forming VWAP.
  - `high`/`low` = extremes of price-forming fills (1m in ingest; rollups keep
    `max(high)` / `min(low)`).
  - The volume-weighted median over the same window is computed alongside and
    `|VWAP / median − 1|` above a threshold flags the candle.
  - Rollups (`rollups.sql`) compute each tier's open/close from the 1m tail of
    the bucket, not from the child tier, and use
    `argMaxIf(…, pf_trade_count > 0)`. This is a DROP + re-CREATE of the six
    MVs under [[0146]]'s rules ([[0142]] drift detection, [[0137]] freshness
    alarm, 0095 invariants). Coordinate with 0146 — ideally one re-CREATE
    window for both.
- **D8 — ADR 0287** (accepted 2026-09-15): `close` and `open` change meaning
  on the wire (window VWAP, not last/first print); `high`/`low` are extremes
  of price-forming fills. The implementation must match it; a deviation
  found while building goes back into the ADR, not silently into the code.
- Pre-roll the coarse tiers (`preroll*.sql`) with the new definitions where
  1m data exists.

### Phase 2 — price from the offer / the reserves, transaction index

- **D2** (`filter.rs`, `price.rs`): order-book fill price = the resting
  offer's `price.N/D` from the offer's ledger-entry change; pool fill price =
  the pool's spot from its reserves (post-swap, i.e. the marginal price);
  amount ratio only as fallback. Requires reading `LedgerEntryChanges` from
  `LedgerCloseMeta`.
- **D3** flips to its final form: offer- and reserves-priced fills always form
  price; the bound stays for the fallback only.
- **D1** (`filter.rs:25`, `tick.rs`, `soroban.rs:642`): `transaction_index`
  from `tx_processing`'s apply order in the key
  `(ledger, transaction_index, operation_index, claim_index)`. Soroban
  extractors sort by transaction hash today; check whether `event_index` is
  per transaction or per ledger before choosing the fix there.

### Phase 3 — history, in place (D10)

- Re-ingest the whole chain (~64 M ledgers, ~16 days, ~4 TB per [[0088]]'s
  rates) with the phase-2 ingest, **overwriting the existing tables**. Because
  `price_ohlcv_1m` is `ReplacingMergeTree(version)` with a deterministic
  `version = ledger·1000 + op`, a re-ingested row ties with the old one, so:
  per monthly partition `FREEZE` → `DROP PARTITION` → re-ingest → volumes
  equal the snapshot to the stroop → next. Then pre-roll the coarse tiers.
- Preconditions: phases 1–2 live and measured on new data; cleanup off for the
  whole run ([[0200]]); [[0137]] deployed; disk budgeted for the snapshots.
- [[0228]]'s pivot re-enrichment campaign runs **after** this, on the repaired
  XLM/USDC reference.

## Out of scope

- D9, the cross-source rule on the read path (median instead of largest
  volume leg). Deferred by 0278; most tokens have one source.
- The pivot's two references (`close_usd` from the end-of-bucket rate,
  `volume_quote_usd` from the bucket VWAP) — 0278 question 5, analysis doc
  §10.3; its own change in `ch_enrich.rs`.

## Acceptance Criteria

Phase 1:
- [ ] A minute holding two 17-stroop fills at 1/17 next to nothing else has
      `vwap = 1/17`, `pf_trade_count = 0` and carries close forward; the same
      minute beside two ordinary fills has `close` from the ordinary fills
      only (unit test in `bucket.rs`, red on `develop`).
- [ ] The 1m row carries `pf_trade_count`, `pf_volume`, `pf_price_volume`,
      and close on every tier equals `Σ pf_price_volume / Σ pf_volume` over
      the D5 window — pinned by a unit test on all six rollup statements and
      by an `#[ignore]` test on 26.3.10.60 reproducing 2026-04-02 (dust in the
      last minute, close from the window).
- [ ] `low ≤ open, close ≤ high` holds on every tier by construction (test).
- [x] ADR accepted for the new meaning of `open`/`close`/`high`/`low` —
      ADR 0287, 2026-09-15, before this task started.
- [ ] Six MVs re-created with APPEND + `sum(version)` + aligned windows
      verified, per-MV freshness recovered ([[0146]]'s checklist).
- [ ] Measured on the first week of new data: residual high/low bias vs
      Bitstamp on XLM, share of minutes with `pf_trade_count = 0`, share of
      buckets whose window hit the 60-minute cap. Recorded here.

Phase 2:
- [ ] A 34-stroop pool fill against reserves quoting 0.1631 prices at 0.1631,
      not 5/34; an order-book fill prices at the resting offer's `N/D`.
- [ ] Key `(ledger, transaction_index, operation_index, claim_index)`; the
      2026-04-02 ledger (path payment first, manage-offer second) picks the
      manage-offer fill as last (unit test).
- [ ] Soroban extractors ordered by apply order, with `event_index`'s scope
      recorded here.

Phase 3:
- [ ] Every monthly partition of `price_ohlcv_1m` re-ingested; per partition
      `sum(volume_base)`, `sum(volume_quote)`, `sum(trade_count)` equal the
      FREEZE snapshot; differences only in OHLC.
- [ ] Coarse tiers pre-rolled; XLM/USDC 1d closes on the seven dust days of
      the analysis are within 5 % of Bitstamp; the count of XLM-quoted
      candles priced from a quantised XLM/USDC close (0278's before-figure:
      22 760 / 143 577 / 85 699 / 30 064 / 10 938 on 15m / 1h / 4h / 1d / 1w)
      is zero after the re-ingest and pre-roll.
- [ ] [[0228]]'s campaign preconditions re-checked against the repaired
      reference before it runs.

## Notes

- Measured basis for every parameter (k = 3, the 0.1 % bound, the offer price
  being on-market): 0278's `notes/R-measurements-2026-09-15.md`. The 60-minute
  cap is a policy choice, unmeasurable on thin pairs.
- The phase-1 unit test for the `vwap` trap only "comes alive" in phase 2
  (before D2 dust is never price-forming), but write it in phase 1 so the
  rollup arithmetic is pinned from the start.
- [[0282]] (candle writes replaced instead of summed across a reconcile run)
  touches the same `version` semantics phase 3 depends on; settle it first.
