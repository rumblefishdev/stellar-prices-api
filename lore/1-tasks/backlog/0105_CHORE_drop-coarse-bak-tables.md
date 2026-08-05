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
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      UNBLOCKED. [[0136]]'s watch period closed today - the DETACH/ATTACH
      recovery ran 2026-08-03 and all six tiers are current as of 10:15Z, with
      `_1M` reaching 2026-08-01 at the 00:00 refresh (the last outstanding tip).
      All nine refreshable MVs Scheduled with empty exception, part counts flat
      against the post-recovery baseline, and the six 0097 Phoenix mutations
      completed on attach. The watch clock restarted from the recovery as this
      task required, not from 2026-07-17. Data integrity was already verified
      live-vs-_bak with FINAL over a pre-incident window: zero delta on all four
      sources plus `_1d` deep history. Ready to run whenever it is picked up -
      still just DROP TABLE on prod, hand the block to the operator.
---

# Drop the six price_ohlcv_*_bak coarse backup tables

> ## ✅ UNBLOCKED 2026-08-05 — [[0136]] is verified
>
> *Was blocked 2026-07-30 through 2026-08-05: the six coarse tables had been
> frozen since 07-21 and these `_bak` copies were the only rollback path for the
> recovery, which resumed merges and let six pending `ALTER … DELETE` mutations
> execute.*
>
> The recovery ran on 2026-08-03 (`DETACH`/`ATTACH` per table) and the watch
> period — restarted from the recovery, as this task demanded, **not** from
> 2026-07-17 — closed on 2026-08-05 with every tier current: `_15m` 10:15,
> `_1h` 10:00, `_4h` 08:00, `_1d` 08-05, `_1w` 08-03, `_1M` **08-01**. All nine
> refreshable MVs `Scheduled` with empty `exception`; part counts flat against
> the post-recovery baseline.
>
> Integrity was verified live-vs-`_bak` with `FINAL` over a pre-incident window:
> **zero delta on all four sources**, plus `_1d` deep history identical. The
> backups have nothing left to roll back.

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
