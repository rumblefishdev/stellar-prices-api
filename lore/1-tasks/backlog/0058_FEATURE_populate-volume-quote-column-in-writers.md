---
id: "0058"
title: "Populate the restored `volume_quote` column in the OHLCV writers"
type: FEATURE
status: backlog
related_adr: ["0004", "0007"]
related_tasks: ["0026", "0038", "0051"]
tags: [layer-ingestion, priority-high, effort-small, clickhouse, schema, writers]
links:
  - "../../../docs/database-schema/database-schema-overview.md"
  - "../active/0026_FEATURE_volume-quote-usd-enrichment-impl/notes/G-local-prototype-spec.md"
  - "../../../packages/sdex-backfill/src/sink.rs"
history:
  - date: 2026-06-09
    status: backlog
    who: claude
    note: >
      Spawned from 0026 future work. Task 0026 restored the
      volume_quote column to price_ohlcv_* (dropped during the ADR-0007
      rewrite) because the enrichment Lambda reads it directly
      (volume_quote_usd = oracle_price x volume_quote, exact). The
      writers must now actually populate it, else every row reads back
      volume_quote = 0 and enrichment produces 0.
---

# Populate the restored `volume_quote` column in the OHLCV writers

## Summary

Task 0026 restored `volume_quote Decimal(38,14)` to `price_ohlcv_*` and
made the enrichment Lambda depend on it. The writers that INSERT OHLCV
rows must populate this column with the per-bucket native quote-asset
volume (`Σ |quote_amount|`) so enrichment has an exact input.

## Context

The decoder already computes `volume_quote` as the intermediate it uses
to derive `vwap = volume_quote / volume_base` (task 0048 spec) — it was
simply discarded after the division. This task stops discarding it and
writes it to the new column. Without it, `volume_quote = 0` everywhere
and the 0026 enrichment yields `volume_quote_usd = 0`.

Depends on task 0051 having added the column to the live schema DDL.

## Implementation

- **`sdex-backfill`** (`src/sink.rs`): the `OhlcvRow` struct has no
  `volume_quote` field and currently writes `volume_quote` *into* the
  `volume_quote_usd` slot (`sink.rs:106`, a placeholder). Add a
  `volume_quote` field, write the native quote volume there, and reset
  `volume_quote_usd` to its `DEFAULT 0` (enrichment fills it).
- **`prices-ledger-processor`** (task 0038 live path): same — carry the
  bucket's `volume_quote` through to the INSERT.
- **`soroban-amm-backfill`** (task 0053): same, if it writes OHLCV rows.
- Match the column position in the explicit INSERT column list to the
  schema (`volume_base, volume_quote, volume_quote_usd, vwap`).

## Acceptance Criteria

- [ ] `volume_quote` populated with `Σ |quote_amount|` per bucket in all
      OHLCV writer paths (sdex-backfill, prices-ledger-processor,
      soroban-amm-backfill)
- [ ] `volume_quote_usd` written as `DEFAULT 0` by writers (no longer
      aliased from `volume_quote`)
- [ ] `vwap = volume_quote / volume_base` still holds against the stored
      `volume_quote`
- [ ] A written row, run through the 0026 enrichment, yields a non-zero
      `volume_quote_usd` when an in-window oracle price exists
