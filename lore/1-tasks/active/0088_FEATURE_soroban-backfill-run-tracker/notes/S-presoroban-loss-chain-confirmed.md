---
title: "Pre-Soroban loss chain CONFIRMED — the daily 03:00 sweep, and why the boundary is 2018-12-13"
type: synthesis
status: mature
spawned_from: ../README.md
spawns: []
tags: [backfill, cleanup, data-loss, pre-soroban, forensics, part-log, incident]
links: []
history:
  - date: 2026-08-04
    status: mature
    who: okarcz
    note: >
      Pass 1 finished 2026-07-27. Traced where its 2016-2018 output went, from
      system.part_log + system.mutations + the run log on fishuser-hero.
      Confirms the 2026-07-20 cleanup incident as the sole cause, corrects two
      recorded claims, and clears pass 2 to run.
---

# Pre-Soroban loss chain — confirmed 2026-08-04

## Verdict

Pass 1 wrote the 2016 → 2018-12-13 era to `prices.price_ohlcv_1m`, and a **daily
job at ~03:00 UTC dropped it, night after night, until cleanup was disabled on
2026-07-20.** The write path was never at fault. **Pass 2 is safe to run** now
that cleanup is off, and it is the correct remedy.

## Pass 1 — complete

Finished **2026-07-27 21:24 UTC** on fishuser-hero (`~/sdex-tail.log`), process
exited, `pgrep -af sdex-backfill` empty.

```
partitions processed:      746
ledgers indexed:           47,641,424
ledgers already in DB:     63,999
SDEX trade ticks:          1,911,936,910
price_ohlcv_1m rows:       689,676,890
total bytes downloaded:    3,127,026,388,741
elapsed:                   1,054,365 s  (12.2 days)
```

Marker state on prod: `frontier 50,457,423` (= activation − 1), `min 3`,
`50,457,421` markers, contiguous, zero gaps.

**Coverage gap in pass 1 itself:** the log jumps from `partition:0` (all 63,999
ledgers skipped as already-marked) straight to `partition:2,816,000`. Partitions
`64,000` – `2,752,000` — 43 partitions, 2,752,000 ledgers — were **never
walked**. The count reconciles: partitions `2,816,000 → 50,432,000` is 745, plus
partition 0 = 746. Low-value (that era logged ~0 trades) but it is not covered,
and pass 2's `[1, 23423999]` range subsumes it.

## The loss chain, from `system.part_log`

Aggregated over `partition_id >= '201603' AND < '201812'`:

| Partitions | Last write | **Swept** |
|---|---|---|
| `201603`–`201707` | 2026-07-18 03:00:06 | **2026-07-18 03:08:39** |
| `201708`–`201803` | 2026-07-19 03:00:06 | **2026-07-19 03:08:20** |
| `201804`–`201811` | 2026-07-20 03:00:06 | **2026-07-20 03:08:30** |

Each night the sweep dropped every historical partition then present; the
backfill kept writing and the next night's sweep took the new ones. **No removal
event after 2026-07-20 03:08:30.**

**The writes were real.** For `201811`: `RemovePart` 75,364 = `NewPart` 43,059 +
`MergeParts` 32,305 — exactly. Every part ever created there was removed, and the
merges processed 393,886,653 rows. This is not a silent write failure.

**Recoverable:** `3,738,473` candles logged below ledger 23,424,000, out of the
run's 689,676,890 total (0.54% of rows, four years of history).

## Why the surviving data starts at 2018-12-13

`201812` has `first_seen 2026-07-20 04:11:24` — minutes *after* that day's final
sweep at 03:08:30, and cleanup was disabled before the 07-21 sweep could run.

> **The `2018-12-13 06:55` boundary is not a property of Stellar history. It is
> where the backfill had walked when cleanup was switched off.** Anyone reading
> it as a data-availability fact will draw the wrong conclusion — as the
> superseded "durable pre-Soroban data" row in the README did.

## Two recorded claims this corrects

1. **Cleanup used `DROP PARTITION`, not `ALTER DELETE`.** There are **no
   `MutatePart` events** on `201603`+ — only `NewPart` / `MergeParts` /
   `RemovePart`. The [[0114]]-derived note that "cleanup deletes by `ALTER
   DELETE` mutation, not `DropPart`" does not hold for this incident, and it
   matters: a mutation-based diagnostic (`system.mutations`) comes back clean and
   would exonerate cleanup. Check `part_log` for `RemovePart` on old partitions
   instead.
2. **`system.mutations` shows nothing after 2026-07-15 10:24.** The `MutatePart`
   events on `201511`/`201512`/`201601`/`201602` at that timestamp belong to a
   separate `DELETE WHERE intDiv(toUInt64(version), 1000) < 50457424`, which
   cleared an **earlier** run's output. Pass 1 did not start until 16:53 that
   day, so that mutation is unrelated to its loss.

## Hazards for pass 2

- ⚠️ **The pre-Soroban delete mutation would erase pass 2's entire output in one
  statement.** `version` is `ledger_seq × 1000 + intra-ledger order`, so
  `DELETE WHERE intDiv(toUInt64(version), 1000) < 50457424` targets exactly and
  only the pre-Soroban rows. Do not re-run it.
- ⚠️ **Re-check cleanup daily during the run.** It swept at 03:00 UTC. Trusting
  the EventBridge rule's state alone is what failed last time — watch for
  `RemovePart` on old partitions as the real signal.
- ⚠️ **The runbook's §7.1 pre-roll gate is marker-based and unsafe.** With
  `min = 3`, `max = 50,457,423`, contiguous, it currently prints
  `BACKFILL COMPLETE — READY TO PRE-ROLL` — while four years of candles are
  missing. **Markers survived every sweep; candles did not.** Gate the pre-roll
  on candle counts per year, never on `backfill_sdex_ledgers`.
- **Straddled activation minute.** Pass 1 logged a WARN: the activation split is
  not minute-aligned, so `minute 1708448400` is written partially by both range
  runs and, because RMT replaces rather than sums, **undercounts**. Reconcile
  that single minute from one pass after both runs, or accept it as a documented
  artifact — but decide, don't discover it later.

## Unexplained, not blocking

At `03:00:06` daily a `NewPart` appears in *every* old partition, minutes before
the sweep. It is not the backfill — by 07-18 that was writing `201707`, not
`201603`. Worth understanding when hardening the preflight guard ([[0109]]), but
it does not change the conclusion.

## Commands that produced this

```sql
-- the loss chain (aggregate; a LIMIT-ed raw listing truncates before the
-- interesting window and shows only routine 202607 merge cleanup)
SELECT partition_id, event_type, count() AS events,
       min(event_time) AS first_event, max(event_time) AS last_event,
       sum(rows) AS total_rows
FROM system.part_log
WHERE table = 'price_ohlcv_1m'
  AND partition_id >= '201603' AND partition_id < '201812'
GROUP BY partition_id, event_type ORDER BY partition_id, event_type;

-- what actually survives (partition-pruned, cheap)
SELECT toYear(timestamp) AS yr, count() AS rows_1m,
       min(timestamp) AS first_ts, max(timestamp) AS last_ts
FROM prices.price_ohlcv_1m WHERE timestamp < '2019-01-01'
GROUP BY yr ORDER BY yr;
```

```bash
# what pass 1 claimed to write, per partition
grep '"partition indexing complete"' ~/sdex-tail.log | python3 -c '
import sys, json
tot = pre = 0
for line in sys.stdin:
    f = json.loads(line)["fields"]
    tot += f["candles"]
    if f["partition"] < 23424000: pre += f["candles"]
print("candles logged, whole run:", tot)
print("candles logged below ledger 23,424,000:", pre)
'
```

AWS profile: `soroban-admin` worked on 2026-08-04; the runbooks say
`soroban-explorer`. Either may be correct — a `UnrecognizedClientException:
security token ... invalid` is an expired SSO session, not a wrong profile.

## Pass 2 — launched 2026-08-04 13:25:15 UTC

Same binary as pass 1 (`target/release/sdex-backfill`, built 2026-07-15 18:22),
tmux `sdex-pass2` on fishuser-hero, log `~/sdex-pass2.log`, `tip 63795749`.

```
loaded completed ledgers from backfill_sdex_ledgers  start:1 end:23423999 completed:0
backfill starting  total_partitions:366  already_done:0  to_process:366
```

`completed: 0` is the confirmation that the §6.2 marker clear worked — with
markers present the run would have skipped all 23.4M ledgers and exited
reporting success. ETA ~2026-08-09/10.

Gates cleared before launch: cleanup `DISABLED`; no sweep-signature removal since
2026-07-20 03:08:30; every partition below `202001` still holds rows (so the
post-07-20 `RemovePart` events were merges, not drops); no backfill process
running; `leftover_low_markers = 0`.

> The unfiltered marker total after the clear is **39,928,612**, not the
> 27,033,424 you get for the pre-Soroban span — the difference is exactly the
> Soroban-era run's `63,352,611 − 50,457,424 + 1 = 12,895,188` markers. Same
> trap as the frontier query: **always filter `WHERE sequence < 50457424`.**
