---
id: "0138"
title: "change_24h_pct/change_7d_pct publish a fabricated -100% for every zero-price asset (889 assets, incl. XLM)"
type: BUG
status: active
related_adr: []
related_tasks: ["0072", "0135", "0040"]
tags:
  ["priority-high", "effort-small", "clickhouse", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../docs/runbooks/0072-current-prices-mv-rollout.md"
history:
  - date: 2026-08-03
    status: active
    who: okarcz
    note: >
      Found during [[0072]] step 5 on ch-prod-01, immediately after the read view
      was applied. XLM published `change_24h_pct = -100` while `vwap_24h` read
      0.17093839119102 and `sources` carried two live venues. Measured across the
      surface: **889 of 4,442 assets (20%) report exactly -100 on
      `change_24h_pct`, and 396 on `change_7d_pct`** — every one of them with
      `price_usd = 0`. Blocks 0072 step 6 (the API deploy): the handler would
      serve a fabricated total price collapse on a fifth of all assets, XLM
      included.
---

# `change_*_pct` fabricates -100% whenever `price_usd` is 0

## Summary

`prices.mv_current_prices` computes `change_24h_pct` as
`(price_usd - open_24h) / open_24h * 100`. The denominator is guarded
(`nullIf(…, 0)` → `ifNull(…, 0)` → the documented "no signal" sentinel), **the
numerator is not.** When `price_usd` is 0 and `open_24h` is a real price, the
expression evaluates to exactly **-100**, which is not a sentinel — it is a
plausible-looking real value meaning "this asset lost all of its value in 24h".

`change_7d_pct` has the identical shape against `r.close_7d_ago`.

## Why it happens — the 0135 asymmetry, amplified

`current.sql:200-209`:

```sql
argMax(close_usd, timestamp)                  AS price_usd   -- NO close_usd > 0 filter
argMinIf(close_usd, timestamp, close_usd > 0) AS open_24h    -- filtered
```

`open_24h` skips un-enriched candles; `price_usd` does not. Enrichment is a
separate, lagging pass, so an asset whose newest `price_ohlcv_1m` candle has not
yet been enriched gets `price_usd = 0` beside a perfectly real `open_24h` — and
the subtraction turns that into -100.

This is the same un-enriched-tip defect [[0135]] owns, but the **blast radius is
50× larger downstream than at the source**: only 17 assets show the
"`price_usd = 0` while `vwap_24h` knows the price" signature 0135 was scoped
around, whereas **889** publish a wrong `change_24h_pct`. A zero price is a
missing value; a -100 percent change is a confident wrong answer.

## Measured on ch-prod-01, 2026-08-03

```
rows_total 4,442
exactly_minus_100 (change_24h_pct)      889   (20%)
  of which price_usd = 0                889   (100%)
chg7d_minus_100                         396
zero_price_but_vwap_known                17   <- 0135's own population
```

XLM itself:

```
price_usd  0        price_xlm 0        change_24h_pct -100
vwap_24h   0.17093839119102
sources    {"phoenix":{"price":"0.17046214442402",…},
            "soroswap":{"price":"0.17176302438627",…}}
```

## Why the sentinel contract makes this worse, not better

`views.sql`'s JOIN interop contract tells consumers (including BE's 0199) that
`0` on these columns means "no signal" and must be treated as unavailable. That
contract is sound — but it only covers `0`. **-100 passes straight through every
consumer-side guard we documented**, because it looks like data.

## Implementation

Guard the numerator symmetrically with the denominator, so a zero `price_usd`
lands on the `0` sentinel rather than computing a change against it:

```sql
-- change_24h_pct
ifNull(
    (nullIf(toFloat64(u.price_usd), 0) - toFloat64(u.open_24h))
        / nullIf(toFloat64(u.open_24h), 0) * 100,
    0)
```

and the identical shape for `change_7d_pct` against `r.close_7d_ago`.
`nullIf` makes the numerator NULL, arithmetic propagates it, and the existing
`ifNull(…, 0)` already lands on the sentinel — so this is a one-token change per
column with no new branching.

- **Redeploy mechanics:** a refreshable MV's definition is fixed at create time,
  so this is `DROP VIEW` + re-`CREATE` (`current.sql` applied whole), not
  `ALTER`. No backfill needed — the MV recomputes every row each refresh.
- Extend `current_mv_it.rs` with a seeded asset whose newest candle has
  `close_usd = 0` and an older one with a real price; assert `change_24h_pct`
  is `0`, **not** `-100`. Include the same case for `change_7d_pct`.
- Consider whether `price_xlm` needs the same treatment — it already uses
  `nullIf` on its denominator and lands on 0, so it is believed correct, but it
  should be asserted rather than assumed.

## Explicitly NOT in scope

**Fixing `price_usd` itself** — adding a `close_usd > 0` filter to the `argMax`
so the headline price stops being 0. That is [[0135]], it changes what the
flagship field reports, and it deserves its own decision. This task only stops a
zero price from being laundered into a fabricated percentage.

## Implementation Notes (2026-08-03)

`current.sql` — one `nullIf` per column, wrapping the numerator so it mirrors the
denominator's existing guard. NULL propagates through the arithmetic and the
already-present `ifNull(…, 0)` lands it on the sentinel, so no new branching and
no change to the clamp.

`current_mv_it.rs` — two new fixtures in the existing 0072 test:

- **asset 7 `ZER`** — priced history (`2.00`, 30 min ago) + an un-enriched tip
  (`close_usd = 0`, 1 min ago), plus a `price_ohlcv_1h` 7-day reference at
  `1.00`. This is the prod shape.
- **asset 8 `DIP`** — `2.00` → `0.0001`, a genuine −99.995% crash that the guard
  must leave untouched.

**Why asset 6 did not already cover this.** The pre-existing `EXO` fixture has
*every* `close_usd` at 0, so `open_24h` is 0 too and the **denominator** guard
alone lands on the sentinel — it passes with or without this fix. The prod case
has a real denominator, so only a numerator guard helps. That distinction is why
a test asserting "change must be 0, not -100" already existed and still missed
889 production assets.

**Non-vacuity verified by reverting.** With `current.sql` stashed and only the
test applied, `current_prices_mv_writes_0072_columns_and_filters_outliers` fails
with `got -100`. Restored, it passes. The test also carries an inline control
asserting the un-guarded expression yields −100 on the fixture, so it cannot
silently stop exercising the bug.

**Gotcha for future tests here:** `scalar_f64`'s `fetch_one::<f64>` misdecodes a
`Nullable(Float64)` into garbage (~5.16e120) rather than erroring. The division
produces a Nullable, so the control query needs an explicit `ifNull(…)`. Existing
uses are safe only because they read non-nullable Decimal columns.

Full package suite green against pinned CH **26.3.10.60**: 8 unit, 10
integration.

## Acceptance Criteria

- [x] `change_24h_pct` returns `0` (the sentinel), not `-100`, when
      `price_usd = 0` and the reference close is non-zero.
- [x] Same for `change_7d_pct`.
- [x] A genuine -100% (a real price actually falling to a real near-zero) is
      still representable — the guard keys on `price_usd` being exactly the 0
      sentinel, not on the computed value. *(asset 8, −99.995% preserved)*
- [x] `current_mv_it.rs` covers both, against pinned CH 26.3.10.60, with a
      non-vacuous control that the un-guarded form produces -100.
- [x] `price_xlm`'s equivalent guard asserted rather than assumed.
- [ ] Applied to ch-prod-01 and verified: `countIf(change_24h_pct = -100)` drops
      to approximately the count of assets that genuinely fell ~100%.
- [ ] [[0072]] step 6 unblocked.
