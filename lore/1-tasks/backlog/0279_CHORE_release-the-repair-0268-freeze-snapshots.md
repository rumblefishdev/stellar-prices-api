---
id: "0279"
title: "Release the 315 repair_0268_ FREEZE snapshots on or after 2026-09-18"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0276", "0268"]
tags: [layer-backend, priority-medium, effort-small, clickhouse, operations, disk]
links:
  - "../../../docs/runbooks/repair-coarse-usd-values.md"
history:
  - date: "2026-09-11"
    status: backlog
    who: akot
    note: >
      Spawned from [[0276]]. The 0268 campaign's rollback point is kept seven
      days after the 2026-09-11 run (Adam's decision) so a complaint about the
      new prices or the method vocabulary can still be answered with a
      restore; after that the snapshots only cost disk on a shared cluster.
---

# Release the repair_0268_ FREEZE snapshots

## Summary

Before the 0268 re-enrichment campaign (2026-09-11, [[0276]]) every partition it
could write was frozen: **315 snapshots**, `repair_0268_prices_<table>_<YYYYMM>`
for `price_ohlcv_1h`, `_4h`, `_1d`, `_1w`, `_1M` × 202101–202603, 807 parts.
They are hardlinks under `/var/lib/clickhouse/shadow/`, free at first, but every
background merge of the re-written partitions leaves the old part alive in the
snapshot — up to ~16.6 GiB (1h 9.7, 4h 4.6, 1d 1.7, 1w + 1M 0.6) on the shared
`ch-prod-01`. Release them on or after **2026-09-18**.

## Context

- The snapshots are only a last-resort rollback: the campaign changed
  `close_usd` / `volume_quote_usd` alone, and the pre-campaign values are
  `close × 1` / `volume_quote × 1`, so a revert is also possible as a versioned
  INSERT without them.
- A restore from them needs the host (copy out of `shadow/`); the release does
  not — `ALTER TABLE … UNFREEZE` is SQL.

## Implementation

1. **Before releasing**, confirm nobody needs a rollback: no open issue about
   USDC-quoted prices or the `external` / `assumed-par` labels since 2026-09-11.
2. Release all 315 over mTLS as a user holding `ALTER` on `prices` (on
   2026-09-11 that was `dev_shared`, Adam's `~/.certs/adamkot-write`). Use the
   per-table `ALTER TABLE … UNFREEZE PARTITION … WITH NAME …`, not the
   runbook's `SYSTEM UNFREEZE`: the latter only works when the server config
   sets `enable_system_unfreeze`, which is not visible in `system.server_settings`
   and could not be confirmed on 2026-09-11.

   ```bash
   q()  { curl -sS --max-time 120 --cert ~/.certs/adamkot-read.crt  --key ~/.certs/adamkot-read.key  --cacert ~/prices-mtls/ca.crt https://ch.sorobanscan.rumblefish.dev/ --data-binary "$1"; }
   qw() { curl -sS --max-time 120 -w ' %{http_code}\n' --cert ~/.certs/adamkot-write.crt --key ~/.certs/adamkot-write.key --cacert ~/prices-mtls/ca.crt https://ch.sorobanscan.rumblefish.dev/ --data-binary "$1"; }
   for T in price_ohlcv_1h price_ohlcv_4h price_ohlcv_1d price_ohlcv_1w price_ohlcv_1M; do
     for M in $(q "SELECT DISTINCT toString(partition) FROM system.parts WHERE database='prices' AND table='$T' AND toUInt32(partition) BETWEEN 202101 AND 202603 ORDER BY 1 FORMAT TSV"); do
       echo -n "$T $M: "; qw "ALTER TABLE prices.$T UNFREEZE PARTITION $M WITH NAME 'repair_0268_prices_${T}_${M}'"
     done
   done
   ```

   Each statement removes that partition's snapshot from `shadow/`. Run the
   loop once with `echo` in place of `qw` first and check it prints 315 lines.
3. Record the free space before and after
   (`SELECT name, formatReadableSize(free_space) FROM system.disks`) — it was
   379.57 GiB free on 2026-09-11 before the FREEZE.

## Acceptance Criteria

- [ ] No rollback request since 2026-09-11 (or, if there was one, it is handled first)
- [ ] 315 `ALTER TABLE … UNFREEZE` statements returned 200
- [ ] Free space before / after recorded here
