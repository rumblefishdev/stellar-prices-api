---
id: "G-128k-parse-and-sizing-test"
title: "G: 128k-ledger local parse-correctness + ClickHouse sizing test (SDEX + AMM + oracle)"
type: G
task: "0063"
status: mature
spawned_from: ["G-64k-sizing-remeasure"]
spawns: []
tags: [clickhouse, sizing, measurement, parsing, sdex, amm, soroban, oracle, hetzner, local-only]
links:
  - "../../../../docs/database-schema/database-schema-overview.md"
  - "../../../archive/0060_FEATURE_prices-clickhouse-crate-combined-backfill-sizing/notes/G-measurement-results.md"
  - "../../../../lore/2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md"
history:
  - date: 2026-06-19
    status: mature
    who: claude
    note: >
      Ran a fresh 128,000-ledger backfill (62848000-62975999, locally cached
      partitions, --keep-partitions, effectively offline) through the real
      prices-clickhouse production schema to verify SDEX + Soroban-AMM + oracle
      parse correctness and measure ground-truth per-table sizing. Fourth
      ground-truth data point alongside 0060's 10k+100k and 0063's prior 64k.
      Key findings: SDEX fully working (16.5M trades); AMM path works but
      historical coverage is in-window-pools-only (864 Aquarius ticks, 0
      Phoenix/Soroswap); full-schema 4.13 KiB/ledger at this high-activity range
      (~22 GiB/yr). LOCAL ONLY — nothing pushed/deployed.
---

# 128k-ledger local parse + ClickHouse sizing test (SDEX + AMM + oracle)

> **Status:** LOCAL ONLY — not pushed, not deployed to Hetzner. Production-ready
> schema applied to a local ClickHouse mirror for parse-correctness verification
> and ground-truth sizing.

- **Date:** 2026-06-19
- **Branch:** `feat/0063_provision-prices-db-on-hetzner-ch-self-served`
- **ClickHouse:** `clickhouse/clickhouse-server:25.6` (local docker, `prices` db)
- **Tool:** `target/release/sdex-backfill` (unified single-pass SDEX + Soroban-AMM + oracle extractor)
- **Source data:** already-downloaded `.xdr.zst` ledgers in `.temp/sdex-backfill/`
  (preserved via `--keep-partitions`)
- **Raw artifacts:** `.temp/0063-test/` (`run.log`, `time.log`, `start.txt`, `end.txt`, `SUMMARY.md`)

## 1. Scope of the run

| Item | Value |
|------|-------|
| Ledger range | **62,848,000 – 62,975,999** (contiguous) |
| Ledgers indexed | **128,000** (2 partitions: FC4103FF + FC4009FF) |
| Mainnet time covered | 2026-06-02 12:46 UTC → 2026-06-11 03:36 UTC (**≈ 8.62 days**) |
| Network | Effectively offline — FC4103FF locally complete (64,000 files); FC4009FF needed a 32-file top-up `aws s3 sync` (read-only public archive). `time -v`: 25.4 GiB filesystem **input** vs 17 MiB output → disk-read-bound, not download-bound. |

Range chosen deliberately: largest contiguous block of already-downloaded ledgers
**and** overlapping the only window where prior runs (task 0060) saw live AMM
activity — so it exercises both the SDEX and AMM paths.

## 2. Parse correctness — does extraction work? ✅ (with one AMM caveat)

Single parse pass produced, per ledger:

| Stream | Trade events | 1m candles (deduped, `FINAL`) |
|--------|-------------:|------------------------------:|
| **SDEX** (classic ClaimAtom) | **16,542,876** | 5,146,672 |
| **AMM — Aquarius** (Soroban) | **864** | 616 |
| **AMM — Phoenix** (Soroban) | **0** | 0 |
| **AMM — Soroswap** (Soroban) | **0** | 0 |
| **Oracle — RedStone** | 15,527 rows | 1 asset |
| **Oracle — Reflector** | 7,377 rows | 3 assets |

- **SDEX: fully working.** 16.5M trades aggregated into clean per-minute OHLCV
  across **13,979** base assets / **3,728** quote assets; sampled candles have
  valid `open/high/low/close/volume`. 61,428 raw claims correctly filtered as
  "zero amount" (the only WARN in the log).
- **AMM: path works end-to-end, coverage window-limited.** The 864 Aquarius ticks
  prove the full Soroban chain: `LedgerCloseMeta` → event extraction →
  `dispatch()` → `aquarius-extractor` → `price_ohlcv_1m (source='aquarius')`.
  Phoenix & Soroswap = **0 — not a parser bug.** The backfill builds its AMM pool
  registry from **factory events seen inside the indexed window** (`new_pair` /
  `add_pool` / `create`). Phoenix/Soroswap pools were created before ledger
  62,848,000, so they are never registered and their swaps are silently skipped.
  Aquarius appears only because some Aquarius pools were `add_pool`-created
  in-window. (Documented limitation, task 0060.)
- **Oracle: working** — both RedStone and Reflector captured.

> **Dedup near-perfect:** raw `price_ohlcv_1m` = 5,147,375 → after `OPTIMIZE …
> FINAL` = 5,147,288 (only 87 duplicate versions collapsed).

### AMM follow-up (to get full AMM coverage)
Historical Phoenix/Soroswap (and complete Aquarius) need the pool registry
**seeded ahead of the window** — a from-genesis factory-event replay, or seeding
from BE's `soroban_events`. Until then, historical backfill AMM = "pools born
in-window only". Live/tip ingestion is unaffected (it sees factory events as they
happen).

## 3. Timing / performance

Measured with `/usr/bin/time -v` plus the tool's own `elapsed` counter.

| Metric | Value |
|--------|-------|
| Wall clock (backfill `elapsed`) | **2,397 s** (39 min 57 s) |
| **Per ledger** | **18.73 ms/ledger** |
| **Throughput** | **53.4 ledgers/s** ≈ **6,900 trade events/s** |
| User CPU | 1,773.8 s |
| Sys CPU | 22.1 s |
| CPU utilisation | 74% (largely single-core; parse is serial) |
| Peak RSS | **98,344 KiB ≈ 96 MiB** |
| FS read | ≈ 25.4 GiB |
| FS write | ≈ 17 MiB (CH writes go over HTTP, not counted here) |

Single-core + disk-read bound — not memory or network bound.

## 4. Database size (production-ready schema)

Schema applied exactly as production: `init.sql` (tables) + `views.sql`
(read-surface views) + `seed.sql` (progress bootstrap). Coarse granularities
populated with `preroll.sql`, then `OPTIMIZE … FINAL` on every table.

> Production's live tip uses the **refreshable** MV chain in `rollups.sql`,
> intentionally **not** applied here: a refreshable MV *replaces* its target with
> only a `now() − 2h` window, which would wipe historical backfilled partitions.
> `preroll.sql` produces identical coarse-table contents for historical data.

### Per-table (active parts, post-`OPTIMIZE FINAL`)

| Table | Rows | Compressed | Uncompressed | Ratio |
|-------|-----:|-----------:|-------------:|------:|
| `price_ohlcv_1m`        | 5,147,288 | **274.91 MiB** | 672.53 MiB | 2.45× |
| `price_ohlcv_15m`       | 2,054,096 | 120.21 MiB | 268.38 MiB | 2.23× |
| `price_ohlcv_1h`        | 1,128,247 | 68.84 MiB  | 147.41 MiB | 2.14× |
| `price_ohlcv_4h`        |   530,625 | 33.40 MiB  | 69.33 MiB  | 2.08× |
| `price_ohlcv_1d`        |   173,835 | 11.35 MiB  | 22.71 MiB  | 2.00× |
| `price_ohlcv_1w`        |    55,354 | 3.75 MiB   | 7.23 MiB   | 1.93× |
| `price_ohlcv_1M`        |    33,870 | 2.15 MiB   | 4.43 MiB   | 2.06× |
| `assets`                |    14,339 | 1.36 MiB   | 1.94 MiB   | — |
| `backfill_sdex_ledgers` |   128,000 | 502.38 KiB | 500.00 KiB | — |
| `oracle_prices`         |    22,904 | 409.51 KiB | 6.19 MiB   | 15.5× |
| `backfill_progress`     |         2 | 350 B      | 125 B      | — |
| **TOTAL (active)**      | **9,288,560** | **516.87 MiB** | **1.17 GiB** | 2.32× |

- On-disk `store/` dir (`du`): **1.7 GiB** — higher than 516.87 MiB because it
  still holds inactive pre-merge parts pending cleanup + system tables. The
  **authoritative compressed footprint is 516.87 MiB** (active parts).

### Per-ledger sizing (this range)

| Metric | Value |
|--------|------:|
| Full schema (all tables) | **4.13 KiB/ledger** |
| `price_ohlcv_1m` only | 2.20 KiB/ledger |

## 5. Production (Hetzner) projection

Mainnet pace from the window: 128,000 / 8.62 days ≈ **14,850 ledgers/day ≈ 5.42M
ledgers/year**.

| Projection | Full schema | `_1m` only |
|------------|------------:|-----------:|
| Per day  | ≈ 60 MiB | ≈ 32 MiB |
| Per year | **≈ 21.9 GiB** | ≈ 11.6 GiB |

### ⚠️ Activity-dependence caveat
This is a **trade-dense** range (~129 trades/ledger, ~40 candles/ledger). The
prior 64k measure (range 62,016,000, [[G-64k-sizing-remeasure]]) was only **1.87
KiB/ledger** full-schema — ~2.2× lower density. **Sizing scales with market
activity, not ledger count.** Treat annual sizing as a **range, ~10–22 GiB/yr**,
and size Hetzner storage toward the high end with headroom. Either way it is
comfortably small for ClickHouse.

## 6. Isolation / hygiene checks

- All writes landed in `prices`; `default` database has **no tables** (no leakage).
- `backfill_sdex_ledgers` holds exactly 128,000 contiguous sequences
  (62,848,000–62,975,999) — complete, no gaps.
- Downloaded ledger partitions **preserved** (`--keep-partitions`).
- Nothing pushed to git or to Hetzner. Artifacts in `.temp/0063-test/`.

## 7. Verdict

| Area | Result |
|------|--------|
| SDEX extraction | ✅ Correct, high-volume, valid OHLCV |
| AMM extraction (mechanism) | ✅ Works end-to-end (Aquarius proven) |
| AMM historical coverage | ⚠️ In-window pools only — needs a seeded factory registry for full Phoenix/Soroswap history |
| Oracle extraction | ✅ RedStone + Reflector captured |
| Production schema fidelity | ✅ `init.sql`+`views.sql`+`preroll.sql`, `OPTIMIZE FINAL` |
| Sizing ground truth | ✅ 516.87 MiB / 128k = 4.13 KiB/ledger (this range) |
| Parse performance | ✅ 18.7 ms/ledger, 96 MiB RSS, single-core bound |

**Schema and parse pipeline are production-ready.** The one substantive gap is
**historical AMM pool discovery** (Phoenix/Soroswap) — a known, separately
tracked limitation, not a defect in this run.

## Appendix — reproduce

```bash
docker exec -i stellar-prices-api-clickhouse-1 clickhouse-client --query "DROP DATABASE IF EXISTS prices"
for f in init views seed; do
  docker exec -i stellar-prices-api-clickhouse-1 clickhouse-client --multiquery \
    < packages/prices-clickhouse/schema/$f.sql
done
CLICKHOUSE_URL=http://localhost:8123 /usr/bin/time -v -o .temp/0063-test/time.log \
  ./target/release/sdex-backfill --start 62848000 --end 62975999 \
    --temp-dir .temp/sdex-backfill --keep-partitions > .temp/0063-test/run.log 2>&1
docker exec -i stellar-prices-api-clickhouse-1 clickhouse-client --multiquery \
  < packages/prices-clickhouse/schema/preroll.sql
# OPTIMIZE FINAL every prices.* MergeTree, then run schema/measure.sql
```
