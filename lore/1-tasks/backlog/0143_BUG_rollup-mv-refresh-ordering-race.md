---
id: "0143"
title: "The rollup MV cascade has no DEPENDS ON — same-cadence tiers race daily and a tier can serve a stale tip"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0136", "0137", "0142", "0095"]
tags: ["priority-medium", "effort-small", "clickhouse", "rollups", "correctness"]
links: []
history:
  - date: 2026-08-04
    status: backlog
    who: okarcz
    note: >
      Found while closing out [[0136]] remaining-work item 1 (confirm _1w / _1M
      advance after 00:00). `_1w` reached 2026-08-03 as expected; `_1M` stayed
      frozen at 2026-07-01 even though `mv_ohlcv_1w_to_1M` had just run
      successfully — status Scheduled, empty exception, last_success_time
      2026-08-04 00:00:00. Cause is refresh ordering, not the 0136 freeze: both
      tail MVs are REFRESH EVERY 1 DAY with identical last_success_time and
      next_refresh_time, and `rollups.sql` declares no DEPENDS ON anywhere.
---

# Rollup MV cascade has no `DEPENDS ON` — tiers race and serve stale tips

## Summary

The six rollup MVs in `packages/prices-clickhouse/schema/rollups.sql` form a
cascade — `1m → 15m → 1h → 4h → 1d → 1w → 1M` — but each is scheduled purely on
its own wall-clock interval:

```sql
CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1d_to_1w
REFRESH EVERY 1 DAY APPEND
TO prices.price_ohlcv_1w AS …

CREATE MATERIALIZED VIEW IF NOT EXISTS prices.mv_ohlcv_1w_to_1M
REFRESH EVERY 1 DAY APPEND
TO prices.price_ohlcv_1M AS …
```

**There is no `DEPENDS ON` in the file.** ClickHouse therefore gives no ordering
guarantee between a tier and the tier it reads from. A consumer MV can sample
its source table *before* the producer has written the current bucket, compute
over the stale contents, and append a result that silently omits the newest
period. The refresh reports success — it did run, it just read too early.

The two `EVERY 1 DAY` MVs are the acute case because they fire at the **same
instant**, so they race on every single refresh.

## Observed on prod (ch-prod-01, 2026-08-04)

Confirming [[0136]]'s recovery, six of seven tables were current and `_1M` was
34 days stale:

```
price_ohlcv_1m   2026-08-04 10:42:00
price_ohlcv_15m  2026-08-04 10:30:00
price_ohlcv_1h   2026-08-04 10:00:00
price_ohlcv_4h   2026-08-04 08:00:00
price_ohlcv_1d   2026-08-04 00:00:00
price_ohlcv_1w   2026-08-03 00:00:00   <- advanced, as expected
price_ohlcv_1M   2026-07-01 00:00:00   <- did NOT advance to 2026-08-01
```

Both MVs had run cleanly, at the same timestamp:

```
mv_ohlcv_1d_to_1w   status Scheduled  last_success 2026-08-04 00:00:00  next 2026-08-05 00:00:00  exception ''
mv_ohlcv_1w_to_1M   status Scheduled  last_success 2026-08-04 00:00:00  next 2026-08-05 00:00:00  exception ''
```

`mv_ohlcv_1w_to_1M` read `price_ohlcv_1w` while `mv_ohlcv_1d_to_1w` was still
writing the `2026-08-03` week row into it, so it saw max `2026-07-27`, whose
`toStartOfInterval(…, INTERVAL 1 MONTH)` is `2026-07-01` — it re-appended July
and the tip never moved.

The source data was present the whole time. Replaying the `_1M` MV's own SELECT
read-only against current `_1w` yields the bucket it skipped:

```
month_bucket  rows_would_emit
2026-08-01              11506
2026-07-01              70174
2026-06-01             153864
```

> ⚠️ **This is NOT the [[0136]] freeze.** There, merges and mutations were inert
> and the MV threw `TOO_MANY_PARTS` 40,377 times. Here the MV runs, succeeds,
> and reports a clean status — the defect is invisible to every signal 0136
> taught us to check.

## Why it matters

- **A stale tier is indistinguishable from a healthy one** by MV status. This is
  the same blind spot that let 0136 run 17 days silent, which is why [[0137]]
  (freshness measured on the **data**, not on MV status) is the companion fix.
- **`_1M` is the visible victim, but the defect is structural.** Every tier can
  serve a one-cycle-stale tip. On the fast tiers a cycle is a minute and nobody
  notices; at a month boundary it is a wrong `/ohlcv` answer at `1M`
  granularity for up to a day.
- **It self-heals but is not self-correcting.** Because the MVs are `APPEND`
  over a bounded recent window, the next refresh usually picks the missed bucket
  up (verified: `_1M` should reach `2026-08-01` at the `2026-08-05 00:00:00`
  refresh). What it does *not* do is guarantee the window is wide enough to
  recover a bucket that has aged out — see acceptance criterion 4.

## Implementation

- Chain the cascade with `DEPENDS ON` so each tier refreshes only after its
  source: `mv_ohlcv_15m_to_1h DEPENDS ON mv_ohlcv_1m_to_15m`, and so on up to
  `mv_ohlcv_1w_to_1M DEPENDS ON mv_ohlcv_1d_to_1w`. Check the semantics on the
  prod pin (**26.3.10.60**) before committing to the shape — in particular how
  `DEPENDS ON` interacts with differing intervals between tiers, since ours are
  not uniform (1 MINUTE → 15 MINUTE → 1 HOUR → 4 HOUR → 1 DAY → 1 DAY).
- Verify locally first, on a CH pinned to the prod version — see
  [[feedback-local-tests-match-prod-version]]. A test that wedges the ordering
  (seed `_1d`, fire both dailies together, assert `_1M` picks up the new month)
  is the regression guard.
- Reconcile with [[0142]] before touching prod: `rollups.sql` is
  `CREATE MATERIALIZED VIEW IF NOT EXISTS` with no `DROP`, so **editing the file
  and re-applying changes nothing on a provisioned target and reports success**.
  Whatever 0142 settles on for safely redefining these MVs is the delivery
  mechanism for this fix too — the two tasks should land together or 0143's edit
  will silently no-op.

## Acceptance Criteria

- [ ] `rollups.sql` declares the cascade order explicitly; no tier can refresh
      ahead of its source.
- [ ] A test on CH 26.3.10.60 reproduces the race (fails without the fix) and
      passes with it.
- [ ] On prod, `price_ohlcv_1M` reaches the current month bucket on the first
      refresh after `_1w` gains the month's first week — not the second.
- [ ] Confirm the bounded `WHERE` windows are wide enough that a missed bucket
      is still recoverable on the next refresh for **every** tier, or document
      the tier where that is not true.
- [ ] Coordinated with [[0142]] so the change actually lands on ch-prod-01.

## Notes

- Not urgent in itself — the effect is a bounded, self-healing lag rather than
  data loss. It is worth fixing because it is cheap, and because it produces a
  wrong answer that looks exactly like a healthy system.
- The July row-count shortfall visible above (70,174 vs June's 153,864) is
  **not** this bug — it is [[0136]]'s 07-21→08-03 gap propagating up through
  `_1w`, and closes with that task's bounded incremental pre-roll.
