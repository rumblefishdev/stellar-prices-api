---
id: "0115"
title: "Two-hop USD pivot for exotic quotes — 55% of trades are served with no USD value"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0114", "0026", "0061", "0107"]
tags: [clickhouse, enrichment, coverage, usd, priority-medium, effort-medium]
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-07-21
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0114]]'s per-quote breakdown. Enrichment reaches 99.7% of
      stablecoin- and XLM-quoted candles but 0% of everything else, which is
      55.2% of trades and ~62% of candles. Currently by design
      (ch_enrich.rs:499-504 treats exotic quotes as unreachable).
---

# Two-hop USD pivot for exotic quotes

## Summary

Enrichment prices a candle in USD only when its **quote** asset is a stablecoin
(direct) or XLM (one pivot hop). Everything else is left at `close_usd = 0` /
`volume_quote_usd = 0` — by design, not by bug (`ch_enrich.rs:499-504`: "the
leftovers have no USD reference at all — their quote is exotic").

That population is not marginal. Measured 2026-07-14 → 07-21:

| quote class | trades | share of trades | candles | USD-enriched |
|---|---|---|---|---|
| stablecoin (USDC/USDT) | 727,833 | 15.9% | 344,150 | 99.7% |
| XLM pivot | 1,322,738 | 28.9% | 660,181 | 99.7% |
| **other (exotic)** | **2,530,298** | **55.2%** | **1,619,310** | **0%** |

**A majority of the trades we serve carry no USD value at all.** A consumer
querying USD volume for an exotic pair gets `0` — which under the current
contract means "no reference", but reads as "no activity".

## The idea

Most Stellar assets trade against XLM somewhere, even when a given *pair* is
quoted in something exotic. So a second hop should be reachable:

```
exotic quote  →  XLM  →  USD
```

We already compute the XLM→USD leg (it is the existing pivot tier). What is
missing is pricing the exotic quote asset itself in XLM, from its own XLM pair,
and then chaining.

## Open questions — answer before building

- **How much of the 55% is actually reachable?** Some exotic quotes will have no
  XLM pair either. Measure the reachable fraction first; this task is only worth
  its cost if it recovers a large share. **This is the first AC.**
- **What error does a two-hop price carry?** Each hop adds spread and staleness.
  A two-hop USD value is materially less trustworthy than a direct stablecoin
  quote, and the API currently exposes no way to say so.
- **Should two-hop values be distinguishable from direct ones?** Strong argument
  yes — a confidence/derivation column, or a separate field. Silently mixing a
  1:1 stablecoin price with a two-hop estimate under one `close_usd` is how a
  consumer builds something wrong. Note `close_usd = 0` is currently a *precise*
  sentinel ("we do not know"); replacing it with a low-quality estimate trades a
  known-unknown for an unmarked guess. That trade needs a deliberate decision,
  not a default.
- **Depeg handling** — the existing peg-pivot tier is depeg-aware
  (`ch_enrich.rs:33-34`). A two-hop chain must not quietly lose that.

## Acceptance Criteria

- [ ] **Reachability measured first:** what fraction of the 55% has a usable XLM
      pair? Report as a share of trades, not candles. Task proceeds only if the
      recovered share justifies the cost.
- [ ] Two-hop pivot implemented as an additional tier, after the existing ones,
      never overwriting a direct or single-hop value.
- [ ] Two-hop-derived values are **distinguishable** from directly-quoted ones in
      the served data (decision recorded in an ADR if the shape changes).
- [ ] Depeg-awareness preserved through the chain.
- [ ] Spot-checked against an independent price source for a sample of exotic
      pairs — a two-hop price that is merely *present* is not the goal.

## Out of scope

- The historical USD hole in the coarse tables — that is [[0114]].
- [[0107]]'s trade-count gap. **Explicitly unrelated:** 0107 is about trades
  absent from our database; this is about trades we hold but cannot price. That
  link was tested and refuted 2026-07-21 (see 0107 §Ruled out).

## Notes

- Priority is **medium, deliberately**. This is a known design boundary with a
  correct sentinel, not an active corruption — unlike 0114, which is writing
  wrong values today. Sequence it behind 0114's repair.
- All figures measured against prod CH 2026-07-21, trade-weighted, current-week
  window, excluding 3 h of normal enrichment lag.
