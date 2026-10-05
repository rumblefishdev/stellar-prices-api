# AMM history is not in `price_ohlcv_1m`

**Measured:** 2026-09-29 (first established 2026-09-08 in [[0264]])

`price_ohlcv_1m` holds **no non-SDEX (AMM) rows before 2026-07-01**. The AMM
backfill pre-rolled its history straight into the coarse tables
(`price_ohlcv_1h`, `price_ohlcv_1d` and the other AS-copies of `_1m`) and never
populated `_1m`. SDEX history in the same table does reach 2015-11 (partition
201511), which is why the gap is easy to miss.

---

## Consequence

Any `_1m` query for AMM history before 2026-07-01 returns zero whether or not the
data exists. It is a **false pass** and proves nothing: "no AMM rows before X"
and "AMM rows exist before X" read the same.

## What to use instead

- `price_ohlcv_1h` for hour resolution, or `price_ohlcv_1d`.
- Classify streams the way the writer does (`sdex-backfill`
  `ingest.rs::note_candles`): `source = 'sdex'` is SDEX, every other source is
  AMM. Do not enumerate venues; a list silently drops the next venue.

## Not a retention effect

Of the candle tables, `cleanup-worker`'s `RETENTION` lists only `_1m` (7 days)
and `_15m` (30 days), and the worker is disabled. `_1h` and coarser are kept
forever. The 7-day `_1m` retention hypothesis was disproved in [[0264]] by SDEX
rows from 201511 still being present in `_1m`.

## Evidence

- [[0264]] "Issues Encountered" and its corrected acceptance criterion: 0264's
  original evidence argued from a `_1m` query returning zero before 2024-03-08.
  That query was unsound, and the conclusion survived only on re-measurement
  against `_1d` and `_1h`.
- Production, measured read-only 2026-09-29 for [[0272]]:

| table            | earliest SDEX row | earliest AMM row |
| ---------------- | ----------------- | ---------------- |
| `price_ohlcv_1h` | 2015-11-18 03:00  | 2024-03-08 19:00 |
| `price_ohlcv_1d` | 2015-11-18 00:00  | 2024-03-08 00:00 |

## Where it is enforced

- The `RECONCILE_QUERY` doc comment in
  `packages/backfill-freshness-probe/src/reconcile.rs`: the weekly claim
  reconcile reads `_1h`, never `_1m`.
- The `reads_1h_not_1m` case in
  `packages/backfill-freshness-probe/tests/backfill_reconcile_it.rs`: an AMM row
  that exists only in `_1m`, earlier than the claim, must not satisfy the check.

## Cited by

[[0101]], [[0264]], [[0271]], [[0272]]
