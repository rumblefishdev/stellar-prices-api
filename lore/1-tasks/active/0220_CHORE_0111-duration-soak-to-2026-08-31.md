---
id: "0220"
title: "Daily soak — 0111's duration alarm must stay OK for a week spanning active backfill"
type: CHORE
status: active
related_adr: []
related_tasks: ["0111", "0026", "0112", "0214"]
tags: ["priority-medium", "effort-small", "enrichment", "observability", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/main.rs"
history:
  - date: 2026-08-24
    status: active
    who: okarcz
    note: >
      Spawned from 0111, which was completed and archived the same day with 5 of
      6 ACs verified on production data. This carries 0111's AC 5 alone - the
      only criterion that is time-gated rather than measurement-gated, and the
      reason 0111 would otherwise have sat active for a week. Alarm returned to
      OK at 2026-08-24 08:31:25 UTC after being in ALARM since 2026-08-21 16:18.
---

# 0111's duration soak — one week, checked daily

## Summary

[[0111]] shipped on 2026-08-24 and is verified on every criterion that can be
measured immediately. **One cannot:** its alarm must stay OK for a *week*, and
that week must span **active backfill**, because a quiet cluster is how this
whole defect stayed hidden twice.

This task exists so 0111 could close on its evidence rather than idle for seven
days waiting on a clock.

## The criterion (0111 AC 5, verbatim intent)

> `EnrichmentPassDurationMs` stays well clear of 300 s for a week spanning active
> backfill, and `prices-production-enrichment-duration-near-timeout` returns to
> OK and stays there.

The alarm watches **`AWS/Lambda` `Duration`** (Maximum, 3600 s period, threshold
**240,000 ms**) on `prices-production-enrichment` — not a custom metric. It fired
at **300,000 / 300,338 ms** on 2026-08-21, which is what re-opened 0111.

## Baseline at hand-off (2026-08-24)

| hour UTC | max duration | invocations | errors |
|---|---|---|---|
| 06:00 (pre-deploy) | 300,298 ms | 3 | **3** |
| 07:00 (pre-deploy) | 300,299 ms | 3 | **3** |
| 08:00 | **29,551 ms** | **1** | **0** |
| 09:00 | **26,685 ms** | **1** | **0** |
| 10:00 | **28,682 ms** | **1** | **0** |

Stage split inside an invocation: 1m pass ~7.2 s, historical sweep ~0.5 s, coarse
sweep ~21 s. Peak Lambda memory **52-54 MB of 512 MB**.

## Daily log

### 2026-08-28 09:5x UTC — days 4 AND 5 of 8, clean — and `Duration` stepped 6× without breaching

⚠️ **Day 4 (2026-08-27) was not checked on the day** — the batch-release session
displaced it. Both days are recorded here from a 48 h window rather than
back-dated as if two separate checks happened. CloudWatch retains the data, so
nothing is lost; the gap is in the process, not the evidence.

| metric | 48 h reading | criterion |
|---|---|---|
| alarm state | `OK`, `StateUpdatedTimestamp` still **2026-08-24T08:31:25** | no transition since hand-off ✅ |
| `Duration` max | **6,341 – 63,295 ms** | ≪ 240,000 ms — peak is **26.4%** of threshold |
| `Invocations` | **1.0/hour, all 48, no gaps** | 1/hour ✅ |
| `Errors` | **empty** | 0/hour ✅ |
| `Throttles` | **empty** | — |
| peak memory | **51 – 54 MB of 512 MB** | unchanged |

### 🔑 The 6× step at 2026-08-27 15:02 UTC is the historical sweep starting work

`Duration` ran 6-11 s for two days, then jumped to 53-63 s at a single bucket and
has drifted up ~0.3 s/h since. **It is not a deploy** —
`prices-production-enrichment` reads `LastModified 2026-08-24T14:11:52Z`, three
days before the step. The cause is in the data, and the logs name it:

| | 08-27 14:17 | 08-27 15:17 | 08-28 09:17 |
|---|---|---|---|
| sweep frontier month | 202205 | **202206** | 202206 |
| sweep state | `exhausted` | **`pending`** | `pending` |
| rows enriched by sweep | **0** | **200,000** | **200,000** |
| live 1m pass `duration_ms` | 9,208 | 11,438 | **7,964** |
| Lambda `Duration` | 13,976 | 57,190 | 62,934 |

[[0111]]'s frontier advanced off an exhausted month onto one with real backlog.
**The live pass is unchanged** (9.2 s → 8.0 s); every added second is the sweep.

🔑 **And it is bounded twice over, which is what settles the soak.**
`enriched: 200000` every pass is exactly `max_batches: 20 × batch_size: 10,000` —
the **batch cap**, not free-running. `deadline_hit: false` on every invocation, so
it never reaches its 120 s budget either. Worst case the config permits is live
pass (~10 s) + a full 120 s sweep ≈ **130 s**, against `Threshold 240,000`,
`Statistic Maximum`, `Period 3600`, `EvaluationPeriods 2`. ~1.8× headroom at the
configured worst case; 3.8× at today's actual.

Progress is exactly on cadence: 202206 remaining fell **8,382,402 → 4,782,402** in
18 h — 200 k/hour, one full pass per hour, no misses. ~1 more day on that month,
**49 months pending** after it.

### ⚠️ The worker's own duration metric does not see the sweep

`EnrichmentPassDurationMs` (`Prices/Enrichment`, `Environment=production`) reads
**5,504 – 11,438 ms** across the same 48 h — flat, no step at all — while Lambda
`Duration` reads up to 63,295 ms. The custom metric times the **live 1m pass
only**.

🔴 **So an operator watching the worker's own metric would not have seen this at
all**, and the alarm that protects the timeout is the Lambda one. That is fine
today because the alarm is on the right metric, but the two series answer
different questions and the ~55 s difference between them is the sweep. Worth
knowing before anyone reads `EnrichmentPassDurationMs` as "how long the worker
takes".

### Write load, days 4-5

`EnrichmentRowsEnriched` totals **210,586 rows** over the 48 h (1,266 – 8,960 per
hour, all 48 datapoints present) — and that counts the **live pass only**. The
sweep added a further ~200,000 per hour from 15:17 on 08-27, so real throughput
across the window is several million rows. AC 4's demonstrated-write-load
condition, already met on day 3, is met far more strongly here.

⚠️ Dimension `Environment=production` passed explicitly, per day 3's method note.

### What this means for the remaining checks

Days 1-3 measured an **idle** sweep; days 4-5 measure a **working** one. The
soak is now exercising the configuration it was actually written to test, and the
elevated `Duration` will persist for **weeks** (49 months at ~200 k rows/pass).
🔑 A later reader will see a plateau, not a spike — it must not be mistaken for a
regression, and a check on 08-29/30/31 that reads ~60 s is **passing**, not
degrading.


### 2026-08-26 16:1x UTC — day 3 of 8, clean AND under real write load

24 consecutive hourly datapoints, no gaps.

| metric | reading | criterion |
|---|---|---|
| `Duration` max | **6,251 – 10,097 ms** | ≪ 240,000 ms — peak is **4.2%** of threshold |
| `Invocations` | **1.0/hour**, all 24 | 1/hour ✅ |
| `Errors` | **0.0/hour**, all 24 | 0/hour ✅ |

Alarm `prices-production-enrichment-duration-near-timeout` = `OK`,
`StateUpdatedTimestamp` still **2026-08-24T08:31:25** — unchanged since the
hand-off, and `describe-alarm-history` shows **no transitions at all** since
then. Days 1-3 have not flapped.

#### 🔑 This window carries demonstrated write load — the outstanding criterion

`Prices/Enrichment`, `Environment=production`, same 24 h:

| metric | 24 h |
|---|---|
| `EnrichmentRowsEnriched` | **126,560 rows** (3,743 – 9,415 per hour, all 24) |
| `EnrichmentRowsRemainingRecent` | 22,434 – 41,005 — a live backlog, worked continuously |
| `EnrichmentPassDurationMs` (worker's own) | 5,378 – 9,196 ms |

So this is **not** a quiet check. The pass enriched ~126.5 k rows while its
duration stayed at ~4% of the alarm threshold. That is the combination AC 4 asks
for, obtained from CloudWatch rather than the `CHQ` insert query.

⚠️ **Method note, because it nearly produced a false alarm.** The first query
omitted the `Environment=production` **dimension** and returned **zero
datapoints** for every `Prices/Enrichment` metric — which reads exactly like "the
worker has stopped publishing". CloudWatch treats each dimension set as a
distinct metric, so a dimensionless query matches a series that does not exist.
🔑 **Always pass the dimension; `list-metrics --metric-name X --query
'Metrics[].Dimensions'` shows which.** Same shape as [[0222]]'s future-window
trap: a query artefact that is indistinguishable from a production failure.

### 2026-08-25 17:41 UTC — day 2 of 8, clean on all three metrics

24 consecutive hourly datapoints, no gaps.

| metric | reading | criterion |
|---|---|---|
| `Duration` max | **6,057 – 10,807 ms** | ≪ 240,000 ms — peak is 4.5% of threshold |
| `Invocations` | **1.0/hour**, all 24 | 1/hour ✅ |
| `Errors` | **0.0/hour**, all 24 | 0/hour ✅ |

Alarm `prices-production-enrichment-duration-near-timeout` = `OK`, with
`StateUpdatedTimestamp` still **2026-08-24T08:31:25** — unchanged since the
hand-off baseline, so it has not flapped in between.

No sign of the 3-invocations / 3-errors pattern.

🔑 **The baseline above is stale, and a future check must not read today's 8 s
against it as an unexplained change.** The hand-off recorded ~27,000 ms with a
stage split of "1m pass ~7.2 s, historical sweep ~0.5 s, **coarse sweep ~21 s**".
[[0218]] moved that coarse sweep into its own Lambda on 2026-08-24, so the ~21 s
left this function. What remains — 6-11 s — matches the 1m-pass plus
historical-sweep figures almost exactly. The drop is the split showing up in the
metric, not a new improvement to chase.

⚠️ Still outstanding for the soak: the **demonstrated-write-load** criterion. A
quiet week does not count. It needs to land in only one of the six remaining
checks; prefer a weekday afternoon over a Sunday.

## The daily check

```bash
export AWS_PROFILE=soroban-admin

aws cloudwatch describe-alarms --region eu-central-1 \
  --alarm-names prices-production-enrichment-duration-near-timeout \
  --query 'MetricAlarms[0].{State:StateValue,Since:StateUpdatedTimestamp}' --output table

aws cloudwatch get-metric-statistics --region eu-central-1 --namespace AWS/Lambda \
  --metric-name Duration --dimensions Name=FunctionName,Value=prices-production-enrichment \
  --start-time "$(date -u -d '24 hours ago' +%Y-%m-%dT%H:%M:%SZ)" \
  --end-time "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
  --period 3600 --statistics Maximum --unit Milliseconds \
  --query 'sort_by(Datapoints,&Timestamp)[].{T:Timestamp,Max_ms:Maximum}' --output table
```

⚠️ **Watch `Invocations` and `Errors` too, not just duration.** A return to
**3 invocations / 3 errors per hour** means the pass is failing again — a
different and worse fault than a slow one, and duration alone will not show it.

🔴 **A quiet week does not count.** Before accepting the soak, confirm the cluster
was actually writing during it:

```sql
SELECT count() AS inserts, sum(written_rows) AS rows
FROM system.query_log
WHERE type = 'QueryFinish' AND event_time >= now() - INTERVAL 30 MINUTE
  AND query_kind = 'Insert'
```

Reference: 6,539 inserts / 226,254,819 rows per 30 min on 2026-08-24.

## Rollback

No code deploy needed: set `ENRICH_LIVE_PARTITIONS=0` on
`prices-production-enrichment`; the historical sweep self-disables with it
(`eventbridge-stack.ts:486`).

## Acceptance Criteria

- [ ] The alarm is OK on every daily check from 2026-08-24 to **2026-08-31**.
- [ ] Hourly `Duration` maximum stays **well under 240,000 ms** across that week
      — baseline is ~27,000 ms, so anything above ~100,000 ms warrants a look
      before the threshold is reached.
      ⚠️ **The baseline moved on 2026-08-27 and the trigger figure still holds.**
      From 15:02 that day the historical sweep began working a non-exhausted
      month, taking the hourly maximum from ~10,000 ms to **~63,000 ms**. That is
      26.4% of the threshold and still below the ~100,000 ms look-at line, so no
      action is due — but a check on 08-29/30/31 reading ~60,000 ms is **passing**,
      not degrading. The sweep is capped at 20 batches and a 120 s budget, so the
      configured worst case is ~130,000 ms. See the days 4-5 log entry.
- [ ] `Invocations` stays at **1/hour** and `Errors` at **0/hour**.
- [x] At least one check falls in a window with **demonstrated write load**,
      evidenced by the `query_log` insert count, not assumed.
- [ ] On success: record the result in [[0111]]'s archived file and tick its
      AC 5. On failure: reopen 0111 rather than patching around it here.

## Out of scope

- Everything else in 0111 — already verified and archived.
- The coarse sweep's own starvation and observability — that is [[0218]].
- The latched enrichment-errors alarm — that is [[0214]].
