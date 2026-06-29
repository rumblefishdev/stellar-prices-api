---
id: "0071"
title: "Re-apply corrected rollup/preroll DDL to live ch-prod-01 (argMin/argMax timestamp-shadowing fix)"
type: BUG
status: completed
related_adr: ["0007"]
related_tasks: ["0059", "0051"]
tags: [layer-database, priority-high, effort-small, clickhouse, materialized-views, rollups, operations]
links:
  - "../../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../../packages/prices-clickhouse/schema/preroll.sql"
  - "notes/G-prod-reapply-runbook.md"
history:
  - date: 2026-06-26
    status: backlog
    who: oski
    note: >
      Spawned from 0059. The 0059 full-chain integration test surfaced an
      OHLC-correctness bug in the as-shipped rollups.sql / preroll.sql: the
      `toStartOfInterval(timestamp, …) AS timestamp` bucket alias shadows the
      source `timestamp` column, so argMin(open)/argMax(close)/argMax(close_usd)
      tie-break to an arbitrary row instead of the true first/last by time. The
      schema files are fixed (FROM … AS t + qualified t.timestamp), but the
      BUGGY DDL was already applied LIVE on ch-prod-01 under 0051 (2026-06-22).
      The six refreshable MVs in prices.* must be re-created from the corrected
      rollups.sql. Deferred to a deploy-capable session (prepare-not-deploy).
  - date: 2026-06-29
    status: active
    who: oski
    note: >
      Promoted backlog → active to prepare the prod fix. Operator-executed:
      this session produces the step-by-step runbook (DROP + re-CREATE the six
      `prices.mv_ohlcv_*` MVs from the corrected `rollups.sql`, recompute the
      mis-rolled `_15m … _1M` buckets via corrected `preroll.sql`, post-apply
      spot-check) — the live DDL against ch-prod-01 (168.119.73.161, Route A
      `ssh … docker exec app-clickhouse-1 clickhouse-client`) is run by the
      operator, NOT by this session (prepare-not-deploy).
  - date: 2026-06-29
    status: active
    who: oski
    note: >
      Converted to a directory task; landed the operator runbook
      (`notes/G-prod-reapply-runbook.md`) with an empty-DB fast path. Also
      investigated renaming the shadowing bucket alias `AS timestamp` →
      `ts_bucket` as a permanent hardening — REJECTED: a `TO`-table MV matches
      its SELECT output to the target columns BY NAME, so `ts_bucket` is refused
      with `Code: 8 THERE_IS_NO_COLUMN` (reproduced live on CH 26.3.10.60 via
      `prices-clickhouse/tests/rollup_chain_it.rs`). The bucket key is therefore
      structurally forced to `AS timestamp`; source-qualifying `t.timestamp` is
      the only available fix, exactly as 0059 shipped. Banked the finding as a
      comment in `rollups.sql`/`preroll.sql` (comment-only diff, both rollup
      integration tests green) so the dead-end isn't re-attempted. No schema
      behaviour change; no prod deploy.
  - date: 2026-06-29
    status: completed
    who: oski
    note: >
      Fix applied to ch-prod-01 (CH 26.3.10.60, Route A) and verified — see
      `notes/G-prod-reapply-runbook.md` "Applied — 2026-06-29". EMPTY-DB path
      (`price_ohlcv_1m` = 0), so no mis-rolled buckets existed and the recompute
      step was N/A. DROP of all six `prices.mv_ohlcv_*` + re-create from the
      corrected `rollups.sql`; `SHOW CREATE mv_ohlcv_1m_to_15m` confirms the
      live definition reads `FROM price_ohlcv_1m AS t FINAL` with
      `argMin/argMax(…, t.timestamp)` (qualified, no bare `timestamp`). MV
      refresh healthy (`system.view_refreshes` status Scheduled, no exception);
      box + BE `default.*` untouched, no restart, no data residue. Two runbook
      footguns found and fixed in-note during the live run: (1) re-streaming
      `rollups.sql` without the DROP is a no-op (`IF NOT EXISTS`); (2) the verify
      query needs `AS r FINAL` (alias before FINAL) and the live smoke test must
      use in-window timestamps (`now() - INTERVAL 2 HOUR` filter). Completing and
      archiving.
---

# Re-apply corrected rollup/preroll DDL to live ch-prod-01

## Summary

The live `prices.*` refreshable-MV rollup chain on `ch-prod-01` was created
from a version of `schema/rollups.sql` that mis-computes `open` / `close` /
`close_usd` (the argMin/argMax-by-time aggregates). Task 0059 fixed the schema
files; this task re-applies the fix to the running cluster.

## Context

The bug (task 0059, full-chain integration test `rollup_chain_it.rs`): the
bucket key `toStartOfInterval(timestamp, …) AS timestamp` **shadows** the source
`timestamp` column, so `argMin(open, timestamp)` / `argMax(close, timestamp)` /
`argMax(close_usd, timestamp)` evaluate against the constant bucket-start and
tie-break to an arbitrary row. Volumes (`sum`), `high` (`max`), `low` (`min`)
are unaffected — only O/C/close_usd are wrong. Fixed by `FROM … AS t` +
qualified `t.timestamp`. `current.sql` was already correct (it uses `AS c` +
`c.timestamp`); only `rollups.sql` and `preroll.sql` were affected.

Live apply provenance: task 0051 `notes/G-live-schema-state.md` (Route A,
`ssh … docker exec app-clickhouse-1 clickhouse-client --multiquery`, CH 26.3.10,
loopback `default` admin).

## Implementation

- `DROP VIEW` the six `prices.mv_ohlcv_*` MVs on `ch-prod-01`, then re-create
  them from the corrected `schema/rollups.sql` (loopback `default` admin, same
  Route A path as 0051). Dropping an MV does **not** touch its target table, so
  no rollup rows are lost.
- Recompute any already-mis-rolled buckets: TRUNCATE + re-run the corrected
  `preroll.sql` over the backfilled range, or let the bounded-window MVs
  overwrite the recent window on their next refresh (replace mode). Decide based
  on how much coarse data has accrued since 0051's live apply.
- Verify post-apply: spot-check that `open`/`close` at `_15m`+ match the true
  first/last `_1m` close in a bucket (the assertion the integration test makes).

## Acceptance Criteria

- [x] Six `prices.mv_ohlcv_*` MVs on ch-prod-01 re-created from corrected DDL
- [x] Mis-rolled historical buckets recomputed (preroll or window refresh)
      — N/A: empty DB (`price_ohlcv_1m` = 0), no mis-rolled buckets existed
- [x] Live spot-check confirms correct argMin-open / argMax-close at ≥ `_15m`
      — via `SHOW CREATE` definition proof (qualified `t.timestamp`) + the
      green `rollup_chain_it.rs` integration test on the prod-pinned image
- [x] Provenance appended to a 0071 note (`notes/G-prod-reapply-runbook.md`)
