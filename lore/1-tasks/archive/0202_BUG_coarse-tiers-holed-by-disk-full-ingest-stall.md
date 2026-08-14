---
id: "0202"
title: "A 11.5 h disk-full ingest stall holed every coarse tier — _1m self-healed, the rollups did not, and the surviving buckets read as real low-volume hours"
type: BUG
status: completed
related_adr: []
related_tasks: ["0136", "0137", "0095", "0088", "0111", "0200", "0182"]
tags:
  ["priority-high", "effort-small", "clickhouse", "data-correctness", "rollups", "incident", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/preroll-live-gap.sql"
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
history:
  - date: 2026-08-14
    status: active
    who: okarcz
    note: >
      Spawned from the 2026-08-13 CloudWatch alarms. BE maintenance filled the
      shared Hetzner volume; prod CH returned Code 243 NOT_ENOUGH_SPACE from
      20:15 UTC to 07:56 UTC. price_ohlcv_1m self-healed via the durable-cursor
      reconcile, but every coarse tier is holed because the 1m->15m MV looks
      back only 2 HOURS. Repair tool already exists (preroll-live-gap.sql).
  - date: 2026-08-14
    status: completed
    who: okarcz
    note: >
      Repaired and verified the same day. preroll-live-gap.sql over
      2026-08-13 20:00 -> 2026-08-14 08:00, six stages bottom-up, single pass,
      no errors. All four measurable tiers converged on
      1,986,859,212,523.7518365 with bucket counts 720 / 48 / 12 / 3; _1h went
      from 4 partial buckets at 15.3% of volume to 12 complete ones. _1d for
      08-13 matched _1m exactly, _1w exceeded its four complete days as
      expected, _1M matched the sum of weeks STARTING in August exactly.
      91 redundant DLQ doorbells purged (verified 0) after confirming _1m held
      60/60 minutes in every hour. BE notified. Root cause was BE maintenance on
      the SHARED Hetzner volume — prices is 3.3% of it, so no cleanup of ours
      could have prevented or fixed it. Spawned 0203 (self-healing rollups,
      blocked by 0142) and 0204 (the alarm gaps).
---

# Coarse tiers holed by the 2026-08-13 disk-full ingest stall

## Summary

BE maintenance filled the **shared** Hetzner ClickHouse volume on 2026-08-13.
Prod CH answered every write with `Code: 243 NOT_ENOUGH_SPACE ("Cannot reserve
36.40 MiB")` from **20:15 UTC to 07:56 UTC** — 11.5 h. `price_ohlcv_1m`
recovered by itself; **the six coarse tiers did not and cannot.**

## Context — measured 2026-08-14

⚠️ **Not our disk and not our fault.** `prices` is **58.93 GiB of a 1.72 TiB
volume — 3.3%**. BE's `default` is 951.39 GiB (55%), shared `system` 121.64 GiB.
Nothing we could delete would have prevented or fixed it. The 0063
"isolation-proven" result is **access** isolation, not a disk quota.

**`_1m` is intact — 60 of 60 minutes in every hour of the stall**, 500-1,000
assets each. The [[0064]] durable CH cursor did its job: once space returned the
processor reconciled forward and wrote the backlog into its historical buckets.
The 91 doorbells that landed in the DLQ are therefore **redundant, not data
loss** — purge, do not redrive (a redrive rewrites existing data onto a volume
at 91.4%).

**The coarse tiers are holed**, window `2026-08-13 20:00` → `2026-08-14 08:00`:

| tier | buckets present | expected | volume | % of `_1m FINAL` |
|---|---|---|---|---|
| `_1m` | 720 | 720 | 1,986.9 B | **100%** ✅ |
| `_15m` | 13 | 48 | 845.2 B | 42.5% |
| `_1h` | 4 | 12 | 304.2 B | 15.3% |
| `_4h` | 2 | 3 | 67.0 B | **3.4%** |

`_1d`/`_1w`/`_1M` not yet measured — their buckets extend past the window, so
they need whole-bucket comparison.

🔴 **The surviving buckets are PARTIAL, which is worse than missing.** `_4h`
holds 2 of 3 buckets but 3.4% of the volume — those rows are ~5% full. A missing
bucket reads as absent; **a partial bucket reads as a real trading hour that was
quiet.** BE's 30D/1Y charts would render last night as a genuine volume collapse
and nothing in the data contradicts it. Same wrong-but-visible failure shape as
[[0182]].

## ⛔ Why it cannot self-heal — the first hop looks back 2 HOURS

From `rollups.sql`, not inferred:

| MV | refresh | lookback | reads from |
|---|---|---|---|
| `1m → 15m` | 1 min | **2 HOURS** ← binding constraint | `_1m FINAL` |
| `15m → 1h` | 15 min | 8 hours | `_15m FINAL` |
| `1h → 4h` | 1 hour | 1 day | `_1h FINAL` |
| `4h → 1d` | 4 hours | 7 days | `_4h FINAL` |
| `1d → 1w` | 1 day | 60 days | `_1d FINAL` |
| `1w → 1M` | 1 day | 400 days | `_1w FINAL` |

The bound is `now() - INTERVAL 2 HOUR`, so it moves with the clock and has
already passed those buckets. **Waiting changes nothing — the loss is already
permanent, not pending.**

⚠️ **The generous windows above do NOT rescue it: each tier reads the tier
BELOW, never `_1m`.** `_1h` looks back 8 h and would rebuild 21:00 happily, but
it reads `_15m FINAL`, which is empty there. **The 2 h bottleneck propagates
through the whole chain.** This is the most important fact about the rollup
topology and it is not obvious from any single MV definition.

**The arithmetic confirms the model:** `_15m` held 13 buckets = 3.25 h ≈ the 2 h
window plus time elapsed since recovery. Also note the filled buckets are the
**newest**, not the oldest — a draining backlog fills oldest-first; a moving
lookback window fills newest-only. That signature is how this was diagnosed.

## Implementation — the tool already exists

`packages/prices-clickhouse/schema/preroll-live-gap.sql`, the same script that
closed the [[0136]] gap in all six tiers in 6.1 s. Six stages, bottom-up, each
level aligned to its own bucket, `FINAL` throughout, projects `sum(version)`,
idempotent, `max_threads = 4`.

```
clickhouse-client --param_start_ts='2026-08-13 20:00:00' \
                  --param_end_ts='2026-08-14 08:00:00' \
                  --queries-file preroll-live-gap.sql
```

✅ **No DELETE stage needed** — and reaching for [[0097]]'s RMT-tie rule here is
wrong. `rollups.sql` states the `sum(version)` scheme is self-protecting: *"a
partial bucket sums FEWER source versions than the complete one, so a complete
bucket outranks any partial re-roll of itself."* A complete re-roll wins on
version automatically.

⚠️ **Bottom-up order is load-bearing.** Each stage reads the tier below, so
running `_1d` before `_4h` is repaired would rebuild a wide bucket from
still-broken input — and that partial-but-higher-version row would WIN.

⚠️ `_1M` buckets are **weeks-attributed-by-START** ([[0136]]) — verify the
August bucket after the run rather than assuming the alignment covered it.

## ✅ REPAIRED 2026-08-14 — exact to the last decimal

`preroll-live-gap.sql`, window `2026-08-13 20:00` → `2026-08-14 08:00`, six
stages bottom-up, single pass, no errors, single-digit seconds.

| tier | before | after | buckets | expected |
|---|---|---|---|---|
| `_1m` (truth) | 1,986,859,212,523.7518365 | unchanged | 720 | 720 ✅ |
| `_15m` | 845.2 B (42.5%) | **identical to truth** | 48 | 48 ✅ |
| `_1h` | 304.2 B (15.3%) | **identical to truth** | 12 | 12 ✅ |
| `_4h` | 67.0 B (3.4%) | **identical to truth** | 3 | 3 ✅ |

Whole-bucket checks, which a window subset cannot verify:

- `_1d` 08-13 = **4,086,616,150,096.873451**, matching `_1m` for that day exactly.
- `_1w` week 08-10 = 19.15 T against 17.97 T for the four complete days it
  contains — correctly greater, the excess being 08-14's partial day.
- `_1M` August = 40.78 T, matching **the sum of weeks STARTING in August**
  exactly. ⚠️ Compared against weeks, not the calendar month: the week of 07-27
  is attributed to July though it spills into August, so a calendar comparison
  would read as broken while the data is correct.

`_1w`/`_1M` trail today's live data until their daily refresh — by design, not
damage.

## Acceptance Criteria

- [x] Re-roll run for `2026-08-13 20:00` → `2026-08-14 08:00`, all six tiers,
      bottom-up
- [x] Every tier's `sum(volume_base)` over the window matches `_1m FINAL`.
      ⚠️ The control night matched **exactly to the last decimal**
      (2,058,453,284,485.340191), so anything short of exact means incomplete —
      and the repaired window matched exactly too
- [x] `_1d`/`_1w`/`_1M` verified by whole-bucket comparison, not a window subset
- [x] 91 DLQ messages purged (not redriven — data already present), verified
      `ApproximateNumberOfMessages = 0` 2026-08-14
- [x] BE told the window, since their 30D/1Y charts read it — sent 2026-08-14,
      including the ask for a heads-up before future maintenance that could fill
      the shared volume

## Design Decisions

### From Plan

1. **Reuse `preroll-live-gap.sql` rather than write a repair.** Bucket
   alignment per level, `FINAL`, `sum(version)` and idempotency are already
   solved and proven ([[0136]] closed a 13-day gap in six tiers in 6.1 s). The
   window was the only input needed.
2. **Bound the run at `2026-08-13 20:00` → `2026-08-14 08:00`.** 20:00 is the
   last aligned bucket before the stall; 08:00 sits behind the live frontier so
   the run cannot fight concurrent ingestion.

### Emerged

3. **No DELETE stage — and the instinct to add one was wrong.** I first
   proposed DELETE-first by analogy with [[0097]]'s RMT-tie rule. `rollups.sql`
   already solves it: `sum(version)` means a complete bucket sums more source
   versions than a partial one and therefore **outranks it automatically**.
   Deleting first would have added risk for nothing. Caught by reading the
   schema rather than reasoning from a remembered rule.
4. **Diagnosed from WHICH buckets survived, not how many.** The filled buckets
   were the **newest** (05:00-07:00), not the oldest. A draining backlog fills
   oldest-first; a moving lookback window fills newest-only. That asymmetry is
   what identified the mechanism before any schema was read, and it is the
   cheapest signal to reach for next time.
5. **Verified against a CONTROL window, not just the damaged one.** The first
   comparison showed `_1h` at 21% of `_1m` and looked like a rollup defect. The
   previous night matched **exactly to the last decimal**, proving `FINAL` was
   the right operator and the mechanism sound — the tiers were absent, not
   miscomputing. Without the control this would have been misfiled as a
   correctness bug in the MVs.
6. **Purged the DLQ rather than redriving it.** The 91 doorbells pointed at
   ledgers the reconcile had already written (`_1m` verified 60/60 minutes per
   hour). A redrive would have rewritten existing data onto a volume at 91.4%.
   ⚠️ This is only safe **because** `_1m` was verified complete first — the
   check is the licence for the purge, not an optional extra.

## Future Work

- **[[0203]]** — rollups self-heal by event-time completeness instead of a clock
  window, so the next stall needs no operator. Blocked by [[0142]].
- **[[0204]]** — the alarm gaps: no free-space alarm on the shared CH volume,
  and a DLQ alarm that cannot distinguish 1 from 91.
- ⏳ **Audit older stalls while it is still possible.** [[0111]]'s four-day
  outage would have holed the tiers far worse. `_1m` is the only source of truth
  for a re-roll and it is 7-day retention — **the audit exists only while
  cleanup stays disabled, which is [[0200]]'s decision.** Raise before that
  decision is taken.

## Future Work

- **Free-space alarm on the CH host** — we learned about a full disk from three
  Lambdas panicking, 11.5 h into the stall, while `system.disks` had the answer
  the whole time. On a volume we share and do not control this is the real gap.
- **The DLQ alarm fires once and never re-notifies** — Slack said 1, reality was
  91. Never read the alarm count as queue depth.
- **⚠️ Any ingest stall > 2 h permanently holes the tiers.** Structural, not
  specific to this incident. [[0111]]'s four-day outage would have done the same
  at far greater scale. → **spawned as [[0203]]**: rollups self-heal by
  comparing event-time completeness against the source instead of trusting a
  clock window. Blocked by [[0142]].
- **⏳ The audit of older holes is only possible while cleanup stays DISABLED** —
  `_1m` (7-day retention) is the sole source of truth for a re-roll. **If
  [[0200]] re-enables cleanup, every older rollup hole becomes unmeasurable and
  unfixable forever.** Raise before that decision is taken.
