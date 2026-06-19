---
id: "G-64k-sizing-remeasure"
title: "G: prices.* footprint — fresh 64k-ledger ground-truth re-measure (capacity check for provisioning)"
type: G
task: "0063"
status: mature
spawned_from: ["G-provisioning-plan"]
spawns: []
tags: [clickhouse, sizing, capacity, cost, hetzner, measurement, shared-vs-sidecar]
links:
  - "../../../../docs/database-schema/database-schema-overview.md"
  - "../../../archive/0060_FEATURE_prices-clickhouse-crate-combined-backfill-sizing/notes/G-measurement-results.md"
  - "../../../archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md"
  - "../../../../lore/2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
history:
  - date: 2026-06-19
    status: mature
    who: claude
    note: >
      Ran a fresh 64,000-ledger backfill (62016000-62079999, cached partition,
      no download) through the real prices-clickhouse pipeline and measured
      system.parts. Third ground-truth data point alongside task 0060's 10k +
      100k runs. Confirms the ~KB/ledger reality (NOT the 74 B/ledger 0046
      estimate) and refreshes the shared-vs-sidecar cost comparison.
---

# G: prices.* footprint — fresh 64k-ledger ground-truth re-measure

## 0. Why this note exists

A cost/architecture question (shared `prices` DB in BE's CH vs a dedicated
prices CH container) was answered first from the **task-0046 estimate
(~74 B/ledger, ~0.45 GB/yr)**. That estimate is a per-event projection and is
**wrong by ~25-50×** — task 0060 already measured ~3.6 KB/ledger but used
windows 62966000+ / 62882700+. This note adds an **independent 64k window**
(62016000-62079999) measured end-to-end, so the provisioning decision rests on
three real data points, not the superseded estimate.

**Fully local** (the standing prepare-only / local-only constraint): docker
ClickHouse 25.6 on `localhost:8123`; the partition was already on disk
(`.temp/sdex-backfill/FC4DB5FF--62016000-62079999`, 64,000 files) so **no S3
fetch**. No prod infra touched.

## 1. Run parameters

| Aspect | Value |
|---|---|
| Window | ledgers `62016000`-`62079999` (64,000; ~3.7 days mainnet) |
| Pipeline | clean `prices.*` schema → `sdex-backfill` → `preroll.sql` (coarse rollups from `_1m FINAL`) → `OPTIMIZE … FINAL` → `measure.sql` |
| Backfill wall-clock | **1,126 s (~18.8 min)**, cached partition (~17.6 ms/ledger — matches 0060's ~17 ms/ledger index rate) |
| SDEX trade ticks | 3,361,790 (~52.5/ledger) |
| AMM trade ticks | 13 (in-window-registry limitation, same as 0060) |
| Oracle rows | 8,161 |
| Distinct assets | 8,983 |
| `close_usd` enrichment | **not run** (same as 0060) — column present but unpopulated, compresses to ~0; see caveat §5 |

## 2. Measured footprint (compressed on disk, after OPTIMIZE FINAL)

| Table | Rows | Disk | B/ledger |
|---|---:|---:|---:|
| price_ohlcv_1m | 1,385,272 | 53.16 MiB | 871 |
| price_ohlcv_15m | 499,257 | 25.61 MiB | 419 |
| price_ohlcv_1h | 286,021 | 16.65 MiB | 273 |
| price_ohlcv_4h | 158,635 | 9.88 MiB | 162 |
| price_ohlcv_1d | 70,615 | 4.62 MiB | 76 |
| price_ohlcv_1w | 23,758 | 1.54 MiB | 25 |
| price_ohlcv_1M | 23,758 | 1.54 MiB | 25 |
| assets | 8,983 | 875.55 KiB | 14 |
| backfill_sdex_ledgers | 64,000 | 251.19 KiB | 4 |
| oracle_prices | 7,971 | 132.07 KiB | 2 |
| backfill_progress | 2 | 350 B | — |
| **TOTAL** | **2,528,272** | **114.23 MiB** | **≈1,872** |

## 3. Three-window comparison — per-ledger cost is activity-driven

| Sample | Window | SDEX ticks/ledger | Assets | `_1m` candles/ledger | **B/ledger** |
|---|---|---:|---:|---:|---:|
| 0060 calib (10k) | 62966000+ | 122 | 4,343 | 31.7 | 3,597 |
| 0060 full (100k) | 62882700+ | 116 | 12,770 | 35.9 | 3,677 |
| **This run (64k)** | 62016000+ | 53 | 8,983 | 21.6 | **1,872** |

The driver is **trading-pair diversity + trade density**, not ledger count.
This window is an earlier, ~half-as-active period, hence ~1,872 vs ~3,677. Real
per-ledger cost is **window/time-dependent, ~1.9-3.7 KB/ledger** — a ~2× spread.
All three are **25-50× the 0046 ~74 B/ledger estimate**, which is now
superseded for sizing/cost purposes.

## 4. Corrected annual projection (per env)

| Basis | Year 1 | Notes |
|---|---:|---|
| Naive per-ledger (this 64k, low-activity) | ~11.8 GB | all grains forever |
| Naive per-ledger (0060, higher activity) | ~23 GB | upper end |
| **0060 per-bucket refined** (rollups amortize) | **~5-6 GB** | realistic, higher activity |
| Scaled to this window's activity | **~3-4 GB** | realistic, this window |
| With `_1h`/`_4h` retention cap @ 1yr | **~9 GB @ 10yr** (vs ~43 GB unbounded) | strongest size lever (0060) |

Realistic Year-1 ≈ **3.5-6 GB/env** — an order of magnitude above 0046's
0.45 GB/yr, still trivial for a 1 TB Hetzner box. Levers (from 0060):
**retention-cap `_1h`/`_4h`** bounds growth; **top-500 asset filter** keeps
93% of trades and ~halves rollup growth.

## 5. Caveats

- **`close_usd` / USD-series unpopulated.** The BE-consumed historical USD
  prices ride as the `close_usd` column on these candle rows (task 0061), not a
  new row class. Enrichment wasn't run, so that column is ~0 and compresses to
  near-nothing here — when populated it adds only a few % (one Decimal/row across
  grains). Footprint is dominated by candle/pair diversity, not the USD layer.
  A follow-up enrich-then-measure would quantify the exact `close_usd` delta.
- **AMM candles ≈ 0** (13 ticks) — Phoenix/Soroswap/Aquarius pools created
  before the window are unresolved without a historical factory-replay registry
  seed. Real production with full AMM coverage is **higher** than measured here.
- **REDSTONE** stored as raw payload (price decode deferred).

## 6. Cost comparison — shared `prices` DB vs dedicated prices container

Box = BE's AX52 (€69/mo). Prices' *incremental* disk cost on an
already-sized box is ~$0; the figures are the fair **goodwill pro-rata
cost-share** (shared) vs **effective resource cost** (separate container). Disk
bytes are identical both ways; the delta is dedicated RAM/CPU + the contract
break. Corrected for the measured ~3.5-6 GB/yr (prices ≈ 10-15% of a ~40 GB/yr
data plane), not the old ~1%.

| Component | **Shared `prices` DB (current / ADR 0007)** | **Dedicated prices container (Alt-3)** |
|---|---|---|
| Measured storage | ~3.5-6 GB/yr → ~10-15% of data plane | same bytes |
| Storage pro-rata of box | ~10-15% × €69 ≈ **$8-11/env/mo** | ~$8-11 (same) |
| Dedicated CH RAM/CPU | $0 (uses BE headroom) | **+$8-12/env/mo** — reserves ~8-16 GB RAM (mark cache/merges/queries) + merge threads, idle or not |
| Box-tier upgrade pressure | none | possible **AX52→AX102** (~+€41/mo shared) |
| **Blended $/env/mo** | **~$8-11** | **~$16-25** |
| **× 3 envs** | **~$24-33/mo** | **~$48-75/mo** |
| Ops surface | 1 CH, 1 `users.d`, 1 backup | 2 CH (lockstep upgrades), 2 `users.d`, 2 backups |
| **BE in-cluster `price_usd_series` JOIN (0199 contract)** | ✅ works | ❌ **breaks** — needs cross-server query / HTTP sync the contract rejected |

**Bottom line:** the corrected footprint raises the honest cost-share with BE
from ~1%/$1-2 to ~10-15%/**$8-11 per env/mo**, but does **not** change the
architecture: even ~9 GB at 10 years (with the `_1h`/`_4h` cap) is trivial for
the shared box, while a dedicated container costs ~2× more **and** breaks the
agreed in-cluster USD-views JOIN. Sidecar stays the **task-0047-RED fallback
only**, per ADR 0007.

## 7. Reproduction

```bash
docker compose up -d clickhouse
curl -s localhost:8123 --data-binary "DROP DATABASE IF EXISTS prices"
CLICKHOUSE_URL=http://localhost:8123 cargo run -q -p prices-clickhouse --bin prices-clickhouse-init
CLICKHOUSE_URL=http://localhost:8123 ./target/release/sdex-backfill --start 62016000 --end 62079999
CONT=$(docker compose ps -q clickhouse)
docker exec -i "$CONT" clickhouse-client --multiquery < packages/prices-clickhouse/schema/preroll.sql
# OPTIMIZE … FINAL each prices.* table, then:
curl -s localhost:8123 --data-binary "$(cat packages/prices-clickhouse/schema/measure.sql)"
```
