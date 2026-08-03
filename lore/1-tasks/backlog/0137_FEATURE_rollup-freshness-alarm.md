---
id: "0137"
title: "Rollup freshness alarm — a starved rollup MV reports success and nothing notices"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0136", "0104", "0109", "0056"]
tags:
  [
    "priority-high",
    "effort-small",
    "clickhouse",
    "observability",
    "milestone-M2",
  ]
milestone: 2
links:
  - "../../../docs/runbooks/0136-coarse-rollup-merge-recovery.md"
history:
  - date: 2026-07-30
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0136]]. Every coarse OHLCV table was frozen for nine days
      and nothing alarmed, because a rollup MV that reads stale input still
      reports `status = Scheduled` with an empty exception. Health was measured
      on the wrong thing — the MV, not the data.
---

# A starved rollup reports success — measure freshness, not exit status

## Summary

[[0136]] froze `price_ohlcv_15m` through `_1M` for **nine days** with no alert.
Eight of the nine refreshable MVs reported `status = Scheduled`, empty
`exception`, every single cycle. Only `mv_ohlcv_1m_to_15m` carried the error;
everything downstream of it rolled up stale input and called that success.

Rolling up nothing is not a failure, so no failure was reported. The health
signal has to be **data freshness**, not MV exit status.

## Context

The gap was found by accident — [[0072]]'s rollout verification noticed
`change_7d_pct` was 0 for every asset, which traced back to `price_ohlcv_1h`
having no rows in the trailing 7 days. Without that coincidence it could have
run indefinitely.

Existing alarms ([[0056]]) cover Lambda/API failure modes. Nothing watches
whether the data at rest is advancing.

## Implementation

- **Signal.** Per coarse table, `now() - max(timestamp)` against an expected
  bound derived from its cadence. A rough starting shape:

  | table | expected lag bound |
  |---|---|
  | `price_ohlcv_1m` | 15 min |
  | `price_ohlcv_15m` | 1 h |
  | `price_ohlcv_1h` | 3 h |
  | `price_ohlcv_4h` | 12 h |
  | `price_ohlcv_1d` | 48 h |
  | `price_ohlcv_1w` | 10 d |
  | `price_ohlcv_1M` | 45 d |

  Tune against real cadence — [[0104]] owns the cadence-vs-window question and
  the bounds should not contradict it.

- **Also alarm on the leading indicators**, which would each have fired days
  before the freeze became visible:
  - any row in `system.mutations` with `is_done = 0` older than ~1 h
    (these sat for 13 days) — overlaps [[0109]]'s guard, which already has to
    watch this table;
  - any `prices` table above ~1,000 active parts (`parts_to_delay_insert`), well
    before the 5,000 throw limit;
  - a non-empty `exception` on any row of `system.view_refreshes`.

- **Where it runs.** Prefer folding into an existing scheduled worker over a new
  Lambda — the enrichment worker already runs hourly and already talks to CH.
  Route to the existing Slack channel used by [[0056]].

- **Access.** Reads `system.parts` (already granted to `prices_writer`),
  `system.mutations` and `system.view_refreshes`. Confirm the latter two are
  readable by the scoped user before designing around them — the runtime users
  are XML-managed by BE and cannot be SQL-`GRANT`ed by us (see [[0134]]). If
  they are not readable, the freshness check on `max(timestamp)` alone is
  sufficient for the primary signal and needs no system-table access at all.

## Acceptance Criteria

- [ ] A freshness check runs on a schedule and alerts when any coarse table
      exceeds its lag bound.
- [ ] Replaying the [[0136]] conditions (a stalled `1m → 15m` rollup) fires the
      alarm within a day.
- [ ] Pending-mutation age and part-count checks are covered, here or in
      [[0109]], without duplicating each other.
- [ ] Alarm routes somewhere a human reads, and a fire-test has passed.
- [ ] Lag bounds are recorded with their rationale, and do not contradict
      [[0104]].

## Notes

- Keep it boring. The failure this catches is "a number stopped moving"; it does
  not need to be clever, it needs to exist.
- The alarm has standalone value regardless of [[0136]]'s outcome — the same
  blind spot covers any future rollup stall, cadence regression, or upstream
  ingestion halt.
