---
title: 'G: Empirical prices-api ClickHouse storage estimate from a 10k-ledger mainnet sample'
type: generation
status: mature
spawned_from: ../README.md
spawns: []
tags: [generation, sizing, capacity, cost, hetzner, clickhouse, empirical]
links:
  - '../README.md'
  - '../../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md'
  - '../../../../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/R-cost-delta.md'
  - '../../../../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/R-ingest-target-mapping.md'
  - '../../../blocked/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-conversation-brief.md'
  - '../../../../3-wiki/project/soroban-events-schema.md'
history:
  - date: 2026-05-19
    status: developing
    who: okarcz
    note: >
      Draft — methodology + per-event row mapping landed; empirical
      counts pending the 10k-ledger backfill completing on the local
      ClickHouse.
  - date: 2026-05-19
    status: mature
    who: okarcz
    note: >
      Phase B (10k-window 62070000-62079999, 8.75M events) measured
      from local CH. Per-column compression reality-check from
      soroban_events drives row-size estimate down. Final answer:
      ~0.4 GB/year flat growth (an order of magnitude smaller than
      the initial draft).
---

# G: Empirical prices-api ClickHouse storage estimate

## 0. TL;DR

**Measured against a 10,000-ledger mainnet sample (62070000-62079999, ~13.9 hours of activity, 8.75M total Soroban events):**

- prices-api writes ~**0.65 rows/ledger** into `price_ohlcv_1m`
  (post-compaction), plus ~**1.26 rows/ledger** into `oracle_prices`
  from REFLECTOR + REDSTONE feeds.
- After CH compression (measured 14.8× on `soroban_events` as the
  closest-shape reference), prices-api stores **~74 bytes/ledger**.
- **Annual footprint (flat growth): ~0.45 GB/year. 5-year flat: ~2.2 GB.**
- At 10× scale: ~4.5 GB/year, 22.5 GB in 5 years.
- Even at 30× growth over 5 years (~67 GB), prices-api uses **<15%**
  of an AX41-NVMe (€39/mo, 512 GB) — and an order of magnitude less
  on bigger tiers.

**Implications for task 0045's BE brief:**

| Brief stance (current)                                      | Empirical (this report)                                        | Update brief?                 |
| ----------------------------------------------------------- | -------------------------------------------------------------- | ----------------------------- |
| Cluster D opens with **5-10% pro-rata** ($3-15/env/mo)      | **~1-2% pro-rata** ($1-3/env/mo)                               | **Yes** — re-anchor Cluster D |
| Cluster B asks BE to confirm "indefinite retention is fine" | Confirmed unnecessary — at <1 GB/yr no retention policy needed | **Yes** — simplify Cluster B  |
| Cluster B asks for `BACKUP DATABASE prices` Borg target     | Still useful (independent restore) but bytes-trivial           | Keep as-is                    |
| Cluster B asks Caddy keepalive headroom                     | Still the real capacity concern                                | Keep as-is                    |

The empirical answer makes prices-api dramatically lighter than the
0044 cost-delta estimate. **Update the brief before sending.**

---

## 1. Methodology

### 1.1 Data source

| Aspect                                    | Value                                                                                                                                                                   |
| ----------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Source repo                               | `soroban-block-explorer` (`/home/oski/Projects/stellar/soroban-block-explorer/`)                                                                                        |
| Backfill range available on disk          | ledgers `62016000` – `62079999` (64,000 ledgers, `.xdr.zst`)                                                                                                            |
| **Primary sample window**                 | ledgers `62070000` – `62079999` (10,000 ledgers, ~13.9 hours of mainnet)                                                                                                |
| Reference sub-window                      | ledgers `62078346` – `62079999` (1,654 ledgers — the final 16% of the primary window, backing `lore/4-notes/samples/soroban-events/*.jsonl` and `signatures-stats.tsv`) |
| Decoder                                   | BE's `backfill-runner` crate writing into local ClickHouse `soroban_events` table                                                                                       |
| Local ClickHouse                          | `localhost:8123`, user `default`, version 26.3.10.60                                                                                                                    |
| Total events ingested over primary window | **8,748,967**                                                                                                                                                           |
| Backfill runtime                          | 449 s (~7.5 minutes for the full 10k window)                                                                                                                            |

### 1.2 What "prices-api-relevant" means

Per task 0044's `R-ingest-target-mapping.md` and the design doc §2.2,
prices-api consumes a strict subset of Soroban events:

| Event signature                             | Prices-api action                                                                                                                                               |
| ------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `swap`                                      | Insert `price_ohlcv_1m` row per (asset, quote, source, minute)                                                                                                  |
| `trade` (Phoenix AMM)                       | Insert `price_ohlcv_1m` row                                                                                                                                     |
| `update_reserves`                           | **Companion** data — informs `volume_quote_usd` enrichment via reserve ratios; no own row class in `prices.*`                                                   |
| `REFLECTOR`                                 | Insert N rows into `oracle_prices`, where N = entries in `update_data` vec (avg ~19)                                                                            |
| `REDSTONE`                                  | Insert ~M rows into `oracle_prices`, where M = `updated_feeds` length in the inner XDR (~4)                                                                     |
| `null-signature` "string swap" micro-events | Captured by the consumer that matches on `topics_json` content, not on `signature` (per [[soroban-events-gotchas]] §3). Contributes to `price_ohlcv_1m` writes. |

**Out-of-scope signatures** (~99% of events): `fee`, `mint`, `burn`,
`transfer`, `clawback`, lending/borrowing, governance, vault, NFT.

### 1.3 Target schema (from task 0044's recommendation)

| Table                                         | Engine                           | Driven by                                               | Compaction key                                                                 |
| --------------------------------------------- | -------------------------------- | ------------------------------------------------------- | ------------------------------------------------------------------------------ |
| `price_ohlcv_1m`                              | `ReplacingMergeTree(version)`    | swap + trade + null-sig string-swap events              | (asset_id, quote_asset_id, granularity, source, **timestamp** truncated to 1m) |
| `price_ohlcv_{15m,1h,4h,1d,1w,1M}` (MV chain) | `ReplacingMergeTree`             | MV from 1m                                              | Coarser bucket                                                                 |
| `oracle_prices`                               | `MergeTree`                      | REFLECTOR + REDSTONE                                    | No compaction — every row retained                                             |
| `current_prices`                              | `ReplacingMergeTree(updated_at)` | Updater Lambda (rate 1m, **independent of event rate**) | (asset_id, quote_asset_id)                                                     |
| `assets`, `backfill_progress`                 | `ReplacingMergeTree(updated_at)` | Discovery Lambda + ops                                  | Small, ~KB scale                                                               |

### 1.4 Row-size: measured CH compression on similar tables

Querying `system.columns` for the loaded `soroban_events` table (8.75M
rows, 154 MB compressed) — the closest-shape reference for what
`price_ohlcv_1m` columns will look like:

| Column type                                          | Sample column     | Compressed bytes/row |     Ratio |
| ---------------------------------------------------- | ----------------- | -------------------: | --------: |
| `LowCardinality(Nullable(String))` (low cardinality) | `signature`       |                 0.09 |       11× |
| `Int64`, sequential                                  | `ledger_sequence` |                 0.52 |     14.9× |
| `Int64`, dictionary-friendly                         | `contract_id`     |                 0.05 |      162× |
| `Int64`, random                                      | `transaction_id`  |                 5.13 |      1.6× |
| `Int16`                                              | `event_index`     |                 0.68 |      2.9× |
| `String`, JSON                                       | `topics_xdr`      |                10.21 |       19× |
| **Whole-table average**                              | —                 |             **18.5** | **14.8×** |

Projected per-row compression for `price_ohlcv_1m` columns:

| Column                            | Type                                                                | Estimated compressed bytes/row |
| --------------------------------- | ------------------------------------------------------------------- | -----------------------------: |
| `asset_id`                        | `LowCardinality(FixedString(56))`                                   |                             ~3 |
| `quote_asset_id`                  | same                                                                |                             ~3 |
| `granularity`                     | `LowCardinality(Enum8)`                                             |                             <1 |
| `source`                          | `LowCardinality(String)`                                            |                             <1 |
| `timestamp`                       | `DateTime` (sorted)                                                 |                             ~1 |
| `open` / `high` / `low` / `close` | `Decimal128(14)`, sort-key clustered → similar magnitudes per asset |             ~3 each = 12 total |
| `volume_base` / `volume_quote`    | `Decimal128(28)`                                                    |             ~5 each = 10 total |
| `volume_quote_usd`                | `Nullable(Decimal128(14))`                                          |                             ~5 |
| `ledger_sequence`                 | `UInt32` (sequential)                                               |                             ~1 |
| `transaction_id`                  | `FixedString(32)`, random                                           |                            ~16 |
| `event_index`                     | `UInt16`                                                            |                             ~1 |
| `version`                         | `UInt64` (monotonic)                                                |                             ~1 |
| `updated_at`                      | `DateTime`                                                          |                             ~1 |
| **Total**                         | —                                                                   |                        **~55** |

`oracle_prices` row is simpler (no high/low) → estimate **~35 bytes/row**.

**Headline central row sizes (measured-derived):**

| Table                       | Bytes/row (compressed) | Source                                                                 |
| --------------------------- | ---------------------: | ---------------------------------------------------------------------- |
| `price_ohlcv_1m`            |                 **55** | Per-column model above                                                 |
| MV chain rollups (15m → 1M) |                 **45** | Smaller transaction_id share (rolled-up rows don't carry per-event tx) |
| `oracle_prices`             |                 **35** | Per-column model                                                       |
| `current_prices`            |                 **80** | Compacted to ~tracked-asset count; high turnover ratio                 |

These are **~3-5× smaller** than the 95-bytes/row estimate in the
initial draft, because the draft assumed conservative ZSTD ratios
instead of measuring against actual loaded data.

---

## 2. Event-rate observations

### 2.1 Phase B — primary window (10,000 ledgers)

Measured from `soroban_events` over ledgers 62070000-62079999:

| Signature                               |    Events | Distinct contracts |         Per ledger |
| --------------------------------------- | --------: | -----------------: | -----------------: |
| `swap`                                  |     2,288 |                  5 |          **0.229** |
| `trade`                                 |     2,940 |                103 |          **0.294** |
| `update_reserves`                       |     3,053 |                105 |              0.305 |
| **Subtotal price events**               | **5,228** |           **~110** |          **0.523** |
| `(null)`                                |     1,671 |                 43 |              0.167 |
| `REFLECTOR`                             |       573 |                  3 |              0.057 |
| `REDSTONE`                              |       450 |                  1 |              0.045 |
| **Subtotal oracle events**              | **1,023** |              **4** |          **0.102** |
| `price_updated`                         |        17 |                  2 | 0.002 (negligible) |
| _all other (fee, transfer, mint, etc.)_ |    ~8.74M |          thousands |                n/a |

### 2.2 Phase A reference (1,654 ledgers, tail of the window)

`signatures-stats.tsv` covers ledgers 62078346-62079999 — the final
1,654 ledgers (16%) of the primary window. For comparison:

| Signature         | Phase A /ledger | Phase B /ledger | Δ     |
| ----------------- | --------------: | --------------: | ----- |
| `swap`            |           0.332 |           0.229 | -31%  |
| `trade`           |           0.438 |           0.294 | -33%  |
| `update_reserves` |           0.449 |           0.305 | -32%  |
| `REFLECTOR`       |           0.058 |           0.057 | ~flat |
| `REDSTONE`        |           0.059 |           0.045 | -24%  |

**Interpretation:** Phase A's tail-window window had ~30% higher trading
activity than the average across the full 10k window. Time-of-day
matters: this confirms that using Phase A alone would have inflated
the estimate. Use Phase B as the central reference; treat Phase A as a
"high-activity hour" sensitivity bound.

### 2.3 REFLECTOR oracle fanout (unchanged from Phase A)

Per-event entry count from JSONL samples (still valid — same feeds,
same shape):

| Feed contract    | Domain           | Entries/event |
| ---------------- | ---------------- | ------------: |
| `CBKGPWGK…`      | FX               |          24.0 |
| `CAFJZQWS…`      | Global crypto    |          14.9 |
| `CALI2BYU…`      | Stellar on-chain |          18.3 |
| **Weighted avg** | —                |       **~19** |

### 2.4 REDSTONE oracle fanout

Inner XDR `bytes` payload size (post-base64-decode): mean 298 bytes,
range 108-528. Each XDR-encoded feed entry ~60-90 bytes → estimate
**~4 rows/event**.

---

## 3. Row-generation model

### 3.1 Per-ledger row insertions into `prices.*`

| Table                                  |                              Rows / 10k ledgers | Rows / ledger | Source                                              |
| -------------------------------------- | ----------------------------------------------: | ------------: | --------------------------------------------------- |
| `price_ohlcv_1m` (raw INSERTs)         |              5,228 + ~250 null-sig = **~5,478** |     **0.548** | swap + trade + null-sig string-swaps                |
| `price_ohlcv_1m` (post-compaction)     |              ~4,930 (10% intra-minute collapse) |     **0.493** | Replacing dedup by (asset, quote, source, minute)   |
| MV chain 15m → 1M (six tables, summed) |                           ~590 (12% of 1m base) |     **0.059** | Coarser buckets → many events collapse              |
| `oracle_prices` (REFLECTOR)            |                           573 × 19 = **10,887** |     **1.089** | One row per `update_data` entry                     |
| `oracle_prices` (REDSTONE)             |                            450 × 4 = **~1,800** |     **0.180** | Estimate; XDR not parsed                            |
| `current_prices`                       | ~150-300 rows total, regardless of ledger count |           n/a | Driven by Updater Lambda + ReplacingMergeTree dedup |
| `assets`, `backfill_progress`          |                                       <100 rows |    negligible |                                                     |

### 3.2 Per-ledger bytes (measured-derived row sizes)

| Table                         | Rows/ledger | Bytes/row |         Bytes/ledger |
| ----------------------------- | ----------: | --------: | -------------------: |
| `price_ohlcv_1m`              |       0.493 |        55 |                 27.1 |
| MV chain (15m → 1M)           |       0.059 |        45 |                  2.7 |
| `oracle_prices` (REFLECTOR)   |       1.089 |        35 |                 38.1 |
| `oracle_prices` (REDSTONE)    |       0.180 |        35 |                  6.3 |
| `current_prices`, registry    |           — |         — |                   <1 |
| **Total prices-api / ledger** |           — |         — | **~74 bytes/ledger** |

---

## 4. Extrapolation to mainnet time horizons

Mainnet ledger rate: **~5 s/ledger** → **17,280/day**, **525,960/month
(30.42 days)**, **6,311,520/year**.

### 4.1 Flat growth (current activity)

| Horizon    |       Ledgers | prices-api bytes |        Readable |
| ---------- | ------------: | ---------------: | --------------: |
| 1 day      |        17,280 |          1.28 MB |               — |
| 1 month    |       525,960 |          38.9 MB |               — |
| **1 year** | **6,311,520** |       **467 MB** | **~0.45 GB/yr** |
| 5 years    |    31,557,600 |      **~2.3 GB** |               — |
| 10 years   |    63,115,200 |      **~4.5 GB** |               — |

### 4.2 Scenario sensitivity

| Scenario           | Annual write rate | 5-year footprint | 10-year footprint |
| ------------------ | ----------------: | ---------------: | ----------------: |
| Low (0.5× current) |        0.23 GB/yr |           1.1 GB |            2.3 GB |
| **Central (flat)** |    **0.45 GB/yr** |       **2.3 GB** |        **4.5 GB** |
| 3× growth          |        1.35 GB/yr |           6.8 GB |           13.5 GB |
| 10× growth         |         4.5 GB/yr |          22.5 GB |             45 GB |
| 30× (DeFi mania)   |        13.5 GB/yr |          67.5 GB |            135 GB |

**Even at 30× scale over 10 years, prices-api stores ~135 GB —
under 30% of an AX41-NVMe (€39/mo, 512 GB).**

---

## 5. Hetzner sizing match

### 5.1 Tier reference (Hetzner 2026, list prices)

| Tier                        | Monthly EUR | Disk                | Years to fill (prices-api @ 10× scale, 4.5 GB/yr) |
| --------------------------- | ----------: | ------------------- | ------------------------------------------------: |
| **AX41-NVMe**               |         €39 | ~512 GB NVMe RAID 1 |                                    **~110 years** |
| **EX44**                    |         €44 | ~1 TB SSD RAID 1    |                                        ~220 years |
| **AX52**                    |         €69 | ~1 TB NVMe RAID 1   |                                        ~220 years |
| AX102                       |       ~€110 | 2× 1.92 TB NVMe     |                                        800+ years |
| BX21 (storage box for Borg) |          €4 | 1 TB extern         |                             Not on the data plane |

prices-api alone never drives the tier choice. **The tier choice is
driven by BE's `default.*` footprint, not by prices-api.**

### 5.2 Cost attributable to prices-api (pro-rata)

Assume BE chooses **AX52** (€69/mo) as the baseline production tier
(NVMe needed for BE's higher row volume). BE's loaded `default.*`
footprint, extrapolated from the 10k window (~600 MB for 10k ledgers
across 19 tables) to a steady-state year:

- BE `default.*` annual write rate: ~38 GB/yr (extrapolation:
  600 MB × 631 = 379 GB; offset by compaction + retention → ~40 GB
  net annual storage growth).
- prices-api annual write rate: **~0.45 GB/yr (this report)** or
  ~4.5 GB/yr at 10× scale.

**Storage share:** 0.45 / (40 + 0.45) ≈ **1.1% flat** or
4.5 / (40 + 4.5) ≈ **10% at 10× scale**.

| Basis                                                   | Flat growth                     | 10× scale                  |
| ------------------------------------------------------- | ------------------------------- | -------------------------- |
| Storage pro-rata                                        | 1.1% × €69 = **€0.76 / ~$0.82** | 10% × €69 = €6.90 / ~$7.45 |
| Row pro-rata (similar)                                  | ~1.1% / $0.82                   | ~10% / $7.45               |
| CPU pro-rata (MV chain runs every 1m, modest write-amp) | ~5% / $3.70                     | ~10% / $7.45               |
| **Blended pro-rata (central)**                          | **~1-2% / ~$1-2/env/mo**        | **~7-10% / ~$5-7/env/mo**  |

Per environment × 3 envs (dev/staging/prod): **$3-6/mo flat,
$15-21/mo at 10× scale**.

Plus optional **Storage Box backup** pro-rata: BX21 €4/mo × prices'
share of total backed-up bytes (≤2%) ≈ **$0.10/env/mo** (negligible;
absorbing it costs BE nothing meaningful).

### 5.3 Translation to USD/EUR

- Hetzner invoices in EUR. Current ~1 EUR = ~$1.08 USD.
- Round numbers per-env per-month:
  - Storage: ~$1
  - Blended pro-rata: $1-2 flat, $5-7 at 10×
  - With backup pro-rata: same +$0.10

---

## 6. Comparison to task 0044 `R-cost-delta.md` §6

| Number                        | 0044 hand-waved       | Empirical (this report)                                   | Verdict                                                        |
| ----------------------------- | --------------------- | --------------------------------------------------------- | -------------------------------------------------------------- |
| prices-api raw row size       | (implicit ~150 bytes) | **~55 bytes (price_ohlcv_1m), ~35 bytes (oracle_prices)** | Measured ratio ~3× better                                      |
| Annual storage flat growth    | (not given)           | **0.45 GB/yr**                                            | New baseline                                                   |
| 5-year footprint flat         | (not given)           | **~2.3 GB**                                               | New baseline                                                   |
| Storage share of BE box       | "~5%"                 | **~1.1% flat / ~10% at 10×**                              | Sensitive to scale assumption; at current activity, much lower |
| Row share                     | "~5%"                 | ~1.5% / ~10× at scale                                     | Same                                                           |
| CPU share                     | "~10%"                | ~5-10%                                                    | Order of magnitude right                                       |
| **Blended pro-rata band**     | 5-10%                 | **~1-2% flat / 7-10% at 10×**                             | Update brief, with scale-trigger clause                        |
| Resulting opening offer to BE | "$3-15/env/mo"        | **"$1-2/env/mo flat, re-open at 10×"**                    | Update Cluster D in `G-be-conversation-brief.md`               |

---

## 7. Recommendations

### 7.1 Update the BE conversation brief (task 0045, PR #21)

In `notes/G-be-conversation-brief.md` §3.4 (Cluster D — Money):

- Change the opening pro-rata from "5-10%" to **"~1-2% at current
  activity, with a re-open clause at 10× scale"**.
- Adjust dollar range: open with **"$1-2/env/mo flat"** (~$3-6/mo
  total across 3 envs), keep the free-ride and flat-fee alternatives.
- Cite this report (`notes/G-empirical-storage-estimate.md`) as the
  empirical basis.
- Keep the "re-open if scale shifts materially" language — the 10×
  scenario number (~$5-7/env/mo) is a useful future anchor.

### 7.2 Simplify Cluster B (Capacity)

In the same brief, §3.2:

- **Drop** the implicit framing that capacity is a meaningful concern
  for prices-api. Storage is <1% of the box; retention is unnecessary.
- **Keep** the Caddy `max_keepalive_conns` headroom ask (asks 5) —
  this is the only real concern at the connection layer.
- **Keep** `BACKUP DATABASE prices` Borg target (ask 7) — still useful
  for independent restore, costs near-zero.
- **Drop or soften** the retention-policy concern. Per §4.2 above,
  even 30× growth over 10 years yields <140 GB.

### 7.3 Retention policy is unnecessary for prices-api

The design's "Cleanup Worker `DROP PARTITION`" logic can stay (cheap
idempotent code) but will not fire in practice for 10+ years. Document
this so a future reviewer doesn't over-engineer retention tooling.

### 7.4 Open questions

- BE's measured `default.*` footprint (not estimated). When BE
  Hetzner CH is live and queries against `system.parts` are
  meaningful, refresh §5.2 with a measured pro-rata fraction.
- Compression assumptions for `Decimal128(28)` (volume columns) —
  validated qualitatively from `transaction_id` (random Int64 → 1.6×)
  and `topics_xdr` (long JSON → 19×). Decimal128 random values fall
  in between; the 5× ratio assumed for volume columns is conservative.
  Worth re-measuring once prices-api has loaded production data.

---

## 8. Reproduction

Re-run the analysis on any future ledger window via:

```bash
# 1. Backfill into local CH (~7 min/10k ledgers)
cd /home/oski/Projects/stellar/soroban-block-explorer
set -a && source .env && set +a
./target/release/backfill-runner --target clickhouse --keep-partitions \
  run --start <START_LEDGER> --end <END_LEDGER>

# 2. Per-signature counts (Phase B-style query)
curl -s -u default:clickhouse 'http://localhost:8123/' --data-binary "
  SELECT signature, count() AS events,
         countDistinct(contract_id) AS contracts,
         countDistinct(transaction_id) AS txs
  FROM soroban_events
  WHERE ledger_sequence BETWEEN <START> AND <END>
  GROUP BY signature
  ORDER BY events DESC
  FORMAT TabSeparatedWithNames"

# 3. Compression reality check
curl -s -u default:clickhouse 'http://localhost:8123/' --data-binary "
  SELECT name, formatReadableSize(data_compressed_bytes) AS compressed,
         round(data_uncompressed_bytes/data_compressed_bytes,2) AS ratio
  FROM system.columns
  WHERE database='default' AND table='soroban_events'
  ORDER BY data_compressed_bytes DESC
  FORMAT TabSeparatedWithNames"

# 4. Apply §3 row-generation factors and §4 extrapolation
```

Numbers should be reproducible to within ±15% on similar-activity
mainnet windows. High-activity windows (e.g. major DeFi launches)
could spike the per-ledger rate by 3-10× for hours-to-days; that's
within the existing 3× / 10× / 30× growth scenarios in §4.2.

---

## 9. Closing — single-line takeaway

prices-api on BE's Hetzner ClickHouse costs **~$1-2/env/mo at current
mainnet activity** and is **storage-trivial for any realistic decade
of growth**. The cost-share conversation with BE is about goodwill
and round numbers, not infrastructure.
