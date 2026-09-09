---
id: "0266"
title: "Historical USD closes dislocate ~25% on USDC-stress dates — two unrelated assets move by an identical ratio, so the cause is shared, not per-market"
type: BUG
status: backlog
related_adr: ["0011"]
related_tasks: ["0127", "0265", "0172", "0197", "0128"]
tags: [layer-backend, priority-medium, effort-medium, milestone-M3, pricing, enrichment, data-correctness, stablecoin]
milestone: 3
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-09-04
    status: backlog
    who: okarcz
    note: >
      Found by [[0127]]'s AC 4 spot-check, comparing our daily closes against an
      independent exchange across 28 dates. 27 of 28 match to a fraction of a
      percent. The exception is not noise — it is two unrelated assets moving by
      the same ratio on the same day, which is a shared-cause signature.
---

# USD closes dislocate on stablecoin-stress dates

## Summary

Our daily closes are **excellent** against an independent major exchange
(Binance `XLMUSDT` / `BTCUSDT` daily closes, UTC-aligned):

| | median \|Δ\| | within 5% | n |
|---|---|---|---|
| XLM (`native`) | **0.06%** | 27/28 | 28 |
| yBTC | **0.48%** | 27/28 | 28 |

Sampled on the 15th of every other month, 2022-01 → 2026-07. **That is the
headline and it should not be lost**: the pipeline reproduces an independent
market to a fraction of a percent across four and a half years.

🔴 **The exception is systematic.** On a small number of dates both assets are
dislocated **by the same ratio**:

| date | XLM ours ÷ reference | yBTC ours ÷ reference |
|---|---|---|
| 2023-03-11 | **0.746** | **0.744** |
| 2023-03-15 | **0.838** | **0.852** |

Two assets with unrelated markets, unrelated liquidity and (per the API's own
`method` field) *different USD derivations* cannot drift by the same 25.4% and
25.6% by coincidence. **Something they share is wrong on those days.**

Every neighbouring day is clean — the surrounding window runs 0.999-1.017:

```
2023-03-09  XLM 1.000   yBTC 1.007
2023-03-10  XLM 1.001   yBTC 1.000
2023-03-11  XLM 0.746   yBTC 0.744   <<<
2023-03-12  XLM 1.017   yBTC 1.008
2023-03-13  XLM 1.003   yBTC 1.005
2023-03-14  XLM 1.003   yBTC 1.001
2023-03-15  XLM 0.838   yBTC 0.852   <<<
2023-03-16  XLM 1.002   yBTC 0.995
```

## Context — what the dates are

**2023-03-11 is the Silicon Valley Bank weekend**, when USDC broke its peg to
~$0.87-0.88. **2023-03-15 is the Credit Suisse panic**, the next stablecoin
stress event in the same fortnight. Both dislocated dates are USDC-stress
dates; none of the clean dates are.

⚠️ **The mechanism is NOT established and must not be asserted.** The obvious
story — "we assume USDC = \$1, USDC really fell, so our prices are wrong" — does
not survive arithmetic. If `ours = onchain_in_USDC × 1` and
`true = onchain_in_USDC × true_USDC`, then `ours ÷ true = 1 / true_USDC`; the
observed 0.745 implies `true_USDC ≈ $1.34`, and USDC **fell** rather than rose.
So the direction is wrong for the simple peg explanation. Whatever the shared
factor is, it is not that.

Candidates to test, in order:

- A shared **pivot** leg (both assets converting through the same intermediate)
  whose own USD rate was dislocated on those days.
- The **close** being the last trade in the bucket on an illiquid venue during
  a panic, where the last on-chain trade sat far from the global market. ⚠️ This
  explains *one* asset moving, not two moving by the same ratio, unless they
  share the leg.
- A USD reference (`usd_rate`, oracle, or the peg placeholder) that was itself
  sourced from a dislocated market for those days.

## A separate, different defect found alongside

**yBTC on 2023-03-18: ratio 0.434** (57% low) while **XLM the same day is
1.002** — clean. That one is *not* shared: it is a single-asset artefact of thin
liquidity. yBTC trades a few hundred times a day against XLM's tens of
thousands, so one off-market trade landing last in the bucket destroys the
close. Worth its own consideration: a close on a thin market is a weak
statistic, and nothing currently flags it.

## Implementation

- Reproduce both dislocated dates from the raw candles, not the API, and
  identify the shared factor. Start by asking what leg XLM and yBTC have in
  common on those days.
- Establish whether the on-chain trades themselves were dislocated (a real,
  if extreme, market event we are correctly reporting) or whether the
  **conversion** was. 🔑 These have opposite conclusions: the first means our
  data is right and the reference is the wrong comparison; the second is a bug.
- Sweep the full history for the signature rather than the dates: days where
  many unrelated assets move by a common ratio against their own trend. That
  is cheap to detect and would find any other event of this class.
- Decide what, if anything, to publish for such days. A dislocated close that
  really happened on-chain is arguably correct and should stay; a conversion
  artefact should not.

## Acceptance Criteria

- [ ] The shared factor behind 2023-03-11 and 2023-03-15 is identified and
      named — conversion artefact or genuine on-chain dislocation, said plainly.
- [ ] The full history is swept for the same signature and every occurrence is
      listed, not just the two found by a 28-date sample.
- [ ] If it is a conversion artefact, it is corrected and the spot-check
      re-run — those dates should then land inside the same fraction-of-a-
      percent band as the other 27.
- [ ] The thin-market close problem (yBTC 2023-03-18) is either fixed or
      documented as a known property of low-liquidity assets.
- [ ] [[0128]] states the accuracy claim with its exception, rather than
      quoting the median alone.

## Notes

- 🔑 **Do not let this finding bury the good news.** 27 of 28 dates on two
  assets match an independent exchange to a median of **0.06%** and **0.48%**.
  The pipeline is right. This is a narrow, dated defect on a handful of days.
- ⚠️ Related but distinct from [[0265]]: that task is about USDC's *own* price
  being asserted rather than measured. This one is about *other* assets' prices
  going wrong on the days USDC was under stress. They may share a root cause —
  or may not — and neither should be closed by assuming the other.
- The reference used was Binance daily klines via `data-api.binance.vision`, a
  public endpoint needing no key, with history back beyond 2022. CoinGecko's
  free tier caps historical queries at 365 days and cannot serve this check.
