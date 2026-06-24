---
id: "0064"
title: "ClickHouse-backed cursor for the Prices Ledger Processor"
type: FEATURE
status: backlog
related_adr: ["0007"]
related_tasks: ["0038"]
tags: [layer-indexing, priority-medium, effort-small, lambda, clickhouse, cursor]
links:
  - "../active/0038_FEATURE_prices-ledger-processor-lambda/notes/G-local-prototype-spec.md"
history:
  - date: 2026-06-24
    status: backlog
    who: oski
    note: "Spawned from 0038 future work (spec Part D.1)."
---

# ClickHouse-backed cursor for the Prices Ledger Processor

## Summary

Replace the Lambda's `StubFileCursor` (a `/tmp` file, lost on cold start)
with a durable cursor read from / written to ClickHouse, so the
doorbell-cursor reconcile loop resumes correctly across container churn.

## Context

Task 0038 ships with `StubFileCursor` as a placeholder. The production
cursor design is the open question in `G-local-prototype-spec.md` Part D.1.
BE's cursor is `max(sequence) FROM default.ledgers`; we only persist
pricing-relevant ledgers, so `max(...) FROM prices.price_ohlcv_1m` undercounts.

## Implementation

- Lean: own single-row `prices.processed_ledgers` (ReplacingMergeTree,
  updated last per run — D.1 option 1).
- Implement `Cursor` over `prices-clickhouse` (mTLS client); wire into
  `main.rs` in place of `StubFileCursor`.
- Decide seed-on-empty behaviour (env `INITIAL_CURSOR` vs first-S3-probe).

## Acceptance Criteria

- [ ] `prices.processed_ledgers` (or chosen design) added to the schema.
- [ ] CH `Cursor` impl; reconcile resumes from CH across cold starts.
- [ ] Idempotent: re-run from the persisted cursor is a no-op past the tip.
