---
id: '0046'
title: 'Empirical prices-api CH storage estimate from 10k mainnet ledgers — extrapolate to Hetzner server + monthly/yearly cost'
type: RESEARCH
status: completed
related_adr: ['0007']
related_tasks: ['0045', '0044', '0047']
tags:
  [
    layer-research,
    priority-high,
    effort-medium,
    hetzner,
    clickhouse,
    sizing,
    cost,
    capacity,
  ]
links:
  - '../blocked/0045_RESEARCH_cross-team-bundle-with-be-on-hetzner-ch-tenancy/notes/G-be-conversation-brief.md'
  - '../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md'
  - '../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/R-cost-delta.md'
  - '../archive/0044_RESEARCH_refactor-architecture-shared-galexie-hetzner-clickhouse/notes/R-ingest-target-mapping.md'
history:
  - date: 2026-05-19
    status: backlog
    who: okarcz
    note: >
      Spawned because task 0045's Cluster D (Money) and Cluster B
      (Capacity) need empirically-grounded numbers, not the
      hand-waved 5-10% pro-rata fraction from 0044's R-cost-delta.
      BE's backfill directory at
      /home/oski/Projects/stellar/soroban-block-explorer/.temp/backfill-runner/FC4DB5FF--62016000-62079999/
      already contains 64,000 production ledgers — enough for a
      10k-ledger sample. Crucial to closing 0045 with real numbers
      in the BE conversation.
  - date: 2026-05-19
    status: active
    who: okarcz
    note: >
      Promoted to active immediately on creation. Blocking 0045
      closure on the empirical capacity / cost numbers, so no
      time spent sitting in backlog.
  - date: 2026-05-19
    status: completed
    who: okarcz
    note: >
      Closed. Backfilled 10k ledgers (62070000-62079999, 8.75M events)
      from BE's mainnet sample into local ClickHouse via BE's
      backfill-runner. Measured per-signature event rates and per-
      column compression on soroban_events (14.8x ratio).
      Headline: prices-api stores ~74 bytes/ledger, ~0.45 GB/year
      flat growth — order of magnitude smaller than the 0044
      hand-waved 5-10% pro-rata estimate. Updates fed into the
      brief (task 0045) shifting Cluster D opening to ~1-2% / $1-2
      per env per month, and Cluster B retention concern dropped.
      Report at notes/G-empirical-storage-estimate.md (PR #22,
      merged as ae0bc6b). Throughput/concurrency verification
      (not in scope here) spawned as task 0047.
---

# Empirical prices-api CH storage estimate from 10k mainnet ledgers

## Summary

Take a **10,000-ledger sample** from BE's backfilled mainnet ledger
directory, count the prices-api-relevant events, model the resulting
ClickHouse row footprint, extrapolate to daily / monthly / yearly
storage, and price the corresponding Hetzner server tier.

The output is empirical replacements for two hand-waved numbers in
task 0044's `R-cost-delta.md`:

1. The ~5% storage / ~5% rows / ~10% CPU pro-rata fraction in
   §6 (basis for the 5-10% cost-share opening proposal in
   task 0045 Cluster D).
2. The "indefinite storage growth is fine on the BE-owned box"
   assumption — i.e. confirm the Hetzner box has years of
   headroom before prices-api alone could fill it.

## Context

- **Source data:** `/home/oski/Projects/stellar/soroban-block-explorer/.temp/backfill-runner/FC4DB5FF--62016000-62079999/` — 64,000 `.xdr.zst` files (~172–191 KB compressed each), one ledger per file, SEP-0054 naming (`{MaxUint32 - seq}--{seq}.xdr.zst`). Ledger range 62,016,000 → 62,079,999 (~mid-April 2026).
- **Target schema:** the `prices.*` table set from task 0044's `R-ingest-target-mapping.md`:
  - `price_ohlcv_1m` (per-source rows, `ReplacingMergeTree(version)`)
  - `current_prices` (`ReplacingMergeTree(updated_at)`)
  - `oracle_prices` (`MergeTree`)
  - `assets`, `backfill_progress` (`ReplacingMergeTree(updated_at)`)
  - Plus the MV chain `1m → 15m → 1h → 4h → 1d → 1w → 1M`.
- **What prices-api consumes from each ledger:** Soroswap / Aquarius / Phoenix pool swap events, oracle price updates (Reflector), SEP-41 token transfers tied to tracked AMM pools. **Not** every Soroban event — only the subset matching the asset / pool registry.

## Research plan

### Step 1: Tooling

Use BE's existing XDR parsing crate (`/home/oski/Projects/stellar/soroban-block-explorer/crates/`) to decompress + parse the ledger files. Write a small Rust binary or script in `scripts/sizing/` that:

- Reads N ledger files from the sample directory.
- Filters events for prices-api-relevant contract calls (initially: the contract address allow-list lives in the design doc §2.2; if not enumerated, derive from the on-chain pool registries).
- Emits per-ledger counts + per-event row-size estimates as CSV/JSON.

### Step 2: Sampling

Pick **10,000 consecutive ledgers** from the sample (e.g. ledgers 62,070,000 → 62,079,999 — the most recent block of the sample). Consecutive over random because the temporal distribution of trading activity matters for event-rate estimation (weekend vs. weekday, US vs. Asia hours).

### Step 3: Event extraction + counting

Per ledger, count:

- Soroswap `swap` events (per pool contract).
- Aquarius `swap` / `deposit` / `withdraw` events.
- Phoenix `swap` events.
- Reflector oracle price updates (`set_price` or equivalent).
- SEP-41 `transfer` events on tracked pool reserves (informational, not all are price events).

Per-event-class statistics: mean, p50, p95, max per ledger; total over the 10k-ledger window.

### Step 4: Row footprint modeling

For each event class, model the resulting CH rows:

- **Soroswap swap** → one `price_ohlcv_1m` row per (asset, quote_asset, source) per minute it lands in. ~150-200 bytes/row in CH after compression (Decimal128, FixedString(56), DateTime).
- **Oracle update** → one `oracle_prices` row. ~120 bytes/row.
- **Current price update** → at most one `current_prices` row per asset per minute (via the Updater Lambda).
- MV chain rollups: each 1m row contributes to 6 downstream rows (15m, 1h, 4h, 1d, 1w, 1M) at decreasing densities — model as multiplier ~1.1× over the base 1m footprint.

Apply ClickHouse compression ratio: assume **5×** as the steady-state baseline (LZ4 + sort-key compression on price/volume columns; cite a CH benchmark or measure on a real prices DB if accessible). Sanity-check against BE's actuals if `default.*` compression ratio is published.

### Step 5: Extrapolation

- Mainnet ledger rate: **~5 seconds/ledger** → **17,280 ledgers/day**, **525,960/month** (30.42 days), **6,311,520/year**.
- Project per-day / per-month / per-year row counts and storage based on the 10k sample's event rate.
- Apply growth scenarios: (a) flat (current rate), (b) 3× (modest pool expansion), (c) 10× (aggressive — many new pools, oracle proliferation).

### Step 6: Hetzner sizing + cost

Match estimated storage growth against Hetzner dedicated server tiers (current 2026 line-up):

- **AX41-NVMe** — €39/mo, ~512 GB NVMe RAID 1.
- **AX52** — €69/mo, ~1 TB NVMe RAID 1.
- **EX44** — €44/mo, ~1 TB SSD RAID 1.
- **AX102** — ~€110/mo, 2× 1.92 TB NVMe.

Output:

- **Year 1**: storage footprint + headroom on each tier.
- **Year 5**: same, with all three growth scenarios.
- **When does the box fill** under each scenario? If prices-api alone could fill the box within 5 years on the assumed tier, this informs the Cluster B retention conversation (do we need to introduce a retention policy on rolled-up granularities)?

Cost output:

- USD/EUR conversion at current rate.
- Monthly + yearly Hetzner cost contribution attributable to prices-api (under each growth scenario).
- Compare against the 5-10% pro-rata fraction in 0044's `R-cost-delta.md` §6 — confirm, adjust up, or adjust down.

## Acceptance Criteria

- [x] `notes/G-empirical-storage-estimate.md` — landed. Methodology, per-event counts (Phase A 1,654-ledger reference + Phase B 10k-ledger primary), measured CH compression on `soroban_events`, row-generation model, year-1/5/10 extrapolations × 5 growth scenarios, Hetzner tier match (AX41-NVMe → AX102), USD/EUR pro-rata cost.
- [~] `scripts/sizing/` — not a separate directory; reproduction commands documented inline in §8 of the report (backfill-runner + curl queries). No bespoke Rust script needed; BE's `backfill-runner` + `clickhouse-client` cover the workflow.
- [x] Cross-link the report from task 0045's `notes/G-be-conversation-brief.md` Cluster B (capacity) and Cluster D (cost-share) — done on PR #21 commit 982ba25. Brief's Cluster D opening shifted from 5-10% to ~1-2% pro-rata; Cluster B retention concern dropped.
- [~] Update task 0044's `R-cost-delta.md` §6 — deferred. The synthesis note already references 0046; the cost-delta note is in archive and refactoring its §6 would be cosmetic. The brief + agreement record are the load-bearing documents.
- [x] **Reproducible**: §8 of the report documents the full command sequence. Re-runs against any 10k-ledger window in ~7-8 minutes wall-clock.

## Implementation notes

- Local docker-compose ClickHouse was reset before this work; the JSONL samples in `lore/4-notes/samples/soroban-events/` represented a prior 1.65k-ledger run. The fresh 10k-window backfill (62070000-62079999) was the primary data source; 1.65k window stayed as a reference for comparison.
- The 1.65k reference window showed ~30% higher trading activity than the 10k average — temporal trading-hour bias. Confirmed by Phase A vs. Phase B side-by-side in §2.2.
- Per-column compression measurement on `soroban_events` was the biggest single insight: 14.8× ratio vs. the 3× assumed in the initial draft. Drove the per-row size estimate from ~95 bytes to ~55 bytes for `price_ohlcv_1m`.

## Design decisions (emerged)

- **Scoped strictly to storage / row volume.** Throughput, concurrent-query load, CPU contention from MV chain — all explicitly out of scope here; surfaced as task 0047.
- **Per-source rows (CH-B) for `price_ohlcv_1m` is the modeled shape.** Compaction reduction was empirically modest (~10%) because event distribution across distinct (pool, minute) buckets is sparse — most events fall in unique buckets.

## Future work / spawned

- **Task 0047** — Cross-tenant throughput verification (spawned from task 0045's BE agreement record, not from this task directly, but informed by this task's exclusion of throughput from scope).
- BE could publish their measured `default.*` compression ratio and storage rate for a true measured pro-rata fraction (this task used estimated denominators).

## Out of scope

- Backfill-data sizing (Stream 1 / Stream 2 of ADR 0001 / 0005) — that's workstation-local, doesn't hit the Hetzner box.
- Compression-ratio measurement against a real CH instance — assume 5× and flag if the brief needs validation against BE's actuals.
- AWS-side Lambda invocation count + cost — already covered in 0044's `R-cost-delta.md` §3-4.
- Hetzner Storage Box (BX21) sizing for backups — daily Borg dedup math, separate small note if needed.

## Notes

- The 10k-ledger sample is **~14 hours of mainnet** (10,000 × 5s = ~13.9 hours). Activity will skew toward whatever window 62,070,000-62,079,999 represents. Sanity-check the day-of-week / time-of-day — if the window is e.g. a weekend morning, weight downward.
- Pool registries: if the contract allow-list is not enumerated in the prices-api design doc, derive it on the fly by scanning the sample for any contract emitting events matching the swap shape — produces a coarser estimate but still empirical.
- Block-explorer team has their own row-count statistics in `default.*`; ask if they can share a compression ratio number to validate the assumed 5×. (Cluster B in the BE brief — this task may slip an ask into the brief.)
- This task is **fast-turnaround** (1-3 person-days) and crucial to closing 0045. Do not let it grow scope — the empirical answer is the deliverable, not a full capacity-planning treatise.
