---
id: "0088"
title: "Execute + track the historical backfill run (SDEX + Soroban AMM) to completion"
type: FEATURE
status: active
related_adr: ["0005", "0009"]
related_tasks: ["0053", "0028", "0082", "0090", "0096"]
tags: [layer-infra, priority-medium, effort-large, milestone-M1, backfill, soroban, sdex, operational, tracker]
milestone: 1
links:
  - "../../../docs/runbooks/continue-soroban-backfill.md"
  - "../../../docs/runbooks/running-ingestion-components.md"
history:
  - date: 2026-07-08
    status: active
    who: okarcz
    note: >
      Spawned from 0053 operational tail. 0053's code + tests + runbook are
      complete and merged (PR #72); the only remainder is executing the
      multi-day two-range archive run and the post-run data spot-check, which
      this task owns and tracks. Backfill currently PAUSED by operator decision,
      to be resumed (ideally from a us-east-2 EC2). First clean forward chunk
      [50688000, 51007999] already landed 2026-07-07.
  - date: 2026-07-09
    status: blocked
    who: okarcz
    by: ["0090"]
    note: >
      Blocked by 0090. Running the backfill revealed it produces nothing
      durable: it writes only price_ohlcv_1m (a 7-day feeder); the rollup MVs
      are live-only (2h window) and the historical preroll step is never run, so
      the nightly cleanup drops backfilled 1m before it reaches the permanent
      coarse tables the consumer reads. Resume once 0090 is applied. Fix runbook:
      docs/runbooks/fix-backfill-history-loss-and-rerun.md (PR #103).
  - date: 2026-07-15
    status: active
    who: okarcz
    note: >
      UNBLOCKED (0090 DONE) + re-scoped. Most of the run is already complete: the
      combined range [activation, 63352611] was written by the full-backfill (Phase 1
      done) and durably pre-rolled in 0090; the pre-Soroban tail is WAIVED (BE
      Soroban-only, 0092 resolved). Remaining: (1) OHLCV-for-soroswap AC is now
      BLOCKED-in-effect by task 0096 (soroswap invisible to the backfill — needs the
      pool_registry-preload fix + a range re-run); (2) confirm /backfill/status final
      state; (3) the docker-gated CH integration tests. Not fully closeable until 0096.
  - date: 2026-07-15
    status: active
    who: okarcz
    note: >
      Soroswap root cause CONFIRMED (0096) = registry SEED-TIMING, not a missing
      backfill preload. The backfill already preloads prices.pool_registry since
      0053; the 221 soroswap rows carry tokens but were seeded 2026-07-14, AFTER
      the Soroban run, so reg.soroswap was empty at run time → 0 soroswap candles.
      0096 shipped the code side (closed a dispatch silent-drop so unresolvable
      pools now surface in unresolved_pools) and HANDED THE OPERATIONAL RE-RUN TO
      THIS TASK. Remaining Soroswap AC here = a bounded combined-mode re-run over
      the Soroswap-affected range now that the registry is seeded (0090 runbook:
      disable cleanup → backfill → pre-roll → re-enable), verified per-source.
  - date: 2026-07-16
    status: active
    who: okarcz
    note: >
      PROGRESS + a debugging gotcha. The pre-Soroban SDEX tail run (fishuser-hero,
      tmux `sdex-tail`, direct-write to Hetzner) is HEALTHY and advancing — it is
      NOT waived (the run being live means the 2026-07-15 "WAIVED" AC is stale and
      should be reconciled). As of 2026-07-16 ~09:50 UTC it has walked the floor up
      to partition ~5.76M (from ~5.44M at 07:57), ~170k ledgers/hr; the full
      pre-Soroban span `5.44M → activation (50,457,423)` is ~45M ledgers ≈ ~11 days
      continuous at this rate. Coverage snapshot: combined `[activation, 63352611]`
      complete (534M candles, 2024→2026); pre-Soroban is being FILLED bottom-up,
      so 2017–2023 SDEX candles are still absent *for now* but arriving as the run
      climbs.
      ⚠️ GOTCHA (self-correction): I earlier misread this run as "dead" from prod
      CH. The backfill is DOWNLOAD-BOUND — each 64k-file partition takes ~22.7 min
      to `aws s3 sync`, so `backfill_sdex_ledgers.max` only jumps once per ~22 min,
      and `last_push_at` only moves on a partition that yields candles (early-2016
      SDEX is sparse → many 0-candle partitions freeze it for long stretches). Two
      CH samples 89 s apart both landed inside one partition's download window and
      showed no motion → a FALSE stall. Correct liveness check: `pgrep -af
      sdex-backfill` + partition number climbing in `~/sdex-tail.log` over ≥25 min,
      or `backfill_sdex_ledgers.max` sampled >22 min apart. Memory:
      [[sdex-backfill-presoroban-gap-0088]], [[local-backfill-throughput-measured]].
  - date: 2026-07-20
    status: active
    who: okarcz
    note: >
      🔴 DATA LOSS FOUND + CONTAINED. The tail run is healthy (floor 23,423,999 =
      46.4%, ~177k ledgers/hr, ~6.3 days to activation) but had been DESTROYING ITS
      OWN OUTPUT since it started 2026-07-15: the nightly `prices-production-cleanup`
      EventBridge rule was ENABLED (re-enabled after 0090's rerun; the tail start
      never re-checked it). `cleanup-worker` drops whole monthly partitions where
      `toUInt32(partition) < toYYYYMM(now() - INTERVAL 7 DAY)` — backfilled candles
      carry HISTORICAL timestamps, so their partitions are eligible the moment they
      land. The "7 day retention" gives backfill output ZERO grace, and each nightly
      fire wiped that day's work.
      LOST: genesis → ~Nov 2018 (ledgers 1 → ~21.4M, ~4.5 of the 5 days spent).
      SURVIVING `price_ohlcv_1m`: `201812` 207,021 rows (PARTIAL — pre-fire portion
      dropped), `201901`–`201904` 1,160,062 rows (trusted, written post-fire),
      `202607` 10.0M (live current month, never eligible). Soroban-era coarse
      UNTOUCHED (1h…1M are not in `RETENTION`).
      CONTAINED 2026-07-20: rule now DISABLED (operator-run `aws events disable-rule`);
      disk headroom verified 564 GB free on ch-prod-01 (gate is ≥20 GB) so cleanup can
      stay off for the full ~12-day recovery. Run left going — everything from the
      containment point forward is durable.
      ⚠️ DETECTION GOTCHA: the tell was a `price_ohlcv_1m` year histogram showing
      2015–2017 entirely absent. That is NOT explainable as "early SDEX was sparse" —
      check the cleanup rule state before reaching for any data-shape explanation.
      Recovery plan added below (§Recovery plan). Memory:
      [[cleanup-rule-shreds-backfill-output]].
  - date: 2026-07-21
    status: active
    who: okarcz
    note: >
      Health check. Pass 1 RUNNING and ON RATE: floor 27,583,999 at 13:35 UTC,
      up 4.16M ledgers in 23h20m = 178.3k/hr, within 1% of the documented rate.
      Markers contiguous 3 -> 27,583,999. Pass 1 ETA 2026-07-26/27 (~5.3 days);
      with pass 2 the real remaining total is ~10.8 days.
      prices-production-cleanup re-verified DISABLED.
      Durable data measured: sdex pre-activation spans 2018-12-13 06:55 ->
      2020-01-07 22:23, 3,588,820 candles. This PINS the wipe boundary at
      2018-12-13, refining the earlier "genesis -> Nov 2018" estimate, and
      confirms the recovery plan: pass 2 must fill 2015 -> 2018-12-13. The plan
      re-walks to 23,423,999 which overshoots the real gap (~21.4M) by ~2M
      ledgers; harmless RMT-dedup overlap, and the wider range is deliberate to
      avoid a boundary error.
      GOTCHA for future checks: max(sequence) on backfill_sdex_ledgers WITHOUT
      a "WHERE sequence < 50457424" filter returns 63,352,611 -- the completed
      combined run's ceiling, not the tail position -- and yields a nonsense
      >100% figure. Always filter to the pre-activation range
      (preroll-incremental-presoroban.md:35 has the correct form).
      GOTCHA 2: the marker percentage OVERSTATES progress. 54.7% of ledgers are
      marked but only ~6.2M of 27.58M have surviving candles, because cleanup
      dropped price_ohlcv_1m partitions while leaving markers intact. That is
      also exactly why recovery step 2 must DELETE the markers before pass 2,
      or the resume logic skips the whole span.
  - date: 2026-07-23
    status: active
    who: okarcz
    note: >
      Health check during the 0114 Phase C session. Pass 1 HEALTHY - frontier
      35,839,999, markers contiguous with zero gaps, 14.62M remaining. Rate has
      eased to ~151k/hr (25.4 min per 64k partition) from a 165k/hr two-day
      average because the run is download-bound and partitions are growing
      (4.90 to 5.49 GB, sync 1483 to 1568s, index flat at ~940s), so ETA slips
      to 2026-07-27/28. NEW FINDING - the soroban_amm leg is DEAD and its status
      field lies: backfill_progress says status=running but last_push_at is
      2026-07-14 17:54:24 and no process exists on fishuser-hero, stopping
      122,864 ledgers short of target at the start of the 0111 outage window.
      Nothing writes a terminal state on crash. ~1h to recover, needs the pool
      registry seeded first. Also confirmed the cleanup EventBridge rule is
      DISABLED and price_ohlcv_1m has NO TTL, so nothing is deleting now -- but
      the 1m hole is far wider than recorded (only 2018-12-13 to 2021-06-14, the
      backfill's own output, plus 2026-07-01 onward from live; everything
      between is gone) and the mechanism of that loss is UNVERIFIED. See 0114
      for the mutation-vs-DropPart correction that follows from it.
  - date: 2026-08-04
    status: active
    who: okarcz
    note: >
      PASS 1 COMPLETE (2026-07-27 21:24 UTC, 12.2 days, 746 partitions,
      689,676,890 rows, 3.13 TB). And the loss mechanism flagged UNVERIFIED on
      07-23 is now CONFIRMED from system.part_log: a daily ~03:00 UTC sweep
      dropped every historical partition as fast as pass 1 wrote it - last
      removals 07-18 03:08:39, 07-19 03:08:20, 07-20 03:08:30, then nothing.
      The writes were real (201811 alone merged 393,886,653 rows; RemovePart
      75,364 = NewPart 43,059 + MergeParts 32,305 exactly, i.e. every part ever
      created was removed), so this was never a write-path failure. 201812
      survives only because it was first written 2026-07-20 04:11:24, minutes
      after that day's final sweep, and cleanup was disabled before the 07-21
      one. CORRECTION - the 2018-12-13 boundary is therefore an artifact of the
      incident, NOT a data-availability fact, and the 07-21 "durable data" row
      that read it as a wipe line is superseded. CORRECTION 2 - the sweep used
      DROP PARTITION, not ALTER DELETE: there are NO MutatePart events on
      201603+, so the 0114-derived mutation-vs-DropPart note is wrong for this
      incident and a system.mutations diagnostic would wrongly exonerate
      cleanup. Pass 2 is UNBLOCKED (cleanup verified DISABLED via profile
      soroban-admin, not the documented soroban-explorer) and recovers
      3,738,473 candles. Also found - pass 1 never walked ledgers 64,000-
      2,815,999 (43 partitions, ~0 trades, subsumed by pass 2's range); the
      handoff runbook's §7.1 pre-roll gate is marker-based and currently prints
      READY TO PRE-ROLL while four years of candles are missing; and the
      pre-Soroban DELETE mutation would erase pass 2's whole output if re-run.
      Full forensics in notes/S-presoroban-loss-chain-confirmed.md.
---

# Execute + track the historical backfill run (SDEX + Soroban AMM)

## Summary

Operational tracker for running the combined single-pass historical backfill
(SDEX + Soroban AMM) to completion and confirming the data. All code, tests,
and the runbook landed under **task 0053** (merged, PR #72) — **no code work
remains**. This task exists solely to execute the multi-day archive run,
resume/track its chunked progress, and close the two operator-run data ACs that
0053 could not confirm without the real run.

**How to run it:** follow **[`docs/runbooks/continue-soroban-backfill.md`](../../../docs/runbooks/continue-soroban-backfill.md)**
(first-timer resume guide — resumes from `soroban_amm.current_ledger` in 320k
chunks, each direct-written to Hetzner and run to completion). Component/run
context: [`docs/runbooks/running-ingestion-components.md`](../../../docs/runbooks/running-ingestion-components.md) §1.

## Context

The run is **download-bound** (~184 KB/ledger, ~4.8 MB/s, ~64 min/100k at home)
and large: the combined remainder is ~2.3 TB / ~5.6 days continuous, and the
pre-Soroban SDEX tail is ~4× larger again. The runbook strongly recommends a
**us-east-2 EC2** (same region as the `aws-public-blockchain` bucket) to
collapse download time before committing weeks of home bandwidth. The operator
paused the run by choice; it will be resumed later.

**Run order (from 0053):** combined `[activation, 63352611]` first, then
sdex-only `[1, activation−1]` with `--tip <live tip>`.

**`--end` floor = 63352611** (SDEX live-ingestion floor − 1) to avoid same-source
minute overlap with live in `ReplacingMergeTree` (which silently undercounts, not
duplicates — see [[backfill-live-no-code-coordination]]). Backfill now
contaminates `min(sdex ledger)` in `price_ohlcv_1m`, so **re-derive the live
floor from a non-backfilled source (or the live cursor) right before the final
combined chunk**; it only moves forward, so 63352611 stays a safe ceiling
meanwhile.

## Progress log

| Range | Status |
|-------|--------|
| `[activation, 50687999]` | data + `pool_registry` landed; run fatal-exited on the 0087 router guard (fixed + merged, PR #92). |
| `[50688000, 51007999]` | ✅ first clean forward chunk (2026-07-07) — 320k ledgers, SDEX 17.22M candles, AMM=0/oracle=0 (expected early epoch), 0 new unresolved. ~3.4 h, 59 GB. |
| `[51008000, 63352611]` combined remainder | 🔴 **STALLED SINCE 2026-07-14 — and its status field lies.** `prices.backfill_progress` reads `task_name: soroban_amm, status: running, current_ledger: 63,352,611`, but `last_push_at` is **2026-07-14 17:54:24** (9 days stale as of 07-23) and **no process exists** — `pgrep -af "sdex-backfill.*soroban"` on fishuser-hero returns nothing, and only `sdex-tail` is in tmux. It stopped **122,864 ledgers short** of its 63,475,475 target, at the start of the [[0111]] enrichment-outage window. Nothing writes a terminal state on a crash, so `status` stayed `running`. Recovery is ~1 h of runtime. ⚠️ Re-run needs the **pool registry seeded first** (a mid-chain AMM backfill with no seed loses volume to `unresolved_pools`) and ranges kept disjoint from live ingestion. |
| `[1, 50457423]` pre-Soroban SDEX tail — **pass 1** | ✅ **COMPLETE 2026-07-27 21:24 UTC** (12.2 days). 746 partitions, 47,641,424 ledgers indexed, 689,676,890 `price_ohlcv_1m` rows, 3.13 TB downloaded; process exited cleanly. Marker state: frontier **50,457,423** (= activation − 1), min 3, contiguous, zero gaps. ⚠️ **Its 2016 → 2018-12-13 output was destroyed by the nightly cleanup sweep and must be recovered by pass 2** — see [[S-presoroban-loss-chain-confirmed]]. ⚠️ Pass 1 also never walked ledgers **64,000 – 2,815,999** (43 partitions; ~0 trades there, and pass 2's range covers them). *Prior reading:* **2026-07-23 16:00 UTC: frontier 35,839,999**, markers contiguous (35,839,997 over `3 → 35,839,999`, zero gaps), remaining **14,617,424**. Rate has eased to **~151k/hr** (25.4 min per 64k-ledger partition, measured from consecutive `indexing complete` lines) against a 165k/hr two-day average — the run is **download-bound and partitions are growing** (4.90 → 5.49 GB, sync 1483 → 1568 s while index time stays flat at ~940 s), so expect further easing. **Revised ETA 2026-07-27/28**, ~1 day later than the earlier estimate. *Prior reading:* **2026-07-21 13:35 UTC: floor 27,583,999** (was 23,423,999 at 07-20 14:15 → **4.16M ledgers in 23h20m = 178.3k/hr**, within 1% of the documented 177k). Markers contiguous: 27,583,997 over `3 → 27,583,999`, no gaps. Remaining 22.87M ledgers ≈ **5.3 days → ETA 2026-07-26/27**. ⚠️ The 54.7% marker figure OVERSTATES real progress — see the durable-data row below. (Liveness: `pgrep -af sdex-backfill` + partition # climbing in `~/sdex-tail.log` over ≥25 min; do NOT sample CH markers <22 min apart — download-bound cadence freezes them transiently.) |
| `[1, 23423999]` pre-Soroban SDEX tail — **pass 2 (recovery)** | ⏳ **RUNNING — launched 2026-08-04 13:25:15 UTC** (fishuser-hero, tmux `sdex-pass2`, log `~/sdex-pass2.log`, `tip 63795749`, the same binary pass 1 used). Pre-flight passed; **`completed: 0`, `already_done: 0`, `to_process: 366`** — the marker clear worked and nothing is being skipped. 366 partitions × 64k = 23,424,000 ledgers; also covers the 64,000–2,815,999 span pass 1 never walked. ETA **~2026-08-09/10** at pass 1's ~23.5 min/partition. Recovers **3,738,473 candles**. Gates cleared before launch: cleanup `DISABLED`; no sweep-signature removal since 2026-07-20 03:08:30; all partitions below `202001` still hold rows; no backfill process running; `leftover_low_markers = 0` (total markers 50,457,421 → 39,928,612, which includes the Soroban-era run's 12,895,188 — the filtered pre-Soroban figure is 27,033,424). |
| **durable pre-Soroban data (re-measured 2026-08-04)** | Pre-2019, `price_ohlcv_1m` holds **only `201812`: 207,021 rows, `2018-12-13 06:55` → `2018-12-31 23:58`**. 2015–2018-11 are **absent** — `system.parts` has no partition below `201812` at all. ⚠️ **The `2018-12-13` boundary is an artifact of the incident, not a data-availability fact**: it is simply where the backfill had walked when cleanup was disabled on 2026-07-20 (`201812` first_seen `2026-07-20 04:11:24`, minutes after that day's final 03:08 sweep). Full forensics: [[S-presoroban-loss-chain-confirmed]]. *Supersedes the 2026-07-21 measurement, which read the boundary as a wipe line and reported spans that pass 1 has since rewritten.* |

> Update this table as chunks land (ledger range, candle counts, duration, any
> new unresolved pools). `GET /backfill/status` reports truthful monotonic
> progress for both streams.

## Recovery plan (pre-Soroban gap, from the 2026-07-20 cleanup incident)

**Invariant for the whole sequence: `prices-production-cleanup` stays DISABLED
until step 4.** `price_ohlcv_1m` is the *only* copy of the pre-Soroban tail until
the pre-roll lands it in the coarse forever-tables. Re-check before each step:

```bash
aws events describe-rule --name prices-production-cleanup --region eu-central-1 \
  --profile soroban-admin --query 'State'   # MUST stay "DISABLED"
```

> Profile: **`soroban-admin`** worked on 2026-08-04; the runbooks say
> `soroban-explorer`. Use whichever is live — a
> `UnrecognizedClientException: security token ... invalid` is an **expired SSO
> session**, not a wrong profile name, so re-login before concluding the profile
> is dead.
>
> ⚠️ **The rule's state is a necessary but not sufficient check.** It read
> "disabled" in prose through the 2026-07-15→20 window while the sweep ran every
> night. The authoritative signal is `RemovePart` activity on old partitions:
>
> ```sql
> SELECT partition_id, max(event_time) AS last_removal
> FROM system.part_log
> WHERE table = 'price_ohlcv_1m' AND event_type = 'RemovePart'
>   AND partition_id < '202000'
> GROUP BY partition_id ORDER BY partition_id;
> ```
>
> Any removal timestamped *after* pass 2 starts means cleanup is live again —
> stop the run.

| # | Step | Duration | Notes |
|---|------|----------|-------|
| 1 | ✅ **Let pass 1 finish** to activation (floor → 50,457,423) | done | **COMPLETE 2026-07-27 21:24 UTC.** |
| 2 | 🟢 **Clear markers, then re-run `[1, 23423999]`** (pass 2) | ~5 days | **Next action.** `DELETE FROM prices.backfill_sdex_ledgers WHERE sequence <= 23423999` **before** launching, else the resume set short-circuits done ledgers (`running-ingestion-components.md:122`) and the re-run silently skips the whole span, leaving an empty 2015–2018 forever. Verify `leftover_low_markers = 0` before launch. Use the existing binary at `~/stellar-prices-api/target/release/sdex-backfill` — a rebuild from newer code could change behaviour mid-recovery. |
| 3 | **Incremental pre-roll** — `preroll-incremental.sql` | hours | `docs/runbooks/preroll-incremental-presoroban.md`. NOT `preroll.sql` (full rebuild wipes the Soroban-era coarse). Pre-flight: floor ≈ 50,457,423, exact activation boundary timestamp, cleanup still DISABLED. ⚠️ **Do NOT gate this on the runbook's §7.1 marker query** — see below. |
| 4 | **Re-enable cleanup** — `aws events enable-rule` | — | Only after step 3 verifies coarse coverage back to genesis. |

> ⚠️ **The pre-roll gate in `backfill-handoff-covering-operator.md` §7.1 is
> marker-based and unsafe.** As of 2026-08-04 it prints `BACKFILL COMPLETE —
> READY TO PRE-ROLL` (`min 3`, `max 50,457,423`, contiguous) while **four years
> of candles are missing** — the markers survived every cleanup sweep, the
> candles did not. Gate step 3 on the per-year candle counts below instead, and
> treat `backfill_sdex_ledgers` as a resume aid only, never as evidence of data.
>
> ⚠️ **Do not re-run `DELETE WHERE intDiv(toUInt64(version), 1000) < 50457424`.**
> `version` = `ledger_seq × 1000 + intra-ledger order`, so it targets exactly the
> pre-Soroban rows and would erase pass 2's entire output in one statement. It
> last ran 2026-07-15 10:24 against an earlier run's data.

> **Step 3 pre-flight — USD coverage note (added 2026-07-21, see [[0114]]).**
> The pre-roll copies `price_ohlcv_1m` **as-is** and nothing re-enriches a coarse
> row afterwards, so the pre-Soroban tail's USD columns will be whatever 1m holds
> at that moment.
>
> **This is NOT a blocker.** An earlier version of this note called it one, on the
> grounds that 1m is 62% `close_usd = 0`. That premise was wrong: those zeros are
> exotic quotes with no USD reference, and the pre-Soroban era has no reference at
> all — 2018-2019 carries 100k–200k XLM-quoted rows/month at 100% zero because
> USDC barely existed then (6–45 rows/month) and `prices.oracle_prices` only
> starts 2025-09. **Enriching before pre-rolling would change nothing.** The
> pre-Soroban tail is USD-less as a data-availability fact.
>
> Run this as a cheap regression guard, not a gate — expect a high number and
> proceed:
> ```sql
> SELECT round(100 * countIf(close_usd = 0) / count(), 1) AS pct_zero
> FROM prices.price_ohlcv_1m FINAL WHERE timestamp < '2024-02-20';
> ```

**Why re-run the full `[1, 23423999]` rather than preserving the trusted
`201901`–`201904`:** `price_ohlcv_1m` has no ledger column, so mapping a month
boundary back to a ledger is awkward and error-prone; the saving is only ~2M
ledgers (~8%). `price_ohlcv_1m` is a `ReplacingMergeTree`, so rewriting identical
keys dedups harmlessly. Not worth a boundary-error risk to save ~half a day.
`201812` is *partial* (cleanup drops whole partitions, so its pre-fire portion is
gone) and must be inside the re-run range regardless.

**Verify after step 2** — expect a smooth ramp from 2015, no missing years:

```sql
SELECT toYear(timestamp) AS yr, count() AS candles
FROM prices.price_ohlcv_1m
WHERE source='sdex' AND timestamp < '2024-02-20'
GROUP BY yr ORDER BY yr;
```

**Disk:** cleanup stays off ~12 days on a **shared** cluster. Verified 2026-07-20:
564 GB avail on `/var/lib/docker` (gate ≥20 GB) — ample, no need for the bounded
per-chunk pre-roll variant. Give the cluster owner a heads-up for the window.

### Follow-up fixes this incident earns

- **Preflight guard in `sdex-backfill`** — refuse to start (or loudly warn) when
  `prices-production-cleanup` is ENABLED. The precondition currently lives only in
  runbook prose and demonstrably did not survive the gap between the 0090 rerun and
  the 2026-07-15 tail start. This is the fix that actually prevents recurrence.
  → **spawned as task 0109** (2026-07-20). Scoped so it cannot block the recovery
  run currently in flight.
- **`CH()` helper is broken** in `docs/runbooks/fix-backfill-history-loss-and-rerun.md:97` —
  it passes `$*`, which drops quoting, so the remote shell splits the query and
  every multi-line example in that runbook fails with
  `Bad arguments: the argument for option '--query' should follow immediately after
  the equal sign`. Replace with a stdin form:
  ```bash
  CHQ() { ssh -i ~/.ssh/sorban-prod_ed25519 deploy@168.119.73.161 \
    "docker exec -i app-clickhouse-1 clickhouse-client"; }
  CHQ <<'SQL'
  SELECT 1
  SQL
  ```
- **Marker-based gates must go.** `backfill-handoff-covering-operator.md` §7.1
  gates the pre-roll on `backfill_sdex_ledgers` contiguity, which survived every
  cleanup sweep — it currently clears a four-year hole for pre-rolling. Any
  "is the data there?" gate must count candles, not markers. Fold into [[0109]]
  or spawn separately.
- **Nothing writes a terminal state on crash** — the dead `soroban_amm` leg has
  read `status: running` since 2026-07-14. Same class of lying health signal as
  the sweep that reported nothing and the [[0136]] freeze.
- **Straddled activation minute.** Pass 1 logged a WARN: the activation split is
  not minute-aligned, so minute `1708448400` is written partially by both range
  runs and, since RMT replaces rather than sums, **undercounts**. Reconcile it
  from one pass after both runs, or accept it as a documented artifact — but
  decide explicitly.
- Per §Out of scope these are code/doc changes, not operational — spawn them as
  their own task(s) rather than doing them under this tracker.

## Acceptance Criteria

- [x] Combined range `[activation, 63352611]` fully written to Hetzner. **Done
      2026-07-15** (via the full-backfill that ran 07-08→07-14, Phase 1 complete;
      verified in 0090: `price_ohlcv_1m` contiguous `2024-02-20 → 2026-07-08`, 532M
      rows, gap `62,642,957→63,352,611` filled). Pre-rolled to the coarse tables in 0090.
- [ ] Pre-Soroban SDEX tail `[1, activation−1]` — ~~WAIVED 2026-07-15~~ **UN-WAIVED
      (waiver stale).** The 2026-07-15 waiver assumed the deep tail had been killed;
      it is in fact live and 46.4% done (fishuser-hero, since 2026-07-15). Closing
      this AC now needs **both** passes: pass 1 → activation (~6.3 days) **and**
      pass 2 re-walking `[1, 23423999]` (~5 days), which the enabled cleanup rule
      destroyed. Verify with the year-histogram in §Recovery plan — a smooth ramp
      from 2015, no absent years. (Historical note: 0092 resolved "BE needs
      Soroban-era only", so this is coverage-completeness, not a BE blocker.)
- [ ] 🔴 **`prices-production-cleanup` stays DISABLED for the entire recovery** — from
      2026-07-20 until **after** `preroll-incremental.sql` has landed the pre-Soroban
      `1m` in the coarse forever-tables (~12 days: pass 1 ~6.3d + pass 2 ~5d + pre-roll).
      **Enabling it early is destructive, not merely a paused retention policy:** it
      deletes the run's output as fast as the backfill writes it and re-creates the exact
      history gap this task exists to close — unrecoverable except by re-downloading the
      span. `price_ohlcv_1m` is the ONLY copy until the pre-roll runs. Re-check before
      each recovery step:
      ```bash
      aws events describe-rule --name prices-production-cleanup --region eu-central-1 \
        --profile soroban-explorer --query 'State'   # MUST be "DISABLED"
      ```
      If something appears to require enabling it (e.g. disk pressure — 564 GB was free
      on 2026-07-20, so there is no headroom argument), **stop and escalate to the
      cluster owner** rather than flipping the rule. Close this AC only once cleanup has
      been deliberately re-enabled as the final recovery step, with coarse coverage back
      to genesis verified first. Memory: [[cleanup-rule-shreds-backfill-output]].
- [ ] **Pre-Soroban `1m` rolled up before cleanup returns** — `preroll-incremental.sql`
      (NOT `preroll.sql`) run per `docs/runbooks/preroll-incremental-presoroban.md`,
      coarse tables verified to cover genesis → activation, and only then
      `aws events enable-rule`. This is recovery step 3→4 in §Recovery plan.
- [~] `GET /backfill/status` monotonic; `soroban_amm`→`completed` (reached the floor
      63352611), `sdex_archive` tail run killed (not needed). Re-confirm the status
      endpoint reflects the final state (`status='paused'`); minor remaining check.
- [ ] **OHLCV for Soroswap pairs verifiable** — **re-run owned HERE** (root cause
      confirmed in 0096). Cause = **registry seed-timing**, not a missing preload:
      the backfill already preloads `prices.pool_registry` (since 0053), but the 221
      soroswap rows were seeded `2026-07-14`, AFTER the Soroban run, so `reg.soroswap`
      was empty at run time → 0 soroswap candles. 0096 shipped the code fix (closed a
      dispatch silent-drop; unresolvable pools now land in `unresolved_pools`). The
      registry is now seeded, so satisfying this AC = a **bounded combined-mode re-run
      over the Soroswap-affected range** (0090 runbook: disable cleanup → backfill →
      pre-roll → re-enable), then verify non-zero `soroswap` candles per-source.
- [ ] Docker-gated CH integration tests greened once locally against prod-pinned
      ClickHouse (`candles_it`, `pool_registry_it`, `progress_it`). *(0053 left
      these to run once against a local CH.)*

## Out of scope

- Any code changes to the backfill engine, sink, or progress writer — all landed
  in 0053 (PR #72) and dependents (#92). If a bug surfaces during the run, spawn
  a fix task; this tracker stays operational.
- Live-ingestion / periodic-worker verification — that is task **0082**.

## Notes

- Standing rules keep this operator-run against real infra (archive fetch +
  Hetzner direct-write); not executed autonomously
  ([[feedback-prepare-not-deploy]], [[feedback-local-only-no-prod-data]] —
  note the read-only public-ledger fetch exception).
- Measured throughput + EC2 rationale: [[local-backfill-throughput-measured]].
- Prod CH access for progress checks (read-only SQL via `docker exec`):
  [[hetzner-ch-prod-ssh-access]].
- **End-of-run pre-roll (prepared, DO NOT FORGET):** when the pre-Soroban SDEX
  tail finishes, roll its `[genesis, activation)` `1m` up to the coarse tables
  with the **incremental, non-truncating** pre-roll —
  `packages/prices-clickhouse/schema/preroll-incremental.sql` — NOT `preroll.sql`
  (a full rebuild would wipe the already-pre-rolled Soroban-era coarse). Appends
  only; safe because RMT(version) lets the higher-version Soroban rows win at the
  activation boundary. Verified locally (prod-pinned CH): append at all six
  granularities, boundary preserved, idempotent. Procedure + pre-flight + the
  cleanup re-enable step: [`docs/runbooks/preroll-incremental-presoroban.md`](../../../docs/runbooks/preroll-incremental-presoroban.md).
  Cleanup rule `prices-production-cleanup` MUST stay disabled until after this runs.
