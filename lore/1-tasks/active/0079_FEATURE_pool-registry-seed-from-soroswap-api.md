---
id: "0079"
title: "Seed prices.pool_registry from the Soroswap /pools API (all venues) — fast live-AMM unblock for 0070"
type: FEATURE
status: active
related_adr: ["0007"]
related_tasks: ["0070", "0078", "0053", "0069", "0035"]
tags: [layer-indexing, priority-high, effort-small, amm, pool-registry, soroswap, phoenix, aquarius, clickhouse, cli, rust]
links:
  - "https://api.soroswap.finance/docs"
history:
  - date: 2026-07-03
    status: active
    who: okarcz
    note: >
      Created + activated. Discovered while sizing the 0053 backfill as 0070's
      AMM-coverage gate: the Soroswap API (bearer-auth) exposes GET /pools for
      network=mainnet & protocol in {soroswap, phoenix, aqua, sdex}, returning
      per-pool address + token pair + poolType. That is the whole pool→token
      classification the live processor needs — retrievable in 3 calls (~541
      pools) instead of a ~3-day activation→tip ledger replay. Build a one-off
      CLI seeder so 0070 can go live for AMM without waiting on the full backfill.
---

# Seed `prices.pool_registry` from the Soroswap `/pools` API

## Summary

A one-off CLI that fetches the current AMM pool set from the Soroswap API and
writes it into `prices.pool_registry`, giving the live ledger-processor (0078)
the pool→token classification it needs to price live AMM swaps for **pre-existing**
pools — without the ~3-day activation→tip backfill the registry seed would
otherwise require. This is the practical unblock for 0070's AMM live coverage.

## Context

- The live processor only learns pools from factory events in its own forward
  stream, so pre-existing pools go unresolved unless `pool_registry` is seeded
  (task 0078). The only seeding path today is the 0053 backfill replaying every
  ledger since Soroban activation (~12.8M ledgers, ~3 days, archive-sync-bound).
- The Soroswap API (`GET /pools?network=mainnet&protocol=…`, bearer JWT / `sk_`
  key) returns the *current* pool set directly. Confirmed live (2026-07-03):
  mainnet indexer covers `soroswap` (199 pools), `phoenix` (12), `aqua` (330),
  `sdex`; each pool object carries `protocol, address, tokenA, tokenB, poolType`.
- This is the "factory-registry seeding" the AMM historical-pool-discovery gap
  flagged as missing for Soroswap/Phoenix. Reuses the shared
  `OhlcvWriter::write_pool_registry` extracted in 0069.
- Ongoing coverage stays complete: this seeds pre-existing pools; the live
  factory-event stream + 0069's asset-discovery maintenance catch new ones. It
  does **not** replace the 0053 historical **OHLCV** backfill — only the
  registry seed.

## Implementation

- New one-off CLI (e.g. `pool-registry-seed`): read `SOROSWAP_API_KEY` from env,
  `GET /pools?network=mainnet&protocol=<v>` for each AMM venue, build a
  `Registries`, call `writer.write_pool_registry(&reg)` (mTLS to prod or local CH).
- Normalization (mandatory): `protocol "aqua" → venue "aquarius"`; **drop
  `sdex`** (order-book, not a pool); `poolType "xyk" → pool_type 0`, log + skip
  any unknown `poolType`. Soroswap/Phoenix store `token0=tokenA, token1=tokenB`;
  Aquarius stores venue only (registry keeps no token detail for it — extractor
  reads tokens from the swap event).
- Fetch **unfiltered** (omit `assetList`) so the full pool set is captured.
- Idempotent by construction (RMT on `contract_id`) — safe to re-run.

## Acceptance Criteria

- [x] CLI fetches all AMM venues (soroswap/phoenix/aquarius) with the env-var
      credential and maps rows into `prices.pool_registry` with the
      `aqua→aquarius` / `xyk→0` / drop-`sdex` normalization. (Fetch covers
      `AMM_PROTOCOLS`; mapping fully unit-tested incl. a verbatim live-API-shape
      payload. Live end-to-end fetch is the operator's run with the real key —
      the API was already confirmed returning 199/12/330 pools manually.)
- [x] Writes via the shared `write_pool_registry` path; re-running is idempotent
      (integration test asserts count unchanged after a second write).
- [x] Integration test against local CH: seed a fixture-shaped payload → rows
      land, round-trip through `load_pool_registry`, `sdex` + unknown `poolType`
      dropped. (`tests/seed_it.rs`, green vs local CH.)
- [ ] Sample spot-check: a handful of API pools cross-checked against on-chain /
      task-0018 WASM-hash identities — **operator step**, run once against the
      live API before trusting the prod seed. (Not code; pending the real run.)
- [x] Credential handled safely: read from `SOROSWAP_API_KEY` env (`hide_env_values`),
      never logged, never a CLI value in shell history; Secrets-Manager path
      documented for automated use ([[prod-ch-schema-current-0076]] namespace note).

## Implementation Notes

New crate `packages/pool-registry-seed/` (~one-off CLI). Reuses the shared
`OhlcvWriter::write_pool_registry` (0069) and `Registries::load_pool_rows`
(0053), so the artifact is byte-identical to the backfill's.

- `src/lib.rs` — pure, testable core: `ApiPool` (serde `camelCase`), `venue_for`
  (`aqua→aquarius`, drop non-AMM), `pool_type_code` (`xyk→0`, else skip),
  `to_registry_rows` (+`MapStats`), `build_registry`, `fetch_pools` /
  `fetch_all_venues` (reqwest, `bearer_auth`, `assetList` omitted for full set).
- `src/main.rs` — clap CLI: `--dry-run` (fetch+map+report, no CH), plaintext
  `--ch-url` (default local) or Hetzner `--ch-domain` + `--mtls-*-path` (mirrors
  `Sink::mtls`). Key from `SOROSWAP_API_KEY` env.
- `tests/seed_it.rs` — gated CH integration (round-trip + idempotency +
  normalization). 5 unit tests + 1 integration test; build + clippy clean.

## Design Decisions

### From Plan

1. **Reuse the shared write path** (`write_pool_registry` / `load_pool_rows`)
   instead of a bespoke insert — one registry writer, no drift.
2. **`aqua→aquarius`, drop `sdex`, `xyk→0`** normalization as specified.

### Emerged

3. **Unknown `poolType` → skip + log, not default-to-0.** Seeding a pool with a
   guessed type the extractor can't decode (e.g. a future Phoenix stable pool)
   would silently mis-price it; better to leave it unresolved. All 541 live
   pools are `xyk` today, so nothing is dropped in practice.
4. **`poolType` mapped, not defaulted.** The live API *returns* `poolType`
   (undocumented in the OpenAPI schema), so `pool_type` is accurate rather than a
   blanket `0` — a bonus over what the task assumed.
5. **Transport mirrors `sdex-backfill`** (plaintext local / mTLS-from-paths) plus
   a `--dry-run` that needs no ClickHouse — lets the operator eyeball coverage
   before any write.

## Future Work

- A **scheduled** variant (periodic API re-seed) if we ever want belt-and-suspenders
  over 0069's factory-event maintenance — deliberately out of scope (one-off).

## Out of scope

- Historical AMM **OHLCV** backfill — still 0053 if wanted; this seeds the
  registry only (live pricing).
- Periodic re-seeding — one-off by design; 0069 owns ongoing registry
  maintenance. A scheduled variant would be a separate follow-up.
- Phoenix **stable**-pool handling — none deployed (task 0035 watch); the seeder
  logs any non-`xyk` `poolType` rather than guessing.
