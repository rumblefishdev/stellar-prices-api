---
id: "0175"
title: "Bounded combined-mode re-run to recover Soroswap OHLCV — the registry was seeded AFTER the Soroban backfill, so soroswap candles are all missing"
type: FEATURE
status: backlog
related_adr: ["0005"]
related_tasks: ["0088", "0096", "0053", "0090"]
tags: [layer-infra, priority-medium, effort-large, backfill, soroswap, soroban, operational]
links:
  - "../../../docs/runbooks/fix-backfill-history-loss-and-rerun.md"
  - "../../../docs/runbooks/preroll-incremental-presoroban.md"
history:
  - date: 2026-08-11
    status: backlog
    who: okarcz
    note: >
      Split out of 0088. It had been carried as an AC on that tracker
      ("re-run owned HERE"), but 0088 is the PRE-SORORBAN recovery and that work
      completed 2026-08-11 - pass 2 plus the pre-roll. This is a different range,
      a different root cause and a fresh multi-day operational campaign, so it
      was blocking 0088 from closing while sharing none of its subject matter.
      Root cause is already settled in 0096; only the operational re-run remains.
---

# Recover Soroswap OHLCV — bounded combined-mode re-run

## Summary

Soroswap pairs have **no OHLCV candles** from the historical Soroban backfill.
The code was never the problem: the backfill has preloaded `prices.pool_registry`
since 0053, but the **221 soroswap rows were seeded `2026-07-14` — after the
Soroban run had already executed.** `reg.soroswap` was therefore empty at run
time and every soroswap swap was dropped, yielding 0 candles.

The registry is now seeded and 0096 shipped the code fix (it closed a dispatch
silent-drop; unresolvable pools now land in `unresolved_pools` instead of
vanishing). So what remains is purely **operational**: a bounded combined-mode
re-run over the Soroswap-affected range, a pre-roll, and verification.

## Context

- **Root cause: seed timing, not a missing preload.** Confirmed in [[0096]].
  Do not re-investigate the code path — it is correct as of 0096.
- ⚠️ **A related figure in circulation is wrong.** The "824k swaps" number from
  the 0096 investigation is incorrect; the measured truth is **536,319**. See
  [[task-0096-soroswap-root-cause]].
- Also from 0096: the Soroswap swap action lives in `topic[1]`, **not**
  `topic[0]` — that dispatch bug is fixed, but it is the reason the count was
  zero rather than merely low.

## Implementation

Follow the 0090 rerun runbook shape — but note the **cleanup state has changed**
since it was written (see Constraints).

1. **Scope the range.** Determine the ledger span where soroswap pools were
   active but unregistered — bounded by the Soroswap factory's first pool and the
   `2026-07-14` registry seed. Do **not** re-run the whole Soroban era.
2. **Confirm the registry is populated** before starting — the entire failure was
   an empty `reg.soroswap` at run time, so assert non-zero rows in
   `prices.pool_registry` for soroswap first. This is the one pre-flight that
   would have prevented the original loss.
3. **Run the backfill** in combined mode over that range.
4. **Pre-roll** the affected span into the coarse tables.
5. **Verify** non-zero `soroswap` candles per source, and that
   `unresolved_pools` is not absorbing them silently.

## Constraints

- ⚠️ **`prices-production-cleanup` is currently DISABLED and must stay that way
  for the duration**, exactly as in 0088 — enabling it mid-run deletes output as
  fast as it is written (cost 5 days on 2026-07-20,
  [[cleanup-rule-shreds-backfill-output]]). It is *already* off as of
  2026-08-11 by operator decision, so unlike previous runs there is no
  "disable it first" step — but **confirm, do not assume**.
- ⚠️ **The pre-roll is `preroll-incremental.sql`, never `preroll.sql`.** And read
  `docs/runbooks/preroll-incremental-presoroban.md` first: three of its steps
  were corrected on 2026-08-11, including a boundary query that returns a
  retention horizon rather than activation.
- **Disk is tighter than it was.** 0088 left 718.6M pre-Soroban `1m` candles
  resident because cleanup was not re-enabled. 450 GB free / 74% used as of
  2026-08-11. Re-check before starting a run that writes more `1m`.
- Standing rules apply: operator-run against real infra, not executed
  autonomously ([[feedback-prepare-not-deploy]],
  [[feedback-local-only-no-prod-data]]).

## Acceptance Criteria

- [ ] The affected ledger range is **derived and written down**, not guessed —
      with the reasoning, so a re-run can be scoped the same way later.
- [ ] `prices.pool_registry` confirmed non-empty for soroswap **before** the run
      starts. This single check is what the original failure lacked.
- [ ] Backfill re-run completes over that range.
- [ ] Output pre-rolled into the coarse tables via `preroll-incremental.sql`.
- [ ] **Non-zero `soroswap` candles verified per source**, at more than one
      granularity — the original defect produced a clean zero, so a clean
      non-zero at every level is the signal.
- [ ] `unresolved_pools` checked — if pools are landing there instead of
      producing candles, the run has a different problem and the count would
      otherwise look like partial success.
- [ ] Cleanup state on completion recorded explicitly, whichever way it is left.

## Notes

- ⚠️ **Markers are not evidence of data.** They survive cleanup; candles do not.
  Any completeness gate here must count candles, not
  `backfill_sdex_ledgers` rows — the lesson that cost 0088 five days
  ([[presoroban-loss-chain-and-sweep-signature]]).
- Related unresolved gap: [[amm-live-pool-registry-preload-gap]] — the *live*
  processor passes an empty `Registries::new()`, so live AMM prices have the same
  class of problem independent of this historical re-run. Not in scope here, but
  fixing this without that leaves a forward-going hole.
