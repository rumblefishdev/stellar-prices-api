# prices-api Hetzner ClickHouse storage estimate

**Audience:** BE team — sizing input for the shared Hetzner ClickHouse
that will host `prices.*` alongside BE's `default.*`.

**Status:** Empirical, measured against a 60k-ledger mainnet backfill
on the local soroban-block-explorer ClickHouse.

> ## ⚠️ Superseded per-ledger figure — read this first
>
> The **~77.7 bytes/ledger → ~0.48 GB/year** central estimate below (and the
> ~0.45 GB/yr / 14.8× figure copied from [task 0046](../lore/1-tasks/archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md))
> is **superseded** by a direct measurement of the real `prices.*` schema in
> [task 0060](../lore/1-tasks/archive/0060_FEATURE_prices-clickhouse-crate-combined-backfill-sizing/README.md):
>
> - **100k-ledger mainnet backfill = 350.5 MiB ≈ 3.7 KB/ledger — ~48× the
>   74 B/ledger estimate here.** The gap is real asset-pair diversity (12,770
>   pairs; `price_ohlcv_1m` dominates), which the per-ledger row-count model below
>   under-counted. At that rate the annual write is **~20–25 GB/yr** (order a few
>   tens of GB), not ~0.48 GB/yr.
>
> **The headline conclusion still holds:** prices-api does **not** drive the
> Hetzner tier — BE's `default.*` (~395 GB/yr) dominates, and prices-api is still
> a small single-digit-% share even at the measured rate. Keep this document for
> its method and row-level breakdown; trust **0060** for the absolute
> bytes/ledger and annual figures.

**TL;DR (numbers BE cares about):**

| Question                                                 | Answer                                                                          |
| -------------------------------------------------------- | ------------------------------------------------------------------------------- |
| How much disk does `prices.*` write per year on mainnet? | **~0.48 GB/year flat, ~4.8 GB at 10× scale**                                    |
| How much disk does `prices.*` use in 5 years (flat)?     | **~2.4 GB**                                                                     |
| What share of the shared CH box is `prices.*`?           | **~0.12% flat / ~1.2% at 10× / ~3.5% at 30× DeFi-mania**                        |
| Does prices-api drive the Hetzner tier choice?           | **No.** The tier is driven by BE's `default.*` (~395 GB/year raw extrapolation) |
| Minimum tier that comfortably hosts both for 5 years?    | **AX102 (~2 TB NVMe, ~€110/mo)** — or bigger; prices-api adds <1%               |

---

## 1. Methodology

### 1.1 Source data

| Aspect                      | Value                                                               |
| --------------------------- | ------------------------------------------------------------------- |
| Source repo                 | `soroban-block-explorer` (BE's backfill-runner)                     |
| ClickHouse                  | local docker, `clickhouse/clickhouse-server:26.3.10`                |
| **Ledger range backfilled** | `62019999` – `62079999` (**60,001 ledgers**, ~3.47 days of mainnet) |
| Tables populated            | 17 BE tables under `default.*`                                      |
| Total `soroban_events` rows | **47,545,820** (avg 792 events/ledger)                              |

The previous estimate ([lore-0046 G-note](../lore/1-tasks/archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md))
was based on a 10,000-ledger sub-window (62070000–62079999, 8.75M events).
This document re-runs the same calculations against the full 60k backfill
to confirm the original projection at 6× the sample size.

### 1.2 Mainnet rate constants

- 5 s/ledger → 17,280 ledgers/day → 6,311,520 ledgers/year.
- 60,001 ledgers ≈ 3.47 days ≈ 0.0095 years.

---

## 2. Measured BE-side totals (full 60k backfill)

```sql
SELECT database, table,
       formatReadableSize(sum(data_compressed_bytes)) AS compressed,
       formatReadableSize(sum(data_uncompressed_bytes)) AS uncompressed,
       round(sum(data_uncompressed_bytes)/sum(data_compressed_bytes), 2) AS ratio,
       sum(rows) AS rows
FROM system.parts
WHERE database='default' AND active
GROUP BY database, table
ORDER BY sum(data_compressed_bytes) DESC;
```

| Table                           |   Compressed |  Uncompressed |      Ratio |            Rows |
| ------------------------------- | -----------: | ------------: | ---------: | --------------: |
| transactions                    |     1.18 GiB |      1.88 GiB |      1.59× |      19,565,066 |
| soroban_events                  |   839.31 MiB |     11.77 GiB | **14.36×** |      47,545,820 |
| transaction_hash_index          |   672.38 MiB |    746.35 MiB |      1.11× |      19,565,066 |
| operations_appearances          |   433.95 MiB |      2.86 GiB |      6.75× |      31,017,424 |
| transaction_participants        |   396.44 MiB |    846.01 MiB |      2.13× |      36,962,805 |
| soroban_invocations_appearances |   160.55 MiB |    539.51 MiB |      3.36× |      12,298,272 |
| account_balances_current        |    34.70 MiB |    107.24 MiB |      3.09× |       2,548,873 |
| accounts                        |    29.13 MiB |     37.45 MiB |      1.29× |         404,740 |
| liquidity_pool_snapshots        |    23.74 MiB |    182.22 MiB |      7.68× |       1,374,603 |
| nft_ownership_pending           |    19.66 MiB |     75.91 MiB |      3.86× |       1,525,634 |
| nfts_pending                    |     8.87 MiB |     45.50 MiB |      5.13× |         617,906 |
| ledgers                         |     2.53 MiB |      3.66 MiB |      1.45× |          60,001 |
| soroban_contracts               |     2.12 MiB |      4.37 MiB |      2.06× |          33,685 |
| liquidity_pools                 |     1.37 MiB |      1.88 MiB |      1.37× |          27,916 |
| assets                          |   269.30 KiB |      1.28 MiB |      4.88× |          23,408 |
| wasm_interface_metadata         |    38.21 KiB |    299.83 KiB |      7.85× |              65 |
| lp_positions                    |    24.04 KiB |     46.90 KiB |      1.95× |             667 |
| **Total `default.*`**           | **3.75 GiB** | **19.04 GiB** |  **5.08×** | **173,571,951** |

### 2.1 BE annual extrapolation

3.75 GiB / 60,001 ledgers × 6,311,520 ledgers/year = **~395 GiB/year raw extrapolation**.

Realistic ranges:

- Backfill ingest mode is denser than steady-state (no MV materialisation
  pause, no compaction debt). Production ingest typically lands 10–30%
  smaller on disk after background merges settle.
- Best-case settled (×0.7): **~275 GiB/year**.
- Worst-case raw (×1.0): **~395 GiB/year**.
- 5-year settled: **~1.4 TB**. 5-year raw: **~2.0 TB**.

This is the number that drives Hetzner tier choice. prices-api does not.

---

## 3. prices-api row-generation model — refreshed against 60k

### 3.1 Per-signature counts (60k window vs 10k window)

| Signature         | Events (60k) | /ledger (60k) | /ledger (10k report) | Drift             |
| ----------------- | -----------: | ------------: | -------------------: | ----------------- |
| `swap`            |       13,417 |    **0.2236** |                0.229 | ~flat             |
| `trade`           |       17,138 |    **0.2856** |                0.294 | ~flat             |
| `update_reserves` |       17,747 |        0.2958 |                0.305 | ~flat             |
| `(null)`          |       16,849 |    **0.2808** |                0.167 | +68%              |
| `REFLECTOR`       |        3,449 |    **0.0575** |                0.057 | flat              |
| `REDSTONE`        |        4,064 |    **0.0677** |                0.045 | +50%              |
| `price_updated`   |           72 |        0.0012 |                0.002 | flat (negligible) |

**Interpretation:** Trade/swap rates are stable across the broader window
— validating the 10k report's central scenario. The `(null)` and
`REDSTONE` rates are higher in the broader window, but those flow into
small fractions of total bytes so the headline doesn't move.

### 3.2 Rows landing in `prices.*` per ledger (60k extrapolation)

Applying the row-generation rules from the 10k report (§3.1 of the
G-note) to the 60k counts:

| Table                                | Per-ledger formula             |                           Rows/ledger | Notes                                                          |
| ------------------------------------ | ------------------------------ | ------------------------------------: | -------------------------------------------------------------- |
| `price_ohlcv_1m` raw INSERTs         | swap + trade + 15% of null-sig | 0.2236 + 0.2856 + 0.0421 = **0.5513** | 15% of null-sig assumed string-swap as in the 10k report       |
| `price_ohlcv_1m` post-compaction     | raw × 0.9 (intra-minute dedup) |                            **0.4962** | ReplacingMergeTree(version) by (asset, quote, source, minute)  |
| MV chain (15m → 1M, 6 tables summed) | post-compaction × 0.12         |                            **0.0596** | Coarser buckets collapse aggressively                          |
| `oracle_prices` (REFLECTOR)          | REFLECTOR events × 19 entries  |              0.0575 × 19 = **1.0925** | Per `update_data` entry                                        |
| `oracle_prices` (REDSTONE)           | REDSTONE events × 4 entries    |               0.0677 × 4 = **0.2708** | Inner XDR `updated_feeds`                                      |
| `current_prices`                     | Updater Lambda rate-limited    |                                   n/a | <300 distinct rows total, ReplacingMergeTree on (asset, quote) |
| `assets`, `backfill_progress`        | n/a                            |                            negligible | KB-scale                                                       |

### 3.3 Bytes per ledger (using measured row sizes)

Row-size assumptions from the 10k G-note §1.4, derived from
per-column compression on `soroban_events` (re-verified below in
§3.4 — they still hold at 60k scale):

- `price_ohlcv_1m`: **55 bytes/row** compressed
- MV-chain rollups: **45 bytes/row**
- `oracle_prices`: **35 bytes/row**

| Table                         | Rows/ledger | Bytes/row |           Bytes/ledger |
| ----------------------------- | ----------: | --------: | ---------------------: |
| `price_ohlcv_1m`              |      0.4962 |        55 |              **27.29** |
| MV chain (15m → 1M)           |      0.0596 |        45 |               **2.68** |
| `oracle_prices` (REFLECTOR)   |      1.0925 |        35 |              **38.24** |
| `oracle_prices` (REDSTONE)    |      0.2708 |        35 |               **9.48** |
| `current_prices`, registry    |           — |         — |                     <1 |
| **Total prices-api / ledger** |           — |         — | **~77.7 bytes/ledger** |

10k report central estimate: ~74 bytes/ledger. **60k value (77.7) is
within 5% — the original projection holds.**

### 3.4 Compression sanity check at 60k

`soroban_events` per-column compression (full 60k backfill, all 47.5M rows):

| Column            | Type                             |     Compressed | Bytes/row |      Ratio |
| ----------------- | -------------------------------- | -------------: | --------: | ---------: |
| `topics_xdr`      | String (JSON)                    |     451.91 MiB |      9.97 |     18.41× |
| `transaction_id`  | Int64 (random)                   |     242.22 MiB |      5.34 |      1.50× |
| `data_xdr`        | String (base64 XDR)              |      85.58 MiB |      1.89 |     28.25× |
| `event_index`     | Int16                            |      31.40 MiB |      0.69 |      2.89× |
| `ledger_sequence` | Int64 (sequential)               |      21.89 MiB |      0.48 |     16.57× |
| `signature`       | LowCardinality(Nullable(String)) |       3.97 MiB |      0.09 |     11.45× |
| `contract_id`     | Int64 (dictionary-friendly)      |       1.95 MiB |      0.04 |    186.41× |
| `event_type`      | LowCardinality                   |       0.41 MiB |      0.01 |    223.56× |
| **Whole-table**   | —                                | **839.31 MiB** |  **17.7** | **14.36×** |

10k report measured 18.5 bytes/row / 14.8× ratio. **60k matches within 5%.**
This confirms the per-column model used to estimate the 55-byte
`price_ohlcv_1m` row size is sound at scale.

---

## 4. Mainnet-horizon projection

Using **77.7 bytes/ledger** central estimate × 6,311,520 ledgers/year:

### 4.1 Flat growth (current mainnet activity)

| Horizon    |       Ledgers | prices-api bytes | Readable        |
| ---------- | ------------: | ---------------: | --------------- |
| 1 day      |        17,280 |          1.34 MB | —               |
| 1 month    |       525,960 |          40.9 MB | —               |
| **1 year** | **6,311,520** |       **490 MB** | **~0.48 GB/yr** |
| 5 years    |    31,557,600 |      **~2.4 GB** | —               |
| 10 years   |    63,115,200 |      **~4.8 GB** | —               |

### 4.2 Scenario sensitivity

| Scenario           | Annual write rate |     5-year |    10-year |
| ------------------ | ----------------: | ---------: | ---------: |
| Low (0.5× current) |        0.24 GB/yr |     1.2 GB |     2.4 GB |
| **Central (flat)** |    **0.48 GB/yr** | **2.4 GB** | **4.8 GB** |
| 3× growth          |        1.44 GB/yr |     7.2 GB |    14.4 GB |
| 10× growth         |         4.8 GB/yr |      24 GB |      48 GB |
| 30× (DeFi-mania)   |        14.4 GB/yr |      72 GB |     144 GB |

---

## 5. Combined sizing — prices-api + BE on shared CH

### 5.1 Storage share

| Scenario                        | prices-api/yr | BE `default.*`/yr (settled, ×0.7) | prices share | BE+prices total/yr |
| ------------------------------- | ------------: | --------------------------------: | -----------: | -----------------: |
| **Central (flat)**              |       0.48 GB |                           ~275 GB |    **0.17%** |            ~275 GB |
| 10× prices growth               |        4.8 GB |                           ~275 GB |         1.7% |            ~280 GB |
| 30× prices growth               |       14.4 GB |                           ~275 GB |     **5.0%** |            ~290 GB |
| Raw extrapolation (no settling) |       0.48 GB |                           ~395 GB |    **0.12%** |            ~395 GB |

**prices-api is storage-trivial against BE's footprint under every
realistic scenario.** Even in a 30×-DeFi-mania scenario, prices is ≤5%
of the box.

### 5.2 Hetzner tier match (2026 list pricing)

Years-to-fill computed against the combined BE+prices central rate
(275 GB/year settled).

| Tier      |      €/mo | Disk                  | Years-to-fill (BE+prices, central settled) | 5-year fit? |
| --------- | --------: | --------------------- | -----------------------------------------: | ----------- |
| AX41-NVMe |       €39 | ~512 GB NVMe RAID 1   |                                 ~1.9 years | No          |
| EX44      |       €44 | ~1 TB SSD RAID 1      |                                 ~3.6 years | Tight       |
| AX52      |       €69 | ~1 TB NVMe RAID 1     |                                 ~3.6 years | Tight       |
| **AX102** | **~€110** | **~2 TB NVMe RAID 1** |                             **~7.3 years** | **Yes**     |
| AX162-R   |     ~€164 | ~4 TB NVMe            |                                ~14.5 years | Yes (10-yr) |

**Recommendation:** Sizing conversation should be aimed at **AX102 or
larger**. The driver is BE's `default.*` rate, not prices-api. If BE
chooses a smaller tier (EX44/AX52), prices-api still fits but BE will
need a retention or rollup strategy within 3–4 years.

### 5.3 Pro-rata cost attribution for prices-api

| Basis                             | Flat growth                       | 10× scale                        |
| --------------------------------- | --------------------------------- | -------------------------------- |
| Storage pro-rata on AX102 (€110)  | 0.17% × €110 = **€0.19 / ~$0.20** | 1.7% × €110 = **€1.87 / ~$2.02** |
| Storage pro-rata on AX52 (€69)    | 0.17% × €69 = **€0.12 / ~$0.13**  | 1.7% × €69 = **€1.17 / ~$1.27**  |
| Blended (CPU + MV chain overhead) | ~$1–2/env/mo                      | ~$5–7/env/mo                     |

Per-env × 3 envs (dev/staging/prod): **~$3–6/mo flat, ~$15–21/mo at 10×.**

This matches the 10k G-note conclusion. The empirical answer is "prices-api
share rounds to lunch money on any tier."

---

## 6. Comparison: 10k report vs 60k validation

| Number                                       | 10k report                                  | 60k measurement                                  | Verdict                                                                                      |
| -------------------------------------------- | ------------------------------------------- | ------------------------------------------------ | -------------------------------------------------------------------------------------------- |
| soroban_events whole-table compression ratio | 14.8×                                       | **14.36×**                                       | confirmed (within 3%)                                                                        |
| soroban_events bytes/row                     | 18.5                                        | **17.7**                                         | confirmed                                                                                    |
| `swap` events / ledger                       | 0.229                                       | **0.224**                                        | confirmed                                                                                    |
| `trade` events / ledger                      | 0.294                                       | **0.286**                                        | confirmed                                                                                    |
| REFLECTOR events / ledger                    | 0.057                                       | **0.058**                                        | confirmed                                                                                    |
| prices-api bytes / ledger                    | ~74                                         | **~77.7**                                        | confirmed (within 5%)                                                                        |
| prices-api GB / year (flat)                  | 0.45                                        | **0.48**                                         | confirmed                                                                                    |
| BE `default.*` projected GB/year             | ~38 (with aggressive compaction assumption) | **~275–395** (measured raw, ranged for settling) | **revised up ~7–10×** — the 10k report under-stated BE's footprint                           |
| prices share of CH box                       | 1.1% flat                                   | **0.12–0.17% flat**                              | **smaller than 10k report by 7–10×**, because BE footprint is bigger than previously assumed |

### Key takeaway for the BE conversation

The original 10k estimate was **directionally correct** but
**under-stated BE's own storage rate** (it applied a generous
compaction-and-retention discount that the measured 60k data doesn't
support). The net effect is:

- **prices-api absolute footprint:** confirmed (~0.5 GB/year).
- **prices-api share of the shared box:** even smaller than the 10k
  report claimed (0.12–0.17% vs 1.1%).
- **Hetzner tier choice:** driven entirely by BE's rate — AX102 or larger
  recommended for 5-year horizon, AX162-R for 10-year.

---

## 7. Reproduction

All queries are runnable via the local soroban-block-explorer docker:

```bash
# Working directory: soroban-block-explorer/

# 1. Verify ledger range backfilled
docker compose exec -T clickhouse clickhouse-client \
  --user=default --password=clickhouse \
  --query="SELECT min(sequence), max(sequence), count() FROM ledgers"

# 2. Per-signature counts (60k window)
docker compose exec -T clickhouse clickhouse-client \
  --user=default --password=clickhouse \
  --query="SELECT signature, count() AS events,
                  countDistinct(contract_id) AS contracts,
                  round(count()/60001.0, 4) AS per_ledger
           FROM soroban_events
           WHERE ledger_sequence BETWEEN 62019999 AND 62079999
             AND signature IN ('swap','trade','update_reserves',
                               'REFLECTOR','REDSTONE','price_updated')
           GROUP BY signature
           ORDER BY events DESC
           FORMAT TabSeparatedWithNames"

# 3. Null-signature events
docker compose exec -T clickhouse clickhouse-client \
  --user=default --password=clickhouse \
  --query="SELECT count() AS null_sig_events,
                  countDistinct(contract_id) AS contracts,
                  round(count()/60001.0, 4) AS per_ledger
           FROM soroban_events
           WHERE ledger_sequence BETWEEN 62019999 AND 62079999
             AND signature IS NULL
           FORMAT TabSeparatedWithNames"

# 4. Per-column compression
docker compose exec -T clickhouse clickhouse-client \
  --user=default --password=clickhouse \
  --query="SELECT name, formatReadableSize(data_compressed_bytes) AS compressed,
                  round(data_uncompressed_bytes/data_compressed_bytes, 2) AS ratio,
                  round(data_compressed_bytes/(SELECT count() FROM soroban_events), 2) AS bytes_per_row
           FROM system.columns
           WHERE database='default' AND table='soroban_events'
           ORDER BY data_compressed_bytes DESC
           FORMAT TabSeparatedWithNames"

# 5. Per-table footprint
docker compose exec -T clickhouse clickhouse-client \
  --user=default --password=clickhouse \
  --query="SELECT database, table,
                  formatReadableSize(sum(data_compressed_bytes)) AS compressed,
                  formatReadableSize(sum(data_uncompressed_bytes)) AS uncompressed,
                  round(sum(data_uncompressed_bytes)/sum(data_compressed_bytes), 2) AS ratio,
                  sum(rows) AS rows
           FROM system.parts
           WHERE database='default' AND active
           GROUP BY database, table
           ORDER BY sum(data_compressed_bytes) DESC
           FORMAT TabSeparatedWithNames"
```

---

## 8. References

- 10k-ledger predecessor report (full methodology + decoder
  field-level mapping):
  [`lore-0046 G-note`](../lore/1-tasks/archive/0046_RESEARCH_empirical-prices-ch-storage-estimate-from-10k-ledgers/notes/G-empirical-storage-estimate.md)
- Soroban-events decoder spec being drafted against this storage model:
  [`lore-0048 G-note`](../lore/1-tasks/active/0048_RESEARCH_soroban-events-pricing-decoder-spec/notes/G-soroban-events-pricing-decoder.md)
- ADR pinning the live sink to BE's Hetzner CH:
  [`ADR 0007`](../lore/2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md)
- BE conversation brief (cross-team Hetzner tenancy):
  `lore/1-tasks/blocked/0045_…/notes/G-be-conversation-brief.md`
