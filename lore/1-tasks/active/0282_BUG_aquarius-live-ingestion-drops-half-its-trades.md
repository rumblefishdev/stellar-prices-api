---
id: "0282"
title: "Candle writes are replaced instead of summed whenever a minute bucket spans a reconcile run — Aquarius loses ~50% of its trades daily, and SDEX is affected too"
type: BUG
status: active
assignee: okarcz
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
  - date: 2026-09-14
    status: active
    who: okarcz
    note: >
      Promoted the day it was filed. 0101 moves to blocked by this task: its
      write run would rewrite aquarius rows this defect is still producing, and
      its acceptance criteria use aquarius as an untouched control. First move is
      the pool-set vs sample question, which decides the mechanism.
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


## 🔑 ROOT CAUSE — confirmed 2026-09-14

**Neither a pool set nor a sample. It is per-bucket write contention.**

`price_ohlcv_1m` is `ReplacingMergeTree(version)` keyed on
`(timestamp, asset_id, quote_asset_id, source)` — **not** on the pool. Any
`(minute, pair)` bucket written **more than once** keeps only the write with the
highest `version`; every earlier write is replaced, not summed.

Measured over ledgers > 63,900,000, joining raw events to the stored candles:

| bucket touched by | raw trades | stored | predicted if only the last write survives | retained |
| --- | --- | --- | --- | --- |
| 1 ledger | 14,222 | 14,222 | 14,222 | **100.0%** |
| 2-3 ledgers | 13,837 | 5,800 | 5,796 | **41.9%** (predicted 41.9%) |
| 4+ ledgers | 23,362 | 3,528 | 3,509 | **15.1%** (predicted 15.0%) |

Stored tracks the last-write prediction to within **0.6%** across all three
classes. A bucket confined to one ledger is perfect; everything else loses all
but its final write.

### Why, in the code

`reconcile.rs:125-126` builds a `CandleAccumulator` per source and the loop is
explicitly *"accumulate across the whole contiguous run, flush once at the
end"*, flushing at `:202` / `:214`. **That is correct — within one run.** In
production a doorbell-driven run is **one ledger**, so "flush once at the end of
the run" means *flush once per ledger*, and a minute bucket spans ~12 ledgers at
5 s each. Every one of those ledgers issues its own write for the same bucket,
and RMT keeps the last.

🔑 **This is [[0065]]'s "cross-invocation minute boundary", and it is not a
run-boundary edge case.** [[0101]] carries it as a hazard to respect when
*choosing backfill bounds*: the accumulator keeps the boundary minute open
within a process run, and "that guard does not span separate invocations". True
— and in live, every ledger is a separate invocation, so the guard never
applies at all. What was filed as an operator footgun is the dominant live
data-loss mechanism.

### Why it grew from ~10% to ~50%

Loss is a function of **ledgers per run**. During the July window the processor
spent long stretches catching up after the proto27 freeze, processing many
ledgers per invocation — so buckets were accumulated properly and loss was
~10%. In steady state it handles one ledger per doorbell, which is the
worst case, and loss settles at ~50%. **The system loses the most data when it
is healthiest.**

### Why aquarius is worst

It is not aquarius-specific. Aquarius has **488 registered pools**, many trading
the same asset pairs, so its `(minute, pair)` buckets are the most crowded and
straddle the most ledgers. Phoenix (19 pools) and Soroswap (221) have the same
defect at lower rates — Phoenix stored 1,306 against 1,442 extracted in the July
window, which this explains and 0099's 7-event gate does not.

### ⛔ RETRACTED — "SDEX IS AFFECTED" was WRONG. SDEX loss is UNMEASURED.

**Retracted 2026-09-14, same day, before any code was written.** The contested
SDEX buckets are **not** partial slices. Captured with every column:

```
sdex 11:14 a=4     q=111 tc=1 vb=0.7684586         vq=0.911451   o=c=1.186077 v=64423870000
sdex 11:14 a=4     q=111 tc=1 vb=0.7684586         vq=0.911451   o=c=1.186077 v=64423870001
sdex 11:14 a=79851 q=3   tc=1 vb=792346985.1833504 vq=0.0000001  o=c=0        v=64423863003
sdex 11:14 a=79851 q=3   tc=1 vb=792346985.1833504 vq=0.0000001  o=c=0        v=64423863004
```

**Byte-identical in every field**, differing only in `version` by one in
`op_index`. That is the **same trade written twice**, not two slices of a
minute. RMT keeps one, `trade_count` and volume come out correct, and **nothing
is lost.** Contrast the genuine aquarius case, where the two rows carry
*different* data (`tc 2 / vol 2,513.67` against `tc 1 / vol 1,000`).

So: **SDEX has no measured loss.** It remains *plausible* on the mechanism —
same loop, same accumulator, same flush — but it is unmeasured, and "contested
bucket count" turned out to be the wrong instrument because it cannot tell a
duplicate from a slice. Distinguish them by **comparing the rows' payloads**,
not by counting them.

### 🔑 A separate, real finding: `operation_index` is not stable across re-processing

`reconcile.rs:6-7` claims re-processing is *"idempotent: ReplacingMergeTree
collapses re-inserts by `version`"*. **It does not.** The pairs above are the
same trade at two different `op_index` values, so `version` differs and RMT
keeps both as distinct rows rather than collapsing them.

Benign today — the duplicates carry identical payloads, so the surviving row is
correct. But the stated idempotency guarantee is false, and it is the guarantee
that makes crash-recovery safe. A re-insert that carries a **higher version and
less data** is exactly the aquarius failure, so this is the same hazard one step
away from firing.

⚠️ Also note `version = ledger_sequence * 1000 + operation_index`
(`bucket.rs:48`) allows only **1000 operations per ledger**. A ledger with more
collides into the next ledger's version space. Not observed, but unbounded by
anything in the code.

### Original (aquarius) evidence, which stands

Caught directly. BE holds no classic-trades table, so there is no external
source of truth for SDEX; instead the **losing writes themselves** were observed
before the background merge destroyed them.

A contested bucket is visible as **more than one physical row** for the same
`(timestamp, asset_id, quote_asset_id, source)` in a non-FINAL read. Sampling
the in-flight minute three times, 20 s apart:

| sample | sdex buckets | contested | |
| --- | --- | --- | --- |
| 1 | 834 | 0 | merges had collapsed them |
| 2 | 940 | 0 | merges had collapsed them |
| 3 | 595 | **23** | **3.87%** |

⚠️ **3.87% is a lower bound, not a rate.** RMT merges collapse duplicate rows
within seconds, so a snapshot sees only what has not yet merged. Two of three
samples saw nothing at all while the defect was certainly occurring.

The captured aquarius example, same mechanism, with both sides still present:

| bucket | trade_count | volume_base | version | ledger |
| --- | --- | --- | --- | --- |
| `11:04`, asset 4 / quote 3 | 2 | 2,513.6719 | 64423761005 | 64,423,761 |
| `11:04`, asset 4 / quote 3 | 1 | 1,000.0000 | 64423766004 | 64,423,766 |

Two writes, five ledgers apart, same minute. RMT keeps the higher version, so
**1 trade / 1,000 survives and 2 trades / 2,513.67 is discarded**. The true
minute is 3 trades / 3,513.67.

⚠️ **Correction to the wording above: it is one write per RUN, not per ledger.**
The accumulator flushes once per reconcile run, and a run covers a few ledgers —
the captured pair is five apart. A bucket is only damaged when it **spans a run
boundary**, which is why single-ledger buckets are always intact. The retention
arithmetic matching last-*ledger* so closely implies most runs are short.

**Quantifying SDEX needs a different instrument**, since the evidence
self-destructs: either tight sampling of the in-flight minute over a long
period (lower bound only), instrumenting the writer to count buckets it writes
more than once, or reconciling against Horizon / the ledger archive.

### What the fix has to do

Make the bucket write **additive rather than replacing**, or ensure exactly one
write per bucket ever happens. The mechanism is already proven in this codebase:
`events-backfill` accumulates the full range before writing one row per bucket
and recovers 100%. Options worth weighing: widen the run so a bucket cannot
straddle it (fragile — it only narrows the window), carry the open minute across
invocations in durable state, or move `1m` to a summing engine so concurrent
partial writes add. The last changes the table contract and needs its own
decision.

## Superseded leads (kept — they were how it was found)

⛔ All three were **wrong**, and are kept only so they are not re-run.

1. ⛔ **Concentrated-liquidity pools.** FALSIFIED — aquarius `trade` events have
   exactly **one** shape in the window (4 topics: `trade`, token_in, token_out,
   user; 446,108 events, 212 pools). There is no shape split to explain a
   split outcome.
   24 aquarius pools also emit `pool_state`, the CLMM marker, and they account
   for **169,286 of 446,031** recent trades — **37.9%**. That is the right order
   of magnitude but does not reach 41-57%, so it cannot be the whole story.
   [[0080]] is exactly this shape check and should be folded in or done first.
2. ⛔ **Registry coverage at live time.** FALSIFIED as the cause — there are
   **zero** minutes where trades exist and no candle was stored, which is what a
   dropped pool set would produce. (Still worth noting separately: all 488
   aquarius and all 19 phoenix `pool_registry` rows have **empty
   `token0`/`token1`**; only soroswap's 221 are populated. It does not cause
   this, because the tokens are carried in the event topics.) The reprice filters by
   `prices.pool_registry`; live builds its own view. 168 aquarius contracts
   emitted trades in the July window against 488 registered. If live's registry
   is narrower than the backfill's at any moment, its swaps are dropped —
   silently, because they do not reach `unresolved_pools` either (see below).
3. ✅ **Something between the doorbell and dispatch** — this one was right, and
   resolved above. The shared extraction chain
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

- [x] **The mechanism is identified** — per-bucket RMT write contention, and
      the pool-set vs sample question is answered: **neither**. See §ROOT CAUSE.
- [ ] The live path is fixed and verified by the same raw-vs-stored comparison
      running at ~0% loss for a full day.
- [x] **Why it grew from ~10% to ~50% is stated** — loss is a function of
      ledgers per run, and steady-state (one ledger per doorbell) is the worst
      case. See §ROOT CAUSE.
- [x] 🔴 **Whether SDEX is affected is answered: YES.** 23 contested buckets of
      595 caught in one snapshot. Magnitude still unknown — see §SDEX.
- [ ] SDEX's loss is **quantified**, with an instrument that does not depend on
      catching duplicates before the merge.
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
