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
- [ ] `Invocations` stays at **1/hour** and `Errors` at **0/hour**.
- [ ] At least one check falls in a window with **demonstrated write load**,
      evidenced by the `query_log` insert count, not assumed.
- [ ] On success: record the result in [[0111]]'s archived file and tick its
      AC 5. On failure: reopen 0111 rather than patching around it here.

## Out of scope

- Everything else in 0111 — already verified and archived.
- The coarse sweep's own starvation and observability — that is [[0218]].
- The latched enrichment-errors alarm — that is [[0214]].
