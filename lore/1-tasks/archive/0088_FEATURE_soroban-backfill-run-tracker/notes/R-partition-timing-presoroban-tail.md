---
title: "Partition-timing log — SDEX pre-Soroban tail (pass 1), 2026-07-22"
type: research
status: mature
spawned_from: ../README.md
spawns: []
tags: [backfill, throughput, sdex, pre-soroban, measurement]
links: []
history:
  - date: 2026-07-22
    status: mature
    who: okarcz
    note: "Captured 8 consecutive partition cycles from the sdex-tail tmux log; derived cadence + ETA"
---

# Partition-timing log — SDEX pre-Soroban tail (pass 1), 2026-07-22

## Context

Direct measurement of pass-1 throughput for the SDEX pre-Soroban tail backfill
(`sdex-backfill --mode sdex-only --start 1 --end 50457423 --tip 63490231
--transport hetzner`), running on **fishuser-hero**, tmux session `sdex-tail`.

Captured to settle the throughput/ETA question **without** relying on CH marker
polling — the marker query only jumps once per completed 64k partition (~25 min),
so any two samples <22 min apart read identical (see the liveness gotcha in the
`sdex-backfill-presoroban-gap-0088` resume note). The tmux log gives per-partition
wall times directly, which is the accurate signal.

State at capture: frontier (max marked `sequence < 50457424`) = **30,655,999**,
i.e. **~61% of the 50.46M tail done**. Pass 1 walks **UP from genesis**, so the
advancing max is the progress signal; `min(sequence)` is pinned at 3.

## Raw log — 8 consecutive partition cycles

Each partition = 64,000 ledgers. `sync` = `aws s3 sync` download; `index` =
parse + write candles to Hetzner. The two pipeline (next download runs during
current index), so end-to-end cadence = the slower leg (download).

| partition (first) | `indexing complete` (UTC) | index wall_secs | sync_ms → min | bytes | trade_ticks | candles |
|---|---|---|---|---|---|---|
| 30208000 | 05:02:34 | 881.0 | 1,349,645 → 22.5 | 2.08 GB | 115,373 | 55,693 |
| 30272000 | 05:27:11 | 871.4 | 1,485,795 → 24.8 | 2.09 GB | 105,385 | 47,698 |
| 30336000 | 05:51:05 | 853.6 | 1,451,128 → 24.2 | 1.91 GB | 93,047 | 44,652 |
| 30400000 | 06:15:34 | 864.1 | 1,457,570 → 24.3 | 2.30 GB | 72,908 | 26,558 |
| 30464000 | 06:40:45 | 887.7 | 1,487,185 → 24.8 | 3.17 GB | 126,544 | 32,641 |
| 30528000 | 07:06:26 | 936.8 | 1,490,370 → 24.8 | 3.28 GB | 135,896 | 36,699 |
| 30592000 | 07:30:39 | 900.8 | 1,488,303 → 24.8 | 2.84 GB | 121,120 | 38,191 |
| 30656000 | (started 07:40:01) | — | 1,462,861 → 24.4 | 2.45 GB | — | — |

`amm_ticks: 0` and `oracle_rows: 0` for every partition — correct for the
pre-Soroban era (no AMM pools, no oracle yet).

## Derived cadence

End-to-end, `indexing complete` → next `indexing complete`:

| interval | Δ |
|---|---|
| 05:02:34 → 05:27:11 | 24m37s |
| 05:27:11 → 05:51:05 | 23m54s |
| 05:51:05 → 06:15:34 | 24m29s |
| 06:15:34 → 06:40:45 | 25m11s |
| 06:40:45 → 07:06:26 | 25m41s |
| 07:06:26 → 07:30:39 | 24m13s |

**Mean ≈ 24.7 min per 64k partition → ~155k ledgers/hr** (measured window).
Long-run rate since the 07-21 13:35 checkpoint (27,583,999 → 30,655,999 over
17h55m) = **~171k/hr**; the recent window is slightly slower.

## Bottleneck

**Download-bound.** Sync ≈ 24.5 min/partition vs index ≈ 15 min (wall_secs
850–940). Because they pipeline, cadence tracks the download leg, not the index.
Confirms the "download-bound, ~64 min/100k on a home line" note in the runbook
(`docs/runbooks/continue-soroban-backfill.md`) and
`[[local-backfill-throughput-measured]]`.

**Lever:** moving the run to a **us-east-2 EC2** (same region as the ledger
bucket) collapses the sync to ~1–2 min, making it index-bound (~15 min/partition,
~1.6× faster). Not required; the current pace still hits the target ETA.

## ETA

- Remaining at capture: `50,457,423 − 30,655,999 = 19,801,424` ledgers ≈ **309
  partitions**.
- At 24.7 min/partition → **~127 h ≈ 5.3 d → pass 1 ETA ~2026-07-27 midday UTC**.
- Consistent with the earlier "pass 1 ETA 07-26/27" estimate.
- Real remaining ≈ **10.8 d** total: pass 1 (~5.3 d) + pass 2 re-walk (~5.5 d).

## After pass 1 completes — required order

`write 1m → pre-roll (preroll.sql, §9) → verify coarse → re-enable cleanup (§10)`.
Cleanup rule (`prices-production-cleanup`) MUST stay DISABLED until the pre-roll
is verified, or the next 02:00 UTC tick drops the un-rolled `1m` history — the
0090 data-loss bug. See `[[cleanup-rule-shreds-backfill-output]]`.
