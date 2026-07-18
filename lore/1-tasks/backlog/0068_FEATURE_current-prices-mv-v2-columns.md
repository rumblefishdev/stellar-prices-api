---
id: "0068"
title: "current_prices MV v2 columns — price_xlm, change_24h/7d_pct, sources, §5.5 outlier filter"
type: FEATURE
status: backlog
related_adr: ["0003", "0007"]
related_tasks: ["0039"]
tags: [layer-backend, priority-low, effort-small, clickhouse, materialized-view, current-prices]
links:
  - "../../../packages/prices-clickhouse/schema/current.sql"
  - "../../../docs/prices-api-general-overview.md"
  - "../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md"
  - "../active/0039_FEATURE_prices-periodic-workers-lambda-set.md"
history:
  - date: 2026-06-25
    status: backlog
    who: claude
    note: "Spawned from 0039 future work — current_prices MV v1 leaves 4 columns at DEFAULT."
---

# current_prices MV v2 columns

## Summary

The task-0039 `prices.mv_current_prices` refreshable MV (v1) populates only
`asset_id, price_usd, volume_24h_usd, vwap_24h, market_cap_usd, updated_at`.
The remaining four `current_prices` columns — `price_xlm`, `change_24h_pct`,
`change_7d_pct`, `sources` — are left at their table DEFAULTs (`0` / `''`).
This task extends the MV to compute them.

## Context

Spawned from task 0039 (PR #56). `mv_current_prices` is the **sole writer** of
`prices.current_prices` (ReplacingMergeTree), so these columns can only be
filled by extending the MV's `SELECT` + the explicit `TO (...)` column list —
no separate writer exists. Because the MV fully recomputes every row each
`REFRESH EVERY 1 MINUTE` from the trailing window, landing v2 needs **no
backfill or migration**: the next refresh after the schema change populates the
new columns for all assets. The table shape already has all 10 columns, so this
is purely an MV-definition change.

## Implementation

Extend `packages/prices-clickhouse/schema/current.sql`:

- **`price_xlm`** — latest price in XLM terms (`argMax(close, timestamp)` over
  XLM-quoted rows). Per ADR 0003, the semantic is "weighted across XLM-quoted
  pairs"; trivial for the common single-XLM-pair case.
- **`change_24h_pct` / `change_7d_pct`** — latest close vs the reference close
  at `now() − 24h` / `now() − 7d` (`(price_now / price_ref − 1) × 100`) via a
  self-join. Note `change_7d_pct` requires widening the MV window past the
  current `WHERE … INTERVAL 24 HOUR`.
- **`sources`** — per-source JSON breakdown aggregated from the per-source
  `price_ohlcv_1m` rows (`groupArray`/JSON → String).
- **`vwap_24h` refinement** — apply the general-overview §5.5 inter-source
  median-outlier filter on top of the existing volume-weighted mean.
- Add all new columns to the `TO prices.current_prices (...)` explicit column
  list (positional-insert footgun — see 0039 review).
- Re-deploy via `DROP VIEW` + re-`CREATE` (a refreshable MV's definition is
  fixed at create time).

Also: ensure the read layer treats `0` / `''` in these columns as
"not yet computed" rather than a real value until this lands.

## Acceptance Criteria

- [ ] `price_xlm` populated with the ADR-0003 XLM-quote semantic.
- [ ] `change_24h_pct` / `change_7d_pct` computed via the reference-close self-join (MV window widened to 7d).
- [ ] `sources` carries the per-source JSON breakdown.
- [ ] `vwap_24h` applies the §5.5 inter-source median-outlier filter.
- [ ] All new columns added to the explicit `TO (...)` column list; `current_mv_it.rs` extended to assert each.
- [ ] No backfill needed — verified the next refresh fills all rows.
