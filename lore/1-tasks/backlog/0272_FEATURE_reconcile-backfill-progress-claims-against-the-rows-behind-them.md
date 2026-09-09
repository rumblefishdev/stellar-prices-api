---
id: "0272"
title: "Nothing compares backfill_progress's claims against the rows behind them — reconcile the stored watermarks on a schedule, not at the end of a run"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0264", "0263", "0176", "0243", "0127", "0200"]
tags: [layer-backend, priority-medium, effort-small, milestone-M2, observability, backfill, data-correctness]
milestone: 2
links:
  - "../../../packages/backfill-freshness-probe/src/lib.rs"
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-09-08
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0264]] acceptance criterion 4, which is **deferred here
      rather than met**. Scoped deliberately as a periodic sweep and NOT as the
      end-of-run check first proposed — see Design below for why that version is
      vacuous.
---

# `backfill_progress` holds claims nobody compares to the data

## Summary

`prices.backfill_progress` stores assertions about coverage — how far back the
data goes, how far a stream has reached — and `GET /v1/backfill/status` publishes
them to consumers. Until [[0127]] nobody had ever compared those assertions to
the rows they describe. Two of the columns turned out to overstate.

Both causes are now fixed at the writer ([[0263]], [[0264]]). Nothing detects the
**general** case: a stored claim that outruns the data behind it.

## Context

The class, from the two known instances:

| column                    | claimed                     | reality                     | fixed by |
| ------------------------- | --------------------------- | --------------------------- | -------- |
| `earliest_data_available` | AMM data from 2024-02-20    | first AMM candle 2024-03-08 | [[0264]] |
| `current_ledger`          | a genesis floor             | a chunk that stopped short  | [[0263]] |

The AMM overclaim stood for roughly eighteen months and was invisible: the
endpoint kept returning a plausible number, nothing errored, and no alarm covers
it. It surfaced only because a human went looking.

## Design — why a periodic sweep, and NOT an end-of-run check

⚠️ **An end-of-run check was proposed first and is vacuous. Do not build it.**

After [[0264]], the claim a run writes *is* the earliest minute that run landed,
derived from the same `source` string handed to `Sink::write_candles`. Comparing
the run's observation against the run's own claim compares a value to itself; it
can only pass. The one legitimate way they differ is `merge_min` pulling in an
older stored value from a prior run, which is correct behaviour.

A real check compares the **stored** row against `min(timestamp)` in the candle
tables. That is a database query either way, so end-of-run and periodic are the
same query differing only in trigger — and the triggers are not equally useful:

| side of the comparison | moves when                                                     |
| ---------------------- | -------------------------------------------------------------- |
| the claim              | a backfill runs — now rare, and correct by construction         |
| the reality            | candles are deleted — partition drops, retention, a repair job  |

Deletion under a frozen claim is the live drift path, and **only a timer catches
it**. `sdex-backfill`'s `sink.rs` is the sole production writer of this table, so
with the archive complete and the AMM stream resting, an end-of-run trigger fires
approximately never — and fires precisely when the writer is most trustworthy.

The deletion path is not hypothetical: `price_ohlcv_1m` carries a nominal 7-day
retention in `cleanup-worker`'s `RETENTION`, and [[0200]] asks whether that worker
should be re-enabled.

## Implementation

- One query per stream: stored `earliest_data_available` vs `min(timestamp)` for
  that stream's sources. **Query `price_ohlcv_1d`, not `_1m`** — see
  [[amm-history-is-not-in-price-ohlcv-1m]]; AMM history is not in `_1m` at all,
  and a `_1m` query returns a false pass.
- Overclaim in seconds (`reality − claim`, positive when the claim is too early)
  published per stream; alarm above zero.
- Home: extend `backfill-freshness-probe`. It already runs on a schedule, already
  reads this table, and already publishes a `Prices/Backfill` metric with an
  alarm wired to Slack — so this is a query and an alarm, not a new service.
  Decide against standing alone with [[0243]], and record the reason.
- **Once a week, not every 15 minutes.** The value changes only when a backfill
  lands older data or candles are removed. `earliest_data_available` is stored
  precisely because computing it live is a full scan (`timestamp` is not the sort
  key) — the scan is cheap on `_1d` (~125k AMM rows) and must not be put on the
  probe's normal cadence.
- Extend to `current_ledger` against `backfill_sdex_ledgers` if the same shape
  fits; [[0263]] left the floor unprovable from the column alone.

## Acceptance Criteria

- [ ] A stored watermark that precedes the data behind it is detected on **either**
      stream, without a human going looking.
- [ ] The check runs against the durable tables; a `_1m`-only query is rejected in
      review as a false pass.
- [ ] Cadence is weekly or slower, with the scan cost measured rather than assumed.
- [ ] Home decided between the existing probe and standing alone, with the reason
      recorded.
- [ ] Verified by inducing: point it at a deliberately wrong stored value and
      confirm it fires, per the repo's standing practice of proving an alarm by
      breaking something.
- [ ] [[0264]]'s AC 4 is closed by reference to this task's outcome.

## Notes

- 🔑 **The general lesson this task exists to institutionalise:** `backfill_progress`
  holds *claims*. Two columns were found asserting more than the data supported,
  by different mechanisms, eighteen months apart in origin and one week apart in
  discovery. Fixing both writers removes those two mechanisms; it does not make
  the table self-checking.
- A one-off manual reconciliation should be run when [[0264]]'s data correction
  lands, independently of this task — that confirms the correction took, and does
  not wait on this being built.
