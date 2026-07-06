---
id: "0086"
title: "oracle-worker writes Reflector prices with epoch/1000 timestamps (seconds-vs-ms) → junk 1970-01 rows"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0083"]
tags: [layer-worker, priority-medium, effort-small, oracle, reflector, clickhouse, data-quality, post-deploy]
links:
  - "../../../packages/oracle-worker/src"
history:
  - date: 2026-07-06
    status: backlog
    who: okarcz
    note: >
      Spawned from 0083. Surfaced while proving the cleanup worker's RBAC: cleanup
      dropped prices.oracle_prices partition 197001, but it reappeared within minutes
      (modification_time 18:32:28, post-drop) because the oracle-watcher keeps
      re-inserting rows with 1970-01 timestamps.
---

# oracle-worker writes Reflector prices with epoch/1000 timestamps

## Summary

The oracle-watcher intermittently writes `prices.oracle_prices` rows whose
`timestamp` is the real epoch **divided by ~1000**, landing them in `1970-01`
(partition `197001`). The `price_usd` values are correct — only the timestamp is
wrong. This is a seconds-vs-milliseconds unit bug on the Reflector path.

## Evidence (from prod, 2026-07-06)

The 6 junk rows in `oracle_prices` partition `197001`:

```
timestamp             asset_id  oracle_name  price_usd          raw_data
1970-01-21 15:22:41   3         reflector    1.00014287729141   {"symbol":"USDC"}
1970-01-21 15:22:41   4         reflector    0.19963717762499   {"symbol":"XLM"}
1970-01-21 15:22:41   111       reflector    0.99949868096361   {"symbol":"USDT"}
1970-01-21 15:22:42   3         reflector    1.00051271277553   {"symbol":"USDC"}
1970-01-21 15:22:42   4         reflector    0.19941785827334   {"symbol":"XLM"}
1970-01-21 15:22:42   111       reflector    0.99948602782064   {"symbol":"USDT"}
```

- `1970-01-21 15:22:41` = **1,783,361 s** since epoch. `× 1000` ≈ `1,783,361,000 s`
  ≈ **2026-07-06** (now) → the stored value is `real_epoch_seconds / 1000`.
- All rows are `oracle_name = reflector`, peg/pivot assets USDC(3)/XLM(4)/USDT(111).
- The two timestamps (`:41`, `:42`) are two oracle-watcher runs (1 s apart in the
  scaled domain = ~1000 s apart in real time), each writing the same 3 assets.
- **Conditional, not constant**: the bulk of Reflector rows land correctly in
  `202607`, so only some code path divides by 1000. Root cause to pin down —
  likely a fallback timestamp field, or a Reflector `timestamp` returned in a
  different unit than the primary (ledger-close) path assumes.

## Impact

- Low analytically: `1970 < now`, so these rows never surface as the "latest"
  oracle price in `current_prices`/MVs.
- Operationally annoying: they re-materialize `oracle_prices` partition `197001`
  on every affected run, so the cleanup worker (0083) can never keep it clean —
  it drops the partition and the next oracle run recreates it.
- Data hygiene: wrong-timestamped rows pollute `oracle_prices` history.

## Investigation / Fix

- Check the Reflector timestamp handling in `oracle-worker` (SEP-40 price oracle).
  Confirm the unit Reflector actually returns (check developers.stellar.org /
  Reflector docs first — see [[feedback-stellar-docs-first]]) vs what the worker
  assumes. Identify the conditional path that divides by 1000.
- Fix the conversion so all Reflector timestamps are stored in seconds.
- Consider a defensive guard: reject/log oracle rows with `timestamp` implausibly
  far in the past (e.g. `< 2020` or `< contract activation`) so a future unit slip
  can't silently pollute the table.
- One-off cleanup of the existing `197001` rows after the fix ships (a manual
  `ALTER TABLE prices.oracle_prices DROP PARTITION 197001` once the worker stops
  recreating them).

## Acceptance Criteria

- [ ] Root cause identified (which Reflector field/path yields epoch/1000).
- [ ] Fix: Reflector prices always written with correct seconds timestamps.
- [ ] Defensive lower-bound guard on oracle `timestamp` (reject/log implausible).
- [ ] Existing `oracle_prices` partition `197001` cleaned up (stays empty after fix).
- [ ] Verified on a live prod oracle-watcher run (no new `197001` rows).
