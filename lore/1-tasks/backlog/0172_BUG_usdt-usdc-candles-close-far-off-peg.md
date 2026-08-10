---
id: "0172"
title: "USDT/USDC candles close at ~0.14 instead of ~1.00 — 891 days of real, high-volume trades at an impossible stablecoin price"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0165", "0139", "0116", "0144", "0026"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "enrichment", "sdex", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-10
    status: backlog
    who: okarcz
    note: >
      Found while verifying the 0165 deploy on prod. USDT was 0165's control
      asset - the peg arm must NOT flatten it - and that check passed, but the
      values it returned are impossible. Not caused by 0165 and not changed by
      it: arm B contributes 0/0 wherever sum(w) > 0, so every one of these
      values is arithmetically identical to what the old view published. This is
      pre-existing and was simply never looked at.
---

# USDT/USDC closes at ~0.14, not ~1.00

## Measured on prod (2026-08-10, after the 0165 apply)

```
price_usd_series, USDT @ canonical issuer:
  method=traded  rows=891  avg_close=0.1923

price_ohlcv_1d, asset_id 111 (= USDT), grouped by quote leg:
  quote=USDC  candles=891  avg_close_usd=0.1923  avg_close_native=0.192333
              min_usd=0.0912  max_usd=0.4915

by source:
  sdex  candles=891  trades=132,685  total_volume_base=491,984.77
```

Two dollar-pegged assets cannot trade at 0.09–0.49 against each other. A
stablecoin pair should sit at 1.0000 ± a fraction of a percent.

`close_usd ≈ close_native` is **correct** and not the bug — a USDC-quoted candle
takes the stablecoin-direct tier, so `close_usd = close × $1`. The defect is in
`close` itself.

## Two hypotheses tested and FALSIFIED — do not re-run them

1. **[[0139]] `asset_id` collision.** ❌ `asset_id = 111` resolves to exactly one
   natural identity (USDT @ `GCQTGZQQ…TG6V`). No sharing.
   *(Separately confirmed while checking: 0139 is real and wide — **3,281
   `asset_id`s serve 6,568 identities**. That number appears nowhere in 0139 and
   should be carried there.)*
2. **The rows are XLM's prices misattributed.** ❌ `usdt_close` 0.142572 vs
   `xlm_close` 0.163999 on 2026-08-10 — close in magnitude, never equal.
   ⚠️ When re-testing this, note `price_ohlcv_1d` holds **one row per source**,
   so a naive join to XLM/USDC fans out ~4× per day. Aggregate or filter source.

## Why it is not a thin-market artefact

891 candles, **one per day with no gaps**, 132,685 trades, ~492k USDT of base
volume. This is an actively traded pair, not dust — so [[0116]] (dust-trade
candles producing absurd prices) does not explain it either.

## Where to look next

- **Is `asset_id 111` really the asset the writer meant?** The reader-side
  collision is ruled out; the writer side is not. Check whether the USDT
  identity has ever been assigned more than one `asset_id`, and whether ids were
  renumbered at any point (historical candles would then point at the wrong
  asset without any duplicate appearing in `assets` today):
  ```sql
  SELECT asset_id, asset_code, issuer_address, contract_address, asset_type
  FROM prices.assets FINAL WHERE asset_code = 'USDT' ORDER BY asset_id;
  ```
- **Does the raw trade agree with the candle?** Pull one day's underlying trades
  and check `volume_quote / volume_base` against `close`. If they agree, the
  candle is a faithful aggregate of trades that are themselves wrong (an
  ingestion/parse issue — amount scaling, or a leg swap at extract time). If
  they disagree, the aggregation is at fault.
- **Is the pair orientation right?** `1 / 0.1426 = 7.01`, which is not 1 either,
  so a simple base/quote inversion does **not** explain it — but a scaling error
  (Stellar amounts are 7-decimal fixed point) is worth checking against a known
  trade.
- **Check other USDT pairs.** The prod query showed USDC as the *only* quote leg
  for id 111. If USDT trades against XLM anywhere, its implied USD price via
  that leg is an independent cross-check.

## Blast radius

`close_usd` is what BE multiplies into TVL. If USDT positions are valued at
~$0.14 instead of ~$1.00, every pool with a USDT leg is understated ~7×. BE's
CSV counts **106 pools with a USDT leg, 102 of them priceable** — so these are
pools that look healthy and are silently wrong, which is worse than the
never-priced pools [[0165]] fixed.

⚠️ It also means **USDT is not a trustworthy control** for peg-related work until
this is resolved — 0165 used it as one.

## Acceptance Criteria

- [ ] Root cause identified and stated (writer-side id, ingestion scaling, leg
      swap, or something else), with the falsified hypotheses above left on the
      record so they are not re-run.
- [ ] Whether the defect is USDT-specific or a class affecting other assets —
      a sweep for assets whose `close_usd` is implausible for their type.
- [ ] Correction plan for the 891 existing daily candles (and their 1h/coarse
      counterparts), or an explicit decision to leave history as-is.
- [ ] Regression test on the 26.3.10.60 pin.
- [ ] BE notified if any published TVL was affected.
