---
id: "0065"
title: "Periodic OHLCV re-aggregation for cross-chunk intra-minute candles"
type: FEATURE
status: completed
related_adr: ["0004", "0007"]
related_tasks: ["0038", "0039", "0101", "0108"]
tags: [layer-indexing, priority-medium, effort-medium, clickhouse, ohlcv]
links:
  - "../archive/0048_RESEARCH_soroban-events-pricing-decoder-spec/notes/G-soroban-events-pricing-decoder.md"
history:
  - date: 2026-06-24
    status: backlog
    who: oski
    note: "Spawned from 0038 future work (cross-invocation intra-minute merge gap)."
  - date: 2026-06-24
    status: backlog
    who: claude
    note: "Added PR #34 review context: finding #1 (live-path frequency correction) and finding #5 (version-namespace overflow caveat for the merge fix)."
  - date: 2026-07-20
    status: completed
    who: okarcz
    note: >
      **DONE — fixed in the backfill/live rewrite, never closed.** Verified in
      the 0108 post-M1 grooming sweep by reading the code, not the task prose.
      Both writers now hold the boundary minute open instead of emitting two
      partial candles for it:
      (1) backfill — run-level accumulator state persists across chunks and
      drains via flush_older_than(current_minute), so a minute straddling a
      chunk boundary stays open (events-backfill/src/run.rs:198-205, :85-93);
      final flush_all() drains the trailing minute (:326-333). There is a
      regression test named for this exact failure mode:
      minute_split_across_calls_is_summed_once_not_undercounted (run.rs:544).
      (2) live — one accumulator per contiguous run with a single flush_all() at
      the end (prices-ledger-processor/src/reconcile.rs:100-101, :151-157),
      which also retires review finding #1 (the "every invocation" framing).
      Chosen approach was accumulator lifetime, not the engine change the task
      sketched — price_ohlcv_1m is still ReplacingMergeTree(version)
      (init.sql:120), so review finding #5 (version-namespace overflow) never
      became load-bearing and is moot as written.
      Residual: the guard is per-process, so two SEPARATE invocations splitting a
      minute still under-count. Salvaged to 0101 §Cross-invocation minute
      boundary, which is the task that actually picks run bounds.
---

# Periodic OHLCV re-aggregation for cross-chunk intra-minute candles

## Summary

Close the intra-minute aggregation gap shared by **both** writers: the live
Lambda (per contiguous run) and the backfill (per partition) accumulate
candles in memory and flush per chunk. When a single minute spans two
chunks/invocations, two rows land with the same PK but different `version`,
and `ReplacingMergeTree(version)` keeps only the latest — dropping the other
chunk's trades for that minute.

## Context

`price_ohlcv_1m` is `ReplacingMergeTree(version)` keyed by
`(asset_id, quote_asset_id, source, timestamp)`. RMT **replaces**, it does
not sum — so per-chunk partial candles for a boundary minute don't merge.
Negligible-but-real (one minute per chunk boundary). Same root cause for live
and backfill since both now use `prices-ingest-core`'s `CandleAccumulator`.

## Review findings (PR #34 review, 2026-06-24)

**Finding #1 — the live-path frequency is NOT negligible.** "One minute per
chunk boundary" holds for the backfill (large partitions), but the live Lambda
calls `flush_all()` every invocation (`reconcile.rs`), and with
`MAX_ITERATIONS=16` a run spans ~80-96s of ledgers — so a minute boundary
falls inside essentially *every* invocation. That is roughly one corrupted
(under-counted volume / wrong `open`) boundary minute per run in the live path,
not a rare edge. The in-code comment equating it with the backfill's partition
boundaries understates it; the fix is materially more impactful for 0038 than
the "negligible" framing suggests.

**Finding #5 — the `version` scheme can invert across ledgers, which
constrains the fix.** `version = ledger_seq*1000 + operation_index`
(`bucket.rs`) assumes `operation_index < 1000`, but the AMM path sets it to
`first_event_index & 0xFFFF` (0..65535; `first_event_index` is `u32` in
`extractors-core`). A tx emitting ≥1000 events overflows the per-ledger
namespace, so a *later* ledger's candle can carry a *lower* `version` than an
earlier one. Any re-aggregation that relies on "higher version wins" must not
assume `version` is monotonic in ledger order — either widen the multiplier /
pack `(ledger, event_index)` without truncation, or make the merge
order-independent (Summing/Aggregating engine). Note: changing the version
formula also touches already-written backfill rows, so it needs a migration
decision.

## Implementation (options to evaluate)

- A periodic worker (task 0039 family) that re-reads raw trades/`_1m FINAL`
  and rewrites boundary minutes with a higher `version`; OR
- An `AggregatingMergeTree` / SummingMergeTree variant for the write path so
  partial candles combine on merge; OR
- Emit candles keyed to include a chunk discriminator and re-roll at read.

## Acceptance Criteria

- [x] A minute split across two runs/chunks aggregates to one correct candle.
      — within a process run; cross-invocation residual → 0101.
- [x] Fix applies to both live (0038) and backfill writers (shared core).
- [x] Regression test with a deliberately split-minute fixture.
      — `minute_split_across_calls_is_summed_once_not_undercounted`, `run.rs:544`.
