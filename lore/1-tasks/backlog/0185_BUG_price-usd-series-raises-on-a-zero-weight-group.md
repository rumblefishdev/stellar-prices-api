---
id: "0185"
title: "A single zero-volume asset can take down price_usd_series entirely — the view RAISES, it does not degrade"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0172", "0165", "0116", "0150"]
tags:
  ["priority-high", "effort-small", "clickhouse", "data-correctness", "read-api", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      Found while porting a 0172 test off USDT. The hazard itself is
      pre-existing and was already noted in views.sql as "its own task", but the
      note describes the WRONG failure mode — measured on the prod pin it raises
      an exception rather than publishing a garbage value, which makes it a
      whole-query availability problem, not a one-row correctness problem.
---

# `price_usd_series` raises `CANNOT_INSERT_NULL_IN_ORDINARY_COLUMN` on a zero-weight group

## The mechanism

The view computes:

```sql
if(max(is_peg) = 1 AND sum(w) = 0,
   CAST(1 AS Decimal(38, 14)),
   CAST(sum(v) / nullIf(sum(w), 0) AS Decimal(38, 14)))
```

`w` is `volume_base`. Arm A admits a candle on **`WHERE p.close_usd > 0` alone**
— it never requires `volume_base > 0`. So an asset whose only priced candles in a
bucket carry zero volume reaches `sum(w) = 0`. If that asset is *not* a peg quote
leg, `max(is_peg)` is 0, the guard cannot fire, `nullIf` yields NULL, and the
`CAST` to a non-Nullable `Decimal(38,14)` fails.

## ⚠️ Corrects the existing note in views.sql

The comment in `views.sql` (and the matching one on
`peg_asset_with_only_zero_volume_candles_falls_back_instead_of_publishing_garbage`)
states this case "still publishes Decimal128::MIN". **Measured on the prod pin
(26.3.10.60), it does not.** It raises:

```
Code: 349. DB::Exception: Cannot convert NULL value to non-Nullable type ...
(CANNOT_INSERT_NULL_IN_ORDINARY_COLUMN)
```

That difference matters a lot for severity. Decimal128::MIN corrupts **one row**
and a consumer might filter it. An exception fails **the entire query** — every
other asset in the same `SELECT` returns nothing. This is an availability
problem on the read surface BE depends on, not a data-quality wart.

## Why it has not fired yet (probably)

It needs `close_usd > 0` **and** `volume_base = 0` in the same candle, for every
candle of that asset in a bucket. The enrichment tiers all require
`volume_quote > 0` before writing `close_usd`, which makes the combination
uncommon — but `volume_quote > 0` with `volume_base = 0` is not impossible, and
[[0116]] (dust-trade candles) is the obvious source.

⚠️ [[0172]] removed USDT from the peg set, which removed USDT's *accidental*
protection: it used to get `max(is_peg) = 1` from arm B whenever it was a quote
leg. Worth confirming on prod that USDT has no zero-volume-only buckets.

## Fix options

1. **Filter arm A on `volume_base > 0`.** Smallest change. Turns the case into
   "asset absent from the view", which matches the existing "misses are absent"
   contract — but silently drops assets that only ever trade at zero volume.
2. **Make the fallback total** — `if(sum(w) = 0, …)` without the `is_peg`
   condition — so any zero-weight group degrades instead of raising. Needs a
   decision on what value/method a non-peg zero-weight group should publish.
3. **Wrap in `ifNull`/`coalesce`** so the CAST can never see NULL. Cheapest, but
   picks a value by accident rather than by design.

Option 1 or 2 needs BE input on whether an omitted row or a fallback row is
better for their join.

## Acceptance Criteria

- [ ] Reproduce on the prod pin with a minimal fixture (an asset that is only a
      zero-volume base, not a peg quote leg)
- [ ] Confirm whether any asset on prod currently satisfies the condition
- [ ] Fix chosen with BE input on the omitted-vs-fallback contract
- [ ] Regression test that fails with code 349 before the fix
- [ ] Correct the stale `Decimal128::MIN` claim wherever it appears
      (`views.sql`, `views_it.rs`)
