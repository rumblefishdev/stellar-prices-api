---
id: "0278"
title: "Decide whether to act on dust prints in OHLCV candles — a 34-stroop pool fill can set the day's close, and through the pivot, the USD price of thousands of other candles"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0276", "0116", "0135", "0217", "0238", "0228"]
tags: [layer-backend, priority-medium, effort-medium, ohlcv, ingest, enrichment, data-quality, decision]
links:
  - "../../../../docs/ohlcv-outlier-prints-analysis.md"
  - "../../../../packages/prices-ingest-core/src/tick.rs"
  - "../../../../packages/prices-ingest-core/src/bucket.rs"
  - "../../../../packages/prices-ingest-core/src/filter.rs"
  - "../../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: "2026-09-11"
    status: backlog
    who: akot
    note: >
      Filed from [[0276]]'s price spot check. XLM on 2023-03-11 read 0.0569
      against Bitstamp's 0.0794; tracing it found the day's close was a
      stroop-quantised pool fill (1/17), that all seven XLM closes more than
      5 % off the market are exact small-integer ratios, and that the order
      key has no transaction index. Full analysis with measurements in
      docs/ohlcv-outlier-prints-analysis.md. This task decides what to do
      about it — including nothing.
---

# Decide whether to act on dust prints in OHLCV candles

## Summary

Every fill weighs the same in a candle's `open`/`high`/`low`/`close`. On SDEX a
fill of a few stroops prices at a ratio of two small integers, so it can land
10–1000 % from the market. When it is the last fill of the day it becomes the
close; when it is the XLM/USDC close it becomes the USD reference for every
XLM-quoted asset in that bucket. This task decides, from the measurements in
[`docs/ohlcv-outlier-prints-analysis.md`](../../../../docs/ohlcv-outlier-prints-analysis.md),
which of the candidate fixes are worth doing, in what order, and whether any of
them changes the API's contract enough to need an ADR.

## Context

Found on 2026-09-11 while spot-checking prices after the 0267/0268 rollout
([[0276]]). The rollout is not the cause — it only scaled stored values by the
measured USDC rate.

What the analysis established (numbers in the doc):

- **Cause A — quantisation.** All 7 XLM daily closes > 5 % off Bitstamp are
  exact ratios (1/3, 1/4, 1/1, 1/17, 4/57, 1/10, 5/34). Horizon shows the fill:
  a classic pool leg of 34 stroops XLM for 5 stroops USDC. Deviation tracks fill
  size exactly: < 10 stroops → median 6.6 % off, 100 % exact ratios; ≥ 1 000
  stroops → indistinguishable from dollar-sized fills.
- **Cause B — order key.** `tick.rs` `lex_key = (ledger, operation_index,
  claim_index)` has no transaction index. Our close is not Horizon's last fill
  on 54 of 60 sampled days; 2026-04-02 (−10 %) is this bug alone.
- **Scale.** SDEX 1d candles with ≥ 20 trades: 7.8 % of closes are exact small
  ratios; high > 50 % above VWAP in 21 %, low > 33 % below in 19 %. Soroban AMM
  sources: almost none.
- **Spread.** 2023-03-11: 7 328 XLM-quoted daily candles priced ~26 % low
  (BTC $15 243 vs ~$20 575).

Prior work: [[0116]] documented dust candles as garbage-in and did not change
candle construction. [[0135]] / [[0217]] / [[0238]] are about the headline
`price_usd` and the median mechanism, not candle OHLC or the pivot reference.
[[0228]] is the $1 assumption on XLM/USDT-quoted legs.

## Questions to decide

Each has a recommendation from the analysis; the decision may be "no".

1. **Order key (layer 1a).** Add the transaction index to the ordering key in
   `prices-ingest-core`. Small, fixes open/close everywhere going forward.
   *Recommendation: yes — it is a correctness bug independent of dust.*
2. **Price-forming fills (layer 1b/1c).** Exclude fills with rounding error
   > 0.1 % (`1/amount_sold + 1/amount_bought > 0.001`) from OHLC, keep them in
   volume/VWAP/trade count; decide what a minute with no price-forming fill
   contributes to the rollups. *Recommendation: yes; the threshold is Stellar
   arithmetic, not tuning. Open: 1c design, Soroban base units.*
3. **Close definition (layer 2).** Publish the 1h-and-coarser close as the VWAP
   of the last minute that traded (CME settlement style) instead of the last
   fill. Measured on XLM: worst day 280 % → 4.5 %, typical error unchanged.
   *This changes what `close` means on the wire → ADR, and possibly a new field
   (`settle`?) instead of redefining `close`.*
4. **High/low (layer 3).** Follows from (2); measured proxy cuts days > 10 %
   off from 534 / 598 to 2 / 0. Decide whether high/low are part of the same
   change or a separate one.
5. **Pivot reference (layer 4).** Price XLM-quoted candles from the XLM/USDC
   bucket VWAP instead of the volume-weighted close; take a cross-source
   median in the read path when sources disagree. *Recommendation: yes, first
   — smallest change with the largest blast-radius reduction, no wire change.*
6. **History.** Re-ingest from ledgers with the fixed ingest, or a candle-level
   repair (1m where it survives, hourly-VWAP rule where it does not), or leave
   history as is and document it.
7. **Do nothing.** Document the behaviour in the API docs and the
   general overview instead. Valid if the cost of 1–6 is judged higher than
   the impact.

## Implementation Plan

### Step 1: Close the gaps the analysis names

- Repeat the XLM measurement for 3–5 less liquid assets (e.g. AQUA, yXLM, a
  single-venue token) — does "VWAP of the last traded minute" still work when
  that minute holds one fill?
- Audit the Soroban AMM extractors (`aquarius`, `soroswap`, `phoenix`) for the
  same order-key issue.
- Count how many candles the pivot priced from a quantised XLM/USDC close
  (all grains), to size question 5.

### Step 2: Decide

- Walk questions 1–7 with the team; record each answer and its reason in
  this file.
- For every "yes", spawn an implementation task (BUG/FEATURE) with the
  measured acceptance numbers from the doc.
- If question 3 is "yes", draft the ADR.

## Acceptance Criteria

- [ ] Step 1's three gaps measured and appended to the doc
- [ ] A recorded answer (yes / no / later, with reason) for each of questions 1–7
- [ ] One backlog task per "yes", linked here, carrying the doc's numbers as
      acceptance criteria
- [ ] ADR drafted if the meaning of `close` (or a new field) changes on the wire
- [ ] If everything is "no": the behaviour documented where API consumers read it

## Notes

- The analysis document is the evidence base; keep numbers there, decisions here.
- Horizon (SDF) keeps ~1 year of history; older fill-level checks need a
  full-history Horizon or our own ledger archive.
