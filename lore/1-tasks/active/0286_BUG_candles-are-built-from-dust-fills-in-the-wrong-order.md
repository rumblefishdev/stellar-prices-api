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
      Activated by Adam; branch fix/0286_candles-are-built-from-dust-fills-in-the-wrong-order
      cut from develop. Phase 1 first
      (price-forming fills, pf_* columns, windowed close in the rollups, ADR
      0287 already accepted).
  - date: "2026-09-15"
    status: active
    who: akot
    note: >
      Specification review against the code resolved, on the branch, before
      phase 1 starts: window anchored at the last price-forming minute (60-min
      cap from there); open = close on 1m, disclosed; rollups maxIf/minIf, no
      carried high/low, stateless ingest with read-side carry-forward; coarse
      close_usd = close × latest priced child's rate (replaces 0146's product
      carry — one MV re-CREATE for both); missing 1m tail falls back to the
      child's close with close_window_fills = 0; close_median stored; pool
      price post-fill; pf_trade_count everywhere; 0142/0137 already live;
      0228's reset campaign unnecessary after the re-ingest, re-enrichment
      added to phase 3's cost; k = 1 vs 3 on XLM/USDC to measure before
      hard-coding; soroban.rs, writers and docs added to scope.
  - date: "2026-09-15"
    status: active
    who: akot
    note: >
      Three more resolutions, decided by Adam (ADR 0287 second amendment):
      coarse open/close come from a settle pass into price_ohlcv_settle,
      joined by the MVs, instead of MVs reading 1m tails (option b); the
      enrichment candidate predicate excludes close = 0 rows, in scope here;
      pool fills keep their execution ratio with the 0.1 % bound, the
      reserve-based price is dropped. Phase 2 shrinks to offer prices and
      the transaction index.
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

- **D3/D4 in ingest** (`bucket.rs`, `tick.rs`, and `soroban.rs` — AMM
  ticks enter through `amm_trade_to_tick`, not `tick.rs`, and the bound must
  be evaluated on the raw `i128` amounts in each token's own decimals, not
  after `AMM_AMOUNT_SCALE`): a fill is *price-forming* iff its price does
  not come from its own rounded amounts, or its rounding bound
  `1/amount_sold + 1/amount_bought ≤ 0.001` holds. Until phase 2 every fill is
  ratio-priced, so the bound applies to all. Every fill still counts in
  `volume_*`, `vwap` and `trade_count`. The ingest keeps **no state** across
  minutes or backfill chunks: a minute with no price-forming fill is written
  with `pf_trade_count = 0` and price fields 0; carry-forward of
  `open`/`close` is the read surface's job, and `high`/`low` are never
  carried (ADR 0287 §6).
- **New 1m columns** (`init.sql`): `pf_trade_count`, `pf_volume` (Σ vᵢ),
  `pf_price_volume` (Σ pᵢ·vᵢ) over price-forming fills, with pᵢ the fill's
  price. ⚠️ Close must be computed from these, never from
  `volume_quote / volume_base` — that ratio *is* the amount-derived price
  (0278 refinement 1). Every tier also gains `close_median`,
  `open_window_fills` and `close_window_fills` (ADR §4, §7). All writers of `price_ohlcv_1m` are
  positional (`OhlcvRow` in `writer.rs`, the backfill sink): they change in
  the same commit as the schema.
- **D5/D6/D7 — one definition on every tier**:
  - `close` = `Σ pf_price_volume / Σ pf_volume` over whole minutes, **anchored
    at the last minute of the bucket with `pf_trade_count > 0`**, extended
    backwards until `Σ pf_trade_count ≥ 3`, never more than 60 minutes before
    the anchor and never before the bucket start; fewer if that is all the
    window reaches. `open` mirrored from the first price-forming minute. On
    the 1m tier `open = close` = the minute's own price-forming VWAP (ADR §2).
  - `high`/`low` = extremes of price-forming fills; rollups use
    `maxIf(high, pf_trade_count > 0)` / `minIf(low, …)` (ADR §3).
  - `close_median` on every tier (1m: weighted median of price-forming fill
    prices, in the ingest; coarse: weighted median of the window's minute
    VWAPs); the read surface flags `|close / close_median − 1| > 1 %`.
  - **Settle pass** (new, enrichment-worker, beside the coarse sweep; ADR
    §4): for every coarse tier, once a bucket has closed, compute its
    `open`/`close` windows, `open_window_fills`, `close_window_fills` and
    `close_median` from the bucket's 1m rows and write them to a new table
    `prices.price_ohlcv_settle` (key: tier, asset_id, quote_asset_id,
    source, timestamp; `ReplacingMergeTree(settled_at)`; plus
    `tail_version` = Σ `version` of the 1m rows the windows read). Re-settle
    only when `tail_version` changes; **never write when the bucket's 1m rows
    are gone**, so 1m retention ([[0200]]) cannot degrade a settled close.
    Thin pairs make the pass scan the whole bucket's 1m once, at settle time.
  - Rollups (`rollups.sql`) **do not read 1m**: they take open/close and the
    window columns from `price_ohlcv_settle` by key, and high/low/volumes
    from the child tier with `maxIf`/`minIf` on `pf_trade_count > 0`. **No
    settle row** (bucket still open, not yet settled, or history whose 1m
    never existed): fall back to the child tier's
    `argMaxIf(close, timestamp, pf_trade_count > 0)` /
    `argMinIf(open, …)` with `close_window_fills = 0`; the next refresh after
    the settle pass replaces it.
  - **Coarse `close_usd`** = `close × argMaxIf(close_usd / close, timestamp,
    close_usd > 0)` over the children — the latest priced child's *rate*, not
    its product (ADR §5). No priced child → 0, the worker's coarse pass
    prices it. This replaces the `argMaxIf(close_usd, …)` carry [[0146]] is
    about to land: coordinate so the six MVs are re-created once, with both.
  - The re-CREATE runs under 0146's checklist (0095 invariants, per-MV
    freshness). [[0142]] (drift detection) and [[0137]] (freshness alarm) are
    already live, so nothing else gates the DROP window.
- **D8 — ADR 0287** (accepted 2026-09-15): `close` and `open` change meaning
  on the wire (window VWAP, not last/first print); `high`/`low` are extremes
  of price-forming fills. The implementation must match it; a deviation
  found while building goes back into the ADR, not silently into the code.
- **Enrichment candidate predicate** (`ch_enrich.rs:488`, `CANDIDATE_PRED`,
  and every statement and gauge built on it; ADR §6): becomes
  `(volume_quote_usd = 0 OR (close_usd = 0 AND close > 0)) AND volume_quote > 0`.
  Without it every dust-only minute (`close = 0`) is re-selected and
  rewritten on every pass, and the "enriched 0 rows despite a non-empty
  backlog" warning fires permanently. In scope here, not in the pivot's
  two-reference change.
- Run the settle pass over the existing history where 1m exists, then
  pre-roll the coarse tiers (`preroll*.sql`) joining the settle table.

### Phase 2 — offer price for order-book fills, transaction index

- **D2** (`filter.rs`, `price.rs`): order-book fill price = the resting
  offer's `price.N/D` from the offer's ledger-entry change (the execution
  price; Horizon does the same). Requires reading the offer entries from
  `LedgerEntryChanges` in `LedgerCloseMeta`. **Pool fills keep the amount
  ratio** — it is their execution price — and the 0.1 % bound excludes pool
  dust (ADR §1; reserve-based pricing rejected, see the ADR's alternatives).
- **D3** flips to its final form: order-book fills always form price; pool
  fills and any fill whose offer change cannot be read take the bound.
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
  reconciled against the snapshot → next. Then pre-roll the coarse tiers.
  "Reconciled" means: SDEX volumes equal to the stroop; AMM sources may come
  back **larger** where [[0282]]'s replaced-instead-of-summed writes lost
  trades, and that delta is accounted for per source and per month. The
  Soroban AMM history entered through its own path (events, pool registry
  seeded first — [[0088]]); phase 3 covers it with the same D1–D3 rules or
  says why not.
- Preconditions: phases 1–2 live and measured on new data; cleanup off for the
  whole run ([[0200]]); disk budgeted for the snapshots; [[0282]] settled,
  because the re-ingest writes with the same `version` semantics.
- **Cost not in the ledger estimate:** re-ingested rows carry `close_usd = 0`,
  so the enrichment worker re-prices the entire history afterwards (for
  scale: 0268's campaign re-priced 9.94 M candles in ~24 min; 0228's, never
  run, was estimated at ~190 M candles and ~4 h 40 m; this is all of them). That same fact makes [[0228]]'s reset-mode campaign
  (Appendix C) **unnecessary**: the refill happens inside the re-enrichment;
  only its after-check (`post_run_0228_it`) still runs, on the repaired
  XLM/USDC reference.

## Out of scope

- D9, the cross-source rule on the read path (median instead of largest
  volume leg). Deferred by 0278; most tokens have one source.
- The pivot's two references (`close_usd` from the end-of-bucket rate,
  `volume_quote_usd` from the bucket VWAP) — 0278 question 5, analysis doc
  §10.3; its own change in `ch_enrich.rs`. (The candidate-predicate change
  above is in scope; this one is not.)

## Acceptance Criteria

Phase 1:
- [ ] A minute holding two 17-stroop fills at 1/17 next to nothing else has
      `vwap = 1/17`, `pf_trade_count = 0` and price fields 0; the same
      minute beside two ordinary fills has `close` from the ordinary fills
      only and `low` untouched by the dust (unit test in `bucket.rs`, red on
      `develop`).
- [ ] The read surface shows a `pf_trade_count = 0` candle with `open`/`close`
      carried from the last price-forming candle and `high`/`low` not
      carried (test on the read path).
- [ ] A dust-only minute (`close = 0`, `volume_quote > 0`) gets
      `volume_quote_usd` priced and is **not** re-selected by the next
      enrichment pass (`#[ignore]` test on 26.3.10.60, red on `develop`).
- [ ] The 1m row carries `pf_trade_count`, `pf_volume`, `pf_price_volume`,
      and close on every tier equals `Σ pf_price_volume / Σ pf_volume` over
      the D5 window — pinned by a unit test on all six rollup statements and
      by an `#[ignore]` test on 26.3.10.60 reproducing 2026-04-02 (dust in the
      last minute, close from the window).
- [ ] `low ≤ open, close ≤ high` holds on every tier by construction (test).
- [ ] Before the window constant is hard-coded: k = 1 vs k = 3 measured on
      XLM/USDC against Bitstamp's daily close (the R- note's query, the
      pivot's reference pair, trending — yXLM was pegged), recorded here.
- [ ] A coarse bucket whose last minute holds one dust fill and whose 1m tail
      holds priced minutes gets `close_usd = close × the latest child's rate`,
      not the child's `close_usd` (`#[ignore]` test on 26.3.10.60).
- [ ] Settle pass: a closed bucket gets exactly one settle row computed from
      its 1m windows; deleting the bucket's 1m rows afterwards and
      re-running the pass and the MV leaves `close` unchanged; a late 1m
      write inside the window re-settles it (`#[ignore]` tests).
- [ ] A coarse bucket with no settle row falls back to the child's close with
      `close_window_fills = 0`, and the first MV refresh after its settle row
      appears replaces it (test); a 1m minute with `pf_trade_count = 0`
      contributes to no `max`/`min`/`argMax` of its parent (test).
- [ ] `close_median` present on every tier and equal to the windowed weighted
      median on a fixture (test).
- [ ] `docs/database-schema/database-schema-overview.md`, the OpenAPI
      description of `open`/`close`/`high`/`low`, and the general overview
      describe the ADR 0287 definitions, `pf_trade_count`, `close_median` and
      `close_window_fills`.
- [x] ADR accepted for the new meaning of `open`/`close`/`high`/`low` —
      ADR 0287, 2026-09-15, before this task started.
- [ ] Six MVs re-created with APPEND + `sum(version)` + aligned windows
      verified, per-MV freshness recovered ([[0146]]'s checklist).
- [ ] Measured on the first week of new data: residual high/low bias vs
      Bitstamp on XLM, share of minutes with `pf_trade_count = 0`, share of
      buckets whose window hit the 60-minute cap. Recorded here.

Phase 2:
- [ ] An order-book fill of 17 stroops against an offer at 0.0794 prices at
      the offer's `N/D` and forms price; a 34-stroop pool fill (5/34) does
      not form price and still counts in volume (unit tests).
- [ ] Key `(ledger, transaction_index, operation_index, claim_index)`; the
      2026-04-02 ledger (path payment first, manage-offer second) picks the
      manage-offer fill as last (unit test).
- [ ] Soroban extractors ordered by apply order, with `event_index`'s scope
      recorded here.

Phase 3:
- [ ] Every monthly partition of `price_ohlcv_1m` re-ingested; per partition
      and per source `sum(volume_base)`, `sum(volume_quote)`,
      `sum(trade_count)` reconciled against the FREEZE snapshot — equal for
      SDEX, ≥ with the delta explained by [[0282]] for AMM sources;
      differences otherwise only in OHLC.
- [ ] Coarse tiers pre-rolled; XLM/USDC 1d closes on the seven dust days of
      the analysis are within 5 % of Bitstamp; the count of XLM-quoted
      candles priced from a quantised XLM/USDC close (0278's before-figure:
      22 760 / 143 577 / 85 699 / 30 064 / 10 938 on 15m / 1h / 4h / 1d / 1w)
      is zero after the re-ingest and pre-roll.
- [ ] The whole history re-enriched (`close_usd > 0` wherever a reference
      exists) and `post_run_0228_it` green on the repaired reference; 0228's
      reset-mode campaign recorded as superseded, not run.

## Notes

- Measured basis for every parameter (k = 3, the 0.1 % bound, the offer price
  being on-market): 0278's `notes/R-measurements-2026-09-15.md`. The 60-minute
  cap is a policy choice, unmeasurable on thin pairs.
- The phase-1 unit test for the `vwap` trap only "comes alive" in phase 2
  (before D2 dust is never price-forming), but write it in phase 1 so the
  rollup arithmetic is pinned from the start.
- [[0282]] (candle writes replaced instead of summed across a reconcile run)
  touches the same `version` semantics phase 3 depends on; settle it first.
- 2026-09-15 specification review (see ADR 0287's second history entry)
  found the record short of the mechanics in five places — coarse
  `close_usd`, the missing 1m tail, the carry-forward mechanism, the window
  anchor, `open = close` on 1m — plus wording drift (column name, pool price
  before/after, 0142/0137 state). All resolved above; no 0278 decision was
  reversed.
