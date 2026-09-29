---
id: "0107"
title: "Reconcile SDEX order-book volume vs Horizon (trade-type split) + classic LP coverage decision"
type: RESEARCH
status: completed
related_adr: []
related_tasks: ["0026", "0096", "0099", "0286", "0282"]
tags: [layer-indexing, priority-medium, effort-medium, sdex, horizon, volume, investigation, amm, liquidity-pool]
links:
  - "../archive/0026_FEATURE_volume-quote-usd-enrichment-impl/README.md"
history:
  - date: 2026-07-20
    status: backlog
    who: okarcz
    note: >
      Spawned from 0026's Horizon volume-credibility check. The check found our
      SDEX **prices** match Horizon to <0.2%, but our **volume** is ~1/6 of
      Horizon's all-trade-types aggregate for the same XLM-quoted pairs — a real
      gap to explain, not a query artifact (both-direction summing was ruled out).
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      **Now carries a full acceptance criterion, not just a follow-up
      question.** [[0026]] was archived today and its last open AC —
      "`current_prices.volume_24h_usd` for ≥3 XLM-quoted assets is credible
      against Horizon's historical aggregates" — closed as a **deferral to
      this task**, explicitly NOT as a pass. Nothing about that AC can be
      satisfied by more enrichment work: the decoder is correct (prices match
      to <0.2%), and the gap is a **population** difference — Horizon
      aggregates order-book + classic protocol-18 LP + path-payment trades,
      our `sdex` decodes order-book offers only. So this task's real output is
      a DECISION on whether to ingest the other trade types, plus a
      like-for-like order-book-only reconciliation that would let the AC be
      judged fairly. Also updated the frontmatter link, which pointed into
      `active/` and broke when 0026 moved to `archive/`.
      ⚠️ Do NOT re-propose the USD-pricing-gap explanation: [[0114]]
      pre-registered a threshold, measured it at ~2.2× against this 6× gap,
      and REFUTED it. The two are orthogonal — 0107 is about trades absent
      from the database, 0114 about trades held but unpriceable.
  - date: 2026-09-25
    status: completed
    who: okarcz
    note: >
      Re-measured on 2026-09-20 (outside 0282's loss window) for SHX, XRP,
      VELO, AQUA and yXLM against XLM. Our sdex fills, base volume and XLM
      volume equal Horizon's all-trade-types aggregate exactly (1 of ~55k
      fills differs: a 0-XLM claim filter.rs skips on purpose). sdex has
      decoded classic liquidity-pool fills since da87008b (2026-06-24), so
      the "order-book only" premise was wrong. The 07-19 shortfall is in
      what was stored that day, and 0286 phase 3 rewrites it; 0286's check
      of the days written live between 2026-07-16 and 09-17 covers it.
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

- [x] Horizon order-book-only volume reconciled against our `sdex` volume for ≥3
      XLM-quoted pairs over one fixed UTC day; match (or gap) quantified. Done
      on 2026-09-20 for 5 pairs: ours = order-book + pool fills, and equal to
      Horizon's all-types aggregate (see Re-measurement).
- [x] Root cause classified: classic-LP coverage gap vs missed order-book
      (path-payment) trades. Neither holds on current data. The July gap is in
      what was stored for that day, not in decoding.
- [x] If a `sdex` order-book undercount is found → fix task spawned (or fixed here).
      None found on 2026-09-20; the 07-19 day is rewritten by [[0286]] phase 3.
- [x] Classic protocol-18 liquidity-pool coverage decision recorded (in/out of
      scope); if in scope, follow-up extractor task spawned. In scope and
      already built: `filter.rs` decodes `ClaimAtom::LiquidityPool` since
      `da87008b` (2026-06-24). No extractor task needed.
- [x] 0026's Horizon volume-credibility AC re-evaluated with the like-for-like
      (order-book-only) comparison. Ours ÷ order-book is 1.05–1.69×, explained
      exactly by pool fills; against all trade types the gap is zero.

## Re-measurement 2026-09-20 (run 2026-09-25)

**In short: on 2026-09-20 there is no volume gap.** For five XLM-quoted
pairs, our `sdex` trade count and base and counter volume equal Horizon's
all-trade-types daily aggregate to the stroop (one expected exception, below).
Horizon's order-book-only figure is *lower* than ours, because `sdex` already
ingests classic protocol-18 pool fills. So both hypotheses in §Summary are
wrong for current data. The ~6× gap measured on 2026-07-19 is still in the
stored data for that day, which points to a problem with that day's stored
candles, not a trade shape we fail to decode.

### Window and method

- UTC day 2026-09-20 = ledgers **64515325 → 64532604**. Checked on both sides:
  `default.ledgers` min/max `closed_at`, and Horizon `/ledgers/64515324` (23:59:58
  on 09-19) and `/ledgers/64532605` (00:00:03 on 09-21).
- Pairs: the three from §Context (SHX, XRP, VELO) plus AQUA and yXLM. Each is
  `(asset, XLM)` with XLM as quote, `asset_id` = 14 / 40 / 28 / 5 / 10,
  `quote_asset_id = 4`. There is no reverse-orientation series on that day. The
  query with `asset_id = 4` returned none of these five.
- The same statistic on both sides: fill count, Σ base amount, Σ counter (XLM)
  amount, for one market and one orientation over one day.

**Ours.** `_1m`, `_1h` and `_1d` give identical results for all five pairs. The
`_1m` table still holds 09-20: 24/24 hours and 1440/1440 minutes have `sdex`
rows network-wide. The `_1h` query:
```sql
SELECT asset_id, quote_asset_id, sum(trade_count), sum(volume_base), sum(volume_quote)
FROM prices.price_ohlcv_1h FINAL
WHERE source='sdex' AND timestamp>='2026-09-20 00:00:00' AND timestamp<'2026-09-21 00:00:00'
  AND asset_id IN (14,40,28,5,10) AND quote_asset_id=4
GROUP BY 1,2
```
**Horizon, order-book only.** `/trades?base_asset_type=credit_alphanum4&base_asset_code=X&base_asset_issuer=…&counter_asset_type=native&trade_type=orderbook&order=asc&limit=200&cursor=<64515325<<32>`.
Paging continues until `ledger_close_time >= 2026-09-21T00:00:00Z`, summing
`base_amount` and `counter_amount`. That took 231 pages in total.
**Horizon, all types.** `/trade_aggregations?…&start_time=1789862400000&end_time=1789948800000&resolution=86400000`.
**Horizon, pools only** (SHX and XRP, as a cross-check). `/trades?…&trade_type=liquidity_pool`, same paging.

### Results

| pair | our fills | our base vol | our XLM vol | Hz OB fills | Hz OB XLM vol | ours ÷ Hz-OB (XLM) | Hz all-types fills | Hz all-types XLM vol | Δ fills (ours − all) | Δ XLM vol | Δ base vol |
|------|----------:|-------------:|------------:|------------:|--------------:|---------:|------------:|------------:|---:|---:|---:|
| SHX  | 7,164  | 28,469,412.35 | 501,941.90 | 6,232  | 296,898.57 | 1.691× | 7,165  | 501,941.90 | −1 | 0 | −0.0000041 |
| XRP  | 7,275  | 20,514.03     | 146,622.38 | 6,864  | 139,782.09 | 1.049× | 7,275  | 146,622.38 | 0 | 0 | 0 |
| VELO | 15,458 | 11,562,506.29 | 272,703.80 | 13,948 | 184,141.35 | 1.481× | 15,458 | 272,703.80 | 0 | 0 | 0 |
| AQUA | 9,338  | 90,656,257.14 | 164,740.32 | 3,646  | 126,950.68 | 1.298× | 9,338  | 164,740.32 | 0 | 0 | 0 |
| yXLM | 16,230 | 288,761.80    | 278,722.54 | 14,876 | 175,809.68 | 1.585× | 16,230 | 278,722.54 | 0 | 0 | 0 |

- **Order-book plus pool fills add up exactly for both pairs checked.** XRP:
  6,864 order-book + 411 pool fills = 7,275, and 139,782.0880557 + 6,840.2931753
  = 146,622.3812310 XLM, which equals ours. SHX: 6,232 + 933 = 7,165, and the
  counter volume sums to 501,941.8983743 exactly.
- **The single SHX fill we lack is dropped on purpose.** Horizon trade
  `277159973392494593-1` (22:14:13) moved 0.0000041 SHX for **0.0000000 XLM**.
  `claim_to_raw_trade` skips any claim with a zero amount
  (`packages/prices-ingest-core/src/filter.rs`, "skipping claim with zero amount").
- **Why `sdex` is higher than Horizon's order-book-only count.**
  `extract_claims` reads `offers_claimed` from ManageSell/ManageBuy/
  CreatePassiveSell and `offers` from both PathPayment variants.
  `claim_to_raw_trade` handles `ClaimAtom::LiquidityPool` alongside `OrderBook`
  and `V0`. That code has been in place since `da87008b` (2026-06-24), before
  either measured day. The note from 2026-07-23 above says "`sdex` decodes
  order-book offers only". That statement is incorrect: pool fills are 5.6 %
  (XRP) to 61 % (AQUA) of our fills on these pairs.

### The original 2026-07-19 day, re-checked today

The same `_1h` query for 07-19 still returns the numbers in §Context:
SHX 3,049 / 72,373.94 XLM, XRP 2,985 / 45,995.60, VELO 2,952 / 33,795.64.
Horizon all-types for that day still shows 27,721 / 22,306 / 23,721 fills. The
gap is real **for that day's stored candles**. It is spread evenly across the
day: network-wide `sdex` fills are about 13k–31k per hour in all 24 hours of
07-19, against about 60k–115k per hour on 09-20, with no missing hours. The same
decoder gets 100 % on 09-20. So the loss is in what was written or kept for
July, not in extraction. **The cause was not investigated here.** A good
candidate is the family in the memory note "candle writes are REPLACED not
summed when a bucket spans a run". [[0286]] phase 3 re-ingests history
oldest-first and will rewrite 07-19. Re-checking this same query after it
passes July would confirm the cause without further work.

### Verdict against the Acceptance Criteria

- **AC 1 (≥3 pairs, one fixed UTC day, gap quantified): PASS.** Five pairs.
  Against Horizon order-book only, ours is 1.05–1.69× higher. Against Horizon
  all types, the match is exact (Δ ≤ 1 fill, 0 XLM).
- **AC 2 (root cause classified): neither hypothesis.** There is no missed
  order-book trade and no classic-pool coverage gap on current data. The
  07-19 gap is a stored-data defect for that period, not a missing trade shape.
- **AC 3 (fix task if order-book undercount): not triggered.** There is no
  order-book undercount.
- **AC 4 (classic-pool coverage decision): de facto already IN scope.** Pool
  fills made through offers and path payments have been ingested as `sdex`
  since 2026-06-24. This needs recording as the decision, not a new extractor
  task. What is still open is *labelling*: those fills are not tagged as pool
  trades in the candle.
- **AC 5 (0026's volume-credibility AC re-evaluated): PASS** for 2026-09-20.
  `sdex` volume for XLM-quoted pairs is credible against Horizon's aggregates,
  including Horizon's own all-types figure.
