---
title: "Phase 5 — backfill the whole history, versioned, view first; what breaks downstream and how to roll back"
type: synthesis
status: developing
spawned_from: S-composition-rule.md
spawns: []
tags: [pricing, stablecoin, migration, clickhouse, decision]
links:
  - "../../0247_FEATURE_backfill-usdc-usd-history-from-an-external-anchor.md"
  - "../../../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-09-04
    status: seed
    who: akot
    note: >
      Builds on [[0247]]'s design (external rows into usd_rate, own method
      value, view-first). Costs from the composed series; the re-enrichment
      cost is [[0168]]/[[0111]]'s and is quoted, not re-measured.
---

# Phase 5 — backfill and migration

## Decision: backfill the full history, not fix-forward [est. on data]

Fix-forward alone leaves the falsifying date in place: the whole point of
the task is 2023-03-11, and a series that is measured from 2026-09 onward and
asserted before still says $1.00 on the SVB weekend. The backfill is also
unusually cheap here, because [[0247]] already established that
`price_usd_series` reads `usd_rate` **at query time**: loading rows fixes the
view with no re-enrichment pass.

What gets loaded: the composed daily series from `data/composed_usdc_usd_1d.csv`
(2 049 rows, 2021-01-25 → 2026-03-10, stopping where our own `oracle`
readings begin so no key overlaps), keyed on canonical USDC. Hourly for
2023-03 only if the product wants the intraday trough visible on `1h`
candles; daily is enough for the acceptance test.

## Two halves, shipped in this order

| half | what changes | cost | risk |
|---|---|---|---|
| **1. view + `/ohlcv`** | INSERT ~2 049 `usd_rate` rows with `method = 'external'` (the value [[0247]] reserves; **never** `'oracle'`) plus the new `source`/`quality` columns; `ohlcv_peg_series` widens its `WHERE method = 'oracle'` to `IN ('oracle','external')` and stops synthesising `1` where an external row exists | one INSERT, one query edit, a deploy | the view and the stored `close_usd` disagree in deep history — *intended*, must be written down |
| **2. candles** (`close_usd` on every USDC-quoted candle before 2026-03-11) | re-enrichment over the 654 291 pre-oracle candles × every grain — [[0168]]'s "known adjacent gap", gated behind [[0111]] | 0182-class job, measured in days | the repair campaign class of risk ([[0182]], [[0201]]); needs the cleanup worker to stay dark |

Half 1 closes this task's acceptance criteria 1–3. Half 2 closes defect B
(the `method: peg` on `native` and USDT) and is its own task.

## Versioning the series

The `usd_rate` key already contains `method`, so `oracle`, `peg` and
`external` rows coexist and "the consumer chooses" (`init.sql:280`). That is
the version mechanism: **the old answer is never deleted**, the view's
preference order changes. Rollback is the preference order reverting to
`oracle`-only, which restores today's behaviour byte for byte. Add a
`loaded_at` / `source_version` column on the external rows so a re-load from
a newer Chainlink read can be told apart from the first one.

## What breaks downstream [est.]

- **API Gateway cache** ([[0122]]): `/ohlcv` responses for USDC are cached
  per TTL; the change is visible within one TTL, no purge needed.
- **Backtests and saved results** that consumed the flat series will move
  by ≤ 12 bps on 99 % of days and by 319 bps on 2023-03-11
  (`data/composed_vs_ours_1d.csv`). Anything sized off USDC's 1-day tail
  was sized off zero (`data/business_impact.md`: VaR 99 % 0 → 6 bps, ES 0 →
  25 bps, worst day 0 → 300 bps). This is the one number for the memo's top.
- **Every USDC-quoted asset's USD history** stays as it is until half 2;
  the view and the candles disagree for 2021-01 → 2026-03. Document it in
  the `backfill_note` field `/ohlcv` already returns.
- **The 0127 spot-check table** excluded USDC on purpose; after half 1 it
  can be re-included with the 2023-03-11 close as the exhibit.

## Rollout and rollback

1. Load the external rows in a **shadow** `method` (`'external-candidate'`)
   and run `analysis/guardrails.py` and the 2023-03-11 fixture against the
   view **before** the preference flips. Nothing user-visible changes.
2. Flip the preference in `price_usd_series*` and `ohlcv_peg_series`, deploy
   `prices-api`. Watch the phase-4 alarms and the [[0125]] dashboard for one
   day.
3. Rollback = revert the preference order (a view DDL and one query line);
   the rows stay, unused.
4. File half 2 as its own task against [[0111]]; file the `method`
   vocabulary change (`peg` on non-stablecoins) with it.

## The ticker→issuer gate, applied

Chainlink's USDC/USD prices Circle's Ethereum-issued USDC. We file it under
the Stellar issuer `GA5Z…KZVN`, which is also Circle's own issuance — the
same entity, redeemable 1:1 at the same counter, unlike the [[0172]] USDT IOU.
[[0173]]'s rule is satisfied by that identity, and the loader is written for
this one asset and refuses any other code.
