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

## 🔑 Root cause CORRECTED 2026-08-28 — float64 precision, not 14-decimal rounding

The filing said the extremes are *"multiplied by the bucket's single rate and
rounded to 14 decimals"*, and read the crossing as a rounding artefact. **The
rounding is real but is three orders of magnitude too small to explain it.**

Measured against the prod values in the summary:

| quantity | value |
|---|---|
| observed gap (`low − close`) | **1.343e-11** |
| one float64 ulp at 76,943.5 | 1.455e-11 |
| **gap ÷ ulp** | **0.92 — inside a single ulp** |
| half-tick of 14-decimal rounding | 5e-15 |
| **gap ÷ half-tick** | **2,686×** |

The derivation runs through `toFloat64` (`queries_ch.rs`: `toFloat64(low) * rate`),
and a 53-bit mantissa carries ~15-16 significant digits. A five-figure price at
`Decimal(38, 14)` carries **19**. So the float product lands up to one ulp from
the true value, and at BTC scale one ulp is ~1.5e-11 — exactly the size of the
crossing.

### ⚠️ This kills option 2 outright — it is a no-op, not a trade-off

The plan offered *"round `close` to the same 14-decimal scale as the derived
fields"* as the alternative to clamping, at the cost of discarding precision.
**There is no cost because there is no effect.** `close_usd` is `Decimal(38, 14)`
— `close` is *already* rendered at exactly 14 decimals
(`76943.51350417596657` has 14 places after the point). Rounding it to 14
decimals returns it unchanged, and the crossing survives.

🔑 Worth recording rather than quietly dropping: the option looked like the
conservative choice and would have shipped as a fix that changes nothing. The
decision between the two was never a matter of taste — one of them does not work.

### The third option, considered and not taken

Doing the derivation in Decimal arithmetic instead of float would fix the cause
rather than the symptom, and would satisfy the invariant on its own: with exact
arithmetic `low * (close_usd/close) <= close * (close_usd/close) = close_usd`.
Not taken here — `rate` is itself a division needing a chosen scale, ClickHouse
decimal multiplication grows scale and can overflow `Decimal128`, and the
practical error float introduces is ~1.8e-16 *relative*, which is meaningless for
a price. The invariant was the only real harm, and clamping addresses it in one
expression. ⚠️ If the derived values themselves ever need to be exact — not just
ordered — this is the change to make, and it is a larger one.

## Implementation — shipped 2026-08-28

`queries_ch.rs`, `Denomination::Usd` arm only:

```
h = greatest(maxIf(h_x, valid), CLOSE_EXACT)
l = least(   minIf(l_x, valid), CLOSE_EXACT)
c =                             CLOSE_EXACT
```

`CLOSE_EXACT` is a named constant because three output columns must agree on it,
and it carries the ulp measurement above so the next reader does not re-derive it.

- **`open` needs no clamp, and this is not an oversight.** `o_x`, `h_x` and `l_x`
  are scaled by the *same* `rate`; multiplication by one positive factor is
  monotonic, and so is rounding to a fixed scale. So `l_x <= o_x <= h_x` holds
  within a row, and `min`/`max` across rows only widens the bracket. The
  exact/derived boundary is the only place the ordering can break.
- **The other two paths are structurally incapable of the defect**, and both are
  now pinned anyway. `Denomination::QuoteLeg` emits the stored columns with no
  rate applied — nothing derived, nothing on a second scale. `ohlcv_peg_series`
  emits `o AS h, o AS l, o AS c`: one value in four fields, so the invariant is
  satisfied by identity. Tests exist to catch a *future* derivation being added
  there, not a present bug.

### The tests, and why they could not use the existing helper

🔴 **`approx()` — the file's own comparison helper — tolerates 1e-6 and routes
through `f64`.** The violation is ~1e-12 at five-figure prices, which f64 cannot
even represent there. A test built on `approx` would have passed against the bug
and read as proof. `rust_decimal` was added as a **dev-dependency** and
`assert_ohlc_ordered` compares exactly; the reasoning is recorded in
`Cargo.toml` beside the dependency, because a stray-looking dev-dep is exactly
what a later cleanup removes.

**Non-vacuity is verified, not asserted.** With the fix stashed, both crossing
tests fail — and they fail at the *predicted* magnitudes, which is the stronger
result:

| test | predicted gap | observed on failure |
|---|---|---|
| derived `low` above exact `close` | 1.24e-12 | **1.24e-12** |
| derived `high` below exact `close` | 6.57e-12 | **6.57e-12** |

The seeds were found by simulating ClickHouse's own arithmetic
(`toFloat64` → `toString` shortest-roundtrip → `toDecimal128OrNull(…, 14)`) in
`Decimal`, then confirmed against a live 26.3.10.60. That the two agree to the
digit is what establishes the mechanism, rather than merely fitting it.

⚠️ **Both directions are covered on purpose.** A clamp on `low` alone passes the
first test and leaves the mirror defect live — a fix that looks complete and is
not.

## Review findings — 2026-08-28

Three findings on PR #263. All three verified independently before acting; two
are addressed here, one is spawned and one claim in the review is corrected.

### 🔴 Finding 2 — ACCEPTED and FIXED: `least`/`greatest` swallow NULL

**This was a regression introduced by the clamp, and it is the important one.**

Verified on 26.3.10.60:

```
greatest(CAST(NULL AS Nullable(Decimal(38,14))), 2.5)  ->  2.5
NULL + 2.5                                             ->  NULL
```

🔑 **`least`/`greatest` IGNORE null arguments rather than propagating them** —
the opposite of ClickHouse's usual behaviour, and of every other expression in
this query. `h_x`/`l_x` are `toDecimal128OrNull` and go NULL on
`Decimal128(38, 14)` overflow, which is reachable: `rate = close_usd / close` is
unbounded above, so a dust `close` at `PRECISION_FLOOR` against a ten-figure
`close_usd` gives `rate = 1e22` and `high * rate = 1e25`. Confirmed by direct
query.

So an extreme the query **could not compute** would have been reported as the
close — a value asserted where the honest answer is absent. Fixed with `isNull`
guards, documented at `CLOSE_EXACT`, and pinned by
`ohlcv_an_unrepresentable_extreme_stays_null_rather_than_becoming_the_close`.
**Non-vacuity verified**: with the guards removed the test fails.

### 🟡 Finding 3 — ACCEPTED as real, NOT fixed here → [[0236]]

The clamp is unbounded, so a genuinely corrupt source row (`high < close`) is
silently rewritten into a well-formed candle. The review is right that this
removes the only thing that was detecting such rows — [[0120]]'s conformance
assertion, which is how 0229 was found at all.

**Not addressed by bounding the clamp.** Bounding buys the signal by serving a
malformed candle to every consumer whenever a source row is corrupt, which pays
in the wrong currency: a read API's job is to return well-formed candles, and a
consumer can do nothing with `high < close` except break.

🔑 The actual defect is that source-row consistency was only ever checked *by
accident*, downstream, by a suite run by hand against a deployed API on 20 assets
over a 7-day window. That belongs at the source. Spawned as [[0236]], which
measures the baseline before designing any alarm.

### ⚠️ Finding 1 — REAL and reproduced, but its stated consequence is wrong

`vwap` is outside the clamp and escapes `[low, high]` by the same mechanism, and
worse: it carries a *second* float round-trip
(`sumIf(toFloat64(w_x) * toFloat64(volume_base)) / sumIf(toFloat64(volume_base))`),
and `(x*v)/v != x` in IEEE754. Reproduced through this file's literal expressions
on 26.3.10.60 over 300,000 single-trade candles at BTC scale: **26,395 rows with
`vwap > high` and 26,387 with `vwap < low`** — ~8.8% each, far above the review's
own 523/511 and far above the OHLC crossing rate this task was filed for.

🔴 **But the review's consequence — *"it will be the next thing a conformance
check finds"* — is false, and it matters.** [[0120]]'s assertion is
`l <= min(o, cl) && max(o, cl) <= h` (`conformance-0120.mjs:451`); `vwap` appears
only in the "all OHLCV values are decimal strings" check. Nothing asserts vwap
in-band, so this will **not** surface on its own. Recorded because a finding that
overstates its own detectability argues for deferring it, when the truth argues
the opposite.

### ✅ Finding 1 — FIXED 2026-08-28 on an explicit call, and it was worse than measured

Folded in after the decision. `vwap` is now clamped into the **published**
`[low, high]` — the values the caller sees, not the raw aggregates — so the
response is self-consistent. A volume-weighted mean of prices inside a bucket
must lie inside that bucket's range, so this restates what vwap *is* rather than
correcting it.

#### 🔴 The as-stored path has it too, and a one-row probe said otherwise

`Denomination::QuoteLeg` applies no rate, so `o`/`h`/`l`/`c` are stored decimals
and genuinely cannot cross — the earlier "structurally incapable" note holds for
*those*. It does not hold for `vw`, which is a float weighted mean on every path.

⚠️ **The first measurement of that arm was a FALSE NEGATIVE and nearly closed the
question.** One source per bucket: **0 violations in 200,000**. Two sources at
equal prices — the boundary case, and the case the aggregate exists for:
**12,017 above `high` and 12,026 below `low` in 200,000 buckets**.

🔑 A one-row probe of a MERGE aggregate tests a path production does not have.
The same mistake then repeated in the test itself: the first XLM seed passed
against the unclamped query, i.e. it was vacuous, and only a search over that
arm's own expression produced a seed that actually crosses (2.048e-11 below the
low). Recorded because both errors had the same shape — a clean result from a
probe that could not have been dirty.

#### Non-vacuity, all three

| test | fails unclamped, gap |
|---|---|
| `ohlcv_vwap_cannot_round_above_the_high` | 1.0e-11 |
| `ohlcv_vwap_cannot_round_below_the_low` | 1.0e-11 |
| `ohlcv_xlm_merged_vwap_stays_inside_the_band` | 2.048e-11 |

`assert_ohlc_ordered` now checks vwap on **every** candle it is applied to, so all
26 integration tests carry the bound rather than only the three that target it.
It skips the zero sentinel — `0` there means "no weighted mean", not "a vwap of
zero", and the as-stored arm's `isNull` branch preserves that rather than
clamping a missing value up to `low`.

## Acceptance Criteria

- [x] A test seeds a bucket whose derived `low` rounds above its exact `close`
      and asserts the response satisfies `low <= open,close <= high`. Verified
      non-vacuous: it must fail against today's code.
      → `ohlcv_derived_low_cannot_round_above_the_exact_close`, plus the mirror
      `ohlcv_derived_high_cannot_round_below_the_exact_close` — a clamp on one
      side only would pass the first and leave the second live.
      **Non-vacuity verified by running them against the stashed fix**: both
      fail, at the magnitudes the ClickHouse-arithmetic simulation predicted
      (1.24e-12 and 6.57e-12, to the digit). ⚠️ They compare with `rust_decimal`,
      not the file's `approx` helper — at 1e-6 tolerance through `f64` the
      assertion would have passed against the bug.
- [x] The fix holds for `base_currency=XLM` and for the peg path, each with a
      test.
      → `ohlcv_xlm_denomination_keeps_ohlc_ordered` and
      `ohlcv_peg_series_keeps_ohlc_ordered`. ⚠️ **Both paths are structurally
      incapable of the defect** and the tests say so in their own doc comments:
      `QuoteLeg` applies no rate, and the peg series emits one value into four
      fields. They pin the property against a future derivation being added
      there; neither was failing.
- [ ] `low <= open,close <= high` passes for all 20 assets in the [[0120]]
      suite, at both granularities, against the deployed API.
      ⏳ **Blocked on a deploy — the API Lambda has not shipped this yet.** The
      pre-fix baseline is recorded: 9 failures of this check in the 2026-08-27
      run (one bucket each on BTC, sUSD, XRP over a 7-day 1h window). Re-run
      after deploy and expect 9 → 0; the other 27−9 failures are [[0230]] and
      [[0178]] and must not be read as this fix falling short.
- [x] The choice between clamping and re-rounding is recorded with its
      reasoning, in ADR 0011 §3 if it changes what that section states.
      → **[[ADR-0011]] §3 amended 2026-08-28.** It did change what §3 states —
      not by contradicting it, but by adding a guarantee a consumer can observe
      (an extreme may now be exactly equal to `close`). ⚠️ **The recorded
      reasoning is that the choice was never a trade-off: re-rounding is a
      no-op**, because `close` is already at 14 decimals and the crossing is
      float-ulp sized, ~2,700× larger than a 14-decimal half-tick.
