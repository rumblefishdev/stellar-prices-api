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
  - date: 2026-06-15
    status: active
    who: claude
    note: >
      Merged develop (0060 deliverables) into the 0061 branch. Landed the
      data-plane plumbing: Step 1 (close_usd column on all 7 grains + writer
      Row), Step 2 (oracle↔asset-id reconciliation — removed synthetic id
      space, resolved Reflector SYMBOL keys via the canonical AssetRegistry;
      also fixes the latent 0026 join bug), Step 3 recent-window oracle tier
      (close_usd = oracle_usd × close in ch_enrich.rs), Step 4 (argMax close_usd
      in rollups + preroll). +7 unit tests; cargo build --workspace + all unit
      tests green. Resolved the Reflector-key-format open question (symbols).
      Deferred: Step 3 peg-pivot deep-history tier (§12.1), SAC resolver
      (§12.4), Step 5 view + status discriminator + API (§12.2/§12.3).
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

**Current state:** Data-plane plumbing landed and verified (build + unit tests
green) on `feat/0061_historical-usd-close-price-series` after merging the 0060
deliverables from `develop`. Done: Step 1 (schema column + writer), Step 2
(oracle↔asset-id reconciliation, the load-bearing fix — also fixes the latent
0026 join bug), Step 3 recent-window oracle tier, Step 4 (rollup/preroll
propagation). Remaining: Step 3 deep-history peg-pivot tier (§12.1), the SAC
resolver (§12.4), and Step 5 (view + status discriminator + API, §12.2/§12.3).
Full design in
[`notes/R-historical-usd-close-design.md`](notes/R-historical-usd-close-design.md).

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

- [x] `close_usd` column added to every OHLCV grain; writer Row populates it.
      (init.sql base table + 7 idempotent `ALTER … ADD COLUMN IF NOT EXISTS`;
      `OhlcvRow.close_usd` writes DEFAULT 0, enrichment fills it.)
- [x] Oracle rows carry canonical `prices.assets` `asset_id` (no synthetic space);
      enrichment ASOF join matches for backfilled data. (Synthetic `oracle_id()`
      space removed; `reflector_key_to_identity` resolves via `AssetRegistry`.)
- [~] Enrichment computes `close_usd` with the tiered reference: oracle USDC/XLM in
      the recent window **(done)**, **USDC≡$1 (USDT≡$1) peg × XLM/USDC candle** for
      deep history **(deferred — peg-pivot tier not yet implemented)** (the primary
      pre-Reflector mechanism, not a fallback). Idempotent re-INSERT. (§12.1)
- [ ] SAC collapses to its underlying classic/native identity — **one `asset_id`,
      one price**; pure Soroban tokens keyed by `contract_address`. Requires the
      `AssetIdentity` contract/SAC resolver (`canonical.rs`, `sink.rs`). (§12.4)
- [x] Rollup chain propagates `close_usd` onto forever-retained grains.
      (`argMax(close_usd, timestamp)` in all 6 rollup MVs + 6 preroll INSERTs.)
- [ ] `prices.price_usd_series` view: one USD close per (asset, bucket), keyed by
      **natural Stellar identity** (`native` / `(code,issuer)` / `contract_address`),
      not `asset_id`. (§12.2)
- [ ] NULL contract: `close_usd` NULL (never error, never drops row) +
      `status` discriminator `ok | no_asset_price | no_reference`, plus a companion
      `prices.usd_reference(bucket)` for systemic-blackout detection. (§12.3)
- [ ] Optional read API endpoints (single-asset primitive `price_usd_at`; also
      serves volume not just TVL — §12.5) implemented or deferred to 0040, noted.
- [~] Tests/fixtures for the reconciliation + enrichment + view + NULL/status cases.
      (Reconciliation unit tests done; enrichment/view/NULL-status integration
      tests pending Step 5.)

## Implementation Notes

Landed on `feat/0061_historical-usd-close-price-series` (after merging `develop`,
which carried the 0060 deliverables: the `prices-clickhouse` schema crate, the
soroban oracle extractor, the rollup/preroll SQL). All changes build clean
(`cargo build --workspace`) and unit tests pass.

- **Step 1 — schema + writer.** `packages/prices-clickhouse/schema/init.sql`:
  `close_usd Decimal(38,14) DEFAULT 0` added to the base `price_ohlcv_1m` CREATE
  (so fresh `AS`-copies inherit it) **plus** 7 idempotent
  `ALTER TABLE … ADD COLUMN IF NOT EXISTS close_usd … AFTER volume_quote_usd` (one
  per grain — for DBs created before 0061, where `AS`-copies don't inherit a
  post-hoc base ALTER). `packages/sdex-backfill/src/sink.rs`: `OhlcvRow.close_usd:
  i128` written as `0` (enrichment fills it, mirroring `volume_quote_usd`). Updated
  the `init_sql_parses_into_statements` count 13 → 20.
- **Step 2 — reconciliation (load-bearing).** `packages/sdex-backfill/src/soroban.rs`:
  deleted the synthetic `oracle_id()` / `oracle_ids` / `oracle_next` (`≥1_000_000`)
  space from `Registries`; added `reflector_key_to_identity()` mapping the symbol
  keys to canonical `AssetIdentity`, resolved through the trade `AssetRegistry`
  (`get_or_assign`). Oracle rows now carry the same `asset_id` used as
  `quote_asset_id` → the enrichment ASOF join matches backfilled data. `USDC_ISSUER`
  /`USDT_ISSUER` promoted to `pub(crate)` in `canonical.rs` (single source of truth).
  4 new unit tests, incl. the USDC oracle-row == trade-quote-id guarantee.
- **Step 3 — enrichment (recent-window tier).**
  `packages/enrichment-worker/src/ch_enrich.rs`: added
  `CAST(o.price_usd * p.close AS Decimal(38,14)) AS close_usd` to the SELECT +
  `close_usd` to the INSERT column list; candidate filter / count extended to
  `(volume_quote_usd = 0 OR close_usd = 0)` so already-volume-enriched rows still
  get `close_usd` backfilled. The deep-history peg-pivot tier (§12.1) is **not**
  implemented — deep-history candles stay at `close_usd = 0` (never a wrong value).
- **Step 4 — rollup propagation.** `schema/rollups.sql` + `schema/preroll.sql`:
  `argMax(close_usd, timestamp) AS close_usd` after `volume_quote_usd` in all 6
  rollup MVs and 6 preroll INSERTs (USD close is a last-value, not a sum). Position
  matches the table column order so the positional `INSERT … SELECT` stays aligned.

## Design Decisions

### From Plan

1. **`close_usd` is enrichment-filled, writer writes 0** — exactly mirrors
   `volume_quote_usd` (sink.rs), per §7. The writer never multiplies; the
   ASOF-join enrichment does.

### Emerged

2. **Reflector keys are ticker symbols, not contract addresses** — resolved the
   §11.1 / §12 open question against
   `lore/4-notes/samples/soroban-events/REFLECTOR.jsonl`: keys are `"sym"` values
   (`XLM`, `USDC`, `USDT`, `EURC`, `BTC`, `EUR`, …). So `reflector_key_to_identity`
   is a symbol→identity match, and the design sketch's `is_contract_address`/SAC
   branch is dead for Reflector (omitted). Only the canonical quotes (`XLM`/`USDC`/
   `USDT`) get an identity; FX/crypto reference symbols return `None` and the sample
   is dropped (they never appear as a candle quote, so they can't match the join).
3. **`close_usd` added to the base CREATE *and* as idempotent per-grain ALTERs** —
   the base CREATE covers fresh installs (AS-copies inherit); the `ALTER … IF NOT
   EXISTS` covers pre-0061 DBs. Kept both in `init.sql` (still idempotent) rather
   than introducing a separate migrations mechanism this repo doesn't have.
4. **REDSTONE keyed to its emitting contract via `AssetIdentity::Contract`** — the
   plan only addressed Reflector. To fully delete the synthetic id space, REDSTONE
   (price_usd = 0, deferred decode, never read by the `reflector` join) now resolves
   its emitting oracle contract through the canonical registry instead of a synthetic
   id. Semantically loose (an oracle contract isn't a tradeable asset) but harmless
   and uniform; documented inline.
5. **Candidate filter widened to `OR close_usd = 0`** — the plan said "extend the
   filter to close_usd = 0". Made it `(volume_quote_usd = 0 OR close_usd = 0)` so
   rows already volume-enriched in a prior pass (before `close_usd` existed) still
   get picked up and back-filled. Safe because real candles always have `close > 0`,
   so the two USD columns flip together on an oracle hit.
6. **Enrichment changes scoped to `ch_enrich.rs` only** — the in-memory prototype
   (`enrich.rs`/`pass.rs`/`sink/sql_file.rs`) targets task 0026's legacy
   `prices.price_ohlcv` shape (with `granularity` + `_inserted_at`), not the real
   schema, and is not part of 0061's `close_usd` delivery. Left untouched.

## Future Work

- **Step 3 deep-history peg-pivot tier (§12.1)** — `close_usd` for candles with no
  in-window oracle row: USDC≡$1 / USDT≡$1 peg for stable-quoted candles, and the
  XLM/USDC candle as the pivot for XLM-quoted candles. The primary pre-Reflector
  mechanism; currently those candles carry `close_usd = 0`.
- **SAC → underlying-classic resolver (§12.4)** — `AssetIdentity` models only
  `Native`/`Credit`/`Contract`; SAC addresses don't yet collapse to their classic
  underlying, so AMM-via-SAC XLM/USDC quotes won't match the oracle `Native`/`Credit`
  identity. Needed for one-`asset_id`-one-price.
- **Step 5 — `prices.price_usd_series` view + `usd_reference(bucket)` companion +
  status discriminator (§12.2/§12.3) + read endpoints (§12.5)**.

## Notes

- Design rationale, code sketches, the conversion identity, coverage contract,
  effort table (~1 week), and open questions are in
  [`notes/R-historical-usd-close-design.md`](notes/R-historical-usd-close-design.md).
- Open questions to close before/early in impl: ~~Reflector asset-key format
  (symbol vs contract address)~~ **resolved: symbols (see Design Decision 2)**;
  cross-source collapse policy (volume-weighted vs
  canonical-source priority); whether the production Oracle Fetcher (0039)
  already assigns `prices.assets` ids so backfill and live paths stay consistent;
  the concrete first XLM/USDC ledger once the production backfill range is locked
  (sets the deep-history USD floor — §12.1).
- BE-confirmed decisions captured in note §12 (2026-06-12): peg-pivot reference,
  natural-identity public key, NULL+status discriminator, one-row SAC collapse,
  single-asset primitive.
- The §5 reconciliation fix is **shared with task 0026** — coordinate so both
  paths land it once.
