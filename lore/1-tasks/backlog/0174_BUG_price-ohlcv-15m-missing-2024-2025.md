---
id: "0174"
title: "price_ohlcv_15m holds no 2024 or 2025 rows while 1h/1d/1M hold all three years — no TTL exists, so it is data loss"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0088", "0095", "0136"]
tags: [layer-data, priority-high, effort-small, clickhouse, coarse-rollups, data-loss, recoverable]
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

> ## 🟢 UPDATE 2026-08-11 — THE DATA EXISTS. This is a RESTORE, not a loss.
>
> A `prices.price_ohlcv_15m_bak` table was found while measuring the backfill's
> disk footprint. It holds **120,095,237 rows in exactly the missing window**
> (`202403 → 202512`), and spans `202402 → 202607` overall — the whole Soroban
> era. Five sibling `_bak` tables exist too (`1h`, `4h`, `1d`, `1w`, `1M`); those
> five were recovered in the live tables and only `15m` was not.
>
> **The §Implementation "repair path is constrained" analysis below is therefore
> SUPERSEDED.** It concluded the only in-cluster source was a down-conversion
> from `1h` that would fabricate intra-hour detail. That is no longer the
> situation — a real 15-minute copy survives.
>
> **Two consistency checks that make the backup credible:**
> - `15m_bak / 1h_bak` across the gap = **120.1M / 63.5M = 1.89×**. The
>   pre-Soroban ratio we just built is `159.22M / 68.59M = 2.32×`. Same order;
>   the difference is what denser Soroban-era trading would produce. A truncated
>   backup would sit well below 1.
> - `1h_bak`'s gap count (63.47M) sits just under the live `1h` deduplicated
>   count for a slightly wider window (64.58M) — backup and live agree wherever
>   both exist.
>
> ### Verification results, measured 2026-08-11
>
> | check | result |
> |---|---|
> | Schema identical to `price_ohlcv_15m` | ✅ no column drift |
> | Continuous across the gap | ✅ **all 22 months** present, 3.7M–7.4M each |
> | No `Decimal128::MIN` artifact | ✅ zero negatives |
> | **Real `close_usd`** | 🔴 **NO — 93.8% zero in 2024, 99.94% in 2025** |
>
> **Provenance is now known.** All six `_bak` tables were created
> **2026-07-17 between 15:28 and 15:35** — one deliberate seven-minute operation,
> the day before the [[0095]] work (the refreshable-MV `ATOMIC REPLACE` that wiped
> coarse). **They are a pre-0095 safety snapshot.**
>
> 🔴 **That reframes this task's cause.** The 0095 recovery restored five coarse
> tables and **missed `15m`**. This is not a mysterious loss — it is an
> incomplete recovery, with its own backup sitting untouched beside it for three
> weeks. It also explains the zeros: a July 17 snapshot predates any enrichment
> that has run since.
>
> ### 🔴 The blocker: restoring would import a wall of zeros
>
> `close_usd` is the column BE multiplies into TVL, and "not yet enriched" and
> "no USD price exists" are **the same value** — zero. See
> [[close-usd-zero-as-missing-defect-class]].
>
> ⚠️ **Re-enrichment is NOT available for this span.** `prices.usd_rate` coverage
> starts **2026-03-11** ([[0167]], corrected), so there is no oracle history to
> recompute 2024–2025 `close_usd` from. Whatever the backup carries is the
> ceiling on what is recoverable.
>
> **The open question that decides the approach** — is the live `1h` table any
> better enriched over the same window?
> - Live `1h` **also ~94% zeros** → the backup is consistent with the rest of the
>   estate; restoring introduces no new inconsistency and `15m` simply rejoins
>   its siblings in a known, pre-existing gap. **Restore.**
> - Live `1h` **is enriched** → restoring the July snapshot creates a *cross-grain
>   contradiction*: `15m` reporting `close_usd = 0` where `1h` reports a real
>   price for the same hour. That is **worse** than the present state, where
>   `15m` is merely absent. **Restore OHLC/volumes but treat `close_usd`
>   separately, or do not restore.**
>
> **Restore shape, once verified — bounded strictly to the gap:**
> ```sql
> INSERT INTO prices.price_ohlcv_15m
> SELECT * FROM prices.price_ohlcv_15m_bak
> WHERE timestamp >= '2024-03-01' AND timestamp < '2026-01-01';
> ```
> The live table holds **nothing** there, so there are no primary-key collisions
> and no RMT tie to resolve.
>
> ⚠️ **Do NOT restore the backup's 2026 portion** (~30.6M rows). The live table
> already holds 10.5M rows there from the rollup MVs; mixing a stale snapshot
> into live data is a distinct and worse problem than the gap.
>
> **Still unanswered, and it should not be skipped just because a fix is
> available:** *why* only `15m` lost its data when five sibling tables kept
> theirs, and what created these `_bak` tables. Restoring without knowing the
> cause invites a repeat. The `_bak` tables' own retention is a second open
> question — 259 MiB, undocumented, and nobody in this session knew they existed.

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

Reordered 2026-08-11 around the restore path.

- [ ] `price_ohlcv_15m_bak` verified fit to restore: **schema identical**
      (including column *order* — `INSERT SELECT` maps by position), **continuous
      across all 22 months** of the gap, and carrying **real `close_usd`** rather
      than a wall of zeros or `Decimal128::MIN`.
- [ ] Gap restored from the backup, bounded to
      `[2024-03-01, 2026-01-01)` — **the 2026 portion deliberately excluded**, as
      the live table already holds MV-written rows there.
- [ ] Post-restore, `15m` shows all months 2024–2025 with counts consistent with
      the `1h` table at roughly the expected 2× ratio.
- [ ] Per-month row counts for **all six** coarse tables, activation → now,
      recorded — so the extent is measured rather than assumed from `15m` alone.
- [ ] `4h` and `1w` explicitly cleared or implicated; the audit does not stop at
      the table that happened to be probed first.
- [ ] **Cause identified** — why only `15m` lost data when five siblings kept
      theirs — or explicitly recorded as undetermined with the evidence checked.
      ⚠️ Do not let the availability of a restore excuse skipping this; an
      unexplained cause invites a repeat.
- [ ] The `_bak` tables' own fate decided: what created them, whether they are
      still needed after the restore, and whether anything should depend on them.
      They are 259 MiB and were undocumented until 2026-08-11.
- [ ] If anything ends up **reconstructed** rather than restored, it is
      **distinguishable** from measured data (the [[0165]] `method`-column
      lesson: a reconstructed value and a measured one must never be
      confusable). A straight restore from `15m_bak` does not raise this — a
      down-conversion from `1h` would.
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
