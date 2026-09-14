---
id: "0198"
title: "A single zero-volume asset can take down price_usd_series entirely — the view RAISES, it does not degrade"
type: BUG
status: completed
assignee: akot
related_adr: []
related_tasks: ["0172", "0165", "0116", "0150", "0171"]
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
  - date: 2026-08-13
    status: backlog
    who: okarcz
    note: >
      Renumbered 0185 -> 0198. The onboarding epic was re-cut into 0183-0195
      four minutes after PR #205 merged these spawned tasks, so both sides
      claimed the same ids with no chance to see the other. Ours moved because
      the other 0183 was already active and its thirteen slices are a contiguous
      block; these three were backlog with no work in flight. Referring sites
      updated: views.sql, views_it.rs, 0172, 0182, 0168.
  - date: "2026-09-14"
    status: active
    who: akot
    note: >
      Activated; taken by akot together with [[0171]], one branch and one PR
      for both — same views.sql expression, and BE's 2026-08-11 answer on 0171
      (omit the row) is the contract decision this task's AC 3 asks for.
  - date: "2026-09-14"
    status: completed
    who: akot
    note: >
      Fixed on PR #312 with [[0171]]: arm A requires volume_base > 0, so the
      zero-weight group never forms. The 349-vs-sentinel disagreement is
      settled and this task's measurement stands: the INTERPRETED CAST raises
      code 349 (compile_expressions = 0, or a cold server before the
      expression has run 3 times); the JIT-COMPILED one publishes
      Decimal128::MIN. Prod (JIT on, threshold 3) therefore raises right
      after a restart and lies once warm. Regression test runs both modes and
      fails with 349 on develop. Prod count: zero priced zero-volume candles
      on either grain today.
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

- [x] Reproduce on the prod pin with a minimal fixture (an asset that is only a
      zero-volume base, not a peg quote leg)
      ✅ `seed_zero_volume_only_base` in `views_it.rs` (BAR/FOO at zero volume
      beside a real FOO/USDC print). On 26.3.10.60 it raises 349 interpreted
      and publishes the sentinel compiled — see Issues Encountered.
- [x] Confirm whether any asset on prod currently satisfies the condition
      ✅ None: 0 candles with `close_usd > 0 AND volume_base = 0` on either
      grain, 0 zero-volume XLM/USDC reference candles (2026-09-14, `dev_read`).
- [x] Fix chosen with BE input on the omitted-vs-fallback contract
      ✅ Option 1 (filter arm A on `volume_base > 0`), which is BE's
      2026-08-11 "omit the row" decision on [[0171]].
- [x] Regression test that fails with code 349 before the fix
      ✅ `a_zero_volume_only_base_is_absent_and_its_neighbours_still_publish`
      and `usd_reference_omits_a_bucket_whose_reference_candles_have_no_volume`
      read with `SETTINGS compile_expressions = 0` first; on `develop`'s
      `views.sql` both panic with `Code: 349`.
- [x] Correct the stale `Decimal128::MIN` claim wherever it appears
      (`views.sql`, `views_it.rs`)
      ✅ Not stale after all — both claims were true. `views.sql`'s series
      header and the 0165 test's doc now say which mode gives which.

## Implementation Notes

Fixed on PR #312 together with [[0171]]; the full record (files, verification,
prod counts, design decisions) is in 0171's Implementation Notes. This task's
own contribution:

- **The failure mode is JIT-dependent, and this task's measurement stands.**
  `SETTINGS compile_expressions = 0`, or a cold server before the expression
  has run `min_count_to_compile_expression` (3) times, raises code 349 and
  fails the whole query — the availability failure described above. Once
  compiled, the same CAST publishes `Decimal128::MIN` — 0171's reading. Prod
  runs `compile_expressions = 1`, threshold 3, so it raises right after a
  restart and lies once warm.
- **The regression test fails with 349 before the fix, deterministically.**
  Both behavioural tests read each view under `SETTINGS compile_expressions =
  0` first, then under the server default; on `develop`'s `views.sql` the
  first pass panics with `Code: 349` on `price_usd_series` and on
  `usd_reference`.
- **USDT check.** Not needed as a separate query: the prod count of candles
  with `close_usd > 0 AND volume_base = 0` is 0 on both grains for every
  asset, USDT included.
- **Fix option chosen: 1** (filter arm A on `volume_base > 0`). Option 2 (a
  total fallback) and option 3 (`ifNull`) both publish a value the consumer has
  to know about, which is exactly what BE rejected on 0171.

## Issues Encountered

See 0171 — the JIT bisect, the readonly `dev_read` user, and the first
draft's wrong "not reproducible" comment.

## Design Decisions

### From Plan

1. **Omit, do not degrade or substitute.** BE's contract decision on 0171.

### Emerged

2. **The "stale claim" acceptance criterion was satisfied by correcting the
   correction.** The `views.sql` note this task set out to fix was not wrong;
   it was half the picture. Both halves are now recorded where the expression
   lives.
