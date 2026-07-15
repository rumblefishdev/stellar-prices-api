---
id: "0092"
title: "Decide + (optionally) run the deep pre-Soroban SDEX tail backfill (2015→2024)"
type: FEATURE
status: done
related_adr: []
related_tasks: ["0090", "0088", "0061"]
tags: ["backfill", "clickhouse", "priority-medium", "effort-large", "decision", "phase-future"]
links:
  - "../../../docs/runbooks/continue-soroban-backfill.md"
  - "../../../docs/runbooks/backfill-sdex.md"
history:
  - date: 2026-07-14
    status: backlog
    who: okarcz
    note: >
      Spawned from the 0090 re-scope. The pre-Soroban SDEX tail (ledgers
      1→50,457,423, ~2015→2024-02) is 0% done and is the only expensive piece of
      the historical program (~50M ledgers, multi-week download). Split out as a
      product DECISION rather than an automatic backfill step.
  - date: 2026-07-15
    status: done
    who: okarcz
    note: >
      DECISION MADE: NO — BE (0199 consumer) needs Soroban-era ledgers only, not
      pre-2024 history. Confirmed with the owner during the 0090 execution. The
      deep tail that an unnoticed run-full-backfill.sh was mid-download of was
      KILLED and its 28-row 2015-2016 fragment deleted from price_ohlcv_1m. No
      download to run → task resolves as "not needed". Archived.
---

# Decide + (optionally) run the deep pre-Soroban SDEX tail backfill (2015→2024)

## Summary

The pre-Soroban SDEX historical tail — ledgers **1 → 50,457,423** (~2015 → 2024-02)
— is **0% backfilled** and is the single expensive piece of the historical program
(~50M ledgers, multi-week download). This task is a **decision first**: does the BE
consumer actually need pre-2024 price history? Only run the download if yes.

## Context

Spawned from the 0090 re-scope. After measuring surviving data, everything from
2024-02 forward is covered cheaply (pre-roll existing 1m + gap-fill the June-2026
hole). The deep tail is the only part needing a large fresh download, so it must not
be an automatic step — it's a cost/value call.

- Current `price_ohlcv_1m` oldest = 2024-02-20; nothing before.
- Tail is **SDEX-only** (no Soroban AMM before activation) → `--mode sdex-only`.
- Best run from a **us-east-2 EC2** (co-located with the archive bucket); home-line
  pace is weeks. See `continue-soroban-backfill.md` §8 / `backfill-sdex.md`.

## Decision to make

- Does the BE prices consumer (**task 0199**; see memory `be-lp-analytics-prices-contract`)
  need OHLC history before 2024-02? If not → close as "won't do (not needed)".
- If yes → how far back (full 2015, or a bounded window)? Cost scales with range.

## Implementation (only if approved)

- `sdex-backfill --mode sdex-only --start 1 --end 50457423` (chunked, resumable),
  from a us-east-2 EC2, writing to prod CH over mTLS.
- Pre-roll the newly-written `1m` range into the coarse tables (per 0090 Phase 1).
- Verify coarse `1h`/`1d` extend back to the chosen start.
- Note: `stellar-xdr 26` decodes this entire (pre-proto27) range fine — no 0091 dep.

## Acceptance Criteria

- [ ] Decision recorded (need pre-2024 history? how far back?) with the BE consumer.
- [ ] If no: closed as won't-do with rationale.
- [ ] If yes: tail backfilled to the agreed depth + pre-rolled + verified.
