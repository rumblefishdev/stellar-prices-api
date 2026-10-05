---
title: "Pre-run baseline for the 0101 reprice — captured 2026-09-14 before any write"
type: research
status: mature
spawned_from: notes/R-soroswap-gap-is-one-bug-resumption-is-a-replay-position.md
spawns: []
tags: ["amm", "backfill", "clickhouse", "measurement", "prod", "baseline"]
links:
  - "../README.md"
history:
  - date: "2026-09-14"
    status: mature
    who: okarcz
    note: >
      Step 2 of the RUN RUNBOOK, captured as dev_read before the binary was run
      in any mode. 0097 skipped this and could only sanity-check afterwards; the
      acceptance criterion for SDEX being untouched is only checkable against
      these numbers. Re-run the identical queries after step 9 and diff.
---

# Pre-run baseline — 2026-09-14, before any write

Captured before the reprice, as `dev_read` over mTLS. **Re-run these exact
queries after the pre-roll and diff.** Anything that moves outside the AMM
sources in the window is a defect.

Run window: ledgers `63352609 .. 63518022` = `2026-07-06 09:35:00` ..
`2026-07-17 12:00:00`.

## 1. SDEX must not move at all

| level | rows | tip |
| --- | --- | --- |
| `1h` | 219,459,762 | 2026-09-14 09:00:00 |
| `4h` | 95,218,188 | 2026-09-14 08:00:00 |
| `1d` | 33,142,470 | 2026-09-14 00:00:00 |
| `1w` | 7,573,727 | 2026-09-07 00:00:00 |
| `1M` | 3,534,433 | 2026-09-01 00:00:00 |

And in the July partition, the only fine-grain rows the run could reach:

| level | source | rows | tip |
| --- | --- | --- | --- |
| `1m` | sdex | 11,132,634 | 2026-07-31 23:59:00 |
| `15m` | sdex | 5,155,308 | 2026-07-31 23:45:00 |

⚠️ The `1h`/`4h`/`1d` tips advance on their own — live is writing. Compare the
**row counts in the July partition** and the `1d` tip's *date*, not the coarse
tips' minute.

## 2. AMM at `1m`, exact run window — the conservation baseline

| source | rows | trades | volume_base | volume_quote | close_usd > 0 | first | last |
| --- | --- | --- | --- | --- | --- | --- | --- |
| aquarius | 51,119 | 90,667 | 3,087,984,254.233871 | 447,032,460.112595 | 26,418 | 07-06 09:35 | 07-17 11:59 |
| phoenix | 1,025 | 1,306 | 668,219.568122 | 114,098.147962 | 1,025 | 07-06 09:52 | 07-17 11:39 |
| soroswap | 1,620 | 2,064 | 2,343,419.090868 | 484,579.397901 | 1,501 | **07-11 21:00** | 07-17 11:52 |

🔑 **Soroswap's `first` is the gap itself.** The window opens 07-06 09:35 and
Soroswap's earliest row is 07-11 21:00 — the 5.4 days this task exists to fill.
After the run that cell must read `2026-07-06 09:3x`.

🔑 **Aquarius is the control.** Its three totals must come back **unchanged**. It
was never miswritten, so any movement means extraction drifted since July and
the run's blast radius is wider than this task.

Expect after the run: phoenix trades **up ~2.1%** (the recovered 7-event swaps),
soroswap rows and trades **up** by the whole gap, aquarius flat.

## 3. AMM coarse, `2026-06-29 .. 2026-08-03`

Window widened to **whole coarse buckets**, because step 5 rebuilds July
entirely rather than a mid-month slice.

| level | source | rows | trades | volume_base | close_usd > 0 |
| --- | --- | --- | --- | --- | --- |
| `15m` | aquarius | 58,855 | 277,709 | 48,338,471,855.852508 | 29,262 |
| `15m` | phoenix | 1,848 | 4,603 | 2,301,059.101934 | 1,661 |
| `15m` | soroswap | 4,804 | 18,054 | 362,174,087.002167 | 3,734 |
| `1h` | aquarius | 33,525 | 277,682 | 48,338,346,847.470881 | 16,594 |
| `1h` | phoenix | 1,171 | 4,597 | 2,298,802.649813 | 1,171 |
| `1h` | soroswap | 3,127 | 18,054 | 362,174,087.002167 | 2,541 |
| `4h` | aquarius | 15,455 | 277,525 | 48,336,772,397.053154 | 7,213 |
| `4h` | phoenix | 584 | 4,583 | 2,292,827.380328 | 584 |
| `4h` | soroswap | 1,830 | 18,050 | 362,173,448.879256 | 1,287 |
| `1d` | aquarius | 3,991 | 277,525 | 48,336,772,397.053154 | 1,832 |
| `1d` | phoenix | 157 | 4,529 | 2,263,085.774036 | 157 |
| `1d` | soroswap | 876 | 18,050 | 362,173,448.879256 | 471 |
| `1w` | aquarius | 720 | 277,525 | 48,336,772,397.053154 | 330 |
| `1w` | phoenix | 30 | 4,603 | 2,301,059.101934 | 30 |
| `1w` | soroswap | 348 | 18,050 | 362,173,448.879256 | 140 |
| `1M` | aquarius | 351 | 374,018 | 2,205,759,448,229.278158 | 169 |
| `1M` | phoenix | 16 | 4,388 | 1,418,304.527847 | 16 |
| `1M` | soroswap | 255 | 24,584 | 1,880,616,095.505804 | 85 |

⚠️ **The `1M` row is NOT a July measure and must not be read as one.** The
window catches two monthly buckets, `2026-07-01` **and `2026-08-01`**, and the
August bucket carries the whole of August — which is why aquarius reads
2.2 × 10¹² against `1d`'s 4.8 × 10¹⁰ over nominally the same span. It is a valid
*baseline* (the same query re-run compares like for like) but a useless
*conservation* check. For conservation at `1M`, scope to the single July bucket.

⚠️ The small drifts between `15m`, `1h` and `4h` are the straddling edge buckets
holding different slices, not a defect.

## Reproduce

`chq` as `dev_read`, **one granularity per query** — a multi-way `UNION ALL` of
FINAL scans runs them concurrently and exceeds the 5.59 GiB per-query quota.

```sql
SELECT source, count() AS rows, sum(trade_count) AS trades,
  round(sum(volume_base),6) AS vol_base, round(sum(volume_quote),6) AS vol_quote,
  countIf(close_usd > 0) AS priced, min(timestamp) AS lo, max(timestamp) AS hi
FROM prices.price_ohlcv_<LEVEL> FINAL
WHERE source IN ('soroswap','phoenix','aquarius')
  AND timestamp >= '<LO>' AND timestamp < '<HI>'
GROUP BY source ORDER BY source
```
