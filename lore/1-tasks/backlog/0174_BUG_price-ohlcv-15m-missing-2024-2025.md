---
id: "0174"
title: "price_ohlcv_15m holds no 2024 or 2025 rows while 1h/1d/1M hold all three years — no TTL exists, so it is data loss"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0088", "0095", "0136"]
tags: [layer-data, priority-high, effort-medium, clickhouse, coarse-rollups, data-loss]
links: []
history:
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      Spawned from 0088's pre-roll pre-flight. Found while auditing the coarse
      tables before writing to them - price_ohlcv_15m returned ZERO rows for the
      whole 2024-02-20 -> 2025-12-31 window, while 1h, 1d and 1M all returned
      2024, 2025 and 2026. init.sql defines no TTL on any of the six coarse
      tables (they are plain `AS price_ohlcv_1m` copies, init.sql:129-134), so
      this is not retention by design. Did NOT block the pre-roll: the gap sits
      entirely ABOVE the 2024-02-20 17:01 boundary, so the pre-roll neither read
      nor wrote it.
---

# `price_ohlcv_15m` is missing 2024 and 2025 entirely

## Summary

`prices.price_ohlcv_15m` holds **no rows at all** between activation
(2024-02-20) and the end of 2025, while `price_ohlcv_1h`, `_1d` and `_1M` each
hold 2024, 2025 **and** 2026 over the same span.

There is **no TTL** on any of the six coarse tables — they are created as plain
`CREATE TABLE … AS prices.price_ohlcv_1m` copies (`init.sql:129-134`) and a grep
for `TTL` across `init.sql` returns nothing. So roughly two years of 15-minute
candles were **lost**, not expired.

## Evidence (prod, 2026-08-11)

Rows at or after the activation boundary, before 2026 (2026 excluded because
live ingestion is actively writing there and would mask the shape):

| table | 2024 | 2025 |
|---|---|---|
| `price_ohlcv_15m` | **0** | **0** |
| `price_ohlcv_1h` | 45,239,505 | 45,460,284 |
| `price_ohlcv_4h` | 22,238,219 | 22,722,030 |
| `price_ohlcv_1d` | 7,109,729 | 7,922,289 |
| `price_ohlcv_1w` | 1,769,907 | 2,154,209 |
| `price_ohlcv_1M` | 472,190 | 1,054,716 |

`15m` is the **only** table with nothing there. It does hold 2026 (10,498,849
rows as of 2026-08-11), and it now holds 2015–2024 as well, because 0088's
pre-roll wrote that range.

A second probe agrees from a different angle: `min(timestamp)` of any row in
`15m` carrying a ledger at or after activation (`intDiv(version,1000) >=
50457424`) is **`2026-06-01`**, not 2024-02-20.

## Why this is suspicious rather than merely odd

Two prior incidents hit these exact tables:

- **[[0095]]** — a refreshable MV writing to a `TO` table did an **ATOMIC
  REPLACE** and wiped coarse data. Resolved by recreating the MVs in APPEND
  mode, but the historical loss window is the shape to check.
- **[[0136]]** — all six coarse tables sat frozen for 17 days, merges and
  mutations inert, recovered by `DETACH`+`ATTACH`.

The surviving 2026 rows have the shape of a materialized view refilling its
**recent re-aggregation window** rather than a full rebuild — which would
explain why 2026 is present and 2024–2025 is not. **That is a hypothesis, not a
finding.** It has not been tested.

## Blast radius is not yet known

⚠️ **`4h` and `1w` were never checked against the same predicate** that exposed
this. The table above shows they have *rows* in 2024/2025, but not whether those
rows are complete. Scope the audit to all six tables before concluding `15m` is
the only casualty.

Consumer impact: any query for 15-minute candles in 2024–2025 returns nothing.
Hourly and coarser are unaffected, so this is invisible to consumers that only
read `1h`+ — including, as far as is known, BE's LP analytics.

## Implementation

- Establish the **extent** first: for each of the six tables, rows per month
  from activation to 2026, not just per year. A per-year count hides a partial
  month.
- Determine the **cause** before repairing — `system.part_log` around the
  suspected window, and whether the loss boundary lines up with 0095's MV
  recreation or 0136's freeze. Repairing without knowing the cause risks
  re-losing it.
- **Repair path is constrained**: `15m` is normally derived from
  `price_ohlcv_1m`, and the Soroban-era `1m` for 2024-03 → 2026-06 was
  **partition-dropped after 0090's pre-roll** (confirmed: `1m` holds `202402`,
  then nothing until `202607`). So `15m` **cannot** be rebuilt from `1m` for the
  missing span. The only in-cluster source is a **down-conversion from `1h`**,
  which cannot recover intra-hour detail — a 15m bucket derived from an hourly
  one is a fabrication, not a recovery.
- That constraint means the realistic options are: (a) accept the gap and
  document it, (b) re-derive from ledger archives via a bounded backfill, or
  (c) down-convert and flag the rows as reconstructed. **This needs a decision,
  not a default** — and (c) risks the same "one value means several things"
  defect as [[0151]] / the `close_usd = 0` class.

## Acceptance Criteria

- [ ] Per-month row counts for **all six** coarse tables, activation → now,
      recorded — so the extent is measured rather than assumed from `15m` alone.
- [ ] `4h` and `1w` explicitly cleared or implicated; the audit does not stop at
      the table that happened to be probed first.
- [ ] Cause identified, or explicitly recorded as undetermined with the evidence
      that was checked.
- [ ] A decision on repair vs accept, with the reasoning written down —
      including the constraint that `1m` no longer exists for the missing span.
- [ ] If anything is reconstructed rather than recovered, it is
      **distinguishable** from measured data (the [[0165]] `method`-column
      lesson: a reconstructed value and a measured one must never be
      confusable).
- [ ] No TTL is introduced as a "fix" — the absence of a TTL is correct here;
      these are forever-tables.

## Notes

- Found during 0088's pre-roll pre-flight, while auditing the coarse tables
  before writing to them. It did **not** block that work: the gap sits entirely
  above the `2024-02-20 17:01:00` boundary, so the pre-roll neither read nor
  wrote it, and its "nothing deleted" verification was unaffected.
- ⚠️ It **did** cost a false step: `15m` was used to try to derive the
  activation boundary and returned `2026-06-01`, its own horizon rather than
  activation. Any query that probes a table's edge to find a *semantic* boundary
  will return that table's retention state instead. See 0088 §Issues Encountered.
- Because `15m` was empty below activation before the pre-roll, 0088's guard
  slice for that table was vacuous — an empty diff there proved nothing. Not a
  gap in that verification's conclusion (the other five tables carried it), but
  worth knowing if those numbers are ever re-read.
