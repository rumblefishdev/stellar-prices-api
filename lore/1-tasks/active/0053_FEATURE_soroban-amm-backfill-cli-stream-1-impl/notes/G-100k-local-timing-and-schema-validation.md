---
title: "100k local backfill — timing + schema validation (task 0053)"
type: generation
status: mature
spawns: []
tags: [clickhouse, backfill, sizing, measurement, timing, amm, schema-validation, milestone-M1]
links:
  - "../../../../docs/runbooks/running-ingestion-components.md"
  - "../../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../../packages/prices-clickhouse/schema/preroll.sql"
  - "../../archive/0060_FEATURE_prices-clickhouse-crate-combined-backfill-sizing/notes/G-measurement-results.md"
history:
  - date: 2026-07-04
    status: mature
    who: okarcz
    note: "100k-from-activation local run: timing measured, backfill_progress verified per-partition, schema + AMM extractor validated. Fully local, nothing pushed to Hetzner."
---

# 100k local backfill — timing + schema validation

Operational validation run of `sdex-backfill --mode combined` over the first
**100,000 ledgers from Soroban activation** (`50463000 → 50562999`), writing to a
**local Docker ClickHouse** (`--transport local`, `http://localhost:8123`).
Purpose: measure wall-clock on the current workstation + link, verify the
`prices.*` schema is well-defined, and confirm `backfill_progress` fills
correctly per partition. **Fully local — nothing pushed to Hetzner.**

Successor to the 0060 100k sizing run
([G-measurement-results](../../archive/0060_FEATURE_prices-clickhouse-crate-combined-backfill-sizing/notes/G-measurement-results.md)).

## Methodology

- Binary: `sdex-backfill` rebuilt from `develop` (the prebuilt Jul-1 bin
  predated the `--transport`/`--clickhouse-url` flags — rebuild required).
  Default features (no `aws-mtls`), so the binary **cannot** write to Hetzner.
- CH: `docker compose up` fresh (`down -v` first) so `init.sql` applies cleanly.
  Pinned `clickhouse/clickhouse-server:26.3.10.60`.
- Source: `aws s3 sync --no-sign-request` from the public `aws-public-blockchain`
  bucket (us-east-1). Read-only public fetch.
- Command:
  ```
  sdex-backfill --mode combined --start 50463000 --end 50562999 \
      --transport local --clickhouse-url http://localhost:8123 --verbose
  ```

## Result 1 — timing (headline)

**100k ledgers = `1:21:45` (4904 s ≈ 82 min), exit 0. Download-bound (CPU 17%).**

| Partition | In-range ledgers | Index (parse+bucket) | Decoded | Total wall |
|-----------|------------------|----------------------|---------|------------|
| `50432000` (p1) | 33,000 | 308.7 s | 4.82 GB | ~40.7 min |
| `50496000` (p2) | 64,000 | 627.7 s | 11.06 GB | ~32.2 min |
| `50560000` (p3) | 3,000 | 31.2 s | 0.60 GB | ~17.3 min |

- Download ≈ 65 min (80% of wall) at **~4.2 MB/s**; indexing ≈ 969 s (20%).
  **16.48 GB downloaded** (three *full* 64k partitions = ~192k ledger files to
  index 100k — the range only needs part of p1/p3, but `PARTITION_SIZE = 64000`
  downloads whole).
- vs 0060 baseline ~61 min/100k → **~35% slower, entirely the download link**
  (parse rate is consistent with 0060).

### Full-chain extrapolation (~13.7 h per 1M ledgers)

- Soroban-era combined `[activation, tip]` (~15M ledgers): **~8–9 days**
- Pre-Soroban SDEX-only `[1, activation)` (~50.5M ledgers): **~29 days**
- **Full chain ≈ 5–6 weeks locally.**

> **Actionable for the real 0053/0070 run.** The source bucket is in
> **us-east-1**. Running the backfill from an EC2 in us-east-1 makes download
> effectively free → reverts to parse-bound (0060 CPU rate), collapsing the full
> chain from **weeks → ~1–2 days**. Strongly recommended over a workstation run.

## Result 2 — `backfill_progress` verified per partition

Snapshotted the table at each of the 3 partition boundaries. `soroban_amm.current_ledger`
advanced by exactly one partition each time (`50495999 → 50559999 → 50562999`),
never downgraded, data-window + `last_push_at` refreshed. Final state:

| task_name | start | target | current_ledger | status | completed_at |
|-----------|-------|--------|----------------|--------|--------------|
| `soroban_amm` | 50463000 | 50562999 | **50562999** (=tip) | **completed** | set |
| `sdex_archive` | 1 | 50562999 | **50463000** (=activation) | **paused** | null |

Exactly the documented dual-row / decisions-6-7 behavior: the combined run drives
`soroban_amm` → completed, sets `sdex_archive.current = activation`, and
auto-pauses `sdex_archive` between the two range runs. No SDEX under-report.
`backfill_sdex_ledgers` = 100,000 (resumable).

## Result 3 — schema is well-defined

- Base tables from `init.sql` correct; `price_ohlcv_1m` = **5,547,345** candles;
  `assets` = **17,301** (written once at end-of-run — `run.rs:266`, after the
  partition loop, NOT per-partition; explains mid-run `assets=0`).
- **Pre-roll** (`preroll.sql`) applied in **2.5 s**; `_15m…_1M` collapse
  monotonically (5.55M → 1.74M → 884k → 432k → 126k → 49.6k → 30.06k) and every
  granularity preserves **30,056** distinct `(asset, quote, source)` pairs
  (`1M` rows == pairs — data spans one calendar month). Note: the docker-compose
  only auto-applies `init.sql`; the rollups are a **separate** `PREROLL_SQL` /
  `ROLLUPS_SQL` step (must be run after a backfill for coarse granularities).
- **Candle correctness:** **0** violations across 5.5M rows for high<low,
  open/close out of [low,high], negative volume, zero trade_count. Sample pair
  (SGB/yQUBIC) candles structurally correct (vwap in-band, single-trade minutes
  collapse to O=H=L=C, trade-less minutes are gaps).

### Finding — `Decimal(38,14)` price floor for dust tokens

28 of 5.5M rows have a non-positive price (`open`/`low`/`close` = 0). All are
**micro-cap dust tokens** (SLNK, ABAY, EBAY, HITACHI, PICK…) priced in XLM below
`1e-14` — the min representable positive at 14 dp — so they round to
`0.00000000000000` (15 rows have `close = 0` exactly). Real assets unaffected.
**Not corruption — a documented precision limit**, but any consumer computing
`1/price` or log-returns must guard against `close = 0`. Worth noting in the
OHLCV contract.

## Result 4 — AMM + oracle extractors validated functional

The 100k-from-activation slice had **`amm_ticks = 0`, `oracle_rows = 0`,
`pool_registry = 0`** — but this is the first week post-activation (Feb 21–27
2024), before AMM pools had swap volume. To rule out an extractor bug, a second
combined run over a **later window** `62,400,000 → 62,404,999` (~May 2026, 5k
ledgers, separate `--temp-dir`):

- **AMM swap extractors fire:** ~600 swaps across **12 pools** recognized, in ≥3
  topic shapes — `[Symbol("swap"), Vec([Address…])]`, `[String("swap"),
  String("sender")]`, `[Symbol("swap")]`.
- **Oracle extractor fires:** 455 oracle rows.
- BUT **`amm_ticks = 0` / 0 AMM candles** — all 12 pools `unresolved`
  (`genuine_gaps = 12`, `fatal = 0`, recorded to `prices.unresolved_pools` with
  `sample_topics`), because the window starts mid-chain with **no seeded
  `pool_registry`** (their factory-creates precede the window).

**Conclusions:**
1. The first-100k `amm=0`/`oracle=0` was **data-timing, not an extractor bug** —
   now definitively ruled out (extractors demonstrably work at ledger 62.4M).
2. **Confirmed operationally:** a mid-chain window WITHOUT a seeded `pool_registry`
   **loses 100% of AMM volume** to `unresolved_pools`. Forward-from-activation
   (organic discovery) or the task-0079 seed is **mandatory** for real AMM
   coverage. The guard behaved perfectly (recorded, non-fatal, topic samples for
   debugging). Reinforces the discovery-gap the design already calls out.

> Not yet validated here: the factory-**create** / registration path (this test
> only exercised swap *recognition*). A full forward run from activation, or a
> seeded run, is what confirms creates → `pool_registry` population.

## Teardown

`docker compose down -v` (CH container + data volume removed). Per-partition
scratch auto-cleaned by the backfill (no `--keep-partitions`). Pre-existing stale
`.temp/sdex-backfill` scratch left in place (unrelated to this run).

## Implications for the task

- The direct-to-Hetzner archive run (0053 operational item 1) should run from
  **us-east-1**, not the workstation, or budget weeks.
- Schema + `backfill_progress` accounting are validated end-to-end for the SDEX
  path; the AMM path's swap recognition is validated; its create/registration
  path and the Nov-2023-era Soroswap OHLCV data check (Tranche-1 AC) still need a
  seeded or full-from-activation run.
