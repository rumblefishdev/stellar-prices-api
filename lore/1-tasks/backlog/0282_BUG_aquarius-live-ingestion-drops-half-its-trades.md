---
id: "0282"
title: "Live ingestion discards ~50% of every Aquarius trade, every day, and has done for at least ten weeks — the same extraction code recovers 100% of them"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0080", "0101", "0100", "0097", "0203"]
tags: [priority-high, effort-medium, milestone-M3, amm, aquarius, ingestion, data-correctness, data-loss, clickhouse]
milestone: 3
links:
  - "../../../packages/prices-ledger-processor/src"
  - "../../../packages/aquarius-extractor/src"
  - "../../../packages/events-backfill/src/run.rs"
history:
  - date: 2026-09-14
    status: backlog
    who: okarcz
    note: >
      Found in the 0101 dry run, which was not looking for it. The dry run
      reported 115,268 aquarius ticks over an 11-day July window where
      price_ohlcv_1m holds 90,667 — a 21% shortfall on a venue 0101 treats as an
      untouched bystander. Checking it against raw events showed the tool is
      exactly right (115,268 ticks = 115,268 raw trade events, 0 failed
      dispatch, 0 unresolved) and the DATABASE is short. Checking recent days
      showed it is not historical: live is losing 41-57% of aquarius trades
      daily, today included. Filed separately from 0101 because it is an active
      production defect, not a historical repair, and because it is larger than
      the gap 0101 exists to close.
---

# Aquarius live ingestion drops about half of every day's trades

## Summary

`prices.price_ohlcv_1m` records roughly **half** the Aquarius trades that exist
in BE's `soroban_events`. Not in a window, not after an incident — **every day,
continuously, including today**. Published Aquarius volume and trade counts are
therefore understated by ~50%, and every coarse tier inherits it.

**It is not the extractor.** `events-backfill` runs the *same*
`classify_amm_groups → dispatch → amm_trade_to_tick` chain over the *same*
events and recovers **100%** of them, with zero dispatch failures and zero
unresolved pools. So the defect is in what the live path is fed or what it does
with it, not in how a trade is decoded.

## Evidence

All measured 2026-09-14 on production as `dev_read`.

### It is current, and it is about half

`raw` = `default.soroban_events` rows with `signature = 'trade'` from contracts
in `prices.pool_registry` where `venue = 'aquarius'`.
`db` = `sum(trade_count)` from `prices.price_ohlcv_1m FINAL`, `source = 'aquarius'`.

| day | raw | db | lost | % |
| --- | --- | --- | --- | --- |
| 2026-09-09 | 11,685 | 6,134 | 5,551 | **47.5** |
| 2026-09-10 | 10,592 | 5,637 | 4,955 | **46.8** |
| 2026-09-11 | 16,958 | 7,314 | 9,644 | **56.9** |
| 2026-09-12 | 6,694 | 3,944 | 2,750 | **41.1** |
| 2026-09-13 | 9,986 | 5,178 | 4,808 | **48.1** |
| 2026-09-14 | 7,429 | 3,446 | 3,983 | **53.6** |

**63,344 raw against 31,653 stored — 50.0% discarded over six days.**

### It is not new, and not an incident

The same comparison over 0101's July window, ledgers 63352609..63518022:

| day | raw | db | % lost |
| --- | --- | --- | --- |
| 2026-07-06 | 9,744 | 6,126 | 37.1 |
| 2026-07-07 | 15,007 | 11,314 | 24.6 |
| 2026-07-08 | 11,174 | 9,077 | 18.8 |
| 2026-07-09 | 8,420 | 7,439 | 11.7 |
| 2026-07-10 | 10,025 | 8,947 | 10.8 |
| 2026-07-11 | 7,334 | 6,642 | 9.4 |
| 2026-07-12 | 7,246 | 6,473 | 10.7 |
| 2026-07-13 | 9,224 | 8,177 | 11.4 |
| 2026-07-14 | 8,788 | 8,087 | 8.0 |
| 2026-07-15 | 11,172 | 9,749 | 12.7 |
| 2026-07-16 | 12,945 | 6,196 | 52.1 |
| 2026-07-17 | 4,189 | 2,440 | 41.8 |

⚠️ **Short on every single day.** Not concentrated in the proto27 freeze
(2026-07-08 → 07-15), which is what a replay artefact would look like. And the
rate has roughly **quadrupled** since July — ~10% then, ~50% now — so whatever
this is, it is getting worse.

### The same code recovers all of them

`events-backfill --dry-run --start 63352609 --end 63518022`, 2026-09-14:

```
events read:               310715
  aquarius ticks:          115268  (swaps failed dispatch: 0)
   phoenix ticks:          1442    (swaps failed dispatch: 70)
  soroswap ticks:          7786    (swaps failed dispatch: 0)
unresolved pools:          0
swaps dropped (unresolved):0
```

115,268 ticks against **115,268** raw `trade` events in the same range — exact.
The live path stored 90,667 for the same span.

## Leads, in order

1. **Concentrated-liquidity pools (first suspect, NOT the answer yet).**
   24 aquarius pools also emit `pool_state`, the CLMM marker, and they account
   for **169,286 of 446,031** recent trades — **37.9%**. That is the right order
   of magnitude but does not reach 41-57%, so it cannot be the whole story.
   [[0080]] is exactly this shape check and should be folded in or done first.
2. **Registry coverage at live time.** The reprice filters by
   `prices.pool_registry`; live builds its own view. 168 aquarius contracts
   emitted trades in the July window against 488 registered. If live's registry
   is narrower than the backfill's at any moment, its swaps are dropped —
   silently, because they do not reach `unresolved_pools` either (see below).
3. **Something between the doorbell and dispatch.** The shared extraction chain
   is exonerated by measurement, so the divergence is upstream of it: which
   events live sees, or which it hands to the reconciler.

⚠️ **Nothing in the system records this.** `prices.unresolved_pools` holds
`source = 'backfill'` rows only — the live path never writes there — so the one
table meant to record "a swap we could not classify" is blind to a defect that
has been discarding half a venue for months. Any fix should close that too.

## Implementation

- Identify the mechanism before changing anything. The discriminator that works
  is **raw `soroban_events` counts vs stored `trade_count`**, per day and then
  per pool; the per-pool split is what will separate "a class of pool" from
  "a fraction of all pools".
- Establish whether the lost trades are a **pool set** (always the same
  contracts) or a **sample** (a fraction of every pool). Those have different
  causes and different fixes.
- Fix the live path, then decide the historical repair separately — the CH-to-CH
  reprice (`events-backfill`) already demonstrably recovers these rows, so the
  repair is cheap once the leak is closed.
- Re-check Phoenix and Soroswap the same way. Phoenix is also short in the July
  window (1,306 stored against 1,442 extracted) for reasons that are **not**
  0099's 7-event gate — every swap group in that window is fully populated — so
  this defect may not be aquarius-only.

## Acceptance Criteria

- [ ] The mechanism is identified and stated plainly, with the pool-set vs
      sample question answered from measurement.
- [ ] The live path is fixed and verified by the same raw-vs-stored comparison
      running at ~0% loss for a full day.
- [ ] It is stated why the shortfall grew from ~10% (July) to ~50% (September).
- [ ] A decision is recorded on repairing the historical estate, with a range.
- [ ] Live-path drops become observable — a dropped swap leaves a trace
      somewhere, rather than nothing at all.
- [ ] Phoenix's parallel shortfall is measured and either folded in or spawned.

## Notes

- Found during [[0101]]'s dry run. 0101's own acceptance criteria assume
  Aquarius is untouched; that assumption is void, and a 0101 write run would
  move aquarius by +27% in its window. 0101 is sequenced behind this decision.
- The reprice tool is **not** implicated and needs no change — its correctness
  is what made this measurable.
