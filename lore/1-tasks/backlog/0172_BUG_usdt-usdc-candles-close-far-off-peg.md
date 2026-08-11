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
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      CONFIRMED BY BE INDEPENDENTLY, and WIDER than this task was filed on. They
      hit it while re-measuring 0165: canonical USDT (GCQTGZQQ...TG6V) publishes
      method='traded' daily closes of 0.129-0.143 for 08-04 -> 08-10 in
      price_usd_series. So the defect is in USDT's OWN published identity series,
      not just the USDT/USDC pair - the 106-pool blast radius understates it, and
      any consumer reading USDT's USD price reads a wrong number.
      Their consumer-visible symptom is USDT FLAPPING between $0.14 and $1.00:
      traded buckets carry this defect while the newest bucket, where USDT does
      not trade as a base, takes 0165's peg fallback of $1. 0165 does NOT cause
      this and does not worsen it (arm B contributes 0/0 wherever sum(w) > 0, so
      every traded value is arithmetically identical to the old view) - it made a
      uniformly wrong column visibly discontinuous, which is a diagnostic
      improvement. Describe the symptom as flapping, but do not imply 0165
      introduced it.
      BE asked to bump priority; the tag is already priority-high and 0172 is
      already first in the queue, so what changed is the justification, not the
      rank.
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

## 🔴 CONFIRMED BY BE 2026-08-11 — and it is WIDER than this task was filed on

BE hit this independently while re-measuring [[0165]], and their reading
**escalates the defect on two axes**:

1. **It is not the `USDT/USDC` pair — it is USDT's own published identity
   series.** They observe canonical USDT (`GCQTGZQQ…TG6V`) publishing
   `method = 'traded'` daily closes of **0.129–0.143 for 08-04 → 08-10** in
   `price_usd_series` — the surface they consume directly. This task was filed
   from the pair; the blast radius is the **asset**. In their words: *"0172 is
   not 'distortion on those pairs' but a wrong published price for USDT itself;
   you may want to bump its priority accordingly."*
2. **A consumer now sees USDT flapping between $0.14 and $1.00.** The traded
   buckets carry this defect's value while the newest bucket — where USDT does
   not trade as a base — takes [[0165]]'s peg fallback of `$1`.

⚠️ **0165 does not cause the flapping and does not make the data worse.** Arm B
contributes `0/0` wherever `sum(w) > 0`, so every traded value is arithmetically
identical to what the old view published. What 0165 changed is **visibility**: a
correct `$1` now sits next to a wrong `$0.14` in one series, so a uniformly
wrong column became a visibly discontinuous one. Describe the symptom as
*flapping* rather than as a quiet 7× understatement — that is what a consumer
now reports — but do not let the framing imply 0165 introduced it.

**On BE's "bump its priority":** the tag is *already* `priority-high` and 0172 is
*already* first in the recorded queue (`0172 → 0170 → 0168 → 0127 → 0128`), so
there is no re-tagging to do — what their message changes is the **justification
and the framing**, not the rank. Worth saying plainly when replying, so they
know it is next rather than merely re-labelled.

## Blast radius

`close_usd` is what BE multiplies into TVL. If USDT positions are valued at
~$0.14 instead of ~$1.00, every pool with a USDT leg is understated ~7×. BE's
CSV counts **106 pools with a USDT leg, 102 of them priceable** — so these are
pools that look healthy and are silently wrong, which is worse than the
never-priced pools [[0165]] fixed.

⚠️ **Update 2026-08-11:** the 106-pool figure now *understates* it, because the
defect is in the asset's own series (above), not only in pools whose two legs
are the affected pair. Any consumer reading USDT's USD price — pool-valuing or
not — reads a wrong number.

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
