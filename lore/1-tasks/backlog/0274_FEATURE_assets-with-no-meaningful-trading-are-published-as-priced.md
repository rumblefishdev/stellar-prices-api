---
id: "0274"
title: "93% of dust-print candles belong to assets that have never traded in meaningful size — we publish a price for a token with no market"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0116", "0147", "0252", "0236"]
tags: [layer-backend, layer-api, priority-medium, effort-medium, milestone-M3, data-quality, liquidity, api]
milestone: 3
links:
  - "../../../packages/prices-api/src/assets/dto.rs"
history:
  - date: 2026-09-10
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0116]]. 0116 set out to find a per-candle rule for absurd
      `close_usd`; measuring it found that the per-candle framing fits only a
      small minority of the population, and that the majority case is an
      asset-level property. Filed rather than absorbed into 0116, because it is
      a different claim about a different subject — 0116 is about a bucket, this
      is about an asset.
---

# Assets with no meaningful trading are published as priced

## Summary

Measuring [[0116]] on prod found that **93% of dust-print candles (2,741 of
2,944 in `1h` 202608) belong to assets with no non-dust trading anywhere in the
month** — not against any quote, not from any source. 1,969 of those carry a
`close_usd` above $1,000.

For those assets there is no reference price to check a candle against, because
the asset has never traded in a size that would establish one. The accurate
statement is not "this candle is wrong" but **"this asset has no market, and we
publish a price for it anyway."**

## Context

[[0116]] documented the per-candle defect and deliberately did not build a
detector, because a size threshold misclassifies a third of the buckets it
catches (a smallest-unit trade of a genuinely expensive asset is a real order at
the right price). The two-stage check that *is* sound — compare a dust candle to
the same pair's non-dust price — can only be applied to the 7% of dust candles
that have such a reference.

This task owns the other 93%.

## Why it is not just a bigger 0116

The subject differs. 0116 asks "is this bucket's close a market price?" — a
per-row question with a per-row answer. This asks "does this asset have a market
at all?" — a property of the asset over a window, which is where it can actually
be answered, and where a consumer would want it surfaced (the asset listing and
the price endpoints, not each candle).

It is close kin to [[0147]] (`price_usd_series` volume-coverage gate) and
[[0252]] (the volume figure overclaims what it measures, 415 issuers publishing
`BTC`). All three are the same underlying gap: **nothing on the wire distinguishes
a real market from a nominal one.** Whether this should be merged into one of
them or stay separate is the first thing to settle.

## Implementation

- Decide the home: fold into [[0147]] / [[0252]], or keep standalone. Settle
  before building.
- Define "meaningful trading" from the measured distribution, not a guess —
  reuse 0116's method (base amount in minimal units, plus trade count), and
  validate against assets that are genuinely thin but real.
- Surface it where a consumer chooses an asset, not per candle: a liquidity or
  coverage signal on the asset listing and price endpoints.
- Re-measure first. 0116's figures are `1h` 202608 and 202502; the traded
  population swings ~25% a day, so the 93% needs a fresh number and a date.

## Acceptance Criteria

- [ ] The 93% figure is re-measured, dated, and confirmed across more than one
      month and granularity.
- [ ] A decision is recorded on whether this merges into [[0147]] / [[0252]] or
      stays its own task, with the reason.
- [ ] "Meaningful trading" is defined from measurement, and validated against a
      sample of thin-but-real assets so they are not swept up — the same
      false-positive discipline [[0116]] applied.
- [ ] A consumer can tell, without running their own aggregation, whether an
      asset's published price rests on a real market.
- [ ] `volume_quote_usd` and volume aggregates are explicitly unchanged.

## Notes

- Do **not** re-propose a bare size threshold on candles. [[0116]] measured it:
  of dust buckets with a non-dust reference for the same pair, a third are
  priced correctly. That result is recorded in `Candle::volume_base`'s doc
  comment so it is not rediscovered from scratch.
- Distinct from [[0236]] (no detector for internally inconsistent OHLCV rows),
  which is about a row contradicting itself, not about the asset's liquidity.
