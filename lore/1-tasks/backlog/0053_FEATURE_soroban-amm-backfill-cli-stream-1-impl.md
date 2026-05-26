---
id: '0053'
title: 'Soroban AMM Backfill CLI (`soroban-amm-backfill`) — Stream 1 implementation per ADR 0001'
type: FEATURE
status: backlog
related_adr: ['0001', '0003', '0004', '0007']
related_tasks: ['0017', '0034', '0037', '0048', '0052', '0051']
tags:
  [
    layer-indexing,
    priority-high,
    effort-large,
    milestone-M1,
    stream-1,
    rust,
    cli,
    workstation,
    clickhouse,
    soroban,
    amm,
    soroswap,
    aquarius,
    phoenix,
  ]
milestone: 1
links:
  - '../../../docs/prices-api-general-overview.md'
  - '../../2-adrs/0001_stream1-clickhouse-sourced-amm-backfill.md'
  - '../../2-adrs/0003_price-ohlcv-pk-includes-quote-asset-id.md'
  - '../../2-adrs/0004_price-ohlcv-multi-source-merge-columns.md'
  - '../../2-adrs/0007_live-data-sink-on-shared-hetzner-clickhouse.md'
  - '../archive/0048_RESEARCH_soroban-events-pricing-decoder-spec/notes/G-soroban-events-pricing-decoder.md'
  - '../archive/0018_RESEARCH_decode-per-amm-swap-event-shapes/notes/G-amm-swap-event-shapes.md'
  - './0017_FEATURE_local-clickhouse-for-prices-backfill.md'
  - './0037_FEATURE_tranche1-ledger-processor-skeleton.md'
  - '../blocked/0034_FEATURE_consumer-multi-xyk-wasm-tolerance.md'
history:
  - date: 2026-05-21
    status: backlog
    who: okarcz
    note: >
      Spawned during Tranche 1 task-set creation. ADR 0001 commits
      Stream 1 to a local-CH-sourced workstation CLI; 0017 covers
      the CH instance setup; 0037 covers the dispatch kernel; 0034
      covers Phoenix WASM tolerance; 0048 carries the decoder spec.
      No task owns the actual `soroban-amm-backfill` binary —
      decode loop, bucket to 1-min OHLCV, write to local Postgres,
      run the one-shot completion push to Hetzner CH. This task
      fills the gap.
---

# Soroban AMM Backfill CLI (`soroban-amm-backfill`)

## Summary

Build the one-shot workstation CLI specified in ADR 0001: a Rust
binary that reads the operator's local ClickHouse `soroban_events`
(populated upfront by BE's `backfill-runner --target=clickhouse`
per 0017), decodes Soroswap / Aquarius / Phoenix swap events via
the `stellar-xdr` crate using the 0048 decoder spec, buckets the
results into per-source 1-min OHLCV rows in a local Postgres, and
runs a one-shot completion push to Hetzner ClickHouse `prices.*`
that lands the historical rows and flips
`prices.backfill_progress.soroban_amm` to `status='completed'`.

## Context

Per ADR 0001 §Decision and the design doc §5.6 Stream 1, the
Soroban AMM historical backfill (Nov 2023 → present, ~8.5M
ledgers) is **not** a Fargate task: it runs as a local Rust CLI
on the operator's workstation. The runtime flow is:

1. BE's `backfill-runner --target=clickhouse` populates a
   Docker-hosted local ClickHouse with `soroban_events` rows
   for the Soroban-activation-onward ledger range (0017 owns
   this prep step).
2. `soroban-amm-backfill` queries the local CH filtered by
   `signature = 'swap'` AND `contract_id IN (Soroswap, Aquarius,
Phoenix registry)`. Note the Aquarius / Phoenix String-typed
   topic[0] issue (task 0031) requires per-AMM filter logic.
3. Each event's `topics_xdr` + `data_xdr` are decoded into a
   `TradeTick` per the 0048 decoder spec.
4. Ticks are bucketed to 1-min OHLCV per ADR 0003 PK shape
   (timestamp, asset_id, quote_asset_id, source) and written to
   a local Postgres (Docker) on the workstation.
5. On completion, the cloud-push step streams the local
   `price_ohlcv_*` rows into Hetzner CH `prices.*` via the 0052
   shared mTLS client, then flips `backfill_progress.soroban_amm`
   to `status='completed'`. The local CH instance is torn down
   post-push.

Total wall-clock: a few hours, dominated by 0017's
`backfill-runner` archive ingestion.

## Implementation Plan

### Step 1: CLI scaffolding

Add `packages/soroban-amm-backfill/` as a binary crate.
Dependencies:

- `clickhouse` (raw, not the 0052 wrapper — reads from the local
  plaintext CH, no mTLS).
- 0052 shared CH client for the cloud-push step (mTLS to Caddy).
- `stellar-xdr` — official SDF Rust XDR types for ScVal decoding.
- `sqlx` (Postgres) — local Postgres sink.
- `clap` — CLI argument parsing.
- `tracing` + `tracing-subscriber` — structured logs.

CLI surface:

```
soroban-amm-backfill
  --local-ch-url URL         (default http://localhost:8123)
  --local-pg-url URL         (default postgres://localhost/prices_backfill)
  --start-ledger SEQ
  --end-ledger SEQ
  --venues VENUES            (comma-separated: soroswap,aquarius,phoenix; default all)
  push                       (subcommand to run the one-shot cloud push)
    --target-ch-url URL      (Caddy:443)
    --target-db NAME         (default 'prices')
  decode                     (subcommand to run only the local extract step)
```

### Step 2: Pool registry loading

Reuse the per-venue registry surface from 0037 (`SwapExtractor`
trait + `PoolRegistry`). For Phoenix, accept the multi-WASM
tolerance from 0034; for Soroswap and Aquarius, use the canonical
factory-derived pool lists per 0018 / 0033.

Pool registries are loaded once at startup and cached for the
lifetime of the run.

### Step 3: Local CH event query

For each (venue, batch of ledgers) page through:

```sql
SELECT
    ledger_seq,
    closed_at,
    tx_hash,
    contract_id,
    topics_xdr,         -- BE's tagged-JSON ScVal, not XDR bytes (task 0030)
    data_xdr
FROM soroban_events
WHERE signature = 'swap' OR (venue = 'soroswap' AND topic0_string = 'SoroswapPair')
  OR (venue = 'phoenix' AND topic0_string = 'swap')
  AND ledger_seq BETWEEN ? AND ?
  AND contract_id IN (?)
ORDER BY ledger_seq, tx_hash
```

The exact predicate depends on whether 0031's hoist has landed
on BE side; if not, use per-AMM raw-topic filters. Document the
filter strategy choice in `notes/S-event-filter-strategy.md`.

### Step 4: Decode per 0048 spec

For each event, call the venue-specific decoder from
`packages/ledger-processor::dispatch` (kernel from 0037), which
returns a `TradeTick { amount_in, amount_out, asset_in,
asset_out, ... }`. The decoder is shared with the live
processor (0038), so identical decode paths are exercised here
and in live ingestion.

### Step 5: Bucket to 1-min OHLCV

For each `TradeTick`:

1. Resolve `asset_id` + `quote_asset_id` against the local
   `assets` table (UPSERT new ones discovered during decode).
2. Compute the per-tick price and per-tick `volume_quote /
volume_base` per 0048 §3.
3. Group ticks into the `(floor_minute(closed_at), asset_id,
quote_asset_id, source)` bucket and merge into local Postgres
   `price_ohlcv_1m` using ADR 0004's incremental-merge semantics
   (preserve open, overwrite close, GREATEST(high), LEAST(low),
   sum volumes + trade_count, recompute vwap).
4. Pre-roll higher granularities (15m, 1h, 4h, 1d, 1w, 1M) in
   the same pass so the cloud-push step writes already-aggregated
   rows directly to the target tables (matches design doc §3.2
   "Backfill scripts produce already-aggregated rows").

### Step 6: Cloud-push (`push` subcommand)

Streams all `price_ohlcv_*` rows + new `assets` rows from local
Postgres to Hetzner CH `prices.*` via the 0052 mTLS client:

1. Open a CH connection per granularity table.
2. Stream rows in chunks (e.g. 10k rows per INSERT) using the
   CH native protocol's bulk-INSERT path.
3. After each table completes, log row counts.
4. On success: `INSERT INTO prices.backfill_progress` row with
   `task_name='soroban_amm', status='completed',
completed_at=now(), last_push_at=now()` (ReplacingMergeTree
   collapses against the seeded row).
5. On any failure: surface the error, do not flip status, exit
   non-zero. Re-run is idempotent because `ReplacingMergeTree`
   collapses on (version) per ADR 0004.

### Step 7: Tests

- Unit: decoder paths covered by 0037 / 0048; here, test the
  bucketing + pre-roll math for at least two venue-pair scenarios.
- Integration: end-to-end against a Docker CH + Docker Postgres,
  seeded with a small recorded `soroban_events` fixture; assert
  the produced 1-min rows match a hand-computed gold file.
- Smoke: cloud-push step against a local Docker CH (no mTLS) +
  a stubbed `prices.backfill_progress` row.

### Step 8: Operator runbook

A short `RUNBOOK.md` documenting the end-to-end run sequence:

1. Run 0017's CH prep (`backfill-runner --target=clickhouse`).
2. Apply 0051's schema to Hetzner CH `prices` if not done.
3. Run `soroban-amm-backfill decode --start-ledger=… --end-ledger=…`.
4. Inspect local Postgres row counts; spot-check via SQL.
5. Run `soroban-amm-backfill push --target-ch-url=…`.
6. Confirm `GET /backfill/status` shows
   `soroban_amm.status: "completed"` (0055).
7. Tear down local CH + Postgres.

## Acceptance Criteria

- [ ] `packages/soroban-amm-backfill` binary builds and runs
      end-to-end against a Docker local CH + Docker local Postgres
- [ ] Decoder paths produce `TradeTick` records that match the
      0048 spec gold-file fixtures for all three venues
- [ ] Pre-rolled higher-granularity rows in local Postgres are
      consistent with the 1-min rows under the §3.2 MV semantics
      (`argMin(open)`, `max(high)`, `min(low)`, `argMax(close)`,
      `sum(volumes)`)
- [ ] `push` subcommand streams local rows to Hetzner CH
      `prices.*` and flips `backfill_progress.soroban_amm` to
      `completed`
- [ ] Idempotent re-run of `push` produces no duplicate rows
      after `ReplacingMergeTree` background merge (`SELECT count()
… FINAL` consistent before and after)
- [ ] OHLCV data for Soroswap pairs verifiable for Nov 2023
      dates (Tranche 1 acceptance criterion)
- [ ] Operator runbook in `RUNBOOK.md` walks through the full
      sequence; tested by a second team member following it cold

## Blocked on

- **0017** — local CH instance setup and `backfill-runner`
  prep tooling. Without 0017, the `soroban_events` source is
  empty.
- **0037** — shared `SwapExtractor` trait + dispatch kernel +
  Phoenix pool registry. Without 0037, the venue-specific decode
  is reinvented here.
- **0034** — Phoenix multi-WASM tolerance. The Phoenix
  extractor must tolerate ≥2 XYK WASM builds; without 0034 the
  PHO/USDC pair is silently dropped.
- **0048** decoder spec (archived) — the spec is mature; this
  task consumes it.
- **0052** — for the cloud-push step's mTLS client. Decode-only
  workflow can run before 0052 lands.
- **0051** — the target Hetzner CH `prices.*` schema must exist
  before the push lands data.
- **0050** — the Hetzner CH credentials and endpoint must be
  provisioned before the push lands.

## Out of scope

- Live Soroban AMM ingestion — handled by 0038 from the moment
  the local CH is populated; this task is purely historical.
- SDEX backfill — see 0027 / 0028.
- Resuming a partially-pushed run with sub-table granularity —
  v1 either pushes all granularities or none, then idempotent
  re-run cleans up. Sub-table resume is a backlog item if
  measured wall-clock makes it worth it.

## Notes

- The local CH instance is **torn down after the push**
  (ADR 0001 §Consequences). Do not treat it as a long-lived
  store.
- The decode loop and the bucketing math are shared with the
  live Ledger Processor (0038); house both in
  `packages/ledger-processor` so a bug fixed in one path also
  fixes the other.
- `backfill_progress.target_ledger` should be set to the live
  tip at the moment the run starts (decode subcommand) so
  `progress_pct` in 0055's response is meaningful during the
  run (even though for Stream 1 the run is fast enough that
  partial progress is rarely observed).
