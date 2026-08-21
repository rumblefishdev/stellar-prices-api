---
id: "0215"
title: "The deployed enrichment Lambda never issues the USDT pivot — refs.usdt is None on prod, and the statement has only ever run by hand"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0209", "0212", "0111", "0172", "0182", "0141", "0213"]
tags: ["priority-high", "effort-small", "enrichment", "clickhouse", "deploy", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-21
    status: backlog
    who: okarcz
    note: >
      Split from 0209, whose 2026-08-20 root cause this falsifies. Measured from
      system.query_log while re-measuring 0111. Cheap to fix and NOT blocked by
      0111 — which is why it is split rather than folded in.
---

# The deployed Lambda never issues the USDT pivot

## Summary

`prices.price_ohlcv_1m` has no USDT-priced rows because the deployed enrichment
Lambda **never runs the statement that would write them**. This is a resolution
or stale-asset defect, not the backlog [[0209]] blamed, and it is not gated on
[[0111]].

## Evidence (prod `system.query_log`, measured 2026-08-21)

`pivot_sql` bakes its reference id into the SQL text as a literal
(`CAST({ref_id} AS UInt32) AS ref_asset_id`), so the log reads the deployed
binary's behaviour directly rather than the source's intent:

| pivot ref | runs | first seen | last seen |
|---|---|---|---|
| XLM (`id 4`) | 7,352 | 2026-08-07 09:20:15 | **2026-08-21 08:27:19** |
| USDT (`id 111`) | 6,493 | 2026-08-18 11:40:08 | **2026-08-18 14:30:04** |

USDT's entire lifetime is a 2 h 50 m window on 2026-08-18 — the run of [[0182]]
from execution host C, using a **locally built** binary. The hourly schedule has
never issued one.

⚠️ The run counts match `%ref_asset_id%` across **all** tiers, so they include
the repair tool's coarse-table work. The `first_seen`/`last_seen` boundaries
carry the finding; the totals do not.

Corroborated a third way by `written_rows`: over the 7 days to 2026-08-21 the
XLM pivot wrote 660-720K rows/day on `price_ohlcv_1m` while the USDT pivot wrote
nothing, because it was never invoked. [[0209]]'s `pivot_written = 0` therefore
describes THIS task, not a throughput limit.

Corroborated independently by the run ratio: through 2026-08-19 the log shows
`peg_insert : pivot_insert` at exactly **1:1** (70:70, 67:67, 66:66), where the
source issues one peg and **two** pivots per step.

## Root cause — `refs.usdt` is `None` in the deployed Lambda

`ReferenceIds::pivot_ids()` returns `[xlm, usdt]`, and `enrich_peg_pivot_step`
iterates it in a single `for` loop with no break between the two. So XLM running
while USDT does not can only mean `pivot_ids()` yields one element — i.e.
`resolve_reference_ids()` left `refs.usdt` at `None`.

The data is not the problem. Prod `prices.assets` holds the canonical pair —
`asset_id = 111`, `asset_code = 'USDT'`, issuer
`GCQTGZQQ5G4PTM2GL7CDIFKUBIPEC52BROAQIAPW53XBRJVN6ZJVTG6V` — which is exactly
what the resolver's predicate asks for.

Two candidates, both deploy-shaped, and the 08-18 binary resolving it correctly
points at the first:

1. **The deployed artifact predates [[0172]]**, which is what added USDT to
   `pivot_ids()`. This is the [[0141]] stale-asset trap, which has now been live
   three times — most recently on 2026-08-20, where the probe binary in
   `target/` was three hours older than its source and every signal reported
   success.
2. **`USDT_ISSUER` in the deployed binary differs** from the issuer on
   `asset_id = 111`.

## Implementation

- Discriminate the two candidates on the deployed artifact before changing any
  code — `strings` the deployed bootstrap for `USDT_ISSUER` and for the second
  pivot, the same discriminator recorded in the archived 0213 file.
- If it is a stale asset: rebuild, verify by `strings`, redeploy, and confirm by
  **measurement** — a `CAST(111 …)` run appearing on the schedule — never by
  deploy exit status ([[oracle-writers-span-two-stacks]]).
- Add a guard so a silently-absent reference is refused rather than reported
  healthy. 0204's gap 4 already carries `resolved_legs` in its metric row for
  exactly this reason; the enrichment pass has no equivalent.
- ⚠️ The enrichment Lambda ships from `eventbridge-stack`, which is where
  `CleanupRule` lives. Check `describe-rule` **before and after** the deploy
  ([[cleanup-rule-shreds-backfill-output]], [[0200]]).

## Acceptance Criteria

- [ ] The deployed artifact is shown to be missing the USDT pivot (or to carry a
      mismatched issuer), by inspecting the binary — not by reading the source.
- [ ] After the fix, `system.query_log` shows `CAST(111 AS UInt32) AS
      ref_asset_id` running on the hourly schedule, outside any hand-run window.
- [ ] `peg_insert : pivot_insert` reaches 1:2 on `price_ohlcv_1m`.
- [ ] USDT-quoted `_1m` rows are measurably written — `written_rows > 0` on the
      USDT pivot, recorded before/after.
- [ ] `CleanupRule` verified `DISABLED` before and after the deploy.
- [ ] A missing reference asset fails loudly instead of silently narrowing
      `pivot_ids()`.

## Out of scope

- The 1.56M peg-valued `_1m` rows already on prod — that is [[0212]].
- The full-table scan and the 556.78M XLM-quoted backlog — that is [[0111]].
  ⚠️ Fixing this task adds a **third** statement per batch to a pass that
  already reads 739.68M rows each time, so it makes 0111 worse. Sequence
  accordingly.
