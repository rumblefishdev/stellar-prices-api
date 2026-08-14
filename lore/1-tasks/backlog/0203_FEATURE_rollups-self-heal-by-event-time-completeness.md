---
id: "0203"
title: "Rollups should self-heal by comparing event-time completeness against the source, instead of trusting a 2-hour clock window"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0202", "0142", "0137", "0095", "0200", "0111", "0064"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "rollups", "data-correctness", "resilience", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../packages/prices-clickhouse/schema/preroll-live-gap.sql"
history:
  - date: 2026-08-14
    status: backlog
    who: okarcz
    note: >
      Spawned from 0202. The 2026-08-13 disk-full stall holed every coarse tier
      and could not self-heal, because the 1m->15m MV looks back a fixed 2 hours
      from now(). Operator proposed resuming from where the output stops rather
      than from the clock; refined to comparing event-time completeness against
      the source, which is the only form immune to out-of-order arrival.
---

# Rollups self-heal by event-time completeness, not by a clock window

## Summary

The six coarse rollups rebuild a window measured **from `now()`**. Any ingest
stall longer than that window permanently holes every tier and **nothing ever
goes back**. Replace the clock window with a rule that keeps the coarse candles
**in agreement with the source tier**, whatever time the source data arrives.

## Context — why the current design cannot recover

Measured during [[0202]] (2026-08-13 disk-full stall, 11.5 h):

| MV | refresh | lookback | reads from |
|---|---|---|---|
| `1m → 15m` | 1 min | **2 HOURS** ← binding constraint | `_1m FINAL` |
| `15m → 1h` | 15 min | 8 hours | `_15m FINAL` |
| `1h → 4h` | 1 hour | 1 day | `_1h FINAL` |
| `4h → 1d` | 4 hours | 7 days | `_4h FINAL` |
| `1d → 1w` | 1 day | 60 days | `_1d FINAL` |
| `1w → 1M` | 1 day | 400 days | `_1w FINAL` |

The bound is `now() - INTERVAL <n>`, so it **slides forward with the clock**.
`price_ohlcv_1m` self-heals — the ledger-processor's durable cursor ([[0064]])
goes back and writes what it missed — but by the time it does, that data is
older than the window and the rollup job never sees it. It is not a queue
working through a backlog; it is a window that has already moved on.

⚠️ **Each tier reads the tier BELOW, never `_1m`**, so the 2 h bottleneck at the
first hop propagates through the entire chain regardless of how generous the
upper windows are.

## The design

### ⛔ Restart from EVENT time, not arrival time

The single most important distinction, and the one that is easy to get
backwards. Every candle carries two times:

- **event time** — when the trade happened; the bucket the row is filed under
- **arrival time** — when we managed to write the row

On 2026-08-13 they came apart by 11.5 hours: at **07:56** the processor wrote
rows **labelled 21:00, 22:00, 23:00…**. The buckets needing rebuild are
**21:00 → 07:00**. Restarting from 07:56 rebuilds one hour and leaves the whole
gap in place.

### ❌ Why "resume from where my output stops" is not enough on its own

The natural first proposal — `WHERE t.timestamp >= (SELECT max(timestamp) FROM
<target>)` — is strictly better than the clock window and **would have prevented
this specific incident** (the tip froze at 20:00 during the stall, so the next
refresh would have rebuilt forward from there).

But it assumes data arrives **in order**, and our recovery path deliberately
writes **out of order**. It fails silently like this:

1. Disk problems start, but intermittently — not every write fails.
2. At 21:30 one write succeeds, so the 21:00 bucket holds *some* data.
3. The job builds 21:00 from that partial data; its tip advances to 21:00.
4. Recovery: the processor back-fills the 21:00-21:29 ledgers that failed.
5. The job asks "where did I stop?" → **21:00 or later**. It never goes back.

The 21:00 bucket is then built from a fraction of the trades that exist, and the
job believes it is finished. That is [[0202]]'s partial-bucket failure — an hour
that reads as real but quiet — arriving through a new door. Not hypothetical:
91 doorbells failed while others succeeded that night, so interleaved
success/failure is the normal shape of these outages.

### ✅ The rule: rebuild any bucket that disagrees with its source

Per bucket, by event time, ignoring arrival entirely:

| bucket | `_1m` holds | `_1h` says | verdict |
|---|---|---|---|
| 20:00 | 1,200 trades | 1,200 | agrees — skip |
| **21:00** | **1,450 trades** | **nothing** | **disagrees — rebuild** |
| **22:00** | **1,380 trades** | **nothing** | **disagrees — rebuild** |
| 05:00 | 1,100 trades | 1,100 | agrees — skip |

When the back-fill lands, the source for 21:00 changes, the comparison for 21:00
fails, 21:00 is rebuilt. **Arrival order stops mattering** — which is the whole
point, and what the tip-based rule cannot give.

### Proposed shape — two passes, not one

Cost is the reason not to make the comparison the only mechanism.

1. **Fast path** — keep the existing per-minute refresh for latency. Optionally
   move its bound from the clock to the output tip (the operator's proposal):
   free, and it alone would have caught this incident.
2. **Completeness sweep** — slower cadence (~30 min), wider range (~1 day),
   rebuilding only buckets that disagree. This is the correctness backstop.

✅ **The safety property already exists.** `rollups.sql` runs `APPEND` with
`sum(version)`: re-rolling a correct bucket is a no-op that RMT collapses, and a
complete bucket outranks any partial one. **That is what makes automatic
rebuilding safe**, and it is why this is a change of *selection rule*, not of
correctness machinery.

## Constraints — read before starting

1. ⛔ **Blocked by [[0142]].** `rollups.sql` uses `CREATE … IF NOT EXISTS` with
   no `DROP`, and refreshable MVs do not accept `OR REPLACE` — so **edits to
   that file silently no-op**. Any change here needs an explicit `DROP`+`CREATE`
   path on prod, which is exactly what 0142 exists to solve.
2. ⏳ **`_1m` retention is the hard ceiling.** Nothing can heal from data that
   has been dropped. At 7-day retention no sweep can ever reach further back,
   and that is only true while cleanup stays off — **[[0200]] therefore sets the
   maximum outage this design can survive.** Couple the decisions.
3. ⚠️ **[[0137]]'s alarm would not have caught this and will not verify the
   fix.** It measures the *tip* — how old the newest row is. On 2026-08-13 the
   tip was current while eight buckets behind it were missing. **A hole with a
   healthy tip is invisible to a staleness check.** Any auto-heal needs a
   completeness signal or nobody will know whether it worked.
4. **A long gap is one enormous catch-up query.** [[0111]]'s four-day outage
   would be a very heavy single pass — advance in chunks rather than all at once.
5. **A job reading the table it writes to is unusual** — verify against local CH
   pinned to the prod version (26.3.10.60) before prod.

## Acceptance Criteria

- [ ] A stall longer than the current window self-heals with no operator action,
      demonstrated by test: stop writes, resume with back-dated rows, assert
      every tier converges to the `_1m FINAL` totals
- [ ] The rebuild is selected by **event time**, proven by a test where arrival
      order is reversed relative to event order
- [ ] A partially-built bucket that later receives more source data is rebuilt
      — the [[0202]] failure mode, pinned as a regression test
- [ ] Completeness signal exists and is alarmable — a hole behind a healthy tip
      is detected (0137 cannot do this today)
- [ ] Verified non-vacuous: restore each defect, confirm the matching test fails
- [ ] Catch-up over a multi-day gap is chunked and bounded in memory
- [ ] 0142's DROP/CREATE path used, and the change verified to have actually
      taken effect on prod rather than silently no-opping
