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
| `[51008000, 63352611]` combined remainder | ⏳ ~12.3M ledgers ≈ ~2.3 TB / ~5.6 days continuous. |
| `[1, 50457423]` pre-Soroban SDEX tail | ⏳ **RUNNING (fishuser-hero, healthy)** — floor at ~5.76M and climbing (2026-07-16 09:50 UTC), ~170k ledgers/hr, download-bound (~22.7 min/partition); ~45M ledgers ≈ ~11 days continuous to activation. 2017–2023 candles arrive as it climbs. Un-waived → reconcile the stale 2026-07-15 WAIVER AC. (Liveness: `pgrep` + partition # climbing over ≥25 min; do NOT sample CH markers <22 min apart — download-bound cadence freezes them transiently.) |

> Update this table as chunks land (ledger range, candle counts, duration, any
> new unresolved pools). `GET /backfill/status` reports truthful monotonic
> progress for both streams.

## Acceptance Criteria

- [x] Combined range `[activation, 63352611]` fully written to Hetzner. **Done
      2026-07-15** (via the full-backfill that ran 07-08→07-14, Phase 1 complete;
      verified in 0090: `price_ohlcv_1m` contiguous `2024-02-20 → 2026-07-08`, 532M
      rows, gap `62,642,957→63,352,611` filled). Pre-rolled to the coarse tables in 0090.
- [~] Pre-Soroban SDEX tail `[1, activation−1]` — **WAIVED 2026-07-15.** BE (0199)
      needs Soroban-era ledgers only; the deep tail was killed and the decision task
      **0092 resolved "not needed" + archived**. No union-to-1 coverage required.
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
