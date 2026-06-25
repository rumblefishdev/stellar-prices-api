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
  - date: 2026-06-24
    status: backlog
    who: claude
    note: "Added PR #34 review context for finding #3 (cold-start rewind + bootstrap; interim INITIAL_CURSOR SSM seed shipped)."
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

## Review findings (PR #34 review, 2026-06-24)

Finding #3 (durable cursor) was confirmed in the PR #34 review, with two
concrete failure modes this task removes:

- **Cold-start rewind / reprocessing.** `/tmp` is per-container ephemeral. On
  every container recycle the cursor is lost and re-seeded from the *static*
  `INITIAL_CURSOR`, so the loop rewinds to a fixed ledger and re-walks the
  whole `INITIAL_CURSOR..tip` span. Idempotent (RMT), but the redundant S3
  fetch + decode + write is paid on every cold start; if the seed is far
  behind it can blow the Lambda timeout and livelock the doorbell.
- **Bootstrap.** Without a seed the loop errors on `cursor.read()` and DLQs
  every doorbell. Interim mitigation already shipped in PR #34: `main.rs`
  seeds from `INITIAL_CURSOR`, wired in CDK from the prices-owned SSM param
  `/prices/{env}/ledger-processor/initial-cursor` (`compute-stack.ts`). This
  task supersedes that stop-gap with the durable CH cursor and should retire
  the static seed (or keep it only as a genuine first-run bootstrap).

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
