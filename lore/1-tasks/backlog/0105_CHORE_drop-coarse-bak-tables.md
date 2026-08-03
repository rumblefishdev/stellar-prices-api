---
id: "0105"
title: "Drop the six price_ohlcv_*_bak coarse backup tables (~18 GiB) after the 0095 watch period"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0095"]
tags: [layer-infra, priority-low, effort-small, clickhouse, housekeeping, cleanup]
links: []
history:
  - date: 2026-07-17
    status: backlog
    who: okarcz
    note: "Spawned from 0095 — coarse backup taken before the APPEND MV recreate; drop once the live rollup has held for a watch period."
  - date: 2026-07-30
    status: backlog
    who: okarcz
    note: >
      **BLOCKED on [[0136]].** The live rollup did not hold — every coarse table
      froze on 2026-07-21, four days after the 0095 change. These backups are the
      only rollback path for the 0136 recovery, which resumes merges and lets six
      pending ALTER DELETE mutations execute. Do not drop them until that is
      verified, and restart the watch period from the recovery rather than from
      2026-07-17.
---

# Drop the six price_ohlcv_*_bak coarse backup tables

> ## ⛔ BLOCKED — do not run until [[0136]] is verified (added 2026-07-30)
>
> All six coarse tables have been frozen since 2026-07-21 and the recovery
> (`docs/runbooks/0136-coarse-rollup-merge-recovery.md`) resumes merges and lets
> six pending `ALTER … DELETE` mutations execute. **These `_bak` tables are the
> only rollback path for that.** Dropping them now would leave the recovery
> irreversible.
>
> The watch period this task waits for was also never really satisfied: the live
> rollup did *not* hold — it stalled four days after the 0095 change. Restart the
> watch clock from the 0136 recovery, not from 2026-07-17.

## Summary

Before the 0095 APPEND rollup MV recreate (2026-07-17), the six coarse tables
were backed up on `ch-prod-01` as `prices.price_ohlcv_<g>_bak` (~18 GiB
compressed total, on a disk with ~573 GiB free). They are the restore path if
the APPEND MVs had wiped history. They did not — deep history verified
byte-identical to the backup, and the live rollup advances autonomously. Once
the rollup has demonstrably held for a watch period, reclaim the space.

## Context

The backup and its faithfulness proof are in 0095. This is pure cleanup — no
code, no schema change, just `DROP TABLE` on prod.

## Implementation

- Wait for a watch period (≈ a few days to a week) of healthy live rollups —
  coarse tips tracking the live frontier, no `system.view_refreshes` errors, no
  history-loss report.
- Optional final safety check before dropping — deep history still matches (or
  simply still present):

  ```sql
  SELECT count() FROM prices.price_ohlcv_1d FINAL WHERE timestamp < '2025-06-01';
  ```

- Drop the six backups (hand the block to the operator; prod CH):

  ```sql
  DROP TABLE IF EXISTS prices.price_ohlcv_15m_bak;
  DROP TABLE IF EXISTS prices.price_ohlcv_1h_bak;
  DROP TABLE IF EXISTS prices.price_ohlcv_4h_bak;
  DROP TABLE IF EXISTS prices.price_ohlcv_1d_bak;
  DROP TABLE IF EXISTS prices.price_ohlcv_1w_bak;
  DROP TABLE IF EXISTS prices.price_ohlcv_1M_bak;
  ```

- Confirm reclaimed: `SHOW TABLES FROM prices LIKE '%_bak'` returns nothing.

## Acceptance Criteria

- [ ] Live rollup confirmed healthy over the watch period before dropping.
- [ ] All six `price_ohlcv_*_bak` tables dropped on `ch-prod-01`.
- [ ] `SHOW TABLES FROM prices LIKE '%_bak'` returns zero rows; disk space
      reclaimed.
