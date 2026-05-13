---
title: "Decision space — how should price_ohlcv key its rows?"
type: question
status: mature
spawned_from: ../README.md
spawns: ["S-recommendation"]
tags: [schema, ohlcv, primary-key, sdex, backfill]
links:
  - "../README.md"
  - "../../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../../../../docs/prices-api-general-overview.md"
  - "../../../../docs/database-schema/database-schema-overview.md"
history:
  - date: 2026-05-13
    status: mature
    who: okarcz
    note: "Captured the decision-space framing the README listed (A/B/C) plus an Option D variant that emerged during the API survey."
---

# Decision space — how should `price_ohlcv` key its rows?

## The problem (one paragraph)

`price_ohlcv` today is `PRIMARY KEY (timestamp, asset_id, granularity)`
where `asset_id` is the **base asset's** surrogate id. SDEX has
many native quote choices (XLM, USDC, USDT, EURT, …) — one base
asset commonly trades against multiple quotes in the same minute
(e.g. USDC/XLM + USDC/USDT). With base-only keying, those collide.
Decode-and-bucket spec [§2.3 of 0022](../../../archive/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md#22-pair-canonicalisation-rule) flagged this as load-bearing
for SDEX correctness and deferred resolution here.

## The four options

| ID | PK shape                                                       | What backfill writes per minute      | API projection effort |
| -- | -------------------------------------------------------------- | ------------------------------------- | --------------------- |
| A  | `(timestamp, asset_id, quote_asset_id, granularity)`           | One row per native pair               | Aggregate over rows for the asset → USD or XLM view |
| B  | `(timestamp, asset_pair_id, granularity)` with new `asset_pairs` table | One row per native pair (via pair surrogate) | Same as A, plus a join |
| C  | `(timestamp, asset_id, quote_kind, granularity)` where `quote_kind ∈ {USD, XLM}` | One row per asset per quote-kind, with native pair normalised at write | None — already in API shape |
| D  | Status quo (`(timestamp, asset_id, granularity)`)              | Collides; either undefined or last-write-wins | Single row per asset already in API shape |

## What the API surface looks like (key data point)

`GET /assets/{asset_identifier}/ohlcv` accepts only
`base_currency = USD | XLM` (param is mislabelled — it's the quote
currency). The OHLCV response is **one series per asset per quote
choice**, not per native pair. See
[docs/prices-api-general-overview.md L427](../../../../docs/prices-api-general-overview.md).

So the API never reveals "USDC/USDT" vs "USDC/EURT" separately —
those would aggregate into the single "USDC priced in USD" series.
But the *storage* needs to keep the distinction because:

1. Each native pair has its own VWAP / volume that contributes to
   the projection.
2. `current_prices.sources` JSONB exposes per-source (SDEX vs
   Soroswap vs Aquarius) breakdown — the per-source attribution
   requires the row-level source label, which collapses if we
   pre-normalise to quote-kind.
3. The "aggregated" multi-source merge contract
   ([0025](../../backlog/0025_RESEARCH_live-multi-source-merge-contract.md))
   needs per-source rows to merge from. Pre-normalising loses
   the constituent values.

## Sub-questions this task resolves

1. Does the API actually need per-native-pair distinction, or
   can we project at write time?
2. Migration cost: how invasive is A vs B given **no rows exist
   yet** (greenfield)?
3. Index size implication: extra `INT` column on the PK?
4. Query ergonomics: which is simpler for the canonical reads?
5. Does B's `asset_pairs` table earn its complexity from
   anything beyond `price_ohlcv`?

Synthesis → [S-recommendation](./S-recommendation.md).
