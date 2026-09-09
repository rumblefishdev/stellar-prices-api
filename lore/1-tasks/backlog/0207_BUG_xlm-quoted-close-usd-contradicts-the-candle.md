---
id: "0207"
title: "218 XLM-quoted candles carry a close_usd the candle itself cannot justify — one is 5.1M× its quote-leg price"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0172", "0168", "0145", "0196", "0173"]
tags: ["priority-medium", "effort-small", "clickhouse", "data-correctness", "enrichment", "oracle"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-18
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0172]]'s sweep criterion, which asked whether the USDT peg
      defect was a class or a one-off. It was a one-off — only USDC shows the
      peg fingerprint and it is legitimately $1 — but the same sweep surfaced
      this, which is a different defect wearing the same symptom. Filed
      separately rather than widening 0172, whose scope is the peg class.
---

# `close_usd` disagrees with the candle on 218 XLM-quoted rows

## Measured on prod 2026-08-18 (`price_ohlcv_1d`)

```
XLM-quoted candles with close > 0 AND close_usd > 0 : 11,012,703
  implied rate (close_usd / close), mean              0.656471
  implied rate, stddev                             1551.634408   <- the tell
  p999                                                0.548       <- a real XLM price
  rows with implied rate > 1                            218
  worst                                         5,149,014.49
```

**The bulk is sound.** `p999 = 0.548` is a plausible XLM/USD price, so 99.998% of
these rows carry a properly measured rate. The mean and the standard deviation
are both artefacts of the tail.

**The tail is not.** XLM never traded above ~$0.9 in this window, so an implied
rate above 1 means `close_usd` claims the base asset is worth more USD than its
own XLM-denominated close can support. At 5.1M× it is off by six orders of
magnitude.

## Why this is not the [[0182]] dust class

The 18 rows 0182 left at `close_usd = 0` are dust — `close` at or below ~4e-14,
where `rate × close` underflows at `Decimal(38, 14)`. That produces a **zero**,
never an inflated value: rounding cannot manufacture magnitude.

A ratio of 5.1M needs a **real** `close_usd` sitting against a near-zero `close`.
So the USD value did not come from `rate × close` at all — it came from somewhere
that disagrees with the candle.

## The prime suspect: the oracle tier

`ch_enrich.rs` runs the oracle tier **first**, and it **wins where it applies**.
Unlike the peg-pivot tier it writes `close_usd` directly from a Reflector price
rather than deriving it from the candle's own quote leg, so the two are never
reconciled. A stale or mis-attributed Reflector price on an illiquid asset
produces exactly this shape.

That is the same failure mode [[0196]] already measured once — 46,378
mis-attributed Reflector rows on the USDT identity — and the same hazard [[0168]]
is filed against. ⚠️ [[0173]] records that `usd_rate`/`oracle_prices` file real
Tether's $1 under our depegged issuer, so oracle↔asset-id attribution is a known
weak point rather than a hypothetical one.

**Not yet checked, and it is the first thing to check:** whether these 218 rows
carry `method = 'oracle'`. If they do, the diagnosis is confirmed and the
question becomes which assets and why. If they do not, the cause is elsewhere and
this task needs re-scoping before anything is written.

## Why it matters at 0.002%

A wrong-but-nonzero `close_usd` reads as a real price at the **~130 unguarded
`argMax(close_usd, …)` sites** ([[0145]]) — the same reason 0182 argued a
wrong-but-visible number is worse than an honest gap. 218 rows is small, but each
one is a maximum, and `argMax` is exactly the aggregate that a six-order-of-
magnitude outlier dominates. One such row can drive a whole asset's published
price.

⚠️ **Do not assume it is 218.** That count is `price_ohlcv_1d` only, and only for
`quote_asset_id = XLM`. The coarse tiers roll the same trades up and USDT is a
second pivot leg; both want measuring before the fix is scoped.

## Implementation

- Confirm the source: `method` on the 218 rows, and whether they cluster by
  `asset_id`, by date, or by source.
- Re-measure across all five forever-tables and both pivot legs (XLM and USDT),
  not just `_1d`/XLM.
- Decide the rule. An oracle price that contradicts the candle's own quote leg by
  orders of magnitude is not more authoritative for being an oracle — the tier
  ordering assumes it is. Either the oracle tier gains a sanity bound against the
  candle, or the attribution that lets a wrong price reach these rows is fixed at
  source.
- Whatever the rule, pin it with an IT on the 26.3.10.60 pin.

## Acceptance Criteria

- [ ] Source confirmed — `method` on the affected rows, and their clustering
- [ ] Population re-measured across all five forever-tables and both pivot legs,
      not extrapolated from `_1d`/XLM
- [ ] A rule decided and stated: what makes an oracle price untrustworthy
      relative to the candle it is pricing
- [ ] Existing rows corrected, or an explicit decision to leave them with the
      reasoning recorded
- [ ] Regression test on the 26.3.10.60 pin
