---
id: "0060"
title: "Local prices ClickHouse schema crate + combined SDEX/soroban backfill — DB size & timing measurement"
type: FEATURE
status: active
related_adr: ["0007"]
related_tasks: ["0038", "0051", "0026", "0058"]
tags: [layer-database, milestone-M1, clickhouse, backfill, sdex, soroban, sizing, effort-large]
links:
  - "../../../docs/database-schema/clickhouse-prod-schema.sql"
  - "../../../docs/database-schema/database-schema-overview.md"
history:
  - date: 2026-06-11
    status: active
    who: okarcz
    note: >
      Created after BE-team meeting. Goal: stand up a production-ready
      prices ClickHouse schema as a dedicated crate (mirroring BE's
      crates/db-clickhouse layout), build it locally, then run a
      combined single-pass SDEX + soroban-events backfill over a
      100k-ledger range to MEASURE total prices.* DB size and backfill
      wall-clock. Numbers feed the production Hetzner sizing/timing plan.
---

# Local prices ClickHouse schema crate + combined SDEX/soroban backfill — DB size & timing measurement

## Summary

Stand up a production-ready `prices` ClickHouse schema as a dedicated crate
(mirroring BE's `crates/db-clickhouse` structure), build it on a local
ClickHouse, then run a **combined single-pass** SDEX + soroban-events backfill
over a 100k-ledger range. The two measurable goals: **(1) total on-disk size of
all `prices.*` tables** (normalised per 10k ledgers) and **(2) backfill
wall-clock time** (estimated). Local numbers are the basis for the production
backfill on Hetzner.

## Status: Active

**Current state:** Just created. Scope locked with the operator (see Decisions).

## Context

Both SDEX trades and soroban contract events live in the **same** raw ledger
`LedgerCloseMeta`. `packages/sdex-backfill` already downloads each ledger from
the S3 public archive, decompresses, and parses it once — SDEX trades are
extracted there today. Soroban events (`xdr_parser::extract_events`) are present
in the same parsed object, so we can extract **both in a single pass**: one S3
download, one decompress, one parse → SDEX candles (`source='sdex'`) + soroban
AMM candles (`source='phoenix'|'soroswap'|'aquarius'`) + oracle rows, all written
to `prices.*`. This removes any dependency on BE's `backfill-runner` /
pre-populated `soroban_events` table, and matches the eventual
`prices-ledger-processor` design (one ledger → both extractions).

Current gaps this task closes:
- No dedicated `prices` ClickHouse schema crate (schema only exists as loose SQL
  in `docs/database-schema/` + a subset in `packages/sdex-backfill/schema/`).
- `phoenix-extractor` XYK is implemented; **`soroswap-extractor` and
  `aquarius-extractor` are `unimplemented!()` stubs** — both required for M1.
- No soroban-event extraction wired into the range backfill.
- No oracle (REFLECTOR/REDSTONE) extraction into `prices.oracle_prices`.

## Decisions (locked with operator 2026-06-11)

1. **Single-pass combined backfill** — extend the `sdex-backfill` per-ledger loop
   (or a sibling binary) to also run soroban event extraction + dispatch in the
   same parse pass. No separate `soroban_events` ingest.
2. **Implement Soroswap + Aquarius extractors** (milestone-M1 requirement), not
   just Phoenix.
3. **Extract REFLECTOR/REDSTONE oracle events** into `prices.oracle_prices` in the
   same pass (dominant size driver per task 0046 footprint note).
4. **10k-ledger calibration first**, then the full 100k run to confirm linearity.

## Parameters

- Current tip ledger: **62982725**
- Test range (100k ledgers): **62882700 → 62982700**
- Calibration slice (10k ledgers): **62972700 → 62982700** (tip-adjacent subset)
- Local ClickHouse: `docker-compose` (prices-api), `http://localhost:8123`, db `prices`

## Implementation Plan

### Step 1: Production `prices` ClickHouse schema crate
Mirror BE `crates/db-clickhouse` layout under `packages/prices-clickhouse`:
`schema/init.sql` (full prod schema: assets, price_ohlcv_1m.._1M, current_prices,
oracle_prices, backfill_progress, backfill_sdex_ledgers, + MV rollup chain),
`config.d/`, `users.d/`, `src/lib.rs` (Config + client + apply_init_sql),
`src/bin/prices-clickhouse-init.rs`, `README.md`. Apply to local CH; verify.

### Step 2: Update schema docs + mermaid
Reconcile `docs/database-schema/` SQL + overview with the crate's `init.sql`;
refresh the mermaid ER/flow diagrams to current table + MV-chain state.

### Step 3: Combined backfill pipeline
- Implement `soroswap-extractor` and `aquarius-extractor`.
- Add soroban-event extraction to the backfill: `collect_tx_metas` →
  `xdr_parser::extract_events` → JSON→`TaggedValue` conversion →
  `ledger-processor::dispatch` → candles with venue `source`.
- Add REFLECTOR/REDSTONE oracle extraction → `prices.oracle_prices`.

### Step 4: Run + measure
- 10k calibration run → record size (all `prices.*` tables) + wall-clock.
- Full 100k run (background) → record total size + wall-clock; confirm linearity.
- Produce measurement note: bytes/ledger per table, per-10k extrapolation,
  projected production size/time on Hetzner.

## Acceptance Criteria

- [x] `packages/prices-clickhouse` crate builds; `init.sql` applies to local CH clean.
- [x] Schema docs + mermaid updated to match the crate.
- [x] Soroswap + Aquarius extractors implemented (with tests/fixtures).
- [x] Combined single-pass backfill writes SDEX + AMM candles + REFLECTOR/REDSTONE
      oracle rows to `prices.*` (AMM = in-window pools only; documented).
- [x] 10k calibration measured + documented (size + time).
- [x] 100k run measured + documented; per-ledger size + time estimate produced.
- [x] Measurement note with production Hetzner projection committed under `notes/`.

## Results (see notes/G-measurement-results.md)

- **Size:** 100k ledgers → **350.5 MiB** (≈**3.7 KB/ledger**), ~48× the prior
  74 B/ledger estimate — driven by 12,770-asset pair diversity (`_1m` dominant).
- **Time:** 100k backfill in **~61 min** (download-bound; ~37 ms/ledger serial).
- **Key levers:** filter assets before writing candles (~10–25× `_1m` cut);
  parallelize the S3 download for any full-history backfill.

## Notes

- Memory: infra is prepare-only; this is **local** measurement (S3 reads via
  `--no-sign-request`, local docker CH) — no cloud deploy / mutating AWS calls.
- Disk: 100k ledgers of XDR is a heavy local download; run in background, clean
  partitions after each (`--keep-partitions` off) unless inspecting.
