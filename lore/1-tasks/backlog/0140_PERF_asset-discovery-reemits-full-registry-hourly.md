---
id: "0140"
title: "asset-discovery re-emits the whole asset registry every hour — 0132's defect in a second component"
type: PERF
status: backlog
related_adr: []
related_tasks: ["0132", "0133", "0067"]
tags: ["priority-medium", "effort-small", "cost", "write-amplification", "clickhouse"]
links: []
history:
  - date: 2026-08-03
    status: backlog
    who: okarcz
    note: >
      Found while verifying that the [[0072]] step-6 deploy had not reverted
      [[0132]]. `system.part_log` shows **~203,955 rows written to
      `prices.assets` every hour**, flat across the whole window and unchanged
      by the deploy — so not a regression, but the same full-registry re-emit
      0132 fixed in the ledger processor, present in `asset-discovery` and
      never addressed.
---

# `asset-discovery` re-emits the full asset registry every hour

## Summary

`packages/asset-discovery/src/lib.rs:237-249`:

```rust
if scanned > 0 {
    writer.write_assets(&registry).await?;          // ← UNCONDITIONAL, full registry
    // Only re-write the registry when the scan changed it. The pre-seeded
    // registry is re-emitted verbatim on every zero-discovery run, and a full
    // RMT re-INSERT each hour would pile up parts and inflate the next FINAL
    // load. ...
    if final_rows != loaded_rows {
        writer.write_pool_registry(&pools).await?;  // ← what the guard protects
    }
    save_cursor(writer, last).await?;
}
```

`write_assets` writes **every row in the registry** on every run where
`scanned > 0`. The rule is `rate(1 hour)` (`infra/envs/production.json:17`).

## ⚠️ The comment is the trap

The comment describing the "only re-write when the scan changed it" guard sits
**directly above the unguarded `write_assets` call**, but the guard it describes
protects `write_pool_registry` below it. Read top-down it looks like
`write_assets` is the thing being guarded. It is not.

That misplacement is very likely why this survived 0132: the fix went into the
ledger processor, and anyone auditing `asset-discovery` afterwards would read
this comment and conclude it was already handled.

## Measured on ch-prod-01, 2026-08-03

```
hr                    parts   rows_written
2026-08-03 08:00:00      19        203953
2026-08-03 09:00:00       2        203954
2026-08-03 10:00:00       1        203953
2026-08-03 11:00:00       2        203955
2026-08-03 12:00:00       1        203954
2026-08-03 13:00:00       6        203966
```

One full-registry re-emit per hour: **~4.9M rows/day** of pure write
amplification into a `ReplacingMergeTree` that collapses essentially all of it on
the next merge. The row count tracks the registry size (~204k), not any real rate
of asset discovery — genuine new assets are a handful per hour at most.

**Not a correctness problem** — RMT dedup means no consumer sees a wrong value,
exactly as in 0132. The cost is egress to Hetzner, merge pressure on a cluster
shared with BE, and `FINAL` read cost against the accumulated parts.

## Implementation

Apply the same shape the pool-registry write already uses: compare the row set
and skip the write when nothing changed, or write only newly-assigned assets as
[[0132]] did for the ledger processor.

- **Move or rewrite the misleading comment** so it sits with the guard it
  describes. This is half the value of the task.
- Check the other `write_assets` call site (`lib.rs:102`, the seed path) — a
  full write is correct there, so the fix must not break seeding.
- Audit the remaining writers found alongside this one for the same pattern:
  `oracle-worker`, `sdex-backfill`, `events-backfill`, `prices-ingest-core`.
- Measure before/after from `system.part_log` on the same query as above.

## Acceptance Criteria

- [ ] A zero-discovery hourly run writes **no** `prices.assets` rows.
- [ ] Newly-discovered assets are still persisted (seed path unaffected).
- [ ] The comment sits with the guard it actually describes.
- [ ] Other `write_assets` callers audited for the same defect.
- [ ] `part_log` shows the hourly ~204k rows gone, measured on prod.
