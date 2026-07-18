---
id: "0058"
title: "Populate the restored `volume_quote` column in the OHLCV writers"
type: FEATURE
status: completed
related_adr: ["0004", "0007"]
related_tasks: ["0026", "0038", "0051"]
tags: [layer-ingestion, priority-high, effort-small, clickhouse, schema, writers]
links:
  - "../../../docs/database-schema/database-schema-overview.md"
  - "0026_FEATURE_volume-quote-usd-enrichment-impl/notes/G-local-prototype-spec.md"
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
  - date: 2026-06-18
    status: blocked
    who: claude
    by: ["0038", "0053"]
    note: >
      Moved active -> blocked. The do-able portion is done: sdex-backfill
      (the only OHLCV writer that exists) populates volume_quote and writes
      volume_quote_usd as literal DEFAULT 0 (sink.rs, schema/init.sql,
      verified in code). The remaining writer paths do not exist yet —
      prices-ledger-processor is fixtures-only (owned by 0038) and
      soroban-amm-backfill has no package (owned by 0053). The final
      integration AC also needs live ClickHouse (same gate as 0026).
      Nothing further is implementable until 0038/0053 land their write
      paths and adopt the volume_quote column contract.
  - date: 2026-07-02
    status: completed
    who: okarcz
    note: >
      Unblocked + completed. Both blockers landed and unified on ONE
      OHLCV writer: 0038's live path (prices-ledger-processor) and 0053's
      combined backfill (sdex-backfill, PR #72 merged) both delegate to
      prices_ingest_core::OhlcvWriter::write_candles (writer.rs:122), which
      writes volume_quote = Σ|quote_amount| and volume_quote_usd = 0 DEFAULT.
      writer.rs:108 is the sole price_ohlcv insert in the codebase. AC #4
      verified against the local prod-pinned ClickHouse: candles_it (writer
      round-trip) + ch_enrich_it (4/4 — rows with volume_quote enrich to
      non-zero volume_quote_usd) both green. All ACs met.
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

- [x] `volume_quote` populated with `Σ |quote_amount|` per bucket in all
      OHLCV writer paths. *Resolved by the writer unification: both the live
      path (`prices-ledger-processor`) and the 0053 combined backfill
      (`sdex-backfill`) now delegate to the single
      `prices_ingest_core::OhlcvWriter::write_candles` (`writer.rs:122`, fed by
      `OhlcvCandle.volume_quote` = `Σ tick.volume_quote` from
      `canonical_volumes`). `writer.rs:108` is the only `price_ohlcv` insert in
      the codebase.*
- [x] `volume_quote_usd` written as `DEFAULT 0` by writers (no longer
      aliased from `volume_quote`) — sdex-backfill `sink.rs`
- [x] `vwap = volume_quote / volume_base` still holds against the stored
      `volume_quote` — unchanged; `finalise_vwap` already divides the same
      accumulated `volume_quote` (`bucket.rs`), now also persisted verbatim
- [x] A written row, run through the 0026 enrichment, yields a non-zero
      `volume_quote_usd` when an in-window oracle price exists. *Verified
      2026-07-02 against the local prod-pinned ClickHouse (26.3.10.60):
      `enrichment-worker` `ch_enrich_it` 4/4 green (inserts rows carrying
      `volume_quote`, asserts non-zero `volume_quote_usd` across the oracle /
      peg / pivot tiers), and `sdex-backfill` `candles_it` green (writer
      round-trip incl. `volume_quote`).*

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

**Resolution (2026-07-02) — the writer was unified, not duplicated.** Both
formerly-missing paths landed and, rather than each carrying their own
`OhlcvRow`, they converge on the shared `prices_ingest_core::OhlcvWriter`:

- **prices-ledger-processor** (task **0038**, live path): `reconcile.rs` →
  `sink/mod.rs` → `OhlcvWriter::write_candles`.
- **0053 combined backfill** (task **0053**, PR #72 merged): lives in
  `sdex-backfill`; its `sink.rs` was refactored to delegate to the same
  `OhlcvWriter::write_candles` (the standalone `OhlcvRow` this task originally
  patched in `sink.rs` is gone). Covers SDEX + AMM sources in one pass.

So the earlier sdex-backfill-local `sink.rs`/`init.sql` change is **superseded**
by `prices_ingest_core::writer.rs` carrying the contract for every writer.
`writer.rs:108` is the only `prices.price_ohlcv_1m` insert in the tree; it writes
`volume_quote` and leaves `volume_quote_usd = 0`. Nothing writer-side remains.

Integration confirmed against the local prod-pinned ClickHouse (26.3.10.60):
`sdex-backfill` `candles_it` (writer round-trip) and `enrichment-worker`
`ch_enrich_it` (4/4 — `volume_quote` → non-zero `volume_quote_usd`) both green.

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
