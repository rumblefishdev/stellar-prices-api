---
id: "0078"
title: "Live ledger-processor must preload prices.pool_registry (AMM pools created before go-live are unresolved otherwise)"
type: BUG
status: completed
related_adr: ["0007"]
related_tasks: ["0070", "0053", "0069", "0054", "0077", "0081"]
tags: [layer-indexing, priority-high, effort-small, rust, lambda, clickhouse, amm, soroban, pool-registry, milestone-M1]
milestone: 1
links:
  - "../../../packages/prices-ledger-processor/src/main.rs"
  - "../../../packages/prices-ledger-processor/src/sink/mod.rs"
  - "../../../packages/prices-ingest-core/src/registry_io.rs"
  - "../../../packages/sdex-backfill/src/sink.rs"
history:
  - date: 2026-07-06
    status: done
    who: okarcz
    note: >
      Closed the two deferred acceptance criteria. Proved the seeded-vs-unseeded
      resolvability claim by extracting `process_ledger`'s classify block into a
      standalone `classify_amm_groups` (behaviour-preserving) and unit-testing it
      with the shared Phoenix xyk fixture — seeded venue reg → `amm_ticks`, empty
      → `unresolved` (Emerged #2). Full XDR E2E was fixture-blocked (bundled
      Galexie ledgers carry zero AMM activity; probed and confirmed) → spawned
      0081 for a real-fixture full-pipeline E2E. Decided persist-on-discovery:
      live path stays a **pure consumer** — 0053 backfill + 0069 discovery worker
      already own `pool_registry` persistence (Emerged #3). 31 prices-ingest-core
      tests green (+1 new); changed code clippy-clean (pre-existing crate lints
      untouched). Unblocks 0070 AMM live coverage.
  - date: 2026-07-03
    status: active
    who: oski
    note: >
      Implemented the preload. Added a shared `OhlcvWriter::load_pool_registry`
      (prices-ingest-core) as the single source of the pool_registry SELECT;
      the live sink delegates to it, and main.rs/cli.rs now pass the loaded
      Registries into Reconciler::new instead of Registries::new(). Refactored
      the backfill sink to delegate to the same shared method (dedup, wraps it in
      the existing startup retry). New live IT (pool_registry_preload_it) + the
      existing backfill pool_registry_it both pass against local CH 26.3.10.60;
      30+7+24 unit tests green; changed files clippy-clean. Remaining before done:
      full swap→priced-row E2E test, and the persist-on-discovery decision.
  - date: 2026-07-03
    status: backlog
    who: oski
    note: >
      Found during 0070 deploy prep. The live ledger-processor constructs its
      Reconciler with an EMPTY pool registry (main.rs:94 `Registries::new()`) and
      never loads `prices.pool_registry` — its sink only preloads `prices.assets`
      (AssetRegistry). So AMM swaps for any pool created BEFORE the live cursor
      start are never classified: they land in `prices.unresolved_pools` and their
      price/volume is lost. Since essentially all real AMM pools already exist,
      forward-only live ingestion yields ~zero useful AMM prices until this is
      wired. The persist/preload round-trip already exists (`registry_io`
      to_pool_rows/load_pool_rows; backfill's `load_pool_registry` at
      sdex-backfill/src/sink.rs:160) — it was simply never called on the live path
      ("registry-as-output" from 0069 was implemented for backfill only).
---

# Live ledger-processor must preload prices.pool_registry

## Summary

The live processor does not preload the discovered AMM `pool_registry`, so at
cold start it only knows pools whose factory-creation events appear in its own
live ledger stream (i.e. pools created after go-live). Every pre-existing AMM
pool's live swaps go to `prices.unresolved_pools` with volume lost. Wire the
live path to load `prices.pool_registry` at cold start (as the backfill already
does), so a seeded registry makes AMM live prices resolvable.

## Context

- SDEX is unaffected — assets are explicit in trades and the `AssetRegistry`
  (`prices.assets`) is preloaded + grown inline (`main.rs:80`).
- AMM needs pool→token classification from `pool_registry`, seeded from factory
  events by the backfill (`sdex-backfill/run.rs` → `write_pool_registry`).
- Gap: `prices-ledger-processor/src/main.rs:94` and `.../bin/cli.rs:87` pass
  `Registries::new()`; the live sink (`sink/mod.rs`) has `load_registry` (assets
  only) but **no** `load_pool_registry`. Reference impl exists on the backfill
  sink (`sdex-backfill/src/sink.rs:160`).
- Depends on the registry actually being seeded → coordinate with **0053**
  (backfill run that discovers + persists pools since Soroban activation).

## Implementation Plan

1. Add a `load_pool_registry()` to the live processor's `ClickHouseSink` (mirror
   `sdex-backfill/src/sink.rs:160`: `SELECT` `pool_registry` → `Registries`).
2. Call it at cold start in `main.rs` (and `cli.rs`) and pass the loaded
   `Registries` into `Reconciler::new` instead of `Registries::new()`.
3. **Decide + document persist-on-discovery for the live path:** when live
   processing discovers a *new* pool from a factory event, should it
   `write_pool_registry` so future cold starts / re-backfills see it? (Backfill
   persists; the live path currently would lose in-memory discoveries between
   cold starts.) Emerged design decision — record under Design Decisions.
4. Tests: cold-start preload rehydrates a seeded pool; an AMM swap for that pool
   resolves to a `price_ohlcv_1m` row (not `unresolved_pools`).

## Acceptance Criteria

- [x] Live processor loads `prices.pool_registry` at cold start into `Registries`
      (`main.rs` preloads via `ClickHouseSink::load_pool_registry`).
- [x] `cli.rs` path updated consistently (real branch preloads; dry-run stays empty).
- [x] Integration coverage for the preload: `pool_registry_preload_it` seeds the
      registry and asserts the live sink rehydrates all venues (green vs local CH).
- [x] With a seeded registry, an AMM **swap** for a pre-existing pool prices
      instead of falling to `unresolved` — proven at the `classify_amm_groups`
      seam (`soroban.rs`): seeded venue reg → `amm_ticks`, empty → `unresolved`,
      driven by the shared Phoenix xyk swap fixture (see Emerged #2 for why this
      seam rather than a full XDR `LedgerCloseMeta` fixture).
- [x] Persist-on-discovery behaviour for the live path decided + documented —
      **pure consumer** (Emerged #3).

## Implementation Notes

- New `OhlcvWriter::load_pool_registry()` in `prices-ingest-core/src/writer.rs` —
  the single home for the `SELECT … FROM prices.pool_registry FINAL` query + the
  `load_pool_rows` mapping. Both the live sink and the backfill sink delegate to
  it, so the two paths can never drift (the drift-avoidance spirit of 0077).
- Live sink: `ClickHouseSink::load_pool_registry` (thin delegate, `map_err(redact)`).
- Wiring: `main.rs` + `cli.rs` pass the loaded `Registries` into `Reconciler::new`;
  removed the now-unused `Registries` import in `main.rs`.
- Backfill: `sdex-backfill/src/sink.rs::load_pool_registry` now delegates to the
  shared method (kept its startup `retry_with_backoff`); dropped the duplicate
  query + `PoolRegistryRow` import.
- Tests: added `prices-ledger-processor/tests/pool_registry_preload_it.rs`;
  `clickhouse` + `extractors-core` added as dev-deps. Existing backfill
  `pool_registry_it` now also exercises the shared method.

## Design Decisions

### Emerged

1. **Shared method on `OhlcvWriter`, not a per-sink copy.** The plan said "mirror
   the backfill impl on the live sink," but copying the SQL is exactly the drift
   we're fixing. Put the query in `OhlcvWriter` (where `load_assets` already lives)
   and had both sinks delegate — one source of truth, backfill keeps its retry wrapper.

2. **Proved resolvability at a `classify_amm_groups` seam, not a full XDR E2E.**
   The remaining "swap → priced row" proof was blocked on fixtures: the bundled
   Galexie ledgers (62460540–42) carry **zero** AMM activity, and no synthetic
   `LedgerCloseMeta` builder exists (the decode/extract path is entirely XDR-native
   via `xdr_parser::extract_events`). Rather than hand-roll XDR or fetch a new
   real-AMM ledger (both disproportionate to an `effort-small` bug), extracted the
   registry-classification block of `process_ledger` into a standalone
   `classify_amm_groups(groups, reg, assets, …)` and unit-tested the exact 0078
   claim: the same Phoenix swap group prices to an `amm_tick` when the pool is
   seeded in `reg.venue` and only falls to `unresolved` when the registry is empty.
   Reuses the shared `phoenix_extractor::test_fixtures` swap fixture; hermetic, no
   ClickHouse. Behaviour-preserving refactor (the block moved verbatim). If a real
   full-pipeline AMM E2E is ever wanted, it's tracked as spawned follow-up 0081.

3. **Live path is a pure consumer of `pool_registry` (no persist-on-discovery).**
   The live processor loads `pool_registry` at cold start and grows its registry
   from live factory events *in-memory* within a warm container, but does **not**
   `write_pool_registry`. Persistence is already owned elsewhere: the 0053 backfill
   and the 0069 asset-discovery worker (now maintaining `pool_registry`, PR #80)
   both persist discovered pools. Making the live Lambda also write would duplicate
   that responsibility and risk write contention on the shared Hetzner CH from the
   hot path. Residual gap — a pool created live *and* first traded before the next
   discovery-worker run *and* the container cold-starts in between — is narrow,
   self-heals on the next discovery/backfill pass, and its dropped swaps are already
   visible in `unresolved_pools` for the post-run re-check. Accepted.

## Issues Encountered

- **No AMM ledger fixture available.** Probed all three bundled Galexie fixtures
  through `process_ledger`: `amm_ticks=0, unresolved=0, venues=0` for every ledger
  — they predate/miss AMM swaps, so a full-pipeline reconcile E2E couldn't be
  driven from committed data. Resolved by testing the classify seam directly
  (Emerged #2); a full-pipeline E2E with a real AMM fixture is spawned as 0081.

## Future Work

- **0081** — full-pipeline AMM reconcile E2E with a real Galexie fixture (the
  end-to-end version of this task's classify-seam unit test, once an AMM-bearing
  ledger fixture exists).

## Notes

- Gates **meaningful AMM live coverage** for the 0070 rollout; SDEX live is fine
  without it. Sequencing: seed registry (0053) → this fix → AMM live is useful.
