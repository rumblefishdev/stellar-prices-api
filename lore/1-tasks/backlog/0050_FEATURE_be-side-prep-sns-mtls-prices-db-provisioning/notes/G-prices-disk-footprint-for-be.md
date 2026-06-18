---
id: "G-prices-disk-footprint-for-be"
title: "BE-side info — prices DB disk footprint on the Hetzner CH box (empirical)"
type: G
task: "0050"
status: mature
spawned_from: []
spawns: []
related_notes:
  - "G-be-prices-db-rbac-ask.md"
links:
  - "../../../archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md"
  - "../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
---

# BE-side info — `prices` DB disk footprint on the Hetzner CH box

> **Audience:** BE team (soroban-block-explorer infra).
> **Purpose:** the concrete disk number behind the "rounding error" /
> "negligible footprint" claims in the sibling `G-be-prices-db-rbac-ask.md`
> (§Architectural context, §Cost). So when you size / provision the
> shared CH box you know exactly what the second tenant adds.
> **Status:** empirically measured, not estimated. Full methodology +
> reproduction in task 0046's report (linked above).

---

## TL;DR — prices adds ~0.45 GB/year to your disk

Measured against a **10,000-ledger mainnet sample** (ledgers
62070000–62079999, ~13.9 h, 8.75M Soroban events) decoded into a local
ClickHouse, with per-column compression taken from the loaded
`soroban_events` table (14.8× whole-table average) as the closest-shape
reference:

- prices-api writes **~74 bytes/ledger** into `prices.*` after CH
  compression.
- At mainnet's ~5 s/ledger (≈6.31M ledgers/year): **~0.45 GB/year
  flat growth.**

| Horizon | prices.* on disk |
|---|---:|
| 1 day | ~1.3 MB |
| 1 month | ~39 MB |
| **1 year** | **~0.45 GB** |
| 5 years | ~2.3 GB |
| 10 years | ~4.5 GB |

## Growth sensitivity (worst-case still trivial)

| Scenario | Annual | 5-year | 10-year |
|---|---:|---:|---:|
| Low (0.5×) | 0.23 GB/yr | 1.1 GB | 2.3 GB |
| **Central (flat, current activity)** | **0.45 GB/yr** | **2.3 GB** | **4.5 GB** |
| 3× growth | 1.35 GB/yr | 6.8 GB | 13.5 GB |
| 10× growth | 4.5 GB/yr | 22.5 GB | 45 GB |
| 30× (DeFi mania) | 13.5 GB/yr | 67.5 GB | 135 GB |

**Even at 30× sustained growth over 10 years (~135 GB), prices stays
under 30% of a single AX41-NVMe (512 GB).** prices-api never drives the
tier choice — that is set by BE's `default.*` footprint (~40 GB/yr).

## Where the bytes go

Per-ledger, across the `prices.*` tables:

| Table | Bytes/ledger | Driver |
|---|---:|---|
| `oracle_prices` (REFLECTOR ~19 rows/event + REDSTONE ~4) | ~44 | bulk of the footprint |
| `price_ohlcv_1m` | ~27 | swap + trade + null-sig string-swaps |
| MV rollup chain (15m → 1M) | ~3 | coarser buckets collapse heavily |
| `current_prices`, `assets`, `backfill_progress` | <1 | small/bounded |
| **Total** | **~74** | |

## Implications for the BE provisioning ask

- **Quota/profile sizing:** disk is not the constraint — the
  `prices_write` profile caps memory/exec-time (noisy-neighbour guard),
  not bytes. No disk quota needed for prices.
- **No retention policy required.** The design's Cleanup Worker
  `DROP PARTITION` logic can stay as cheap idempotent code but won't
  realistically fire for 10+ years. Don't over-engineer retention for
  `prices.*`.
- **Backup scope (§4 of the rbac ask):** if BE extends the snapshot to
  include `prices`, the added backup bytes are ≤2% of the box — Storage
  Box (BX21) pro-rata ≈ $0.10/env/mo, i.e. absorbing it costs nothing
  meaningful. (Default remains option (b): treat `prices.*` as
  re-derivable from ledger history.)
- **Cost-share:** the ~1–2%/env/mo pro-rata in the parent README is
  anchored to this storage share (~1.1% flat / ~10% only at 10× scale).

## Caveat

Numbers reproducible to ±15% on similar-activity windows. A major DeFi
launch could spike per-ledger write rate 3–10× for hours-to-days — that
transient is already inside the 3×/10× rows above. Refresh §5.2 of the
0046 report with a *measured* pro-rata fraction once BE's Hetzner CH is
live and `system.parts` queries are meaningful.
