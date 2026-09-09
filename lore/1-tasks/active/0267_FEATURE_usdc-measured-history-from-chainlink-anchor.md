---
id: "0267"
title: "Serve USDC's measured USD history from the Chainlink anchor — load external rates, stop synthesising 1.0, say on the wire what each point is"
type: FEATURE
status: active
related_adr: ["0011"]
related_tasks: ["0265", "0247", "0168", "0173", "0111", "0125", "0127", "0266", "0268"]
tags: [layer-backend, layer-api, priority-high, effort-medium, milestone-M3, pricing, enrichment, data-correctness, stablecoin]
milestone: 3
links:
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/memo.md"
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/S-composition-rule.md"
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/S-backfill-migration.md"
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/analysis/guardrails.py"
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-09-07
    status: backlog
    who: akot
    note: >
      Spawned from [[0265]]'s decision memo. 0265 is the research (why, from
      where, how to compose); this is the implementation. [[0247]] designed
      the usd_rate loading path and becomes step 1 here rather than a
      separate ticket — its acceptance criteria are folded in below.
---

# Serve USDC's measured USD history

## Summary

`/ohlcv` for canonical USDC publishes a literal `1.0` for every bucket before
2026-03-11 (`queries_ch.rs:933`, reached only through the `is_peg_asset`
gate at `handlers.rs:643`). [[0265]] located the cause, chose the source and
proved the composition rule over the whole history. This task ships it:
load the measured series, make the read path prefer it, and put
`source`/`quality` on the wire so a consumer can tell "was 1.00" from "we
do not know".

The acceptance fixture is the falsifying date: **2023-03-11 must close at
0.9681, not 1.0.**

## Context

- The source decision and its evidence: [[0265]] `notes/memo.md`. Primary
  Chainlink USDC/USD rounds (on-chain, key-less, 2021-02-17 →), fallback
  Bitstamp, cross-check Kraken. USDT-quoted venues and peer stablecoins
  rejected on measured dispersion and correlation.
- The composed daily series already exists (`compose_usdc.py` output,
  2 049 rows 2021-01-25 → 2026-03-10: 98.6 % measured, 1.2 % fallback,
  0.2 % disputed, 0 missing). Re-running the script regenerates it.
- The loading path is [[0247]]'s: rows into `usd_rate` with a `method`
  distinct from `oracle`, view-first, no re-enrichment, ticker→issuer gate
  from [[0173]] satisfied because Stellar USDC `GA5Z…KZVN` is Circle's own
  issuance.
- The cost of not doing it: one-day 99 % VaR of the served series is 0 bps;
  measured it is 6 bps, ES 25 bps, worst day 300 bps.

## Implementation

1. **Loader** (from [[0247]]): the composed daily series →
   `prices.usd_rate`, canonical USDC only, `method = 'external'` added to
   `init.sql`'s vocabulary beside `oracle`/`peg`/`pivot`/`pivot2`, plus
   `source`, `quality`, `n_obs`, `xcheck_spread_bps`, `loaded_at`. Range
   2021-01-25 → 2026-03-10 so no key overlaps our own `oracle` readings.
   Load under a shadow value first (`'external-candidate'`), run
   `guardrails.py` and the 2023-03-11 check against the view, then rename.
   The loader refuses any asset code other than canonical USDC.
2. **Read path**: `ohlcv_peg_series` (`queries_ch.rs:952-957`) and
   `price_usd_series*` (`views.sql:362`, `:570`) accept `external` beside
   `oracle`; the literal `toDecimal128(1, 14)` remains only for buckets with
   neither. `n_obs` and the cross-check spread ride through.
3. **DTO**: `Candle` gains `source` (`chainlink` · `bitstamp` · `oracle` ·
   `` ), `quality` (`measured` · `measured-disputed` · `fallback` ·
   `missing`), `n_obs`, `xcheck_spread_bps`. `method` stays for the
   enrichment vocabulary. `backfill_note` states that stored `close_usd` on
   USDC-quoted candles still carries the assumption until [[0111]]'s
   re-enrichment (defect B).
4. **CI fixture**: an integration test over `/ohlcv?timeframe=all&granularity=1d`
   for canonical USDC asserting close(2023-03-11) ≠ 1.0, plus the two reject
   invariants from `guardrails.py` (30-day share of `trade_count = 0` ≥ 0.9;
   flat-and-no-trades ≥ 0.9) for every asset with a fiat-peg code. Fails
   today by design.
5. **Alarms**: the four alert invariants (exact-1.0 share, constant run,
   30-day realised vol, zero-volume share) as custom metrics on the [[0125]]
   dashboard, one evaluation per day, thresholds as in
   `notes/S-guardrails.md`.
6. **Evidence**: re-include USDC in the [[0127]] spot-check table with the
   0.9681 close.
7. **Ongoing**: a scheduled read of new Chainlink rounds (1–3/day in calm)
   appended as `external` **only** for buckets our own `oracle` did not
   observe — the oracle stays primary from 2026-03-11 on.

## Acceptance Criteria

- [ ] `GET /v1/assets/USDC:GA5Z…/ohlcv` returns `close = 0.9681`,
      `source = chainlink`, `quality = measured` for 2023-03-11
- [ ] No bucket of that series carries `method = peg` between 2021-01-25
      and 2026-03-10; `trade_count`/`n_obs` reflect rounds, not 0
- [ ] `usd_rate` carries the external rows with `method = 'external'`,
      no key overlapping an `oracle` row; `price_usd_series` and `_1h`
      publish them and there is no discontinuity artefact at 2026-03-11
      (from [[0247]])
- [ ] Rollback verified: reverting the preference order restores today's
      output byte for byte; rows are not deleted
- [ ] CI fixture green after the deploy, red before it
- [ ] The four alarms exist on the dashboard and do not fire on the
      2026-08 → 2026-09 window
- [ ] The source, its granularity, the fetch date and the ticker→issuer
      decision are recorded in this task (from [[0247]])
- [ ] [[0247]] closed as folded into this task; [[0265]] archived

## Out of scope

- Correcting stored `close_usd` on USDC-quoted candles and removing
  `method: peg` from non-stablecoins (defect B) — [[0268]];
  [[0266]]'s ~25 % dislocations on stress dates are a different mechanism
  and stay their own task.
- Any asset other than canonical USDC.
