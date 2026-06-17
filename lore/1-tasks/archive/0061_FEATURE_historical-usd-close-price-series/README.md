---
id: "0061"
title: "Historical USD-quoted price series — price_usd(asset, t) primitive for Block Explorer LP analytics"
type: FEATURE
status: completed
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
  - date: 2026-06-15
    status: active
    who: claude
    note: >
      Implemented the Step 3 peg-pivot deep-history tier (§12.1) in
      ch_enrich.rs: run() now runs an oracle phase then a peg-pivot phase.
      Peg = USDC/USDT quotes × $1; pivot = XLM quotes × volume-weighted
      XLM/USDC close (ASOF forward-fill, keyed ref_asset_id for the ASOF
      equality). Fills close_usd + volume_quote_usd (no-clobber guard);
      exotic quotes stay 0. +4 unit tests. Validated against live
      ClickHouse in a scratch DB: correct peg/pivot values, idempotent
      re-INSERT, oracle-set volume_quote_usd preserved. cargo build
      --workspace + all unit tests green.
  - date: 2026-06-15
    status: active
    who: claude
    note: >
      Step 5 read-surface views (schema/views.sql, VIEWS_SQL): price_usd_series
      (volume-weighted USD close per natural-identity asset/bucket, §12.2) and
      usd_reference (per-bucket XLM/USDC pivot for blackout detection, §12.3),
      both on the forever-retained _1d grain reading baked close_usd (not the
      retention-capped oracle_prices). status discriminator is a documented
      read-time LEFT JOIN (a view can't enumerate untraded asset×bucket combos).
      Applied by default by prices-clickhouse-init. Float64 weighting fixes a
      Decimal-division overflow. +1 builder test; both views created and queried
      against live ClickHouse (correct native/credit/contract rows + weighting).
      HTTP endpoints deferred to 0040. Only §12.4 SAC resolver remains.
  - date: 2026-06-15
    status: active
    who: claude
    note: >
      Implemented the §12.4 SAC resolver — the last design item. AssetRegistry
      (canonical.rs) derives each classic asset's deterministic SAC address
      (sha256 of the ContractId preimage → C-strkey) into a reverse index
      (pre-seeded XLM/USDC/USDT + every interned classic); soroban.rs AMM path
      resolves SAC tokens to the classic identity before interning, collapsing
      AMM-via-SAC and SDEX-classic onto one asset_id. New deps stellar-strkey +
      sha2. +4 unit tests incl. exact match of the known native-XLM mainnet SAC
      CAS3…XOWMA. cargo build --workspace + all workspace tests green. All
      §12.1–§12.5 design items now implemented; remaining is durable live-CH
      integration tests and the 0040 HTTP layer.
  - date: 2026-06-15
    status: active
    who: claude
    note: >
      Added live-CH integration tests (#[ignore]d, isolated scratch DB built by
      rewriting the real schema onto the scratch name): ch_enrich_it.rs
      (oracle/peg/pivot tiers + no-reference + idempotency) and views_it.rs
      (natural-identity series + cross-quote weighting + usd_reference). Both
      pass against the local ClickHouse; skipped in normal runs so CI without CH
      stays green. enrichment-worker gains a prices-clickhouse dev-dep for
      INIT_SQL/apply_sql. Remaining: SAC end-to-end backfill check (needs ledger
      fixtures) and the 0040 HTTP layer.
  - date: 2026-06-15
    status: active
    who: claude
    note: >
      End-to-end validated the whole close_usd pipeline against the local CH over
      10k real mainnet ledgers (62030000-62039999): oracle reconciliation lands
      canonical asset_ids (no synthetic space), oracle+peg+pivot enrichment fills
      144603/222801 candles, rollup propagates to _1d, views return economically
      sound USD (XLM $0.158, BTC $71k, USDC depeg captured), idempotent re-run.
      Added examples/run_ch_pass.rs (local driver for the production pass).
      Reviewed BE 0199 contract (soroban-block-explorer lp-analytics): our impl
      matches all agreed points (write-time close_usd per grain, tiered reference,
      natural-identity key incl. XLM-as-native, NULL+discriminator+usd_reference,
      SAC resolver). One gap closed: views were _1d-only; the contract names
      1h/1d. Added price_usd_series_1h + usd_reference_1h (zero marginal storage —
      _1h already exists + forever-retained). Open w/ BE: grain-selection
      ownership, view-name confirmation (price_usd_at vs price_usd_series),
      optional {code}:{issuer} string key; operational: backfill to 2024-02-20.
  - date: 2026-06-15
    status: active
    who: claude
    note: >
      Decided grain-selection ownership (was open w/ BE): VIEWS = caller-passes
      (consumer JOINs the per-grain view its query needs — one consistent grain
      per chart, views stay retention-agnostic), and the 0040 HTTP point-lookup
      primitive price_usd_at(id,ts) = view-picks (maps ts → finest-retained grain).
      No view change (already shipped per-grain). Documented in views.sql header,
      design note §12.6, Design Decision 18; breadcrumb added to task 0040.
      Remaining w/ BE: confirm they own grain choice at the JOIN layer.
  - date: 2026-06-15
    status: active
    who: claude
    note: >
      BE interop round (§12.7). Documented the JOIN contract in views.sql
      (asset_code trimmed String; bucket grain-floored DateTime join on
      toStartOf{Hour,Day}(closed_at); close_usd Decimal(38,14)). Added two
      views: current_price_usd (live spot over current_prices for BE's
      ingest-time TVL materialization — natural-id key + updated_at; data from
      0039) and identity_by_contract (SAC read-seam — the §12.4 collapse is
      write-time so a SAC leg's price is under the classic identity; persist
      assets.sac_address via AssetRegistry::sac_address_of and resolve a leg's
      contract → natural identity). +2 unit/integration assertions; all 6 views
      validated against live CH. BE confirmed: JOIN price_usd_series directly
      (no alias), structured columns (no asset_key string). Operational ask from
      BE: backfill range must cover ledger 50457424 (2024-02-20).
  - date: 2026-06-17
    status: active
    who: claude
    note: >
      Code-review follow-up round (recall-biased /code-review of the 0061 diff →
      10 findings, all resolved). Fixes: #1 Tier-2 premature-peg gate on
      oracle_drained (020894c); #2 REDSTONE oracle feed kept out of the asset read
      surface via a reserved sentinel (5584644, revised Design Decision 4); #3 EURC
      USD/XLM-only reference scope — doc only (79cab22); #4 oracle-tier
      volume_quote_usd write-once guard + #7 warn on zero-enrichment backlog + #8
      USDC/USDT issuers de-duplicated into prices-clickhouse (924dfe0); #6 views
      enforce contract⇒asset_code='' (db2720f); #5 enrichment pass snapshot-bound to
      a max(timestamp) watermark, run()→run_through (4094c2d); #9 network-scoped SAC
      silent-failure documented (9cfd789); #10 part 1 XLM/USDC pivot reference
      materialized once per run (ab673d6), part 2 (rows-affected) documented as
      blocked by the pinned clickhouse 0.13 execute() API. Design Decisions 19–27
      added. All unit + live-CH integration tests green (+ new watermark, contract-
      normalization, pivot-ref, and reference-cleanup test coverage).
  - date: 2026-06-17
    status: completed
    who: claude
    note: >
      Done. All 9 acceptance criteria met; §12.1–§12.5 design items implemented,
      unit + live-CH integration tested, and validated end-to-end over 10k real
      mainnet ledgers. Code-review round closed (10 findings, Design Decisions
      19–27). Delivered via PR #39 (→ develop). Remaining breadth is tracked in
      other tasks: HTTP read endpoints → 0040; enrichment rows-affected progress →
      backlog 0062 (gated on a clickhouse-crate upgrade); SAC end-to-end backfill
      validation noted in Future Work (needs ledger XDR fixtures).
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

**Current state:** Data-plane landed and verified (build + unit tests green; the
peg-pivot tier additionally validated against live ClickHouse) on
`feat/0061_historical-usd-close-price-series` after merging the 0060 deliverables
from `develop`. Done: Step 1 (schema column + writer), Step 2 (oracle↔asset-id
reconciliation, the load-bearing fix — also fixes the latent 0026 join bug),
Step 3 **both enrichment tiers** (recent-window oracle + deep-history peg-pivot,
§12.1), Step 4 (rollup/preroll propagation), Step 5 (the `price_usd_series` +
`usd_reference` read-surface views, §12.2/§12.3; HTTP endpoints deferred to 0040),
and the §12.4 SAC→underlying resolver. **All design items (§12.1–§12.5) are now
implemented.** Remaining is hardening (durable live-CH integration tests) and the
0040 HTTP layer. Full design in
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
- [x] Enrichment computes `close_usd` with the tiered reference: oracle USDC/XLM in
      the recent window, **USDC≡$1 (USDT≡$1) peg × XLM/USDC candle** for deep
      history (the primary pre-Reflector mechanism, not a fallback). Idempotent
      re-INSERT. (§12.1) — both tiers implemented in `ch_enrich.rs`; peg + pivot
      validated against live ClickHouse (correct values, idempotent, no-clobber of
      an oracle-set `volume_quote_usd`).
- [x] SAC collapses to its underlying classic/native identity — **one `asset_id`,
      one price**; pure Soroban tokens keyed by `contract_address`. (§12.4) —
      `AssetRegistry` derives each classic asset's deterministic SAC address
      (`sha256(ContractId preimage)` → `C`-strkey) into a reverse index; the AMM
      path resolves SAC tokens to the classic identity before interning. Verified:
      the derivation reproduces the known native-XLM mainnet SAC exactly.
- [x] Rollup chain propagates `close_usd` onto forever-retained grains.
      (`argMax(close_usd, timestamp)` in all 6 rollup MVs + 6 preroll INSERTs.)
- [x] `prices.price_usd_series` view: one USD close per (asset, bucket), keyed by
      **natural Stellar identity** (`native` / `(code,issuer)` / `contract_address`),
      not `asset_id`. (§12.2) — `schema/views.sql`; validated against live CH.
      Provided at **both 1h and 1d grains** (`price_usd_series` / `_1h`,
      `usd_reference` / `_1h`) per the BE 0199 contract (1h/1d).
- [x] NULL contract: `close_usd` NULL (never error, never drops row) +
      `status` discriminator `ok | no_asset_price | no_reference`, plus a companion
      `prices.usd_reference(bucket)` for systemic-blackout detection. (§12.3) —
      `usd_reference` view shipped; `no_asset_price` is inherently a miss-condition
      so the discriminator is a **documented read-time** join of the two views
      (a view can't enumerate untraded (asset×bucket) combos). Misses are absent
      rows (NULL on the reader's LEFT JOIN), never errors.
- [x] Optional read API endpoints (single-asset primitive `price_usd_at`; also
      serves volume not just TVL — §12.5) **deferred to 0040** (the Prices API
      Gateway + axum read-handlers task), noted. The views are the primary delivery
      surface — BE reads `prices.*` directly (§8), HTTP is 0040's concern.
- [x] Tests/fixtures for the reconciliation + enrichment + view + NULL/status cases.
      Unit: reconciliation, peg/pivot SQL builders, SAC derivation (vs the known
      native SAC). Integration (`#[ignore]`d, live CH, isolated scratch DB):
      `enrichment-worker/tests/ch_enrich_it.rs` (oracle/peg/pivot tiers +
      no-reference + idempotency) and `prices-clickhouse/tests/views_it.rs`
      (natural-identity series + cross-quote weighting + `usd_reference`). Both pass
      against a local ClickHouse.

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
- **Step 3 — enrichment, both tiers.**
  `packages/enrichment-worker/src/ch_enrich.rs`. **Tier 1 (oracle):** added
  `CAST(o.price_usd * p.close AS Decimal(38,14)) AS close_usd` to the SELECT +
  `close_usd` to the INSERT column list; candidate filter / count extended to
  `(volume_quote_usd = 0 OR close_usd = 0)` so already-volume-enriched rows still
  get `close_usd` backfilled. **Tier 2 (peg-pivot, §12.1):** `run()` now runs a
  second phase over what the oracle tier left at `close_usd = 0` —
  `enrich_peg_pivot_step` issues a **peg** statement (USDC/USDT quotes →
  `close_usd = close × $1`) and a **pivot** statement (XLM quotes →
  `close_usd = close × xlm_usd`, where `xlm_usd` is the volume-weighted XLM/USDC
  candle close, ASOF-forward-filled). Reference `asset_id`s resolved from
  `prices.assets` via `resolve_reference_ids`. Exotic-quote candles stay
  `close_usd = 0`. +4 unit tests; peg + pivot exercised against live ClickHouse
  (correct values, idempotent re-INSERT, oracle-set `volume_quote_usd` preserved).
- **Step 4 — rollup propagation.** `schema/rollups.sql` + `schema/preroll.sql`:
  `argMax(close_usd, timestamp) AS close_usd` after `volume_quote_usd` in all 6
  rollup MVs and 6 preroll INSERTs (USD close is a last-value, not a sum). Position
  matches the table column order so the positional `INSERT … SELECT` stays aligned.
- **Step 5 — read-surface views.** New `packages/prices-clickhouse/schema/views.sql`
  (`VIEWS_SQL` const; applied by default by `prices-clickhouse-init` after the
  tables, before the opt-in rollups). Two plain views, both keyed by natural
  Stellar identity and reading the baked `close_usd` (NOT the retention-capped
  `oracle_prices`), both on the forever-retained `price_ohlcv_1d` grain:
  - `prices.price_usd_series` — one volume-weighted USD close per
    (`asset_kind` ∈ native/credit/contract, `asset_code`, `issuer_address`,
    `contract_address`, `bucket`); cross-source/cross-quote collapse per ADR 0004.
  - `prices.usd_reference` — per-bucket XLM/USDC pivot value; a bucket's presence
    is the durable "USD reference is up at T" signal for blackout detection.
  Read-time status (`ok | no_asset_price | no_reference`) is documented in the
  file header as a LEFT-JOIN of the two views. +1 builder test; both views created
  and queried against live ClickHouse (correct natural-identity rows + weighting).
  HTTP endpoints deferred to task 0040. Provided at **1h and 1d grains** (4 views)
  per the BE 0199 contract — the daily collapses a whole day to one close, hourly
  serves read-time TVL keyed to a ledger's `closed_at`. Zero marginal cost: both
  `_1h`/`_1d` OHLCV tables already exist + carry `close_usd` and are forever-
  retained; a plain view is stored query text only (measured ~2× read vs daily,
  single-digit ms). Integration test covers both grains.
- **§12.4 — SAC resolver.** `packages/sdex-backfill/src/canonical.rs`: `AssetRegistry`
  now derives each classic asset's deterministic Stellar Asset Contract address —
  `C`-strkey of `sha256(HashIdPreimage::ContractId{network, Asset})` — into a
  `sac_index` (SAC address → classic identity), pre-seeded with XLM/USDC/USDT and
  populated for every interned classic. `soroban.rs` `amm_trade_to_tick` resolves
  each AMM token via `resolve_sac` before canonicalising, so a SAC token collapses
  onto its classic `asset_id` (one row, one price); a pure Soroban token keeps its
  `Contract(address)` identity. New deps: `stellar-strkey`, `sha2`. +4 unit tests,
  anchored on the known native-XLM mainnet SAC (`CAS3…XOWMA`).
- **Live-CH integration tests.** `#[ignore]`d (run with `cargo test -- --ignored`
  against a local ClickHouse), each in an isolated scratch database built by
  rewriting the real `prices.*` schema onto the scratch name (so the schema stays
  the single source of truth and `prices` is never touched), dropped at the end:
  - `enrichment-worker/tests/ch_enrich_it.rs` — `ChEnrichmentPass::run()` over a
    fixture spanning a recent oracle-covered candle, deep peg (USDC) + pivot (XLM)
    candles, and an exotic-quote candle; asserts the per-tier `close_usd`, that the
    oracle value beats the peg, that an exotic quote stays 0, and idempotency.
  - `prices-clickhouse/tests/views_it.rs` — applies schema + views, asserts the
    natural-identity rows, the cross-quote volume-weighting, and `usd_reference`.

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
4. **REDSTONE oracle feed carries a reserved no-asset sentinel** (revised — see
   below) — the plan only addressed Reflector. *Originally* (to fully delete the
   synthetic id space) REDSTONE resolved its emitting oracle contract through the
   canonical registry via `AssetIdentity::Contract` instead of a synthetic id.
   **Superseded by review finding #2:** interning the oracle contract persisted it
   to `prices.assets` and leaked it into the contract-keyed read surface
   (`identity_by_contract`, `current_price_usd`), where a consumer resolving a
   pool-leg contract address could match an oracle feed as if it were a token. Fix
   at the source: `decode_redstone` no longer interns anything — the row carries a
   reserved `ORACLE_FEED_NO_ASSET_ID = 0` sentinel the registry never assigns (ids
   start at 1), so the feed stays out of `prices.assets` entirely and no read
   surface can surface it. Safe because the REDSTONE `asset_id` was always dead
   (price_usd = 0, oracle_name = 'redstone', never read by the `reflector` ASOF
   join); the raw payload is still recorded for the byte-footprint measurement.
   (Commit `5584644`.)
5. **Candidate filter widened to `OR close_usd = 0`** — the plan said "extend the
   filter to close_usd = 0". Made it `(volume_quote_usd = 0 OR close_usd = 0)` so
   rows already volume-enriched in a prior pass (before `close_usd` existed) still
   get picked up and back-filled. Safe because real candles always have `close > 0`,
   so the two USD columns flip together on an oracle hit.
6. **Enrichment changes scoped to `ch_enrich.rs` only** — the in-memory prototype
   (`enrich.rs`/`pass.rs`/`sink/sql_file.rs`) targets task 0026's legacy
   `prices.price_ohlcv` shape (with `granularity` + `_inserted_at`), not the real
   schema, and is not part of 0061's `close_usd` delivery. Left untouched.
7. **Peg and pivot are two separate statements, not one COALESCE** — the peg is a
   timeless constant (`$1`) with no join; the pivot is a time series needing an
   `ASOF` forward-fill. They don't unify cleanly under one ASOF (a constant has no
   timestamps), so the step runs peg then pivot over the disjoint quote sets
   (USDC/USDT vs XLM).
8. **Pivot ASOF needs an equality predicate** — ClickHouse `ASOF JOIN` requires an
   equi-condition, but all XLM candles pivot against the *same* XLM/USDC series.
   Solved by keying the pivot subquery as `{xlm_id} AS ref_asset_id` and joining
   `ON r.ref_asset_id = p.quote_asset_id AND r.timestamp <= p.timestamp` — natural
   ("the USD price of the candle's quote asset") and extensible to more pivots.
9. **Float64 for the volume-weighted pivot multiplier** — `sum(close × volume_base)`
   in `Decimal(38,14)` can overflow; the weighted average is computed in Float64
   then cast back to `Decimal(38,14)`. Ample precision for a USD reference price.
10. **Peg-pivot fills BOTH `close_usd` and `volume_quote_usd`** (the latter only
    when still 0, via `if(volume_quote_usd > 0, …)`) — the same quote-USD multiplier
    applies to both, it keeps the candidate set converging cleanly, and the guard
    guarantees an oracle-set (depeg-aware) `volume_quote_usd` is never overwritten
    by the `$1` peg. Validated against live CH.
11. **`pivot_window_s` default = 1 day** (vs the oracle tier's 300s) — deep history
    is sparser, and XLM/USDC is liquid so a 1-day forward-fill bound rarely bites;
    configurable via `PIVOT_WINDOW_S`.
12. **`status` discriminator is read-time, not a stored column** — `no_asset_price`
    means "this asset has no row at bucket T", which a view cannot enumerate (it
    would need the full asset×bucket grid). So `price_usd_series` carries only
    priced rows; the reader classifies a miss by LEFT-JOINing `usd_reference`
    (bucket present ⇒ `no_asset_price`, absent ⇒ `no_reference`). The canonical
    query is documented in the `views.sql` header. This matches §12.3's own
    "Computable: … BE can LEFT JOIN" framing.
13. **Views keyed/grouped by natural identity, built on `_1d`, Float64 weighting** —
    GROUP BY (code, issuer, contract, bucket) resolves `asset_id`→identity and
    incidentally collapses any duplicate ids for the same identity; `_1d` is the
    forever-retained series grain (§8); the volume-weighted average is computed in
    Float64 then cast back to `Decimal(38,14)` (same `Decimal`-overflow fix as the
    pivot — ClickHouse rejected the all-`Decimal` division at view-creation time).
14. **Views applied by default in `prices-clickhouse-init`** (not behind a flag like
    `--rollups`) — they are plain views with no ClickHouse-version constraint and
    are the core read surface, so they should always exist after a schema apply.
15. **HTTP read endpoints deferred to task 0040** — §12.5/Step 5 explicitly allow
    "implemented or deferred to 0040, noted". 0040 owns the API Gateway + axum
    handlers; BE consumes the views directly over the shared cluster (§8), so the
    view *is* the delivery here and the HTTP layer is genuinely 0040's scope.
16. **SAC resolved by forward derivation, not reverse lookup** — a SAC address is a
    one-way hash, so we derive each *known classic* asset's SAC and index it, rather
    than trying to invert an arbitrary contract address. Sufficient because the
    split only matters when an asset trades on BOTH SDEX (classic → in the registry,
    so its SAC is indexed) and AMM; a token seen only on AMM has no classic form to
    merge with, so leaving it as `Contract(address)` splits nothing.
17. **SAC index pre-seeded with XLM/USDC/USDT + mainnet hard-coded** — covers the
    load-bearing quotes regardless of intra-run sighting order (an AMM SAC swap
    processed before that asset's first SDEX trade). Network passphrase is fixed to
    mainnet (the backfill is mainnet-only); make it a parameter if testnet is ever
    needed. Residual gap (documented): an asset *first* seen as a SAC on AMM,
    strictly before any classic sighting and not in the loaded registry, won't
    collapse that run — self-heals next run once the classic is persisted.
18. **Grain-selection ownership: views = caller-passes, 0040 API = view-picks**
    (decided 2026-06-15; §12.6). The in-cluster views are per-grain and the
    consumer JOINs the grain its query needs (one consistent grain per chart;
    views stay a dumb, retention-agnostic surface). The "finest-retained-for-T"
    routing belongs to the 0040 point-lookup endpoint (`price_usd_at(id, ts)`),
    not the views. No new view design — already shipped per-grain. Documented in
    `views.sql` header + design note §12.6.

### Emerged — code-review follow-ups (2026-06-16/17)

A recall-biased `/code-review` of the 0061 diff surfaced 10 findings; all are now
resolved (commits on `feat/0061_historical-usd-close-price-series`). The
load-bearing decisions:

19. **Tier-2 gated on the oracle tier having *drained*, not on `remaining > 0`**
    (#1, commit `020894c`). The Tier-1 loop exits either by no-progress *or* by
    exhausting `max_batches` while still making progress; firing Tier 2 on the
    latter pegged unreached oracle-eligible USDC/USDT candles to a flat $1 (sticky
    once `close_usd > 0`). Added an `oracle_drained` flag; un-drained leftovers
    roll to the next run's oracle tier instead of being pegged.
20. **EURC stays out of the reference set by design — doc only** (#3, commit
    `79cab22`). EURC is Reflector-covered *and* a tradeable Stellar classic, so an
    EURC-quoted candle reads as `no_reference`. Confirmed intended: the USD-close
    reference set is deliberately **USD-pegged stables + XLM only** (we do not
    price through a EUR stable). Corrected the `reflector_key_to_identity` comment
    to distinguish "no Stellar identity" (EUR/BTC/XAU) from "deliberately out of
    scope" (EURC); no code change.
21. **Oracle tier makes `volume_quote_usd` write-once** (#4, commit `924dfe0`).
    Added the `if(volume_quote_usd > 0, …)` guard the peg/pivot statements already
    had, so a row re-admitted by the widened filter (#5 of From-Plan) can't have a
    depeg-aware value silently rewritten from a different ASOF match; `close_usd`
    stays unconditional (it is the column the pass owns).
22. **Reconciliation-failure backlog gets a `warn!`** (#7, commit `924dfe0`). A
    pass that enriches 0 rows over a non-empty backlog is the fingerprint of the
    failure 0061 fixes (oracle↔id join matches nothing / no reference assets);
    warn rather than let it go info-only. Low-noise: only fires when both tiers do
    nothing.
23. **USDC/USDT issuers de-duplicated into `prices-clickhouse`** (#8, commit
    `924dfe0`). The load-bearing join keys were hand-synced literals across 4+ Rust
    sites; promoted to a single `pub` const in `prices-clickhouse`, re-exported by
    `sdex-backfill` and `enrichment-worker`. The `views.sql` copy stays a SQL
    literal (SQL can't reference a Rust const) but is annotated as hand-synced.
24. **Natural-identity views enforce `contract ⇒ asset_code=''`** (#6, commit
    `db2720f`). `price_usd_series`, `_1h`, and `current_price_usd` now force
    `asset_code`/`issuer_address` to `''` for the `contract` kind
    (`if(contract_address != '', '', …)`) rather than passing them through, so the
    documented JOIN-interop contract holds even if discovery/metadata ever
    populates a Soroban token's symbol.
25. **Enrichment pass is snapshot-bound to a watermark** (#5, commit `4094c2d`).
    Progress/termination was inferred from a global `count_candidates()` delta,
    which a concurrent live insert (0038) could inflate, falsely tripping the
    no-progress break. Capture `watermark = max(timestamp)` at start and bound
    every count + enrich statement to `timestamp <= watermark`; newer candles roll
    to the next run. Split `run()` → `run_through(watermark)` for testability.
26. **Network-scoped SAC silent-failure documented, not parameterized** (#9, commit
    `9cfd789`). The mainnet passphrase is hard-coded in `from_existing`; a
    non-mainnet run would silently derive wrong SACs. Per agreed scope, expanded
    the `MAINNET_PASSPHRASE` note to spell out the failure mode and name the single
    bake-in point to parameterize, rather than threading network through config.
27. **XLM/USDC pivot reference materialized once per run** (#10 part 1, commit
    `ab673d6`). The pivot re-aggregated the whole XLM/USDC series under `FINAL` on
    every batch; now materialized once into a run-scoped MergeTree table
    (watermark-bounded, uniquely named, dropped on all paths) that the per-batch
    pivot ASOF-joins. #10 part 2 (per-batch `FINAL` count scans → rows-affected) is
    **documented but not done**: the pinned `clickhouse` 0.13 `execute()` doesn't
    surface `X-ClickHouse-Summary`, so it would need a raw-HTTP bypass; the #25
    watermark at least pins each count to a fixed population.

## Future Work

All design items (§12.1–§12.5) are implemented, with unit + live-CH integration
tests for the enrichment tiers and the views. Remaining is handoff / breadth:

- **End-to-end backfill validation of the SAC collapse** — the derivation is unit-
  tested against the known native SAC and the registry collapse is unit-tested; a
  full backfill over a window with both SDEX-classic and AMM-via-SAC trades of the
  same asset would confirm one merged `asset_id`/`price_usd_series` row end to end
  (needs ledger XDR fixtures — the gitignored `fixtures/ledgers/`).
- **HTTP read endpoints (§12.5)** — deferred to task 0040 (API Gateway + axum).
- **Enrichment progress via INSERT rows-affected** — code-review #10 part 2,
  spawned as backlog task **0062** (gated on a `clickhouse`-crate upgrade that
  surfaces `written_rows`; see Design Decision 27).

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
