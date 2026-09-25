---
id: "0274"
title: "1-stroop offer-priced fills still publish a USD price — e.g. XAUa at $4,375 on $0.003 of volume"
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
  - date: 2026-09-25
    status: backlog
    who: okarcz
    note: >
      NARROWED on the operator's 2026-09-25 decision, from the 2026-09-24 read-only
      check. The original premise is mostly fixed by [[0286]]'s work: live
      dust-only assets are now published as `unpriced` (65 of 65 checked). Still
      open: 1-stroop fills priced at the resting offer's price still publish a
      price, e.g. XAUa at $4,375 on $0.003 of volume. Narrowed to that. Asset
      103120 `USD` ($0.10 published vs a 0.5 XLM close) is out of scope here; it
      goes to its own new task. Retitled, because the old 93% headline describes
      a population that is now `unpriced`. Original text kept below.
---

# 1-stroop offer-priced fills still publish a USD price

## Summary (narrowed 2026-09-25)

**The original premise is mostly fixed.** After [[0286]]'s work, live assets
whose only trading is dust are published with `price_status = "unpriced"`
(65 of 65 checked, 2026-09-24).

**What still publishes:** an asset whose recent fills are 1-stroop trades priced
at a resting offer's price. The candle close is the offer's limit price, not a
market-clearing price, and the asset is published as priced. Example: **XAUa at
$4,375 on $0.003 of volume.**

[[0116]]'s caution still applies: a smallest-unit trade of a genuinely
expensive asset is a real order at the right price. So this cannot be a bare size
threshold. The fix has to tell "a real market that happens to be thin" apart
from "one offer being hit for a stroop".

Out of scope: asset 103120 `USD` ($0.10 published while a 0.5 XLM close implies
about $0.20). That is a different defect and has its own task.

## Acceptance Criteria

- [x] ~~Dust-only assets are not published as priced.~~ **Met via [[0286]]:**
      live dust-only assets are `unpriced`, 65 of 65 checked on 2026-09-24.
- [ ] The remaining population (assets published as priced whose price rests on
      1-stroop, offer-priced fills) is measured and dated, with XAUa as the
      reference case.
- [ ] A rule separates them from thin-but-real assets, validated against a sample
      of those so they are not swept up. This is the same false-positive
      discipline [[0116]] applied, and it is not a bare size threshold.
- [ ] A consumer can tell, without running their own aggregation, that such a
      price does not rest on a real market (e.g. it is `unpriced`, or carries a
      signal that says so).
- [ ] `volume_quote_usd` and volume aggregates are explicitly unchanged.

## Original scope (before 2026-09-25 narrowing)

> Kept verbatim for the record (headings demoted one level). Original title: *Assets with no meaningful trading are published as priced*. The Summary and Acceptance Criteria above supersede it.

### Summary

Measuring [[0116]] on prod found that **93% of dust-print candles (2,741 of
2,944 in `1h` 202608) belong to assets with no non-dust trading anywhere in the
month** — not against any quote, not from any source. 1,969 of those carry a
`close_usd` above $1,000.

For those assets there is no reference price to check a candle against, because
the asset has never traded in a size that would establish one. The accurate
statement is not "this candle is wrong" but **"this asset has no market, and we
publish a price for it anyway."**

### Context

[[0116]] documented the per-candle defect and deliberately did not build a
detector, because a size threshold misclassifies a third of the buckets it
catches (a smallest-unit trade of a genuinely expensive asset is a real order at
the right price). The two-stage check that *is* sound — compare a dust candle to
the same pair's non-dust price — can only be applied to the 7% of dust candles
that have such a reference.

This task owns the other 93%.

### Why it is not just a bigger 0116

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

### Implementation

- Decide the home: fold into [[0147]] / [[0252]], or keep standalone. Settle
  before building.
- Define "meaningful trading" from the measured distribution, not a guess —
  reuse 0116's method (base amount in minimal units, plus trade count), and
  validate against assets that are genuinely thin but real.
- Surface it where a consumer chooses an asset, not per candle: a liquidity or
  coverage signal on the asset listing and price endpoints.
- Re-measure first. 0116's figures are `1h` 202608 and 202502; the traded
  population swings ~25% a day, so the 93% needs a fresh number and a date.

### Acceptance Criteria

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

### Notes

- Do **not** re-propose a bare size threshold on candles. [[0116]] measured it:
  of dust buckets with a non-dust reference for the same pair, a third are
  priced correctly. That result is recorded in `Candle::volume_base`'s doc
  comment so it is not rediscovered from scratch.
- Distinct from [[0236]] (no detector for internally inconsistent OHLCV rows),
  which is about a row contradicting itself, not about the asset's liquidity.
