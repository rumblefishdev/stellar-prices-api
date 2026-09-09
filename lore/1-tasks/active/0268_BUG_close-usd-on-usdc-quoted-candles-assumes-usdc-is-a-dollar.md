---
id: "0268"
title: "close_usd on every USDC-quoted candle before 2026-03-11 assumes USDC = $1 — re-enrich 654,291 candles from the external rate, and stop stamping method: peg on assets that are not stablecoins"
type: BUG
status: active
related_adr: ["0011"]
related_tasks: ["0265", "0267", "0168", "0182", "0111", "0266", "0247"]
tags: [layer-backend, priority-medium, effort-large, milestone-M3, clickhouse, enrichment, data-correctness, stablecoin, history]
milestone: 3
links:
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/S-phase0-root-cause.md"
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/S-backfill-migration.md"
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-09-07
    status: backlog
    who: akot
    note: >
      Spawned from [[0265]]'s Future Work as "defect B". [[0168]] named this
      the "known adjacent gap" it deliberately did not close; [[0267]] closes
      USDC's own series (defect A) and leaves this one, because it is a
      re-enrichment of stored candles, not an INSERT. Sized from the sweep in
      0265 phase 0 (135 of 232 assets carry method: peg).
  - date: 2026-09-07
    status: active
    who: akot
    note: "Activated; taken by akot after closing 0265."
---

# Stored USD prices assume USDC is a dollar

## Summary

Every USDC-quoted candle enriched before the oracle window (2026-03-11)
has `close_usd = close × 1.00`. The `1.00` is the enrichment peg tier's
literal, not a measurement ([[0247]] measured it: **654,291** pre-oracle
USDC-quoted candles, one implied rate, exactly 1.0). On 2023-03-11, when
USDC traded at 0.87–0.97, every such asset's USD price is overstated by
the depeg. XLM had 36,208 trades that day and reads `method: peg`.

Two symptoms, one fix:

1. **The stored number is wrong** on the stress days and slightly wrong
   everywhere else (USDC sits 1–10 bps off par on ordinary days).
2. **The label lies to consumers.** `method: peg` appears on 135 of 232
   swept assets (`native` on 1,865 days, canonical USDT on 1,858) and there
   means "USD denomination assumed", while on USDC it means "no measured
   rate". A reader cannot tell the two apart without the source code
   (0265 phase 0, `data/synthetic_profile.csv`).

## Context

- [[0267]] loads the measured USDC/USD series (Chainlink primary, Bitstamp
  fallback) into `usd_rate` as `method = 'external'` and fixes the *view*
  and `/ohlcv` for USDC. It does not touch stored `close_usd`, so after it
  ships the view and the candles disagree for 2021-01 → 2026-03. That
  disagreement is intended and announced in `backfill_note`; this task ends
  it.
- [[0182]] is the precedent: 567,760 candles corrected across five
  granularities after the USDT peg fix. Same class of job, same risks (it is
  what the cleanup worker shredded in the 0182/0201 campaign, which is why
  that worker is dark).
- [[0111]]'s enrichment redesign is the gate: the re-enrichment must run as a
  bounded, resumable pass, not a full-table rescan.
- [[0266]]'s ~25 % dislocations on the same dates are a different mechanism
  (two unrelated assets moving by an identical ratio) and stay separate;
  after this task lands, re-measure 0266's table to see what remains.

## Implementation

1. **Rate source**: the `external` rows [[0267]] loads (daily; hourly for
   2023-03 if 0267 ships it). The enrichment reads `usd_rate` for the bucket
   with preference `oracle` → `external` → literal 1.0, and records which
   one it used.
2. **Re-enrich** `close_usd` (and `volume_quote_usd`, `vwap` where derived)
   on every candle with `quote_asset_id = USDC` and `timestamp < 2026-03-11`,
   on all forever granularities, in bounded batches with a resumable cursor
   — the 0182 runbook shape, under 0111's constraints. Do not touch candles
   already enriched from an `oracle` reading.
3. **Vocabulary**: split `method` so it no longer overloads `peg`:
   - `traded` / `oracle` / `pivot` / `pivot2` keep their meaning;
   - `external` = the imported USDC/USD rate was the input;
   - `assumed-par` (or drop `peg` from non-stablecoins entirely) = the
     literal 1.0 was the input, i.e. nothing measured. Add it to
     `init.sql`'s vocabulary block and to the API docs.
4. **Verification fixtures**: XLM and yBTC daily closes on 2023-03-11 and
   2023-03-15 against 0266's Binance reference; the 0265 guardrail
   invariants over the swept peg-coded assets; a count of remaining
   `implied_from_candles == 1.00000000` rows that must be zero for
   USDC-quoted candles before 2026-03-11.
5. **Rollout**: shadow column or shadow partition first, compare, then swap;
   rollback is the previous parts (never delete before the swap is verified).
   Keep the cleanup worker dark for the duration.

## Acceptance Criteria

- [ ] No USDC-quoted candle before 2026-03-11 carries `close_usd == close`
      exactly where an `external` rate exists for its bucket
- [ ] `native` on 2023-03-11 publishes a USD close that reflects the USDC
      rate that day (≈ 3 % below the USDC-denominated close), on every
      granularity
- [ ] `method: peg` no longer appears on any non-stablecoin; the new value
      is documented in `init.sql` and the API reference
- [ ] The view and stored `close_usd` agree in deep history; the
      `backfill_note` caveat from [[0267]] is removed
- [ ] The pass is bounded and resumable; runtime and rows touched recorded
      here, as [[0182]] did
- [ ] [[0266]]'s dislocation table re-measured after the pass, with the
      result recorded there

## Out of scope

- USDC's own series — [[0267]].
- Any quote asset other than canonical USDC.
