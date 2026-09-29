---
title: "Assets whose current price rests on offer-priced dust fills: population, XAUa, and how far those prices sit from a real fill"
type: research
status: mature
tags: [dust-prints, measurement, clickhouse, current-price, data-quality]
links:
  - "../../../../2-adrs/0287_candle-prices-come-from-price-forming-fills-and-a-windowed-close.md"
  - "../../../archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/notes/R-measurements-2026-09-15.md"
history:
  - date: "2026-09-29"
    status: mature
    who: akot
    note: >
      Run read-only against production ClickHouse (`dev_read`) on 2026-09-29
      at about 13:30 UTC, over the 24 h window `mv_current_prices` reads
      `price_usd` from. One point in time; the traded population moves about
      25 % a day. Raw per-asset rows were kept outside the repository
      (`.planning/quick/260929-mi2-…/pop_2026-09-29.tsv`).
---

# Assets priced by offer-priced dust fills (2026-09-29)

## What is counted

A 1m row is **dust** when its price-forming fills are certainly below ADR
0287's rounding bound (`1/a + 1/b > 0.001` on the raw stroop amounts):

- `source = 'sdex'` — below the bound, only an order-book fill can be
  price-forming (it is priced at the resting offer, ADR 0287 §1); pool fills
  below the bound never form price, and every other source is a pool;
- and either one fill (`pf_trade_count = trade_count = 1`) whose
  `volume_base` / `volume_quote` fail the bound, or a row whose `pf_volume` or
  `pf_price_volume` is under 1 000 stroops in total.

This test is **sufficient, not exact**: a minute with several dust fills whose
totals exceed 1 000 stroops on both sides reads as real. The counts below are
lower bounds. The float form here also differs from the ingest's integer form
(`price.rs`) at exactly 1 000 stroops; the published rule uses the integer
form.

Window and population: the 1m rows with `pf_trade_count > 0 AND close_usd > 0`
in the last 24 h, grouped by the BASE asset (the same candle set and the same
leg `mv_current_prices.base_tip` reads), restricted to assets whose current row
has `price_usd > 0 AND method = 'traded'`. Reference for deviation: the median
`close_usd` of the same asset's non-dust price-forming rows over 7 days.

## Results

| Group | Assets |
| --- | --- |
| Priced from trades (24 h) | 3 733 |
| **A** — every price-forming row in 24 h is dust | **132** |
| **B** — real fills exist, but the current price was set by a dust row | **20** |
| Current price set by a real fill | 3 581 |

Group A would have been `unpriced` before 0286 phase 2 (2026-09-22): a
1-stroop order-book fill was ratio-priced then and failed the bound.

**Group A** — 114 of 132 have no non-dust fill in 7 days, so nothing inside
our data can confirm or refute their price. About 111 are priced below $0.01
(junk tokens; 21 are "Indian stock" codes of one issuer, `GADLWV…`). The ~21
priced at $0.01 or more are mixed:

| Asset | Published | Compared with | Reading |
| --- | --- | --- | --- |
| BTC `GBVFOW…` | $85,806 | BTC `GDPJAL…` $84,300 (real market) | on market, +1.8 % |
| PAXG `GC7WK3…` | $2,194 | gold ≈ $4,200 (XAUa, XAUZ) | off, −48 % |
| XGold `GBLY3A…` | $6,564 | gold ≈ $4,200 | off, +56 % |
| OOO, XAg, XCu, CFX / GEAGLE70 (both exactly $4,495), XMR | $165,817 … $550 | nothing | unknown |

Among the 18 with a 7-day reference, some are far off (xLMNR ≈ 0 vs $0.18,
AUCOPPER × 0.001). Ten sit at 1.05–1.09 of the reference, which is more likely
XLM/USD moving over the week than the dust price being off — the reference is
in USD, not in quote units.

**Group B** — the assets with a real market sit close to their real fills:
BTC `GDPJAL…` +0.2 %, BCH +0.6 %, XAUa +1.7 %, XPi +3.5 %. Five of 20 are more
than 2× off (XTAI × 1590, LAUNCH × 123, TESLA × 47, XDCB × 0.011,
GoogleCore × 0.001), all tokens with a few dollars of real volume.

**Volume** carried by dust rows in group A: $35 over the 24 h, all 132 assets
together. This is a price problem, not a volume problem.

## XAUa, the task's reference case

XAUa (`GB2I4Y…`, asset 201042) is bought for 1 stroop against XLM every 4 h at
:33 — a bot. The task's "$4,375 on $0.003" is one of those pings (2026-09-24
10:33, `volume_quote_usd` 0.0006). Until 2026-09-27 16:18 those pings were its
only trades; since then it also has real fills of about $12. On 2026-09-29 its
price was $4,249 (set by a ping) against a real-fill median of $4,179, and
another gold token, XAUZ, stood at $4,172. **The offer-priced dust price is on
market here** — 0116's caution about small trades of expensive assets, in
person.

## Reading

- D3 of ADR 0287 ("an order-book fill always forms price") was measured on
  three liquid pairs only; 0278's note says no thin pair was measured. Group
  B is the first thin-pair evidence: where a market exists, the offer price is
  close to it for every asset that trades more than a few dollars.
- Where no market exists (group A), the offer price is sometimes right (BTC
  `GBVFOW…`) and sometimes badly wrong (PAXG, XGold), and our data cannot tell
  which. A size threshold would blank the right ones with the wrong ones;
  withholding is therefore not justified by this data. What can be said
  truthfully is that no real fill supports the price.
- Group B's junk outliers are an off-market-print problem, [[0310]]'s scope.

## Limits

- One day. A second run on another day is owed before the rule is final.
- Sufficient dust test: misses undercount group A.
- The deviation reference is in USD over 7 days; deviations under ~10 % are
  not evidence either way.
- Not yet validated: that the published rule flags **no** thin-but-real
  asset. By construction it cannot (one confirmed row in 24 h reads `trades`),
  but it has not been run on prod.

## Query (core)

```sql
WITH rows AS (
  SELECT asset_id, timestamp, close_usd, volume_quote_usd,
    source = 'sdex' AND (
      (pf_trade_count = 1 AND trade_count = 1 AND
        (1/greatest(toFloat64(volume_base)*1e7,1e-9)
       + 1/greatest(toFloat64(volume_quote)*1e7,1e-9)) > 0.001)
      OR toFloat64(pf_volume) < 1e-4 OR toFloat64(pf_price_volume) < 1e-4) AS is_dust
  FROM prices.price_ohlcv_1m FINAL
  WHERE timestamp >= now() - INTERVAL 7 DAY AND timestamp <= now()
    AND pf_trade_count > 0 AND close_usd > 0
),
d24 AS (
  SELECT asset_id, count() n_rows, countIf(is_dust) n_dust,
         argMax(is_dust, timestamp) last_is_dust
  FROM rows WHERE timestamp >= now() - INTERVAL 24 HOUR GROUP BY asset_id
),
ref7 AS (
  SELECT asset_id, countIf(NOT is_dust) n_real_7d,
         quantileIf(0.5)(toFloat64(close_usd), NOT is_dust) ref_usd
  FROM rows GROUP BY asset_id
)
SELECT d.*, r.*, toFloat64(c.price_usd) / r.ref_usd AS ratio
FROM d24 d
JOIN (SELECT asset_id, price_usd FROM prices.current_prices FINAL
      WHERE price_usd > 0 AND method = 'traded') c USING asset_id
JOIN ref7 r USING asset_id
WHERE d.last_is_dust   -- group A: n_dust = n_rows; group B: n_dust < n_rows
```
