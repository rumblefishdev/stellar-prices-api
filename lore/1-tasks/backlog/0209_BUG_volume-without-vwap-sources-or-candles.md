---
id: "0209"
title: "Fresh current-price rows report volume_24h_usd > 0 with vwap_24h = 0, empty sources — and four of them have zero candles in 30 days"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0120", "0207", "0182"]
tags: [layer-backend, priority-high, effort-medium, milestone-M2, api, pricing, defect]
milestone: 2
links:
  - "../../../tools/scripts/conformance-assets.json"
history:
  - date: 2026-08-19
    status: backlog
    who: stkrolikiewicz
    note: >
      Spawned from the 0120 conformance run (report
      conformance-0120-report-2026-08-19T0810.json).
---

# volume_24h_usd without vwap, sources, or even candles

## Summary

On production, 11 of the 20 conformance assets — including §9 majors **AQUA**
and **BTC** (`GDPJ…5O2MZM`), plus the top-volume soroban asset `CBIJ…` — have
fresh `updated_at` current-price rows with `volume_24h_usd > 0` while
`vwap_24h = "0"` and `sources: {}`. Four of them (AUD, RON, BOL, EQL, ranked
top-20 by that volume, $6k–$65k/24h) additionally return **zero OHLCV buckets
for the last 30 days**.

A volume number with no contributing sources, no VWAP, and no candles behind
it is internally inconsistent — either the volume is wrong (stale or
mis-attributed) or the vwap/sources pipeline misses trades the volume pipeline
counts.

## Context

Found by [[0120]]. Distinct from [[0207]] (that is price_usd source
selection; here the whole source breakdown is empty while volume is not).
Possibly adjacent to [[0182]]'s mis-valued candles. The M1 evidence doc showed
AQUA with 17k+ sdex candles/24h a month ago, so `sources: {}` for AQUA now may
also be a regression.

## Implementation

- Reconcile one asset end-to-end (AQUA): raw trades → candles → 24h volume →
  vwap/sources materialization; find where the paths diverge.
- Check whether AUD/RON/BOL/EQL volume comes from rows the candle store no
  longer holds (retention? backfill overwrite?) or from another venue class.
- Fix or correct the volume; re-run `npm run conformance:0120` — the
  `vwap_24h`/`sources` sentinel checks and the four empty-window failures must
  clear.

## Acceptance Criteria

- [ ] No conformance asset reports `volume_24h_usd > 0` with empty `sources`
- [ ] `vwap_24h` non-zero wherever `volume_24h_usd` is non-zero
- [ ] AUD/RON/BOL/EQL either show candles backing their volume or a corrected
      volume
- [ ] 0120 suite passes these checks
