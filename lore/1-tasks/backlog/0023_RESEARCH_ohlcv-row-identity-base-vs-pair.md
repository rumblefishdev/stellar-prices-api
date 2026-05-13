---
id: "0023"
title: "OHLCV row identity: decide base-only PK vs (base, quote) PK before SDEX backfill implementation"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0022", "0012"]
tags: [priority-high, effort-small, schema, ohlcv, sdex, backfill, blocking]
links:
  - "../active/0022_RESEARCH_sdex-filter-and-extraction-spec/notes/G-sdex-decode-and-bucket-spec.md"
  - "../../../docs/database-schema/database-schema-overview.md"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-05-13
    status: backlog
    who: claude
    note: >
      Spawned from 0022 future-work item 1. Decode spec §2.3 + §6
      surfaced this as a load-bearing schema gap before task 0012's
      Rust implementation can start.
---

# OHLCV row identity: decide base-only PK vs (base, quote) PK

## Summary

`price_ohlcv` today keys on `(timestamp, asset_id, granularity)`
where `asset_id` is the **base asset only**. SDEX trades commonly
have one base trading against multiple quotes in the same minute
(e.g. `USDC/XLM` and `USDC/USDT` both produce candles with
`asset_id = USDC.id`). They collide on the PK.

Pick one of three resolutions and write the schema-change ADR
before task 0012 starts.

## Context

Task 0022's decode-and-bucket spec §2.3 and §6 item 1 flagged
this as a real ambiguity. Three options:

A. **Add `quote_asset_id` to the PK.**
   `PRIMARY KEY (timestamp, asset_id, quote_asset_id, granularity)`.
   Simplest; preserves existing schema shape; doubles the row count
   for assets that trade against many quotes (still bounded — a
   handful of quotes per asset in practice).

B. **Introduce `asset_pair_id` as a surrogate.**
   New `asset_pairs` table; `price_ohlcv.asset_pair_id` replaces
   `asset_id`. Cleaner relational model; bigger migration; needs
   the `asset_pairs` table to exist and be populated.

C. **Status quo: aggregate across quotes per asset.**
   Drop the per-quote distinction; one "USDC candle" per minute
   that merges USDC/XLM + USDC/USDT + … trades. Loses
   information; defensible only if API consumers never need
   per-quote granularity (they probably do, for USDC vs USDT
   stablecoin depeg detection).

Option A is the default working assumption from 0022's spec.
This task confirms / refutes that and produces the ADR.

## Implementation

- Audit API endpoints for any consumers that need per-quote OHLCV.
- Decide A vs B vs C with the data team.
- Write ADR (`lore/2-adrs/000N_ohlcv-row-identity.md`).
- Open follow-up issue for the schema migration (or fold into task
  0012 if Option A).

## Acceptance Criteria

- [ ] ADR landed in `lore/2-adrs/` documenting the choice and
      reasoning.
- [ ] Task 0012's spec is updated to reference the chosen row
      identity.
- [ ] If the choice is Option B, an `asset_pairs` table DDL is
      drafted.
