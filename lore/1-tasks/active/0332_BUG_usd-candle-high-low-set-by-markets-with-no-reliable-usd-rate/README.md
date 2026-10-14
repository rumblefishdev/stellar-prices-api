---
id: "0332"
title: "USD candles take high/low from USDT-quoted markets, whose USD rate we cannot measure — set the extremes from USDC- and XLM-quoted markets only"
type: BUG
status: active
assignee: akot
related_adr: ["0011", "0287"]
related_tasks: ["0170", "0229", "0236", "0172", "0212", "0286", "0331"]
tags: [layer-backend, priority-high, effort-small, api, ohlcv, methodology, data-correctness, defect]
links:
  - "../../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../../packages/prices-api/src/assets/handlers.rs"
  - "../../../../packages/prices-api/tests/ohlcv_it.rs"
  - "../../../2-adrs/0011_base-currency-is-a-denomination-not-a-pair-filter.md"
  - "../../../../web/portal/public/openapi.json"
history:
  - date: "2026-10-14"
    status: active
    who: akot
    note: >
      Code in PR #402 (5 commits, not merged). ITs green on CH 26.3.10.60:
      ohlcv_it 45, ignored suite 318/0 over 40 targets, workspace 1437/0.
      Step 1 measured on prod (AC 6, 7); client notice drafted (AC 10).
      Left: merge, compute deploy, sending the notice.
  - date: "2026-10-14"
    status: active
    who: akot
    note: >
      Decided: fallback buckets get no response flag (measured by AC 7
      instead); ADR 0011 is not amended. Implementation branch opened.
  - date: "2026-10-14"
    status: active
    who: akot
    note: >
      Opened from Adam's analysis of USD-mode /ohlcv high/low. Methodology change
      agreed on Slack the same day: Marek ("wyklucz usdt w liczeniu high/low")
      and Oskar (same reason USDT was dropped from the USD reference). Scope is
      step 1 only (read path); the per-minute rate (step 2) is a follow-up.
---

# USD candles take high/low from USDT-quoted markets

## Summary

USD-mode `/ohlcv` merges every market of an asset (pair × source) into one
candle: each market is scaled by one rate per bucket, then `high = max`,
`low = min`. USDT-quoted markets are scaled by a USDT/USD rate we cannot
measure, so their extremes are guesses and max/min picks the worst ones. XLM on
2022-06-18 comes back $0.1767 / $0.0815 against Binance's $0.1135 / $0.1001.
The stored data is correct; the read path's merge is not. Only USDC- and
XLM-quoted markets will set the price fields; every market still counts for
volume.

- Analysis, case study, prior art: [R-usdt-legs-widen-usd-candles](notes/R-usdt-legs-widen-usd-candles.md)
- Decision: [S-price-setting-markets-are-usdc-and-xlm](notes/S-price-setting-markets-are-usdc-and-xlm.md)
- Measured on prod: [R-step-1-measured-on-production](notes/R-step-1-measured-on-production.md)
- Client notice draft: [G-client-notice-usd-candle-prices](notes/G-client-notice-usd-candle-prices.md)

## Status: Active

**Current state:** PR #402 open, not merged; not deployed. Notice drafted, not sent.

## Implementation Plan

All in `packages/prices-api/src/assets/queries_ch.rs` unless stated.

1. **XLM id kept apart.** `UsdRefs` (~L703) gains the price-setting quote ids
   (USDC + XLM when tracked). `handlers.rs` ~L560 already resolves XLM; today it
   sits in `pivots` next to USDT. `pivots` and the `traded` label stay as is.
2. **`band` predicate + per-bucket fallback.** `usd_projection` (~L1003):
   `band = valid AND quote_asset_id IN (<price-setting>)`. `usd_aggregates`
   (~L979): `if(countIf(band) > 0, <agg>If(.., band), <agg>If(.., valid))` for
   `o`, `h`, `l`, `c`, `meth`. Keep `CLOSE_EXACT` / `HIGH_PUBLISHED` /
   `LOW_PUBLISHED` (~L783–799) as single named expressions so `vw`/`pfvw` still
   clamp against what the caller sees; keep the `isNull` overflow guards.
3. **Tests, table-driven.** ITs in `tests/ohlcv_it.rs`: USDC+USDT bucket (USDT
   extremes excluded, its volume kept); USDT with the larger volume (o/c/method
   still from USDC); XLM+USDT (XLM sets the band); USDT-only (unchanged); vwap
   clamped into the new band; the 2022-06-18 shape as a regression.
   `ohlcv_converts_each_leg_before_merging_across_them` still discriminates.
   Plus the ClickHouse-free SQL-string tests.
4. **Contract text.** `/ohlcv` description in `handlers.rs` ~L398 ("against
   every quote asset — are merged") rewritten; `openapi.json` regenerated.
5. **Measure and announce.** Read-only before/after on prod; client notice
   drafted in `notes/` (sending is Adam's).

## Acceptance Criteria

- [x] USD mode: a bucket with a valid USDC- or XLM-quoted row takes
      `open`/`high`/`low`/`close`/`method` from those rows only.
- [x] A bucket with no such row returns the same candle as before.
- [x] Volumes and trade counts unchanged for every bucket.
- [x] `vwap` and `pf_vwap` lie within the published `[low, high]`.
- [x] `base_currency=XLM` (`QuoteLeg` arm) unchanged.
- [x] XLM 1d 2022-06-18 returns high ≈ $0.1137, low ≈ $0.1005; XLM p95 high/low
      deviation vs Binance recorded before/after and set against the analysis'
      step 1+2 figures. Prod: $0.1137 / $0.1005; on the analysis' 557 days step
      1 alone gives its 1.14 % / 2.16 % (see the R- note).
- [x] Affected assets/buckets counted, including non-USDT legs the allowlist
      also drops (the analysis' 419 counts USDT only), and fallback buckets
      (no USDC/XLM market) counted separately. 37,582 buckets / 419 assets,
      all USDT (no other quote is convertible); fallback 1,345 / 160. 1d only.
- [x] ITs pass on CH 26.3.10.60; full ignored suite green.
- [x] OpenAPI description updated in the same PR as the code.
- [x] Client notice drafted in `notes/`.

## Decisions (2026-10-14, Adam)

1. **No flag on fallback buckets.** A bucket with no USDC/XLM market is computed
   as today and is not marked in the response: a new field changes the response
   shape (OpenAPI, portal types, conformance suite, client notice). AC 7 counts
   fallback buckets; a flag becomes its own task if there are many.
2. **ADR 0011 is not amended.** The change ships as code, the `/ohlcv`
   description and the client notice. The fallback keeps the denomination (USD)
   and only narrows which markets set the extremes — the same kind of
   data-dependent input choice as today's highest-volume open/close.

## Follow-ups (not spawned)

- **Step 2 — per-minute rate for USDC/XLM markets.** Needs `_1m` priced
  end-to-end (0286 USD re-pricing) and a choice: read `_1m` at query time, or
  store USD extremes in the rollups (schema + re-ingest). AC 6: for XLM, step 1
  alone already gives the analysis' step 1+2 figures; any gain from step 2
  would come from XLM-quoted tokens (ETH, XRP in the analysis).
- **Cheap single fills on the USDC/XLM markets themselves.** After step 1,
  most remaining XLM low outliers are Aquarius XLM/USDC fills far below the
  market (62 of 121 days >5 % off, 2025–26, drifting from $0.20 to $0.03). This
  looks like a stale pool that nobody arbitrages; which pool is not yet
  established. The analysis' ±50 % band (rule 3) removed these prints. See the
  R- note.

## Notes

- Pre-2021 ([[0331]]): USDT-quoted rows carry a par `close_usd` there; the
  allowlist still keeps them out of the band when an XLM leg traded — intended.
- Execution: `/gsd-quick --full` with a BRIEF, in a worktree.
