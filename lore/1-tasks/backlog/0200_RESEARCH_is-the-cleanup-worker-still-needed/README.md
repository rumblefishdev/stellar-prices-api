---
id: "0200"
title: "Is the cleanup worker still needed at all? Decide whether prices-production-cleanup is enabled or disabled"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0088", "0090", "0111", "0167", "0174", "0046", "0063"]
tags:
  ["priority-medium", "effort-small", "clickhouse", "operational", "cost", "retention", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/cleanup-worker/src/lib.rs"
  - "../../../infra/src/lib/stacks/eventbridge-stack.ts"
history:
  - date: 2026-08-13
    status: backlog
    who: okarcz
    note: >
      Spawned from 0088, which is closing with the pre-Soroban backfill complete.
      0088's last open AC was "cleanup stays DISABLED for the entire recovery";
      that condition lapsed 2026-08-11 when the pre-roll landed, so the rule is
      now off by standing decision rather than by necessity, and nobody has
      decided whether it should ever go back on. The operator's prompt for this
      task: measured 1m disk usage came in far below the assumption the retention
      policy was designed against, so the premise for having a cleanup worker at
      all is worth re-testing rather than assumed.
---

# Is `prices-production-cleanup` still worth running?

The rule has been **DISABLED since 2026-07-20** and prod has been healthy
without it for over three weeks. It is time to decide deliberately rather than
leave it off by inertia.

## Why this is being asked now

Retention was designed against a **storage estimate that turned out to be
wrong**. [[0046]] projected a footprint that justified aggressive expiry of the
fine-grained tables; the measured figure is **~1.9–4.2 KB/ledger (~10–22 GB/yr)**
(see [[prices-ch-footprint-measured]]), and `ch-prod-01` had **564 GB free** when
it was last checked on 2026-07-20. If a year of *everything* costs tens of GB,
expiring `price_ohlcv_1m` after 7 days is buying very little.

Against that, the rule has a track record of causing real damage:

- **2026-07-20** — it destroyed ~4.5 days of backfill output (genesis → ~Nov 2018)
  as fast as the run wrote it, because backfilled candles carry *historical*
  timestamps and a 7-day window gives them zero grace. Confirmed from
  `system.part_log`: every part ever created in `201811` was removed. Cost the
  whole of pass 1. See [[cleanup-rule-shreds-backfill-output]].
- It had already forced the same dance during [[0090]]'s rerun.
- It is why the [[0182]] repair run, and every future historical write, needs an
  explicit "is the rule off?" pre-check.

## What it actually does

`cleanup-worker/src/lib.rs` — the list is **opt-in**, so an unlisted table is
retained forever:

| table | retention |
|---|---|
| `price_ohlcv_1m` | 7 days |
| `price_ohlcv_15m` | **30 days** |
| `oracle_prices` | 13 months |

⚠️ Three things that are easy to get wrong and have each cost time:

- **`15m` is 30-day, not 15-minute-ish.** Grepping `init.sql` for `TTL` finds
  nothing and proves nothing — retention here is a **job**, not a TTL. That
  mistake produced [[0174]], filed and closed the same day
  ([[cleanup-worker-retention-table-list]]).
- **`prices.usd_rate` is deliberately ABSENT and must stay that way** ([[0167]]).
  It exists *because* `oracle_prices` expires at 13 months and takes the earliest
  depeg-aware history with it. Adding it would silently recreate the problem it
  was built to solve, unrecoverably.
- **It drops whole partitions (`DROP PARTITION`), not rows.** There are no
  `MutatePart` events, so a `system.mutations` diagnostic wrongly exonerates it
  (0088's 2026-08-04 correction).

## The real question, in three parts

1. **Does anything still need `1m`/`15m` expiry?** What do they cost per month
   now, and what would they cost in a year at the current ingest rate? If the
   answer is "tens of GB against 564 GB free", retention is solving a problem we
   do not have.
2. **Does `oracle_prices` need its 13 months?** That one has a different
   character — it is the largest of the three and [[0167]] already extracted the
   part worth keeping into `usd_rate`. It may deserve a different answer from the
   OHLCV tables. **Do not assume one verdict covers all three.**
3. **What replaces it if it goes?** Unbounded growth is a real cost even if it is
   a slow one. A disk-headroom alarm is the obvious substitute and is much harder
   to get catastrophically wrong than a partition-dropping cron.

## ⚠️ Whatever is decided, the CDK must be made to agree

Right now the template and reality **disagree**, and that is its own defect
independent of the verdict. `eventbridge-stack.ts:147-151` declares the rule with
no `enabled: false`, so **the synthesized template asserts ENABLED** while the
live rule is DISABLED (set by an out-of-band `aws events disable-rule`). Every
deploy of that stack is therefore a chance to silently re-enable it, which is why
0088 and [[0182]] both carry a "check `describe-rule` before *and* after every
deploy" instruction.

That instruction is a workaround for drift, not a control. Closing this task
should delete the need for it.

## Suggested method

- Measure per-table on-disk size and monthly growth from `system.parts`
  (`FINAL`-agnostic — use `active = 1`), split by table and partition.
- Project 12 and 24 months at the current ingest rate. Note that `1m` is by far
  the largest producer (718.6M rows landed in the [[0088]] pre-roll alone).
- Check current free space on `ch-prod-01` and what else shares the host — this
  is a **shared cluster** ([[0063]]), so the decision is not ours alone if it
  materially changes the footprint.
- Consider the middle option explicitly: keep the worker but widen the windows so
  historical writes are not eligible on arrival (the property that caused both
  incidents), e.g. retain by *ingest* time or a much longer floor.

## Acceptance Criteria

- [ ] **DECISION RECORDED: `prices-production-cleanup` should be ENABLED or
      DISABLED.** One of the two, stated plainly, with the measurement behind it.
      A third option (keep it, but change the retention windows or the eligibility
      predicate) is acceptable — but it must be written as a decision, not left as
      "needs more thought".
- [ ] Per-table size and growth measured on prod, projected 12/24 months, against
      measured free space.
- [ ] `oracle_prices` answered separately from the OHLCV tables, with `usd_rate`'s
      dependency on that expiry ([[0167]]) explicitly addressed.
- [ ] **The CDK matches the decision** — `eventbridge-stack.ts` no longer asserts
      a state that differs from the live rule, so the "check `describe-rule` after
      every deploy" workaround can be retired from the runbooks and from [[0182]].
- [ ] If cleanup is retired: a replacement signal (disk-headroom alarm) exists
      before the worker is removed, not after.
- [ ] If cleanup is kept: the historical-write hazard is addressed, so a backfill
      or repair run can no longer have its output deleted as it lands.

## ⛔ Until this is decided

**Cleanup stays DISABLED.** That is the deliberate current position, not an
oversight — recorded here so the next session does not "fix" it. Do not enable
the rule while [[0182]]'s repair run is outstanding: it writes into historical
partitions, which is exactly the shape the 2026-07-20 incident destroyed.
