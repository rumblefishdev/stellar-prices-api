---
id: "0177"
title: "Six undocumented price_ohlcv_*_bak tables on prod — decide whether the pre-0095 snapshot is kept or dropped"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0174", "0095", "0114"]
tags: [layer-data, priority-low, effort-small, clickhouse, housekeeping]
links: []
history:
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      Split from 0174, which closed as not-a-defect. Six _bak tables were found
      on ch-prod-01 while measuring the backfill's disk footprint; nobody in the
      session knew they existed. They are a pre-0095 safety snapshot and one of
      them holds data that exists nowhere else.
---

# Six undocumented `price_ohlcv_*_bak` tables

## Summary

`ch-prod-01` carries six tables nobody in the 2026-08-11 session knew about:

| table | rows | on disk |
|---|---|---|
| `price_ohlcv_15m_bak` | 153,060,193 | 137.18 MiB |
| `price_ohlcv_1h_bak` | 82,167,926 | 70.46 MiB |
| `price_ohlcv_4h_bak` | 42,116,687 | 34.23 MiB |
| `price_ohlcv_1d_bak` | 14,351,533 | 10.76 MiB |
| `price_ohlcv_1w_bak` | 3,702,185 | 4.07 MiB |
| `price_ohlcv_1M_bak` | 1,341,814 | 2.35 MiB |

**259 MiB total** — trivial as storage. The question is not cost, it is that
undocumented state on a shared production cluster gets misread. It already was:
0174 was filed as a data-loss bug partly because these tables made the live
`15m` look anomalous.

## Provenance — known

All six were created **2026-07-17 between 15:28 and 15:35** (`system.tables`
`metadata_modification_time`), engine `ReplacingMergeTree`, spanning partitions
`202402 → 202607`. One deliberate seven-minute operation, the day before the
[[0095]] work (the refreshable-MV `ATOMIC REPLACE` that wiped the coarse tables).

**They are a pre-0095 safety snapshot.** That is inferred from timing and
content, not from any note — no runbook, ADR or task mentions them.

## The one that is not merely stale

⚠️ **`price_ohlcv_15m_bak` holds 120,095,237 rows of 2024–2025 15-minute data
that exists nowhere else.** The live `price_ohlcv_15m` has a **30-day retention**
by design (`cleanup-worker/src/lib.rs:32`), so deep 15-minute history is normally
dropped, and `price_ohlcv_1m` for that span was dropped by cleanup on 2026-07-18.

So this snapshot is the **only** surviving 15-minute-granularity record of the
Soroban era. That may be worth keeping deliberately, or may be worth dropping —
but it should not be deleted by someone tidying up `_bak` tables without knowing.

⚠️ Its `close_usd` is largely unenriched — **93.83% zero in 2024, 99.94% in
2025** — against 65.80% / 71.96% in the live `1h`. So it is a snapshot of
*structure* (OHLC, volumes, trade counts), not of USD values. Anyone valuing it
should value it on that basis.

## Implementation

- Confirm the 0095 provenance against `system.query_log` / `part_log` around
  2026-07-17 15:28–15:35, or from the 0095 task record, rather than leaving it
  inferred.
- Decide: keep (and **document** — a note in the schema or a runbook, so the
  next person to find them does not re-file 0174), or drop.
- If keeping `15m_bak` for its unique deep history, say **why** and **for how
  long**, and make sure the cleanup worker does not acquire them by accident if
  its table list is ever generalised.
- If dropping, move deliberately — these are the only copy of some data, and the
  repo's file-deletion policy exists for exactly this class of irreversibility.

## Acceptance Criteria

- [ ] Provenance confirmed rather than inferred.
- [ ] A keep-or-drop decision recorded with reasoning.
- [ ] If kept, they are **documented** somewhere a future session will find
      before treating the live tables as anomalous.
- [ ] The unique 2024–2025 15-minute data in `15m_bak` is explicitly considered
      in that decision, not swept up as generic backup cruft.
- [ ] Cleanup-worker behaviour toward `*_bak` tables confirmed safe either way.

## Notes

- Found while answering "how much disk did the backfill use?" — a
  `system.parts` query with a `table LIKE 'price_ohlcv%'` filter, which is a
  reminder that broad audits surface things targeted checks never will.
- Related: [[0174]] (closed not-a-defect; these tables contributed to the
  misdiagnosis), [[0095]] (the wipe they were taken to guard against),
  [[0114]] (`coarse-repair`, which has since improved the live tables past the
  snapshot's enrichment level).
