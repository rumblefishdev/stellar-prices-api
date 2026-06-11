---
title: "Prices DB size + backfill timing measurement (task 0060)"
type: generation
status: mature
spawns: []
tags: [clickhouse, sizing, backfill, measurement]
links:
  - "../../../../docs/database-schema/database-schema-overview.md"
  - "../../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-06-11
    status: developing
    who: okarcz
    note: "Calibration (10k) captured; 100k run pending."
  - date: 2026-06-11
    status: mature
    who: okarcz
    note: "100k run complete; two-point scaling + production projection + timing finalized."
---

# Prices DB size + backfill timing measurement

Local measurement of the `prices.*` ClickHouse footprint and combined-backfill
wall-clock, for projecting the production Hetzner backfill. **Fully local**:
docker ClickHouse 25.6 on `localhost:8123`, read-only `--no-sign-request` public
ledger fetch.

## Methodology

- Schema: `packages/prices-clickhouse/schema/init.sql` (12 tables), applied to a
  clean DB before each run.
- Backfill: `sdex-backfill` (single-pass SDEX + soroban). SDEX trades from op
  results; soroban AMM (Phoenix/Soroswap/Aquarius) + oracle (REFLECTOR/REDSTONE)
  from the same `LedgerCloseMeta`.
- Rollups `_15m … _1M` populated post-backfill by `schema/preroll.sql`
  (deterministic re-aggregate from `_1m FINAL`).
- Sizes from `system.parts` (active) after `OPTIMIZE … FINAL`.

## Calibration — 10,000 ledgers (62966000–62975999, ~13.9 h)

Counts: SDEX trade ticks 1,220,568 · AMM ticks **0** · oracle rows 16,690 ·
assets 4,343 · `price_ohlcv_1m` 317,122.

| Table | Rows | Compressed (B) | Uncompressed (B) | B / ledger |
|-------|-----:|---------------:|-----------------:|-----------:|
| price_ohlcv_1m | 317,122 | 18,013,544 | 43,446,711 | 1,801.4 |
| price_ohlcv_15m | 126,776 | 7,114,389 | 15,340,462 | 711.4 |
| price_ohlcv_1h | 75,421 | 4,472,116 | 9,126,303 | 447.2 |
| price_ohlcv_4h | 39,608 | 2,472,407 | 4,792,848 | 247.2 |
| price_ohlcv_1d | 23,751 | 1,550,855 | 2,874,039 | 155.1 |
| price_ohlcv_1w | 15,793 | 984,422 | 1,911,065 | 98.4 |
| price_ohlcv_1M | 15,793 | 984,422 | 1,911,065 | 98.4 |
| assets | 4,343 | 200,418 | 366,495 | 20.0 |
| oracle_prices | 16,690 | 138,885 | 1,623,627 | 13.9 |
| backfill_sdex_ledgers | 10,000 | 40,183 | 40,000 | 4.0 |
| **TOTAL** | **645,297** | **35,971,641** | — | **3,597** |

Total: **34.31 MiB / 10k ledgers ≈ 3.6 KB/ledger compressed** (≈2.6× CH
compression vs uncompressed).

## Findings

1. **~48× the prior estimate.** Task 0046 projected ~74 B/ledger; measured 3,597.
   Driver: pair diversity — **4,343 assets** (many low-volume/likely-spam tokens,
   no `is_active`/min-volume filter) → 317k 1m candles for 10k ledgers
   (~31.7 candles/ledger, ~57 B each compressed). Production sizing must either
   (a) budget for this, or (b) filter assets (min-volume / curated list) before
   writing candles.
2. **Rollups don't amortize in a short window.** `_15m … _1M` scale with
   *time-buckets × active-pairs*, not ledgers. Over ~14 h each pair occupies few
   buckets, so the rollups look large per-ledger (711 … 98 B/ledger). At
   multi-day/scale these shrink sharply per-ledger. **The 10k total is NOT safe
   to extrapolate linearly** — `_1m`/`assets`/`oracle` scale ~linearly; rollups
   are sub-linear per-ledger. The 100k run + a two-point fit refine this.
3. **AMM candles = 0.** Confirms the in-window-registry limitation: Phoenix/
   Soroswap/Aquarius pools were created before the window, so no factory
   `new_pair`/`add_pool`/`create` events were seen → unresolved. Extractors are
   implemented + unit-tested; full AMM coverage needs a historical factory-replay
   registry seed (future work).
4. **Oracle works** (16,690 rows, 14 B/ledger) — REFLECTOR decoded per-asset;
   REDSTONE captured as raw payload (price decode deferred).

## Timing (10k, cached partition)

Total `execute()` elapsed: 237 s (includes the `aws s3 sync` re-check of the
cached 64k-file partition + parse + write). Bytes read: 2.08 GB compressed XDR
for the 10k clamped range (~208 KB/ledger on disk). A clean download of one 64k
partition earlier took ~10–25 min over the public archive. Per-ledger
parse+extract+write throughput refined by the 100k run.

## Full run — 99,969 ledgers (62882700–62982700)

Counts: SDEX ticks 11,618,709 · **AMM ticks 913** (Aquarius — some pools created
in-window) · oracle 165,991 · assets 12,770 · `_1m` 3,592,135 (sdex 3,591,484 +
aquarius 651). 32 ledgers skipped (archive tail-lag, as designed).

| Table | Rows | Compressed (B) | B / ledger |
|-------|-----:|---------------:|-----------:|
| price_ohlcv_1m | 3,592,135 | 203,427,061 | 2,034.9 |
| price_ohlcv_15m | 1,463,928 | 79,473,475 | 795.0 |
| price_ohlcv_1h | 819,221 | 46,349,483 | 463.6 |
| price_ohlcv_4h | 394,743 | 23,015,662 | 230.2 |
| price_ohlcv_1d | 132,148 | 8,020,833 | 80.2 |
| price_ohlcv_1w | 50,576 | 3,200,107 | 32.0 |
| price_ohlcv_1M | 30,895 | 1,832,556 | 18.3 |
| oracle_prices | 165,991 | 1,304,678 | 13.1 |
| assets | 12,770 | 543,209 | 5.4 |
| backfill_sdex_ledgers | 99,969 | 401,789 | 4.0 |
| **TOTAL** | **6,762,376** | **367,568,853** | **3,676.8** |

Total: **350.54 MiB / 100k ledgers**.

### 10k → 100k scaling (two-point)
- **`_1m` rises** (1,801 → 2,035 B/ledger): more distinct pairs appear over the
  longer window (assets 4,343 → 12,770). Scales ~linearly with ledgers **and**
  grows with asset diversity.
- **Coarse rollups amortize** (`_1d` 155→80, `_1w` 98→32, `_1M` 98→18): each
  coarse bucket spans many ledgers, so per-ledger cost falls as the window grows.
- **`assets` amortizes** (20 → 5.4): bounded table over more ledgers.
- Net per-ledger ≈ flat (3,597 → 3,677) — the `_1m` rise offsets rollup
  amortization at this window size.

## Timing — 100k run

Wall-clock **3,661 s (~61 min)** for 99,969 ledgers (download + index + write;
the middle partition was cached). 21 GB read. **Download-bound** — see
[breakdown](#timing-breakdown). ~37 ms/ledger wall-clock at this serial rate.

### Timing breakdown
| Phase | Rate | Notes |
|-------|------|-------|
| S3 download | 1 partition (64k files / 13 GB) in ~29 min (~37 files/s, ~7.5 MB/s) | latency-bound (many small GETs); serial per-partition with 1-ahead prefetch |
| Index + extract + write | ~17 ms/ledger | CPU-bound (XDR parse + extraction); overlaps with the next download |

From scratch (no cache), 100k = 3 partitions ≈ 27 GB / 135k files → **~60–65 min**,
dominated by download.

## Production projection (Hetzner)

**Steady-state, retention-bounded fine tables** (the practical near-term size):
- `_1m` (7-day retention): 2,035 B/ledger × 120,960 ledgers/7d ≈ **246 MB** (cap)
- `_15m` (30-day retention): ≈ 795 B/ledger × 518,400 ledgers/30d ≈ **~410 MB**
  (upper bound; real value lower as `_15m` amortizes over 30 d)
- → bounded fine tables ≈ **0.5–0.65 GB** at steady state.

**Forever-retained tables** (`_1h…_1M`, oracle) grow with time. Per-YEAR (6.31 M
ledgers) from the 100k snapshot (UPPER BOUNDS — coarse per-ledger still inflated
at a 5.8-day window): `_1h` ~2.9 GB/yr, `_4h` ~1.5 GB/yr, `_1d` ~0.5 GB/yr,
`_1w` ~0.2 GB/yr, `_1M` ~0.12 GB/yr, oracle ~0.08 GB/yr → **≲5 GB/yr (upper
bound)**, shrinking per-ledger as buckets amortize.

**Bottom line:** at current mainnet density the prices DB is **~3.7 KB/ledger**
and a year of live operation is **on the order of a few GB** — materially above
the prior 0.45 GB/yr (task 0046) estimate, but not alarming. The first-year total
is dominated by the retention-capped `_1m`/`_15m` plus the forever-retained
hourly+ rollups.

### Total prices DB size on Hetzner (summary)

Per environment, compressed on-disk. ~17,280 ledgers/day, ~6.31 M/year at
current mainnet density. Per-ledger figures from the 100k run.

**Per-table projection — as measured (12,770 assets, unfiltered):**

| Table | Retention | Scaling | Projected size |
|-------|-----------|---------|---------------:|
| price_ohlcv_1m | 7 days | capped | **0.24 GB** |
| price_ohlcv_15m | 30 days | capped | **0.41 GB** |
| oracle_prices | 13 months | capped | **0.09 GB** |
| assets + current_prices | bounded | capped | ~0.01 GB |
| price_ohlcv_1h | kept | +**2.9 GB/yr** | grows |
| price_ohlcv_4h | kept | +**1.45 GB/yr** | grows |
| price_ohlcv_1d | kept | +0.5 GB/yr | grows |
| price_ohlcv_1w | kept | +0.20 GB/yr | grows |
| price_ohlcv_1M | kept | +0.12 GB/yr | grows |
| | | **capped subtotal** | **≈ 0.75 GB** (flat) |
| | | **forever subtotal** | **≈ +5.2 GB/yr** |

**Total prices DB size over time (per environment):**

| Horizon | Unfiltered (as measured) | Filtered (top-500, **measured**) |
|---------|-------------------------:|---------------------------------:|
| Steady-state capped | 0.75 GB | 0.62 GB |
| **Year 1** | **≈ 6 GB** | **≈ 3.5 GB** |
| Year 3 | ≈ 16 GB | ≈ 9 GB |
| Year 5 | ≈ 26 GB | ≈ 15 GB |
| Year 10 | ≈ 52 GB | ≈ 30 GB |

Caveats: the forever-table annual rates are derived from a 5.8-day window and
**lean high** — coarse grains (`_1d`/`_1w`/`_1M`) amortize further at multi-year
scale (the 10k→100k two-point fit already shows `_1d` 155→80 B/ledger), so the
real Year-3+ totals trend below these. **`oracle_prices` is oracle-feed-driven
(~16 feeds), not pair-driven — it stays ~90 MB regardless of asset filtering.**
The **filtered** column is the measured top-500 scenario from the experiment
below (keeps 93.4 % of trades).

### Asset-filtering experiment (measured, on the 100k data)

Tested how much filtering to the most-active assets shrinks the DB. Per-asset
activity = total `trade_count` where the asset is the base **or** quote; a candle
survives a top-N cut iff **both** its base and quote rank ≤ N (12,764 distinct
assets total). Survival curve on `price_ohlcv_1m`:

| Keep top-N assets | `_1m` rows kept | trades kept (≈ real volume) |
|------------------:|----------------:|----------------------------:|
| 100 | 50.7 % | 75.2 % |
| 200 | 68.2 % | 85.4 % |
| 500 | 84.5 % | 93.4 % |
| 1,000 | 93.5 % | 97.3 % |
| 2,000 | 98.4 % | 99.4 % |

**Key correction:** an earlier guess put filtering at ~25× — that is **wrong**.
Mainnet has genuine pair diversity, so top-500 still holds **84.5 %** of `_1m`
rows. BUT filtering bites the **forever-retained coarse rollups far harder** — at
top-500, rows kept are `_1m` 84.5 % → `_1h` **65.1 %** → `_1M` **26.3 %** (a dust
asset that trades once still costs 1 row in *every* granularity, so the tail
dominates coarse-table row counts). Since the coarse rollups drive long-term
growth, top-500 filtering ≈ **halves multi-year size** (Year-10 52 → 30 GB) while
dropping only **6.6 %** of trades. So filtering is a **moderate lever — most
effective on long-term rollup growth, not the `_1m` snapshot.**

### Recommendations
1. **Asset filtering is a moderate lever (measured), not a silver bullet.** It
   barely shrinks the 7-day `_1m` (top-500 keeps 84.5 %) but roughly **halves the
   forever-retained rollup growth** (the dominant multi-year cost) while keeping
   93 % of trades — top-500 takes Year-10 from ~52 → ~30 GB. Worth applying a
   min-volume / `is_active` cut, but it does **not** reach the old 74 B/ledger
   target. For bigger long-term savings, also reconsider **rollup retention**
   (e.g. cap `_1h`/`_4h` instead of keeping forever) — they dominate growth.
2. **Parallelize the backfill download.** The bottleneck is serial per-partition
   `aws s3 sync`. Parallel partition syncs / higher S3 concurrency would cut the
   ~60 min/100k materially — critical for any full-history (~62 M-ledger) backfill,
   which is weeks at the serial rate.
3. **AMM coverage** needs a historical factory-replay registry seed for full
   Phoenix/Soroswap/Aquarius candles (in-window-only today → 913 ticks / 651
   candles over 100k).

## Coverage caveats
- AMM (Phoenix/Soroswap/Aquarius) = in-window-created pools only. Oracle REDSTONE
  stored as raw payload (price decode deferred). AMM amounts assume 7-decimals.
- The live tip partition isn't fully published → the run indexed 99,969 of the
  100,001-ledger range (32 archive-tail gaps).
