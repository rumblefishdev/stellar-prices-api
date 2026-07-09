---
id: "0090"
title: "Backfill loses history — wire preroll + cleanup-coordination into the backfill workflow"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0088", "0053", "0039", "0051", "0059", "0060"]
tags: [layer-infra, priority-high, effort-medium, milestone-M1, backfill, clickhouse, retention, rollup, data-loss, blocker]
milestone: 1
links:
  - "../../../docs/runbooks/continue-soroban-backfill.md"
  - "../../../packages/prices-clickhouse/schema/preroll.sql"
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../packages/cleanup-worker/src/lib.rs"
history:
  - date: 2026-07-09
    status: backlog
    who: okarcz
    note: >
      Discovered while running the 0088 backfill from a second machine. The
      backfill writes price_ohlcv_1m only; the live rollup MVs are refreshable
      LIVE-only (2h window) and ignore historical rows; the cleanup worker
      drops historical 1m partitions nightly (7d retention). Net: every
      backfilled candle is deleted un-rolled — the coarse forever-tables
      (1h/4h/1d/1w/1M) that the BE consumer reads are empty. BLOCKS 0088.
---

# Backfill loses history — wire preroll + cleanup-coordination into the backfill workflow

## Summary

The historical backfill (`sdex-backfill`, task 0088) writes candles **only** to
`prices.price_ohlcv_1m`. But `price_ohlcv_1m` is a **transient 7-day feeder**, not
a store of record. The durable history is supposed to live in the forever-retained
coarse tables (`price_ohlcv_1h/4h/1d/1w/1M`), populated by the rollup chain. Two
facts make the backfill's output vanish:

1. The rollup MVs (`schema/rollups.sql`) are **refreshable, LIVE-only, `now() - INTERVAL 2 HOUR`** windowed — they deliberately ignore historical/backfilled rows (their own header says so). Historical data must be rolled up by **`schema/preroll.sql`** instead.
2. The **cleanup worker** (`prices-{env}-cleanup`, EventBridge `cron(0 2 * * *)`) drops every monthly `1m` partition older than 7 days — i.e. **all** backfilled history — nightly.

The backfill runbook has no preroll step and no cleanup coordination, so backfilled
`1m` data is written, never pre-rolled, and partition-dropped (often within hours,
at the next 02:00–03:00 UTC cleanup). **The coarse forever-tables are empty; the BE
consumer's 1h/1d surface has no history.**

This is a **workflow gap, not a code bug** — the extractor and rollup SQL are
correct (proven below). It **blocks 0088** (the backfill produces nothing durable
until fixed).

## Context / Evidence (measured on ch-prod-01, 2026-07-09)

- `price_ohlcv_1m`: 40.4M sdex rows, oldest `2024-07-11`, newest `2026-07-08 00:37`.
- `price_ohlcv_15m` / `_1h` / `_1d`: **empty**. `_1M`: only `2026-07-01`, ~5k rows.
- `SHOW CREATE TABLE price_ohlcv_1m` → **no TTL** (retention is the cleanup worker, not DDL).
- `system.query_log`: nightly `ALTER TABLE prices.price_ohlcv_1m DROP PARTITION 2024xx`
  at ~03:00 UTC (rows: 202403–202407 dropped 2026-07-09 03:00).
- Rollup MVs present in prod: `mv_ohlcv_1m_to_15m … _1w_to_1M` (all 6 + `mv_current_prices`).
- `rollups.sql` MV body: `FROM price_ohlcv_1m FINAL WHERE t.timestamp >= now() - INTERVAL 2 HOUR`.
- Extractor proven correct: local decode of archive ledger `51050000` (pre-floor,
  no candles in DB) → 133 SDEX trades → **114 candles** through the real pipeline
  (`decode_object` → `extract_trades` → `raw_trade_to_tick` → `CandleAccumulator`).
  Archive files below the floor are full-size (data is present). So candles are
  generated correctly; they are lost at the destination, not at extraction.
- Cleanup retention (`cleanup-worker/src/lib.rs`): `1m`=7d, `15m`=30d, `oracle`=13mo;
  `1h/4h/1d/1w/1M` retained forever.

Also observed (separate, worth its own check): **live ingestion appears stopped** —
newest `1m` row is `2026-07-08 00:37` (~1.3 days stale), which is also why the 2-hour
live MVs currently produce nothing even for live data.

## Corrected workflow (the fix)

A bulk historical backfill must keep the full `1m` range present long enough to
pre-roll it into the coarse tables, then let cleanup reclaim the `1m` space. Order:

1. **Disable the cleanup worker** for the duration:
   `aws events disable-rule --name prices-production-cleanup` (re-enable after).
2. **Run the backfill** into `1m` (task 0088). Disk on ch-prod-01 must hold the full
   `1m` history transiently — size this first (see risks).
3. **Pre-roll** the coarse tables from the fully-written `1m` via `preroll.sql`
   (6 INSERT…SELECT statements, `1m→15m→1h→4h→1d→1w→1M`). Optionally `TRUNCATE`
   the coarse tables first for a clean, idempotent result.
4. **Verify** coarse tables now hold the full historical range (min timestamp back
   to genesis / activation; per-source counts sane).
5. **Re-enable the cleanup worker**:
   `aws events enable-rule --name prices-production-cleanup`. It drops the now-redundant
   historical `1m`/`15m` partitions; the coarse forever-tables retain the history.

Alternative to weigh (design decision for the owner): have the backfill itself write
directly to (or pre-roll per-chunk into) the coarse tables, avoiding the need to hold
the entire `1m` history at once. Per-chunk preroll bounds disk but complicates the
`FINAL`/dedup semantics at chunk seams.

## Acceptance Criteria

- [ ] Decision recorded: full-range preroll (disable-cleanup) vs per-chunk/direct-to-coarse.
- [ ] `docs/runbooks/continue-soroban-backfill.md` updated with the preroll + cleanup-
      coordination steps (currently stops at writing `1m`).
- [ ] Cleanup-disable / re-enable procedure documented (rule name, env scoping, who owns it).
- [ ] Disk-headroom check for holding full `1m` history on ch-prod-01 documented.
- [ ] After a re-run: coarse `1h`/`1d` hold the backfilled historical range (verified query).
- [ ] Separately: confirm/ξ triage whether live ingestion is stopped (newest `1m` stale).

## Risks / Notes

- **Disk**: holding the entire `1m` history (2015→now) on the shared ch-prod-01
  defeats the point of the 7d retention temporarily; must confirm headroom before
  disabling cleanup, or use per-chunk preroll.
- **Shared cluster**: cleanup rule + any prod schema/DML changes affect the shared
  BE ClickHouse — coordinate + get owner sign-off (see [[flag-container-restarts]],
  [[feedback-prepare-not-deploy]]).
- **Idempotency**: `preroll.sql` and the backfill are ReplacingMergeTree-idempotent;
  re-runs collapse duplicates. Safe to re-run.

## Investigation artifact

`packages/prices-ingest-core/examples/decode_probe.rs` was added to prove the
extractor produces candles for pre-floor ledgers. Keep as a diagnostic or move to
`.trash/` — it is not part of the fix.
