---
id: "0061"
title: "Historical USD-quoted price series — price_usd(asset, t) primitive for Block Explorer LP analytics"
type: FEATURE
status: active
related_adr: ["0003", "0004", "0007"]
related_tasks: ["0060", "0026", "0039", "0051", "0040"]
tags: [layer-database, clickhouse, oracle, reflector, usd-pricing, enrichment, cross-team-be, effort-medium]
links:
  - "../../../../docs/database-schema/database-schema-overview.md"
  - "../../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../../packages/sdex-backfill/src/soroban.rs"
history:
  - date: 2026-06-12
    status: active
    who: okarcz
    note: >
      Spawned from the 0060 cross-team Block Explorer LP-analytics
      discussion (point 3: per-asset USD-quoted historical series).
      Carries the R-historical-usd-close-design research note off the
      0060 branch onto its own branch for clean separation. Scopes the
      close_usd column, the oracle↔asset-id reconciliation fix, the
      enrichment/rollup propagation, and the price_usd_series view + API.
---

# Historical USD-quoted price series — `price_usd(asset, t)`

## Summary

Provide a single primitive the Block Explorer team needs for read-time LP
analytics (volume, fee_revenue, TVL in USD): the **historical USD price of any
asset at a given ledger's close** — `price_usd(asset, t)`. Deliver it as a
prices-owned ClickHouse view (`prices.price_usd_series`) plus optional REST
endpoints, derived from existing OHLCV candles × Reflector oracle USD via the
identity `close_usd = close × usd_price(quote_asset, t)`.

## Status: Active

**Current state:** Design complete. Full feasibility/design captured in
[`notes/R-historical-usd-close-design.md`](notes/R-historical-usd-close-design.md).
Implementation not started.

## Context

The conversion is **cheap** — ~90% of the machinery already exists in the
task-0026 `volume_quote_usd` enrichment ASOF join; a USD close is the identical
join multiplying `close` instead of `volume_quote`. Two parts are the real work:

1. **Oracle ↔ asset-id reconciliation** (load-bearing). The backfill oracle
   extractor mints oracle assets in a synthetic id space `≥ 1_000_000` keyed by
   symbol/contract, never written to `prices.assets`
   (`packages/sdex-backfill/src/soroban.rs:42-56`). The enrichment join keys
   `o.asset_id = p.quote_asset_id`, so with backfilled data it **matches
   nothing** today — a latent bug shared with task 0026. Fix: resolve Reflector
   keys through the same `AssetRegistry` used for trades.
2. **Reflector-genesis boundary.** On-chain USD reference exists only from
   ~2024 (Soroban mainnet). 2024→now is fully coverable with a full-history
   backfill; pre-Soroban classic history has no on-chain oracle → `NULL`.

Delivered against the 0060 schema crate and the 0026 enrichment worker.

## Implementation Plan

### Step 1: Schema — `close_usd` column per grain
`ALTER TABLE prices.price_ohlcv_{1m,15m,1h,4h,1d,1w,1M} ADD COLUMN close_usd
Decimal(38,14) DEFAULT 0` (AS-copies don't inherit post-hoc ALTERs; apply per
table). Update the writer Row struct.

### Step 2: Oracle ↔ asset-id reconciliation (`soroban.rs`)
Replace synthetic `oracle_id()` with canonical `AssetRegistry` resolution so
oracle rows carry the same `asset_id` used as `quote_asset_id`; UPSERT reference
assets into `prices.assets`. Confirm Reflector key format vs captured samples.

### Step 3: Enrichment — compute `close_usd`
`enrichment-worker/src/ch_enrich.rs`: add
`CAST(o.price_usd * p.close AS Decimal(38,14)) AS close_usd` to the SELECT;
extend the candidate filter to `close_usd = 0`. Idempotency / `version+1` /
`FINAL` unchanged.

### Step 4: Rollup propagation
`schema/rollups.sql` + `schema/preroll.sql`: add `argMax(close_usd, timestamp)`
per grain (USD close aggregates like `close` — last value, not a sum).

### Step 5: View + API
`CREATE VIEW prices.price_usd_series` (reference assets direct from oracle;
others = volume-weighted USD close across quotes/sources). Optional endpoints:
`GET /assets/{id}/price/at?ledger=N` and `/price/history?from=&to=&interval=`.

## Acceptance Criteria

- [ ] `close_usd` column added to every OHLCV grain; writer Row populates it.
- [ ] Oracle rows carry canonical `prices.assets` `asset_id` (no synthetic space);
      enrichment ASOF join matches for backfilled data.
- [ ] Enrichment computes `close_usd` with the tiered reference: oracle USDC/XLM in
      the recent window, **USDC≡$1 (USDT≡$1) peg × XLM/USDC candle** for deep
      history (the primary pre-Reflector mechanism, not a fallback). Idempotent
      re-INSERT. (§12.1)
- [ ] SAC collapses to its underlying classic/native identity — **one `asset_id`,
      one price**; pure Soroban tokens keyed by `contract_address`. Requires the
      `AssetIdentity` contract/SAC resolver (`canonical.rs`, `sink.rs`). (§12.4)
- [ ] Rollup chain propagates `close_usd` onto forever-retained grains.
- [ ] `prices.price_usd_series` view: one USD close per (asset, bucket), keyed by
      **natural Stellar identity** (`native` / `(code,issuer)` / `contract_address`),
      not `asset_id`. (§12.2)
- [ ] NULL contract: `close_usd` NULL (never error, never drops row) +
      `status` discriminator `ok | no_asset_price | no_reference`, plus a companion
      `prices.usd_reference(bucket)` for systemic-blackout detection. (§12.3)
- [ ] Optional read API endpoints (single-asset primitive `price_usd_at`; also
      serves volume not just TVL — §12.5) implemented or deferred to 0040, noted.
- [ ] Tests/fixtures for the reconciliation + enrichment + view + NULL/status cases.

## Notes

- Design rationale, code sketches, the conversion identity, coverage contract,
  effort table (~1 week), and open questions are in
  [`notes/R-historical-usd-close-design.md`](notes/R-historical-usd-close-design.md).
- Open questions to close before/early in impl: Reflector asset-key format
  (symbol vs contract address); cross-source collapse policy (volume-weighted vs
  canonical-source priority); whether the production Oracle Fetcher (0039)
  already assigns `prices.assets` ids so backfill and live paths stay consistent;
  the concrete first XLM/USDC ledger once the production backfill range is locked
  (sets the deep-history USD floor — §12.1).
- BE-confirmed decisions captured in note §12 (2026-06-12): peg-pivot reference,
  natural-identity public key, NULL+status discriminator, one-row SAC collapse,
  single-asset primitive.
- The §5 reconciliation fix is **shared with task 0026** — coordinate so both
  paths land it once.
