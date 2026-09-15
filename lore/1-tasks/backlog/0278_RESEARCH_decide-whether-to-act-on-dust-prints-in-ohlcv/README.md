---
id: "0278"
title: "Decide whether to act on dust prints in OHLCV candles — a 34-stroop pool fill can set the day's close, and through the pivot, the USD price of thousands of other candles"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0276", "0116", "0135", "0217", "0238", "0228", "0200", "0286"]
tags: [layer-backend, priority-medium, effort-medium, ohlcv, ingest, enrichment, data-quality, decision]
links:
  - "notes/S-close-estimator-and-pivot-reference.md"
  - "notes/R-measurements-2026-09-15.md"
  - "../../../../docs/ohlcv-outlier-prints-analysis.md"
  - "../../../../packages/prices-ingest-core/src/tick.rs"
  - "../../../../packages/prices-ingest-core/src/bucket.rs"
  - "../../../../packages/prices-ingest-core/src/filter.rs"
  - "../../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../../docs/runbooks/repair-coarse-usd-values.md"
  - "../../archive/0228_BUG_xlm-oracle-rate-is-measured-then-discarded-while-the-pivot-derives-it/README.md"
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
  - date: "2026-09-15"
    status: backlog
    who: akot
    note: >
      Two decisions by akot about [[0228]]'s re-enrichment campaign (runbook
      Appendix C, not run when 0228 closed), recorded here because this task
      now gates it. (1) The campaign runs AFTER question 5 is settled, and
      after its fix is deployed if the answer is yes — otherwise the
      ~190 M-candle reset runs twice. (2) `_15m` is in the campaign's scope. No
      task opened for the campaign itself. See "Decided: 0228's campaign waits
      for question 5".
  - date: "2026-09-15"
    status: backlog
    who: akot
    note: >
      Three additions, no decision taken. (1) Horizon read and measured:
      resting-offer price for order-book fills, a 10 % rounding-slippage
      filter on pool fills, a full TOID order — its close is right on both
      dust days it still holds, its high/low still carry 5/34 and 1/10
      (analysis doc §10.1). (2) Question 5 as written is right for USD volume
      but wrong for USD close on 1d and coarser: a bucket VWAP is the average
      of the period, not its end (§10.2–10.3; 2023-03-11 would land 5.5 %
      high). (3) notes/S-close-estimator-and-pivot-reference.md: the error
      model and the estimator recommendation, in the order 1 → 2 → 3–4 → 5 →
      6.
  - date: "2026-09-15"
    status: backlog
    who: akot
    note: >
      Decisions on the candle fix recorded (section "Decisions on the candle
      fix"). Adam decided D1 (transaction index), D2 (offer/reserves price),
      D4 (carry-forward + price_trade_count), D7 (one method for every tier)
      and D8 (redefine close on the wire, ADR). D3, D5 and D6 were measured on
      prod and Horizon (notes/R-measurements-2026-09-15.md): offer-priced dust
      is inside the noise floor (max 1.4 %), k = 1…3 equivalent and k ≥ 5
      worse, VWAP and weighted median indistinguishable at k = 3; their
      recommendations await confirmation. D9 and D10 open. A review of the
      whole set found five refinements, the first of which (vwap is the
      amount ratio, so close needs price-forming sums with the D2 price)
      changes the 1m schema. Analysis doc and ADR untouched by Adam's request.
  - date: "2026-09-15"
    status: backlog
    who: akot
    note: >
      D10 decided by Adam: re-ingest the history from ledgers and overwrite
      the existing tables in place, no parallel `_v2` set. Recorded with the
      constraint that makes "in place" safe: `price_ohlcv_1m`'s version is
      deterministic, so a re-ingested row ties with the old one under
      ReplacingMergeTree; the procedure is therefore FREEZE → DROP PARTITION
      → re-ingest → verify volumes, one monthly partition at a time, then
      pre-roll the coarse tiers. Open for confirmation: D3, D5, D6.
  - date: "2026-09-15"
    status: backlog
    who: akot
    note: >
      D3, D5 and D6 confirmed by Adam as measured and recommended: threshold
      only for ratio-priced fills; window to 3 price-forming fills, capped at
      60 minutes and at the bucket; VWAP published with the weighted median
      as a flag. Every item D1–D10 is now decided except D9 (cross-source
      median, deferred). Next: spawn the three phase tasks and archive.
  - date: "2026-09-15"
    status: backlog
    who: akot
    note: >
      Spawned [[0286]] — ONE implementation task for the whole candle fix
      (Adam: not one per phase), phases 1–3 inside it. Every question 1–7
      answered inline. Still open here: the ADR (0286 phase 1's first
      deliverable) and the count of candles the pivot priced from a
      quantised close (0286 phase 3's validation). Stays in backlog until
      those two land, then archive.
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
**Answered 2026-09-15** — the answer follows each question in bold; the
decision it rests on is in "Decisions on the candle fix" below, and the
implementation is [[0286]] (one task for all of it, Adam's choice).

1. **Order key (layer 1a).** Add the transaction index to the ordering key in
   `prices-ingest-core`. Small, fixes open/close everywhere going forward.
   *Recommendation: yes — it is a correctness bug independent of dust.*
   **Yes (D1), phase 2 of 0286.** No longer load-bearing for candles once
   close is a minute-granular window VWAP; kept for fill-level correctness.
   Soroban extractors have the same class of defect (sort by transaction
   hash).
2. **Price-forming fills (layer 1b/1c).** Exclude fills with rounding error
   > 0.1 % (`1/amount_sold + 1/amount_bought > 0.001`) from OHLC, keep them in
   volume/VWAP/trade count; decide what a minute with no price-forming fill
   contributes to the rollups. *Recommendation: yes; the threshold is Stellar
   arithmetic, not tuning. Open: 1c design, Soroban base units.*
   **Yes (D2, D3, D4).** Price from the resting offer or the pool's reserves
   (D2); the 0.1 % bound only where the price comes from the amounts (D3,
   measured: offer-priced dust is inside the noise floor); 1c = carry forward
   with `price_trade_count = 0` (D4).
3. **Close definition (layer 2).** Publish the 1h-and-coarser close as the VWAP
   of the last minute that traded (CME settlement style) instead of the last
   fill. Measured on XLM: worst day 280 % → 4.5 %, typical error unchanged.
   *This changes what `close` means on the wire → ADR, and possibly a new field
   (`settle`?) instead of redefining `close`.*
   **Yes, redefined, on every tier (D5, D6, D7, D8).** Close = VWAP over the
   last minutes until 3 price-forming fills, 60-minute cap, within the bucket
   (k measured, cap is policy); VWAP published, weighted median as a flag;
   `close` itself is redefined, no `settle` field; ADR in 0286.
4. **High/low (layer 3).** Follows from (2); measured proxy cuts days > 10 %
   off from 534 / 598 to 2 / 0. Decide whether high/low are part of the same
   change or a separate one.
   **Yes, same change (D7).** Extremes of price-forming fills on every tier;
   the minute-VWAP variant only if the residual bias after D3 measures too
   high on new data.
5. **Pivot reference (layer 4).** Price XLM-quoted candles from the XLM/USDC
   bucket VWAP instead of the volume-weighted close; take a cross-source
   median in the read path when sources disagree. *Recommendation: yes, first
   — smallest change with the largest blast-radius reduction, no wire change.*
   **Partly.** The bucket VWAP is right for `volume_quote_usd` and only a
   stop-gap for `close_usd`; once close is repaired it is the reference again
   (analysis doc §10.2–10.3). Two references, not one — its own change in
   `ch_enrich.rs`, outside 0286. The cross-source median is D9, deferred.
   [[0228]]'s campaign waits for 0286 phase 3.
6. **History.** Re-ingest from ledgers with the fixed ingest, or a candle-level
   repair (1m where it survives, hourly-VWAP rule where it does not), or leave
   history as is and document it.
   **Re-ingest from ledgers, in place (D10).** Partition by partition:
   FREEZE → DROP PARTITION → re-ingest → verify volumes; then pre-roll.
   Phase 3 of 0286.
7. **Do nothing.** Document the behaviour in the API docs and the
   general overview instead. Valid if the cost of 1–6 is judged higher than
   the impact.
   **No.**

## Decided: 0228's campaign waits for question 5

Decided by akot on 2026-09-15. [[0228]] closed with its re-enrichment campaign
not run: runbook Appendix C, twelve passes (six coarse tables × the XLM and
USDT legs). Until it runs, every pre-epoch XLM- and USDT-quoted coarse candle
still assumes USDC = $1. The campaign's operator checklist and its spawn list
item 1 are in 0228's archived README.

**Why this task gates it.** The pivot stores
`close_usd = close × XLM reference in USDC × USDC/USD rate`. 0228 fixed the
third factor; question 5 would fix the second. `pivot_sql` only prices rows
with `close_usd = 0`, so neither fix reaches history until the campaign
resets those rows and re-prices them. Run the campaign before question 5 lands
and the reset has to be repeated afterwards. On 2023-03-11, where the XLM/USDC
daily close is the 1/17 dust print, running first would move 7,328 XLM-quoted
1d candles from 0.0588 to 0.0569 against a market of ~0.0794 (0228 spawn list
item 1).

1. **The campaign runs after question 5.** If the answer is yes, it runs
   after that fix is deployed, so one campaign repairs both factors. If the
   answer is no, nothing holds the campaign back any more.
2. **`_15m` is in the campaign's scope** (0228 Issues 5, about 8.9 M
   pre-epoch rows). This does not wait for [[0200]]: if cleanup is ever
   re-enabled it removes those rows anyway, and if not they would otherwise
   keep USDC = $1 for good. Appendix C's `_15m` count still runs at campaign
   time to size that pass.

**Not decided:** question 6 (history). A repair that rewrites historical
closes changes the first two factors too, so if 6 is yes, its order relative
to the campaign needs the same decision.

**Read before walking the questions (2026-09-15):**
[notes/S-close-estimator-and-pivot-reference.md](notes/S-close-estimator-and-pivot-reference.md)
— the rounding error is bounded and known per fill, so the method is a
filter, not a robust statistic; and question 5 needs **two** references, not
one (analysis doc §10.2–10.3). Horizon's own handling is measured in §10.1.

## Decisions on the candle fix — 2026-09-15

Taken by Adam in a working session, against the ten-point decision list
derived from the S- note. Three were settled by measurement
([notes/R-measurements-2026-09-15.md](notes/R-measurements-2026-09-15.md))
and confirmed by Adam the same day. Mapping to the questions above is in the
last column.

| # | Decision | Status | Q |
| --- | --- | --- | --- |
| D1 | Transaction index in the order key: `(ledger, transaction_index, operation_index, claim_index)`. Soroban extractors too — they sort by transaction hash today; check whether `event_index` is per transaction or per ledger first | **decided** | 1 |
| D2 | Fill price from the resting offer (order book) and from the pool's reserves (AMM); amount ratio only as fallback | **decided** | 2 |
| D3 | The 0.1 % rounding bound applies only to fills whose price comes from their own amounts (fallback); offer- and reserves-priced fills always form price. Until D2 ships every fill is ratio-priced, so phase 1 runs the all-fills variant by construction | **decided** (measured: offer-priced dust max 1.4 % off, inside the noise floor) | 2 |
| D4 | A minute with no price-forming fill keeps its volume, carries the price fields forward, and marks it with `price_trade_count = 0`; rollups use `argMaxIf(…, price_trade_count > 0)` | **decided** | 2 (1c) |
| D5 | Close = VWAP over the last minutes of the bucket until ≥ 3 price-forming fills, capped at 60 minutes, never beyond the bucket; fewer if that is all there is | **decided** (k = 3 measured: k = 1…3 equivalent, k ≥ 5 worse; the 60-min cap is a policy choice, unmeasurable on thin pairs) | 3 |
| D6 | VWAP is the published estimator; the volume-weighted median is computed alongside as a quality flag | **decided** (measured: indistinguishable from the median at k = 3; the median wins only at k ≥ 5) | 3 |
| D7 | One method for every tier: open/close from the D5 window on every tier (computed from 1m), high/low = extremes of price-forming fills on every tier (rollups keep `max`/`min`) | **decided** (one method); residual high/low bias to be measured after D3 ships | 3, 4 |
| D8 | Redefine `close` (and `open`) on the wire, with an ADR | **decided** | 3 |
| D9 | Cross-source rule on the read path: median across sources instead of largest-volume leg | explained; **open**, low priority | 5 |
| D10 | History: re-ingest the whole chain from ledgers with the new ingest, **overwriting the existing tables in place** (Adam, 2026-09-15: no `_v2` table set). Because `price_ohlcv_1m` is `ReplacingMergeTree(version)` with a deterministic `version = ledger·1000 + op`, a re-ingested row ties with the existing one and the survivor is merge-order dependent — so in place means per monthly partition: `FREEZE` → `DROP PARTITION` → re-ingest the month → volumes equal the snapshot to the stroop → next month; then pre-roll the coarse tiers with the new definitions. After D1–D3 are live; cleanup off for the whole run ([[0200]]); [[0137]]'s freshness alarm deployed; [[0228]]'s campaign after this | **decided** (in place) | 6 |

### Refinements found when reviewing the set as a whole

1. **`vwap = volume_quote / volume_base` is the amount ratio**, i.e. exactly
   the price D2 replaces. Close must be computed from price-forming sums that
   use the D2 price: the 1m row gains `pf_trade_count`,
   `pf_price_volume = Σ pᵢ·vᵢ` and `pf_volume = Σ vᵢ`, and every tier's close
   is `Σ pf_price_volume / Σ pf_volume` over its window.
2. **The window's unit is the minute**, not the fill: rollups see minutes.
   "≥ 3 fills" means "the last minutes until Σ pf_trade_count ≥ 3", which is
   what the yXLM measurement computed. The 1m candle's own close is the
   minute's price-forming VWAP.
3. **D1 is no longer load-bearing for the candles.** All fills of a ledger
   share a minute and the window is minute-granular, so intra-ledger order
   affects no candle field. It stays as fill-level correctness.
4. **D2's payoff is smaller after D5**: a correctly priced dust fill still
   weighs ~nothing in a VWAP. Its real gains are minutes with only dust (5.6 %
   on AQUA, 0 % on XLM/USDC) and an exact end-of-bucket price for AMM sources.
   Suggested as phase 2; until then D3 runs in the all-fills variant.
5. **D7 changes the six rollup MVs** (open/close read the 1m tail of each
   bucket instead of chaining tiers), which is a DROP + re-CREATE under
   [[0146]]'s rules, depends on 1m retention ([[0200]]), and means [[0228]]'s
   campaign should either wait for D10 or use the last-hour VWAP reference
   for history (analysis doc §10.2).

### Suggested phasing

| Phase | Scope | Contract |
| --- | --- | --- |
| 1 | D3 (all-fills variant), D4, `pf_*` columns in 1m, D5/D7 in the rollups, D8's ADR | changes (ADR) |
| 2 | D2, then D3 → offer/reserves variant, D1 | none |
| 3 | D10 in-place re-ingest partition by partition, pre-roll of the coarse tiers, then [[0228]]'s campaign on the repaired reference | none |
| later | D9 | none |

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

- [ ] Step 1's three gaps measured and appended to the doc — **two of three**:
      other assets measured (yXLM, AQUA — liquid ones; no reference exists for
      a thin pair) and the Soroban order-key audit done (sort by transaction
      hash, `event_index` scope still to check) in
      notes/R-measurements-2026-09-15.md; the count of candles the pivot
      priced from a quantised close is **not** done — it belongs to 0286
      phase 3's validation.
- [x] A recorded answer (yes / no / later, with reason) for each of questions 1–7
      ✅ 2026-09-15, inline under each question, resting on D1–D10.
- [x] ~~One backlog task per "yes"~~ **One task for all of it** — [[0286]],
      by Adam's decision; carries the measured numbers as acceptance criteria.
- [ ] ADR drafted if the meaning of `close` (or a new field) changes on the wire
      — it does (D8); the ADR is 0286 phase 1's first deliverable, not drafted
      here.
- [x] ~~If everything is "no"~~ — not the case.

## Notes

- The analysis document is the evidence base; keep numbers there, decisions here.
- Horizon (SDF) keeps ~1 year of history; older fill-level checks need a
  full-history Horizon or our own ledger archive.
