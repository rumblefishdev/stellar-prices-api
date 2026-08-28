---
id: "0229"
title: "Derived O/H/L rounds to 14 decimals while `close` stays exact, so `/ohlcv` can return `close` BELOW `low` — a malformed candle"
type: BUG
status: active
related_adr: ["0011"]
related_tasks: ["0170", "0225", "0120", "0127"]
tags: ["priority-medium", "effort-small", "api", "data-correctness", "read-surface", "ohlcv", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../tools/scripts/conformance-0120.mjs"
history:
  - date: 2026-08-28
    status: active
    who: okarcz
    note: >
      ACTIVATED. Picked up the morning after [[0227]] closed, as the one
      consumer-facing defect among the three tasks [[0170]]'s verification run
      spawned. It is 9 of the 27 remaining [[0120]] failures, and the only one
      of them that is a real defect rather than a suite-vs-ADR mismatch.
  - date: 2026-08-27
    status: backlog
    who: okarcz
    note: >
      Found by the [[0120]] conformance re-run against the deployed API, the
      run that verified [[0170]]'s denomination fix. 9 failures of
      `low <= open,close <= high on every bucket`, one bucket each on BTC, sUSD
      and XRP over a 7-day 1h window. Not a data error and not a regression in
      the values — a **scale mismatch introduced by the denomination path**:
      `close` is exact while the extremes are derived and rounded, so the two
      can cross by ~1e-11. Spawned rather than folded into 0170 because 0170's
      gate is met and this is a distinct, independently testable defect.
---

# Derived O/H/L can round past an exact `close`

## Summary

`GET /v1/assets/{id}/ohlcv` in USD mode can return a candle whose `close` is
**below its `low`**. Measured on prod, BTC 1h, bucket `2026-08-25T03:00:00Z`:

```json
{
  "low":   "76943.51350417598",
  "close": "76943.51350417596657"
}
```

`close` is smaller than `low` by `2e-11`. The same shape appears once each on
`sUSD` and `XRP` in the same 7-day window — rare, but the invariant is absolute
as far as a consumer is concerned.

## Root cause

ADR 0011 §3: on the normal path **`close` is exact** — it is `close_usd` as
stored, `Decimal(38, 14)` rendered at full precision — while
`open`/`high`/`low`/`vwap` are **derived**, each multiplied by the bucket's
single rate and rounded to 14 decimals.

Exact and derived are therefore on **different scales**, and when the true value
sits within half a tick of the rounding boundary the derived extreme lands above
(or below) the exact close. Nothing is wrong with either number in isolation;
they are simply not comparable at the precision they are emitted with.

⚠️ This is not the `close_usd`-is-zero class ([[0145]]) and not the precision
floor ([[0170]] §7). Both of those are about inputs being meaningless. Here the
inputs are fine and the *output rendering* breaks an invariant.

## Why it matters

`low <= open,close <= high` is what every charting library assumes. A candle
that violates it renders inverted, is dropped, or throws, depending on the
library — and the failure is intermittent and asset-dependent, which is the
worst shape for a consumer to debug. It is also an explicit assertion in the
[[0120]] suite, so it blocks that suite going green.

## Implementation

Two candidate fixes; decide with the numbers rather than by taste.

1. **Clamp the derived extremes to include the exact close** —
   `low = min(low_derived, close)`, `high = max(high_derived, close)`. Preserves
   `close`'s exactness, which is the property ADR 0011 §3 deliberately kept, and
   cannot introduce a value the market did not reach beyond the tick already
   implied by rounding.
2. **Round `close` to the same 14-decimal scale as the derived fields.** Makes
   every field comparable by construction, at the cost of discarding precision
   the store actually holds.

Option 1 is the starting recommendation — it keeps the documented distinction
between measured and derived rather than erasing it — but the choice should be
recorded either way, since it is a contract detail consumers will see.

⚠️ Whatever is chosen must hold for the **XLM denomination** path too, and for
the synthesized peg series, where all fields come from one rate and the
invariant is trivially satisfied today.

## Acceptance Criteria

- [ ] A test seeds a bucket whose derived `low` rounds above its exact `close`
      and asserts the response satisfies `low <= open,close <= high`. Verified
      non-vacuous: it must fail against today's code.
- [ ] The fix holds for `base_currency=XLM` and for the peg path, each with a
      test.
- [ ] `low <= open,close <= high` passes for all 20 assets in the [[0120]]
      suite, at both granularities, against the deployed API.
- [ ] The choice between clamping and re-rounding is recorded with its
      reasoning, in ADR 0011 §3 if it changes what that section states.
