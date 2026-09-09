# Backfill and database usage summary

**Audience:** anyone sizing `prices.*`, planning a backfill, or deciding what is
safe to delete on `ch-prod-01`.

**Status:** Empirical. Measured directly against **production** `ch-prod-01` on
**2026-08-12**, after the [lore-0088](../lore/1-tasks/active/0088_FEATURE_soroban-backfill-run-tracker/README.md)
pre-Soroban backfill and the [lore-0136](../lore/1-tasks/active/0136_BUG_coarse-rollup-tables-frozen-since-2026-07-21.md)
gap pre-roll both completed on 2026-08-11.

> ⚠️ **`prices-production-cleanup` was DISABLED when these numbers were taken**
> (off since ~2026-07-20). Retention-managed tables are therefore holding far
> more than their steady state — see §5. Every figure below is a snapshot of an
> abnormally full database, which is exactly why it is worth recording: it is the
> only measurement of the backfill's full output before cleanup reclaims it.

> **Relationship to [`prices-api-hetzner-storage-estimate.md`](./prices-api-hetzner-storage-estimate.md):**
> that document _projects_ footprint from local backfills; this one _measures_
> the real thing. Where they disagree, trust this one for absolute size and that
> one for the per-ledger method. §7 reconciles them.

---

## 1. Headline figures

| #   | Metric                           | Value                                                      |
| --- | -------------------------------- | ---------------------------------------------------------- |
| 1   | All `1m` candles on disk         | **18.33 GiB** — 735,882,094 rows, 343 parts                |
| 2   | Pre-Soroban era on disk          | **29.87 GiB** — ~982.3M rows                               |
| 3   | Soroban-era on disk              | **12.84 GiB** — 228,736,540 rows                           |
| 4   | Distinct assets, pre-Soroban era | **123,160**                                                |
| 5   | Distinct assets, total           | **201,834** observed / **201,822** registered              |
| 6   | All `prices.*` on disk           | **59.19 GiB** — 1,606,194,877 rows, 23 tables, 2,437 parts |

Era boundary throughout this document is **Soroban activation, ledger
50,457,424 = `2024-02-20 17:01:00`** (measured by BE; authoritative, do not
re-derive).

---

## 2. Footprint by era

Covers the **7 live OHLCV tables**. `*_bak` snapshots are excluded so the eras do
not double-count; they are accounted separately in §3.

| Era                       |              Rows |       On disk | Share | Bytes/row |
| ------------------------- | ----------------: | ------------: | ----: | --------: |
| pre-Soroban (`<= 202401`) |       965,955,895 |     29.38 GiB | 67.9% |    32.7 B |
| boundary month `202402`   |        26,920,845 |      1.04 GiB |  2.4% |    41.5 B |
| Soroban-era (`>= 202403`) |       228,736,540 |     12.84 GiB | 29.7% |    60.3 B |
| **live chain total**      | **1,221,613,280** | **43.26 GiB** |       |           |

**The boundary month resolves cleanly for `price_ohlcv_1m`:** 16,358,020 rows
before the boundary, **0 after**. The backfill stopped exactly at activation and
nothing straddles it. That 0.49 GiB is folded into the headline pre-Soroban
figure in §1, giving **29.87 GiB firm**. The remaining ~0.55 GiB of `202402`
lives in the six coarse tables and is genuinely mixed, so the pre-Soroban upper
bound is **30.42 GiB**.

⚠️ **Soroban-era rows cost 1.84× more per row than pre-Soroban ones** (60.3 B vs
32.7 B). They are 18.7% of rows but 29.7% of bytes. The cause is write pattern,
not content: bulk backfill inserts arrive pre-sorted in
`ORDER BY (asset_id, quote_asset_id, source, timestamp)` and compress 8–9×,
while incrementally-written live rows compress ~2.8×. **Size future capacity
from the live figure, never the blended one.**

---

## 3. Footprint by table

| Table                 |              Rows |       On disk |     Parts |
| --------------------- | ----------------: | ------------: | --------: |
| `price_ohlcv_1m`      |       735,882,094 |     18.33 GiB |       343 |
| `price_ohlcv_1h`      |       183,160,940 |      9.69 GiB |       387 |
| `price_ohlcv_15m`     |       171,585,239 |      7.67 GiB |       251 |
| `price_ohlcv_15m_bak` |       153,060,193 |      7.39 GiB |        52 |
| `price_ohlcv_4h`      |        88,637,806 |      5.02 GiB |       349 |
| `price_ohlcv_1h_bak`  |        82,167,926 |      4.51 GiB |        50 |
| `price_ohlcv_4h_bak`  |        42,116,687 |      2.47 GiB |        67 |
| `price_ohlcv_1d`      |        30,005,492 |      1.77 GiB |       264 |
| `price_ohlcv_1d_bak`  |        14,351,533 |    918.53 MiB |        57 |
| `price_ohlcv_1w`      |         8,087,105 |    517.82 MiB |       249 |
| `price_ohlcv_1M`      |         4,232,450 |    276.05 MiB |       269 |
| `price_ohlcv_1w_bak`  |         3,702,185 |    235.00 MiB |        33 |
| `price_ohlcv_1M_bak`  |         1,341,814 |     82.60 MiB |        45 |
| **OHLCV total**       | **1,518,331,464** | **58.83 GiB** | **2,416** |

Database-wide composition:

| Group                                                               | Tables |              Rows |       On disk |
| ------------------------------------------------------------------- | -----: | ----------------: | ------------: |
| live OHLCV chain                                                    |      7 |          1,221.6M |     43.26 GiB |
| `*_bak` snapshots                                                   |      6 |            296.7M |     15.58 GiB |
| everything else (registry, cursors, `usd_rate`, ledger tracking, …) |     10 |             87.9M |     ~0.36 GiB |
| **`prices.*` total**                                                | **23** | **1,606,194,877** | **59.19 GiB** |

Notes:

- **There is no `price_ohlcv_1m_bak`.** The `*_bak` snapshot set covers the six
  coarse tiers only.
- **`*_bak` is 26.5% of the whole database** — see
  [lore-0177](../lore/1-tasks/backlog/0177_CHORE_undocumented-bak-coarse-tables.md).
  Its `priority-low` tag looks understated at 15.58 GiB.
- ⚠️ **`price_ohlcv_15m_bak` (7.39 GiB) is not cruft.** It is a pre-0095
  snapshot holding 2024–2025 at 15-minute grain, which exists **nowhere else**
  precisely because the live `15m` table is designed to drop that era (§5).
  Deleting the other five is housekeeping; deleting this one is a retention
  policy decision.

---

## 4. `price_ohlcv_1m` by year

The backfill's output, aggregated from the 102 monthly partitions.

| Year                        |                           Rows |       On disk |
| --------------------------- | -----------------------------: | ------------: |
| 2015 (Nov–Dec)              |                             23 |      4.85 KiB |
| 2016                        |                            110 |     31.81 KiB |
| 2017                        |                         68,021 |      3.73 MiB |
| 2018                        |                      2,653,887 |    142.34 MiB |
| 2019                        |                      4,217,732 |    217.02 MiB |
| 2020                        |                      2,615,091 |    133.53 MiB |
| 2021                        |                     22,600,095 |      1.26 GiB |
| 2022                        |                    211,765,920 |      6.35 GiB |
| 2023                        |                    434,909,822 |      8.07 GiB |
| 2024 (Jan–Feb, to boundary) |                     40,451,066 |      1.22 GiB |
| **2024-03 → 2026-06**       | **absent — retention, see §5** |             — |
| 2026 (Jul–Aug)              |                     16,599,716 |       938 MiB |
| **total**                   |                **735,881,483** | **18.33 GiB** |

> The 735,881,483 here is from the per-partition query; §1's 735,882,094 came
> from a later query in the same session. The 611-row delta is live ingestion
> between the two executions, not a discrepancy.

**719.3M of these rows are pre-Soroban**, which independently corroborates the
718.6M the 0088 pre-roll reported pushing into the six coarse tiers — confirming
the pre-roll covered the whole backfill rather than a subset.

Compression ratio climbs from ~2.2× (2017) to a peak of **9.05× (2022-12)** as
per-asset row density rises, then settles at ~2.8× for live-written months.

---

## 5. Retention — why 27 months are absent

Retention on `ch-prod-01` is a **scheduled job** (`prices-production-cleanup`
issuing `ALTER TABLE … DROP PARTITION`), **not** a ClickHouse `TTL` clause.
Grepping `init.sql` for `TTL` finds nothing and proves nothing — that inference
produced a wrongly-filed task (0174) that was closed the same day.

`packages/cleanup-worker/src/lib.rs:31-32`:

| Table                            | Retention                        |
| -------------------------------- | -------------------------------- |
| `price_ohlcv_1m`                 | 7 days                           |
| `price_ohlcv_15m`                | 30 days                          |
| `1h` / `4h` / `1d` / `1w` / `1M` | **forever — the durable record** |

### The dual-TTL cross-check

Both short-retention tables share a gap starting `202403`, but **end it at
different months, each exactly its own TTL back from one common sweep**:

| Table             | TTL     | Last month absent | First month present |
| ----------------- | ------- | ----------------- | ------------------- |
| `price_ohlcv_1m`  | 7 days  | `202606`          | `202607`            |
| `price_ohlcv_15m` | 30 days | `202605`          | `202606`            |

Intersecting the two brackets dates the **last sweep to 2026-07-08 → 07-31**,
matching the ~07-20 disable on record.

> ✅ **Use this as the standard test for "is this gap retention or data loss?"**
> Data loss cannot produce two gaps whose edges differ by exactly the difference
> between the two tables' TTLs. Control: `price_ohlcv_1h` held **all 130 months**
> (`201511 → 202608`, no gaps) at the same moment `15m` held 103.

The 27 months missing from `15m` are `202403` → `202605`, contiguous. Nothing is
lost — the forever-tables carry that era.

### Consequence for reclaimable space

| Block                  |                           Size | Reclaimed by        |
| ---------------------- | -----------------------------: | ------------------- |
| `_1m` pre-Soroban rows |                      17.39 GiB | re-enabling cleanup |
| `15m` pre-Soroban rows |                      ~6.97 GiB | re-enabling cleanup |
| `*_bak` tables         |                      15.58 GiB | lore-0177           |
| **total**              | **~40 GiB of 59.19 GiB (67%)** |                     |

⚠️ **Re-enabling cleanup deletes the backfill's `1m`/`15m` output. That is
correct, not loss** — the durable product is `1h` and coarser, and the pre-roll
is complete. But it must not be re-enabled _during_ a backfill or before its
pre-roll; doing so on 2026-07-20 cost five days.

---

## 6. Assets

| Population                                   |       Count |
| -------------------------------------------- | ----------: |
| observed in candles, pre-Soroban era         |     123,160 |
| observed in candles, Soroban era             |     126,115 |
| **observed in candles, total**               | **201,834** |
| distinct `asset_id` in `prices.assets FINAL` |     201,822 |
| rows in `prices.assets FINAL`                |     205,113 |

Counts are of distinct `asset_id` across **both legs** (`asset_id` and
`quote_asset_id`), taken from `price_ohlcv_1h` — the forever-table with complete
130-month coverage. Using `_1m` would under-count the Soroban era by 27 months.

### Era turnover

| Segment          |  Count | Share |
| ---------------- | -----: | ----: |
| pre-Soroban only | 75,719 | 37.5% |
| Soroban-era only | 78,674 | 39.0% |
| present in both  | 47,441 | 23.5% |

**Only 23.5% of assets span the boundary.** The traded asset population largely
turned over at Soroban activation rather than carrying forward.

### ⚠️ Two registry anomalies worth acting on

1. **205,113 registry rows for 201,822 distinct `asset_id`s — 3,291 excess.**
   At minimum 1.6% of assets carry a duplicate registry row and will fan out
   wherever a join keys on `asset_id`. This is the measured blast radius for
   [lore-0139](../lore/1-tasks/backlog/0139_BUG_current-price-usd-fans-out-on-duplicate-asset-id.md),
   which previously had no sizing.
2. **Candles reference 12 more distinct `asset_id`s than the registry contains
   after `FINAL`** (201,834 vs 201,822). Those are orphan ids resolving to no
   asset. Small, but real — record against 0139 rather than opening a new task.

> ⚠️ While 0139 is open, treat both figures as _ids observed_, not verified
> distinct assets. No count keyed on a resolved `asset_id` is trustworthy without
> first checking that id for collisions.

---

## 7. Reconciliation with the storage estimate

[`prices-api-hetzner-storage-estimate.md`](./prices-api-hetzner-storage-estimate.md)
has been revised twice: ~77.7 bytes/ledger (~0.48 GB/yr) originally, superseded
by task 0060's **~3.7 KB/ledger (~20–25 GB/yr)**.

**Production confirms 0060, not the original.** Derived from the `202607`
partition (13,283,917 rows / 768.97 MiB of `_1m`), which holds ~19 days —
2026-07-13 (the last sweep's 7-day horizon) through month end:

| Derived                                    |        Value |
| ------------------------------------------ | -----------: |
| `_1m` bytes/day                            |    ~40.5 MiB |
| `_1m` bytes/ledger (at 17,280 ledgers/day) |    ~2.40 KiB |
| with coarse tiers                          | ~3 KB/ledger |
| `_1m` at full retention, annualised        | ~14.4 GiB/yr |

That lands on 0060's 3.7 KB/ledger, and roughly **50× the original estimate's
77.7 bytes/ledger**. The original under-counted real asset-pair diversity.

⚠️ **The `202607` span is inferred from the sweep-date bracket in §5, not
measured directly.** Pin it with `min(timestamp)` on that partition before
quoting these as hard numbers.

**What has not changed:** `prices.*` still does not drive the Hetzner tier. At
59.19 GiB total against ~435 GiB free and 1.29 TiB used on a 1.72 TiB box, the
overwhelming majority of consumption is BE's `default.*`. The estimate doc's
tier recommendation stands.

---

## 8. Reproduction

All queries are read-only against production. Run them through the operator's
`CHQ` wrapper; do not `ssh`/`docker exec` into prod ClickHouse directly.

```sql
-- §1.1, §3 — footprint per table
SELECT table, sum(rows) AS rows,
       formatReadableSize(sum(bytes_on_disk)) AS on_disk, count() AS parts
FROM system.parts
WHERE database = 'prices' AND active AND table LIKE 'price_ohlcv_%'
GROUP BY table
ORDER BY sum(bytes_on_disk) DESC;

-- §1.6 — whole-database footprint
SELECT uniqExact(table) AS tables, sum(rows) AS rows,
       formatReadableSize(sum(bytes_on_disk)) AS on_disk, count() AS parts
FROM system.parts
WHERE database = 'prices' AND active;

-- §2 — footprint by era (live tables only; _bak excluded)
SELECT
    multiIf(partition <  '202402', 'pre-Soroban  (<= 202401)',
            partition =  '202402', 'boundary month 202402',
                                   'Soroban-era  (>= 202403)') AS era,
    sum(rows)                              AS rows,
    formatReadableSize(sum(bytes_on_disk)) AS on_disk,
    uniqExact(table)                       AS tables
FROM system.parts
WHERE database = 'prices' AND active
  AND table LIKE 'price_ohlcv%' AND table NOT LIKE '%\_bak'
GROUP BY era
ORDER BY era;

-- §2 — split the boundary month. No FINAL: this is a ratio, and unmerged
-- duplicates hit both sides alike. Do NOT reuse as absolute row totals.
SELECT
    countIf(timestamp <  toDateTime('2024-02-20 17:01:00')) AS pre_rows,
    countIf(timestamp >= toDateTime('2024-02-20 17:01:00')) AS post_rows
FROM prices.price_ohlcv_1m
WHERE toYYYYMM(timestamp) = 202402;

-- §4 — per-month footprint
SELECT partition AS month, sum(rows) AS rows,
       formatReadableSize(sum(bytes_on_disk)) AS on_disk,
       formatReadableSize(sum(data_uncompressed_bytes)) AS uncompressed,
       round(sum(data_uncompressed_bytes) / sum(bytes_on_disk), 2) AS ratio,
       count() AS parts
FROM system.parts
WHERE database = 'prices' AND table = 'price_ohlcv_1m' AND active
GROUP BY partition
ORDER BY partition;

-- §5 — the dual-TTL cross-check: which months 15m lacks that 1h has
SELECT partition AS missing_from_15m
FROM system.parts
WHERE database = 'prices' AND active AND table = 'price_ohlcv_1h'
  AND partition NOT IN (
      SELECT partition FROM system.parts
      WHERE database = 'prices' AND active AND table = 'price_ohlcv_15m'
  )
GROUP BY partition ORDER BY partition;

-- §6 — assets observed in candles, both legs, from the complete forever-table
SELECT
    uniqExactIf(a, t <  toDateTime('2024-02-20 17:01:00')) AS assets_pre_soroban,
    uniqExactIf(a, t >= toDateTime('2024-02-20 17:01:00')) AS assets_soroban_era,
    uniqExact(a)                                           AS assets_total_observed
FROM (
    SELECT timestamp AS t, arrayJoin([asset_id, quote_asset_id]) AS a
    FROM prices.price_ohlcv_1h
);

-- §6 — registry, and the 0139 duplicate-id measurement
SELECT count() AS registry_rows, uniqExact(asset_id) AS distinct_asset_ids
FROM prices.assets FINAL;
```

⚠️ **`active` is mandatory in every `system.parts` query.** Without it you sum
superseded parts that ReplacingMergeTree has not yet dropped, which can double or
triple the answer.

---

## 9. References

- [`prices-api-hetzner-storage-estimate.md`](./prices-api-hetzner-storage-estimate.md)
  — the projection this measurement supersedes for absolute size.
- [`runbooks/preroll-incremental-presoroban.md`](./runbooks/preroll-incremental-presoroban.md)
  — the procedure that produced the pre-Soroban rows measured here.
- [lore-0088](../lore/1-tasks/active/0088_FEATURE_soroban-backfill-run-tracker/README.md)
  — pre-Soroban backfill; cleanup must stay disabled until its pre-roll lands.
- [lore-0136](../lore/1-tasks/active/0136_BUG_coarse-rollup-tables-frozen-since-2026-07-21.md)
  — coarse-rollup freeze and the 13-day gap pre-roll.
- [lore-0139](../lore/1-tasks/backlog/0139_BUG_current-price-usd-fans-out-on-duplicate-asset-id.md)
  — duplicate `asset_id` fan-out, sized in §6.
- [lore-0177](../lore/1-tasks/backlog/0177_CHORE_undocumented-bak-coarse-tables.md)
  — the undocumented `*_bak` tables, sized in §3.
