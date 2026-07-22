---
id: "0107"
title: "Reconcile SDEX order-book volume vs Horizon (trade-type split) + classic LP coverage decision"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0026", "0096", "0099"]
tags: [layer-indexing, priority-medium, effort-medium, sdex, horizon, volume, investigation, amm, liquidity-pool]
links:
  - "../active/0026_FEATURE_volume-quote-usd-enrichment-impl/README.md"
history:
  - date: 2026-07-20
    status: backlog
    who: okarcz
    note: >
      Spawned from 0026's Horizon volume-credibility check. The check found our
      SDEX **prices** match Horizon to <0.2%, but our **volume** is ~1/6 of
      Horizon's all-trade-types aggregate for the same XLM-quoted pairs — a real
      gap to explain, not a query artifact (both-direction summing was ruled out).
---

# Reconcile SDEX order-book volume vs Horizon + classic-LP coverage decision

## Summary

Our `sdex` source prices are correct (verified vs Horizon), but our reported
**volume** is materially below Horizon's aggregate for the same pairs. Determine
whether the gap is (a) **classic protocol-18 liquidity-pool** trades we don't
capture (a scope decision), or (b) **order-book trades from path-payment
operations** we miss (a real extractor bug), then act accordingly.

## ❌ Ruled out: the USD-enrichment gap ([[0114]]) does NOT explain this

Tested 2026-07-21 and refuted. Recorded because it is the cheapest available
explanation and will suggest itself again to anyone who reads 0114's headline
figure (55–62% of candles carry no USD value).

Three independent reasons, strongest first:

1. **This task's numbers are denominated in XLM, not USD** — see the table below
   ("our XLM vol" vs "Horizon XLM vol"). USD enrichment cannot move a figure that
   is never expressed in USD.
2. **The affected pairs are the ones we price best.** 0107 measures XLM-quoted
   pairs; those are 99.7% USD-enriched on live data. The unpriced population is
   exotic quotes, which this task does not measure.
3. **Wrong magnitude anyway.** Trade-weighted, the USD-reachable share is 44.8%
   (2026-07-14 → 07-21), implying a ~2.2× understatement against the 6× volume /
   7.5–9× trade gap here. Off by 3–4×.

**Categorically:** 0107 is about trades **absent from our database**; 0114 is
about trades we **hold but cannot price**. Orthogonal. No USD-pricing work
creates a trade that was never ingested. The two hypotheses in §Summary —
classic-LP trades or path-payment order-book trades — remain the live ones.

## Context

From the 2026-07-20 0026 Horizon check (fixed UTC day 2026-07-19), top
XLM-quoted SDEX pairs, our numbers vs Horizon `trade_aggregations` (all trade
types):

| pair | our OB trades | Horizon trades | our XLM vol | Horizon XLM vol | vol gap | trade gap |
|------|---------------|----------------|-------------|-----------------|---------|-----------|
| SHX  | 3,049  | 27,721 | 72,374 | 426,117 | 5.9× | 9.1× |
| XRP  | 2,985  | 22,306 | 45,996 | 288,816 | 6.3× | 7.5× |
| VELO | 2,952  | 23,721 | 33,796 | 182,327 | 5.4× | 8.0× |

- **Prices match to <0.2%** (our implied XLM/asset price vs Horizon `avg`), so
  extraction+pricing of the trades we DO capture is correct.
- Ratio is consistent *within* a pair (base and counter scale together) but
  *varies across* pairs (5.4–6.3×) → a volume-attribution / coverage difference,
  not a units bug.
- Direction-split ruled out: summing both `(X, XLM)` and `(XLM, X)` orientations
  equals the single-orientation sum exactly (no reverse series exists).
- Horizon `/trades` confirms **both `orderbook` and `liquidity_pool`** trades are
  live on these pairs. Horizon counts ~8× the trades but ~6× the volume → the
  uncaptured trades are on average smaller than ours (LP / path-payment micro-fills).
- Our AMM extractors cover **Soroban** venues (Soroswap/Aquarius/Phoenix) only;
  **classic protocol-18 liquidity pools appear to be captured by neither the
  `sdex` order-book path nor the Soroban-AMM path** — a likely coverage gap.

## Implementation

- Sum Horizon **order-book-only** volume for a few XLM pairs over a fixed UTC day
  (`/trades?trade_type=orderbook`, paged with cursor to the day boundary) and
  compare to our `sdex` volume for the same pairs/window.
  - If Horizon order-book-only ≈ ours → we capture the order book fully; the gap
    is classic-LP (+ path-payment-via-LP) volume → this becomes a **scope
    decision** (do we want classic-LP coverage? if yes, spawn an extractor task).
  - If Horizon order-book-only ≫ ours → we are **missing order-book trades**
    (likely path-payment-induced crossings) → a real `sdex` extractor bug; fix it
    (sibling of 0096/0099 "missed a trade shape/source").
- Document the classic protocol-18 liquidity-pool coverage decision (in scope or
  not) and, if in scope, spawn the classic-LP extractor task.

## Acceptance Criteria

- [ ] Horizon order-book-only volume reconciled against our `sdex` volume for ≥3
      XLM-quoted pairs over one fixed UTC day; match (or gap) quantified.
- [ ] Root cause classified: classic-LP coverage gap vs missed order-book
      (path-payment) trades.
- [ ] If a `sdex` order-book undercount is found → fix task spawned (or fixed here).
- [ ] Classic protocol-18 liquidity-pool coverage decision recorded (in/out of
      scope); if in scope, follow-up extractor task spawned.
- [ ] 0026's Horizon volume-credibility AC re-evaluated with the like-for-like
      (order-book-only) comparison.
