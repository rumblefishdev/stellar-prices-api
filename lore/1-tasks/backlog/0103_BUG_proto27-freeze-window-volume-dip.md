---
id: "0103"
title: "Investigate the ~45% volume dip on 2026-07-09→07-11 (proto27 freeze-window recovery)"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0064", "0094", "0095"]
tags: [layer-indexing, priority-medium, effort-small, clickhouse, ingestion, data-quality]
links:
  - "../../../packages/prices-clickhouse/schema/preroll-live-gap.sql"
history:
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

## Acceptance Criteria

- [ ] Dip broken down per source, and attributed: market quiet vs lost data.
- [ ] If data was lost: affected range re-ingested + pre-rolled, and the daily
      counts land where the on-chain record says they should.
- [ ] If market quiet: recorded here with the cross-check that proves it, so the
      next person to notice this dip does not re-investigate it.

## Notes

- Do **not** point an SCF reviewer at daily volumes for this week until this is
  settled — the dip invites exactly the question this task answers.
- The pre-roll that surfaced it is verified correct: the 07-06 week's daily
  counts sum to **exactly** its weekly bucket (7,857,262), so the dip is in the
  source data, not in the rollup.
