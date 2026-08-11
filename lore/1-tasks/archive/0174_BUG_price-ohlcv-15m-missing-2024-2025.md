---
id: "0174"
title: "price_ohlcv_15m has no 2024/2025 rows — INVESTIGATED, NOT A DEFECT: the table has a 30-day retention by design"
type: BUG
status: completed
related_adr: []
related_tasks: ["0088", "0095", "0114", "0177"]
tags: [layer-data, effort-small, clickhouse, coarse-rollups, not-a-defect]
links:
  - "../../../docs/runbooks/repair-coarse-usd-values.md"
history:
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      Spawned from 0088's pre-roll pre-flight. price_ohlcv_15m returned ZERO
      rows between activation and end-of-2025 while 1h, 1d and 1M all returned
      2024, 2025 and 2026. init.sql defines no TTL, so it was filed as data loss.
  - date: 2026-08-11
    status: completed
    who: okarcz
    note: >
      CLOSED THE SAME DAY - NOT A DEFECT. price_ohlcv_15m has a 30-DAY RETENTION
      BY DESIGN, applied by the cleanup worker's DROP PARTITION rather than by a
      ClickHouse TTL: cleanup-worker/src/lib.rs:31-32 lists
      ("price_ohlcv_1m", "INTERVAL 7 DAY") and ("price_ohlcv_15m",
      "INTERVAL 30 DAY"). The forever-tables are 1h/4h/1d/1w/1M - exactly the set
      that HAS 2024-2025 data. docs/runbooks/repair-coarse-usd-values.md states
      it outright: "price_ohlcv_15m has a 30-day retention (cleanup drops it), so
      it holds no deep history to repair."
      The filing error was grepping init.sql for TTL, finding none, and
      concluding retention was not by design - when retention here is implemented
      by an external job, the same job that has been the central hazard of 0088
      all session.
      The observed state CONFIRMS the design: 15m holds 2026-06-01 onward,
      exactly what a 30-day-retention table looks like when cleanup was disabled
      2026-07-20 and stopped pruning.
      Residual split to 0177 (the six undocumented _bak tables).
---

# `price_ohlcv_15m` has no 2024/2025 rows — **not a defect**

## Resolution

`prices.price_ohlcv_15m` is a **short-retention rollup, not a forever-table.**
The cleanup worker drops its partitions after **30 days**:

```rust
// packages/cleanup-worker/src/lib.rs:31-32
("price_ohlcv_1m",  "INTERVAL 7 DAY"),
("price_ohlcv_15m", "INTERVAL 30 DAY"),
```

The **forever-tables are `1h`, `4h`, `1d`, `1w`, `1M`** — precisely the set that
holds 2024–2025 data. `15m` having none is correct behaviour, not loss.

`docs/runbooks/repair-coarse-usd-values.md` says so directly:

> `price_ohlcv_15m` has a 30-day retention (cleanup drops it), so it holds no
> deep history to repair, and its recent 30 days are enriched live.

**The observed state confirms the design rather than contradicting it.** `15m`
holds `2026-06-01` onward — exactly what a 30-day-retention table looks like when
cleanup was disabled on 2026-07-20 and stopped pruning. Two months of
accumulation, not a gap.

## Why it was filed wrong — worth recording

The reasoning was: *"No TTL exists in `init.sql`, so this is data loss, not
retention by design."*

**Retention on this cluster is not implemented by ClickHouse TTL.** It is a
scheduled external job issuing `ALTER TABLE … DROP PARTITION`. Grepping the
schema for `TTL` can never find it. That job — `prices-production-cleanup` — is
the same one that destroyed pass 1's output on 2026-07-20 and that 0088 spent
three weeks working around; there was every reason to consider it and it was not.

> **Generalisable:** *absence of a declarative retention rule is not evidence of
> absence of retention.* Check the cleanup worker's table list before concluding
> anything about what a `prices.*` table should hold.

## What the investigation did produce

Two findings worth keeping, both real:

1. **Six `_bak` tables exist and are undocumented** — `price_ohlcv_{15m,1h,4h,1d,1w,1M}_bak`,
   259 MiB, all created **2026-07-17 15:28–15:35**, i.e. one deliberate
   seven-minute operation the day before the [[0095]] refreshable-MV
   `ATOMIC REPLACE`. A pre-0095 safety snapshot. Nobody in the 2026-08-11 session
   knew they existed. **Split to [[0177]].**

   ⚠️ Note `price_ohlcv_15m_bak` retains 120M rows of 2024–2025 15-minute data
   that the live table is *designed* to drop. That is not a recovery source for a
   defect that does not exist — but it is the only deep-history 15m data
   anywhere, so 0177 should decide deliberately whether that is worth keeping
   rather than deleting it as stale.

2. **Enrichment coverage of the coarse tables is poor and was measured**
   (2024-03 → 2026-01):

   | table | 2024 zero `close_usd` | 2025 zero `close_usd` |
   |---|---|---|
   | `live_1h` | 65.80% | 71.96% |
   | `live_1d` | 68.09% | 74.67% |
   | `bak_15m` (July snapshot) | 93.83% | 99.94% |

   The live tables improved on the July snapshot, which is [[0114]]'s
   `coarse-repair` doing its job. But **two thirds of coarse rows still carry
   `close_usd = 0`** in the Soroban era, and that is the
   [[close-usd-zero-as-missing-defect-class]] surface BE multiplies into TVL.
   Not this task's problem; already owned by 0114 / 0148, and re-measured here
   in case the figures are useful.

## Notes

- ⚠️ **Consequence for [[0088]]:** the pre-roll wrote **159.22M rows / 6.97 GiB**
  of pre-Soroban data into `15m`, and **all of it will be dropped when cleanup is
  re-enabled.** That is correct — `15m` is scaffolding for building `1h`, and the
  durable record is `1h`/`4h`/`1d`/`1w`/`1M`. Recorded so a future session does
  not rediscover the absence and file this same task again.
- Re-enabling cleanup therefore reclaims `1m` (17.37 GiB) **plus** `15m`
  (6.97 GiB) = **~24.3 GiB**, not the 17.37 GiB first stated.
