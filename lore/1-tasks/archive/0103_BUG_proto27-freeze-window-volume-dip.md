---
id: "0103"
title: "Investigate the ~45% volume dip on 2026-07-09→07-11 (proto27 freeze-window recovery)"
type: BUG
status: completed
related_adr: []
related_tasks: ["0064", "0094", "0095"]
tags: [layer-indexing, priority-medium, effort-small, clickhouse, ingestion, data-quality]
links:
  - "../../../packages/prices-clickhouse/schema/preroll-live-gap.sql"
history:
  - date: 2026-07-17
    status: completed
    who: okarcz
    note: >
      RESOLVED — genuine market behaviour, NOT lost data. The dip is confined to
      SDEX trade_count; SDEX volume_base is normal-to-high on the dip days and
      candle coverage is a full 24 h/day (Query A/B below). You cannot lose ~45%
      of trades while keeping 100% of their volume — every trade carries volume —
      so nothing was dropped. Average SDEX trade size ~doubled (3.2M → 6–9M) then
      returned, i.e. many small trades vanished while large trades continued —
      the signature of HFT/market-maker order-book activity thinning around the
      proto27 upgrade. Confirmed EXTERNALLY: Horizon trade_aggregations for
      XLM/USDC shows the same shape independently — 63,393 (07-08) → 31,432 trough
      (07-10, ~50%) → 79,129 recovery (07-12), matching our all-SDEX trough day
      (07-10) and recovery day (07-12). Two extractor bugs (0096 soroswap, 0099
      phoenix) are unrelated — AMM is a tiny share of trade_count. No re-ingest
      needed; the OHLCV price/volume data for the week is complete.
  - date: 2026-07-17
    status: backlog
    who: okarcz
    note: >
      Spawned while verifying the live-gap pre-roll (PR #120). Daily trade
      counts show 2026-07-09/10/11 at ~45% of a normal day, inside the proto27
      live-freeze window (0064/0094). May be genuine market quiet, or the freeze
      recovery may not have fully backfilled those days.
---

# Investigate the ~45% volume dip on 2026-07-09 → 07-11

## Summary

Verifying the live-gap pre-roll surfaced a daily-volume dip that does not
obviously fit either "market quiet" or "known freeze". Measured on prod
2026-07-17 from `prices.price_ohlcv_1d` (all sources, `sum(trade_count)`):

| day | weekday | trades | vs normal |
|---|---|---|---|
| 2026-07-04 | Sat | 1,847,919 | normal |
| 2026-07-05 | Sun | 1,751,297 | normal |
| 2026-07-06 | Mon | 1,487,561 | normal |
| 2026-07-07 | Tue | 1,462,343 | normal |
| 2026-07-08 | Wed | 1,490,280 | **normal — despite the freeze starting 12:31 this day** |
| **2026-07-09** | Thu | **764,938** | **~51%** |
| **2026-07-10** | Fri | **644,838** | **~43%** |
| **2026-07-11** | Sat | **711,127** | **~48%** |
| 2026-07-12 | Sun | 1,296,175 | ~87% |
| 2026-07-13 | Mon | 1,328,791 | ~89% |

## Why it is suspicious

The proto27 live freeze (tasks 0064 / 0094) stalled ingestion from **2026-07-08
12:31** until the durable-cursor drain caught up on ~07-16. If the drain fully
recovered the window, those days should look normal. Two details cut against a
simple explanation:

- **07-08 reads a FULL day (1.49M) even though the freeze began at 12:31 that
  day** — so the drain clearly did backfill at least part of the window
  correctly.
- **07-12 and 07-13 are back near normal**, so whatever suppressed 07-09→07-11
  stopped.

A weekday/weekend effect does not explain it either: the dip spans Thu/Fri/Sat
while the preceding Sat/Sun (07-04/05) are the *highest* days in the sample.

## Hypotheses

1. **Genuine market quiet** — plausible; trade volume is not uniform, and the
   surrounding week is trending down (the 07-06 week totals 7.86M vs 9–15M for
   the prior five weeks). Cheapest to confirm against an external reference.
2. **Partial freeze recovery** — the drain re-ingested the window but lost or
   skipped part of 07-09→07-11 (e.g. a cursor jump, a DLQ'd batch never
   replayed, or ledgers processed while the extractor was mid-deploy).
3. **Something source-specific** — the numbers above are all-sources; the dip may
   be confined to one source, which would point at that extractor rather than at
   ingestion.

## Implementation

- Break the dip down **per source** first — it separates hypothesis 3 from 1/2
  in one query:
  ```sql
  SELECT timestamp AS day, source, sum(trade_count) AS trades
  FROM prices.price_ohlcv_1d FINAL
  WHERE timestamp >= '2026-07-04' AND timestamp < '2026-07-14'
  GROUP BY day, source ORDER BY day, source
  SETTINGS max_threads = 2;
  ```
- Cross-check against ground truth in BE's `default.soroban_events` (AMM) and
  the ledger archive / Horizon aggregates (SDEX) for the same days. If on-chain
  activity really was ~half, close this as market behaviour.
- If our data is short, check the DLQ (`prices-ingest-dlq-production`) and the
  `prices.ingest_cursor` history around the drain for skipped ranges, and reprice
  the affected range (`events-backfill` for AMM; `sdex-backfill` for SDEX), then
  re-run `schema/preroll-live-gap.sql` over it.

## Resolution (2026-07-17) — genuine market behaviour

**Attributed: market, not lost data.** Three independent lines of evidence:

1. **Per source (Query A):** the dip is entirely SDEX `trade_count`. AMM
   (aquarius/phoenix/soroswap) is a tiny, steady share and does not move the
   total. So this is not the 0096/0099 AMM extractor bugs.
2. **Volume + coverage intact (Query A/B):** SDEX `volume_base` on 07-09→11
   (4.86T / 5.98T / 3.44T) is normal-to-high — 07-10 is *above* the surrounding
   days — and SDEX had candles in all **24 hours** every day. Losing trades
   necessarily loses their volume; volume is whole, so nothing was dropped. What
   changed is trade *size*: the SDEX average roughly doubled (3.2M → 6–9M) then
   returned, i.e. many small trades disappeared while large trades continued —
   the fingerprint of HFT / market-maker order-book activity pausing around the
   proto27 network upgrade, then resuming (07-12/13 back to normal, same drain,
   same build → not a decode/extractor artefact).
3. **External cross-check (Horizon `trade_aggregations`, XLM/USDC daily
   `trade_count`):** an independent source that never touched our pipeline shows
   the same dip —

   | day | Horizon XLM/USDC | vs 07-08 | our all-SDEX | vs 07-08 |
   |---|---|---|---|---|
   | 07-08 | 63,393 | — | 1,481,083 | — |
   | 07-09 | 40,278 | 64% | 757,421 | 51% |
   | 07-10 | 31,432 | 50% | 635,764 | 43% |
   | 07-11 | 42,077 | 66% | 704,271 | 48% |
   | 07-12 | 79,129 | 125% | 1,289,358 | 87% |

   Same trough day (07-10, ~50%), same recovery (07-12). Reproduce with:
   `GET https://horizon.stellar.org/trade_aggregations?base_asset_type=native&counter_asset_type=credit_alphanum4&counter_asset_code=USDC&counter_asset_issuer=GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN&resolution=86400000&start_time=1783468800000&end_time=1783900800000&order=asc`

**No re-ingest needed.** The OHLCV price/volume data for the week is complete; only
the discrete-trade count is lower, and that is real.

## Acceptance Criteria

- [x] Dip broken down per source, and attributed: **market**, not lost data.
- [x] No re-ingest required — data was not lost (volume + 24 h coverage intact).
- [x] Market behaviour recorded here with the external Horizon cross-check that
      proves it, so the next person to notice this dip does not re-investigate it.

## Notes

- Do **not** point an SCF reviewer at daily volumes for this week until this is
  settled — the dip invites exactly the question this task answers.
- The pre-roll that surfaced it is verified correct: the 07-06 week's daily
  counts sum to **exactly** its weekly bucket (7,857,262), so the dip is in the
  source data, not in the rollup.
