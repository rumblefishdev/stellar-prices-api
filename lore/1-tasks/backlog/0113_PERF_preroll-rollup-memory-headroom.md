---
id: "0113"
title: "Pre-roll rollup INSERTs use half the CH memory quota — check headroom before 0088's recovery step 3"
type: PERF
status: backlog
related_adr: ["0007"]
related_tasks: ["0088", "0111", "0090", "0095"]
tags: [layer-database, clickhouse, preroll, rollup, memory, priority-medium, effort-small]
links:
  - "../../../packages/prices-clickhouse/schema/preroll-incremental.sql"
  - "../../../docs/runbooks/preroll-incremental-presoroban.md"
history:
  - date: 2026-07-21
    status: backlog
    who: okarcz
    note: >
      Spawned from the 0111/0112 enrichment investigation. While reading prod
      system.query_log for the enrichment outage, the single most expensive
      statement in the window was not enrichment at all: the price_ohlcv_15m
      rollup INSERT at 217s avg / 301s max, reading 177M rows and peaking at
      2938 MB against a 5.59 GiB quota. Two Code 241 memory-limit exceptions
      occurred in the same window. 0088's recovery step 3 requires running this
      pre-roll, so headroom should be checked before committing to it.
---

# Pre-roll rollup INSERTs use half the ClickHouse memory quota

## Summary

The `price_ohlcv_15m` rollup `INSERT … SELECT` peaks at **2,938 MB against a
5.59 GiB query quota** — over half — while reading 177M rows. In the same window
ClickHouse threw two `Code: 241` memory-limit exceptions
(`would use 5.60 GiB … maximum 5.59 GiB`).

This has not failed the pre-roll yet. The concern is that **0088's recovery
step 3 depends on running `preroll-incremental.sql`** over the pre-Soroban tail,
which will be a larger input than anything measured here.

## Correction to how this was first described

Initially recorded (in 0111's out-of-scope) as *"same 300 s wall, different
statement"*, implying a Lambda timeout. **That is wrong.** `price_ohlcv_15m` is
not referenced anywhere in `infra/`, and the pre-roll runs as operator SQL via
`docker exec … clickhouse-client` (`docs/runbooks/preroll-incremental-presoroban.md:65`).
There is no 300 s timeout on it — the 301 s max is simply a slow query, and the
resemblance to enrichment's 300 s wall is a coincidence.

The real exposure is **memory quota**, not duration. Recorded because the wrong
framing would have sent the next reader looking for a timeout that does not exist.

## Evidence

Prod `system.query_log`, 2026-07-10 → 07-18:

| statement | runs | avg | max | rows read | peak mem |
|---|---|---|---|---|---|
| `INSERT INTO prices.price_ohlcv_15m SELECT toStartOfInterval(…)` | 9 | 217 s | 301 s | 177 M | **2,938 MB** |

Exceptions in the same window:

```
2026-07-15  ExceptionWhileProcessing  Code: 241 … would use 5.60 GiB … maximum 5.59 GiB
2026-07-17  ExceptionWhileProcessing  Code: 241 … would use 5.59 GiB … maximum 5.59 GiB
```

**Not established:** whether those exceptions came from the rollup INSERT or a
different statement. The query that found them grouped by day and type, not by
query shape. That is the first thing to determine — it decides whether this is a
live failure or spare headroom.

## Implementation

1. **Attribute the Code 241 exceptions.** Re-query `system.query_log` filtering
   `type != 'QueryFinish'` and grouping by `normalizeQuery(query)`. If the rollup
   INSERT is among them, this is already failing intermittently.
2. **Measure headroom for the pre-Soroban input**, which is larger than the
   Soroban-era rows measured here. `preroll-incremental.sql` chunks by year; 0090
   found month-chunking necessary at one point for exactly this reason.
3. **If headroom is thin**, options in rough order of preference: chunk more
   finely (month rather than year — the 0090 precedent), set a per-query
   `max_memory_usage`/`max_bytes_before_external_group_by` for the pre-roll
   session, or ask the cluster owner about the quota.
4. Fold the outcome into `docs/runbooks/preroll-incremental-presoroban.md` as a
   pre-flight check, so the operator knows before starting a multi-hour run.

## Acceptance Criteria

- [ ] The `Code: 241` exceptions are attributed to specific query shapes.
- [ ] Peak memory for the pre-Soroban pre-roll is estimated or measured against
      the 5.59 GiB quota, with the chunking strategy that keeps it clear.
- [ ] `preroll-incremental-presoroban.md` carries a memory pre-flight step.
- [ ] 0088 recovery step 3 can be started without an open question about whether
      it will OOM partway through a multi-hour run.

## Out of scope

- The enrichment full-table scans — that is [[0111]]. Different statements,
  different cause; they merely showed up in the same query-log window.
- Rollup MV cadence/window tuning — that is 0104.
