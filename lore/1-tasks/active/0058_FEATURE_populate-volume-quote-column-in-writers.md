---
id: "0058"
title: "Populate the restored `volume_quote` column in the OHLCV writers"
type: FEATURE
status: active
related_adr: ["0004", "0007"]
related_tasks: ["0026", "0038", "0051"]
tags: [layer-ingestion, priority-high, effort-small, clickhouse, schema, writers]
links:
  - "../../../docs/database-schema/database-schema-overview.md"
  - "../blocked/0026_FEATURE_volume-quote-usd-enrichment-impl/notes/G-local-prototype-spec.md"
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
  - date: 2026-06-09
    status: active
    who: okarcz
    note: >
      Promoted from backlog to active. Fixed the stale 0026 link
      (active -> blocked, 0026 was re-blocked on 0012/0051).
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

- [~] `volume_quote` populated with `Σ |quote_amount|` per bucket in all
      OHLCV writer paths — **done for sdex-backfill**; prices-ledger-processor
      and soroban-amm-backfill writer paths do not exist yet (see Notes)
- [x] `volume_quote_usd` written as `DEFAULT 0` by writers (no longer
      aliased from `volume_quote`) — sdex-backfill `sink.rs`
- [x] `vwap = volume_quote / volume_base` still holds against the stored
      `volume_quote` — unchanged; `finalise_vwap` already divides the same
      accumulated `volume_quote` (`bucket.rs`), now also persisted verbatim
- [ ] A written row, run through the 0026 enrichment, yields a non-zero
      `volume_quote_usd` when an in-window oracle price exists — integration,
      deferred (needs live ClickHouse; same gate as 0026)

## Implementation Notes

Landed the **sdex-backfill** path only — it is the sole OHLCV writer that
currently exists:

- `packages/sdex-backfill/schema/init.sql` — added
  `volume_quote Decimal(38,14) DEFAULT 0` between `volume_base` and
  `volume_quote_usd`, matching the restored doc schema (§3.2).
- `packages/sdex-backfill/src/sink.rs` — added the `volume_quote: i128`
  field to `OhlcvRow` (schema-column order) and write
  `decimal_to_i128(candle.volume_quote)` into it. Reset `volume_quote_usd`
  to a literal `0` (the `DEFAULT`); it was previously aliased from
  `candle.volume_quote` (the placeholder bug this task fixes).
- `OhlcvCandle` already accumulates `volume_quote` (`bucket.rs:73`) and
  derives `vwap = volume_quote / volume_base` (`bucket.rs:110`), so no
  decoder/accumulator change was needed — the value was simply being
  written to the wrong column.

`cargo clippy -p sdex-backfill` clean (pre-existing `canonical.rs` warnings
only); `cargo test -p sdex-backfill` 5/5 pass.

**Deferred to the owning tasks** (the writers don't exist yet):

- prices-ledger-processor (`packages/prices-ledger-processor/` is fixtures
  only — task **0038** has not written its OHLCV INSERT path). When 0038
  builds that path it must write `volume_quote` and leave `volume_quote_usd`
  at `DEFAULT 0`.
- soroban-amm-backfill — no such package; task **0053** not started.

This task stays `active` until 0038 (and, if applicable, 0053) land their
write paths and adopt the same column contract.

## Design Decisions

### Emerged

1. **`volume_quote_usd: 0` literal, not `decimal_to_i128(Decimal::ZERO)`**:
   the slot is a `DEFAULT 0` placeholder the 0026 enrichment overwrites;
   a plain `0` is clearest and avoids implying a meaningful conversion.
2. **Also patched `schema/init.sql`**: the task framed this as a writer-code
   change, but sdex-backfill's local bootstrap schema lacked the column, so
   the INSERT would fail locally. The live DDL remains task **0051**'s job;
   this only touches the crate-local prototype schema.
3. **Scoped to sdex-backfill; left the other two writers to their owning
   tasks** rather than stubbing code for packages that don't exist.
