---
id: "0131"
title: "0088 step-3 pre-roll: gate/warn when 1m USD coverage for the span is below a threshold"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0114", "0088", "0144", "0147", "0145"]
tags: [clickhouse, enrichment, backfill, pre-roll, guard, priority-low, effort-small]
links:
  - "../../../packages/prices-clickhouse/schema/preroll-incremental.sql"
  - "../../../docs/runbooks/continue-soroban-backfill.md"
history:
  - date: 2026-07-24
    status: backlog
    who: okarcz
    note: "Spawned from 0114 future work — the last 0114 AC; belongs in 0088's step-3 pre-roll flow, which hasn't run yet."
---

# 0088 step-3 pre-roll: gate/warn when 1m USD coverage is below a threshold

## Summary

Add a cheap **coverage pre-flight** to the [[0088]] recovery **step-3** pre-roll
(`preroll-incremental.sql` over the pre-Soroban tail): before rolling `1m` up into
the coarse forever-tables, check what fraction of `close_usd` is non-zero for the
target span, and **warn loudly (or refuse)** if it is below a threshold — so a
pre-roll can't silently bake an un-enriched (zero-USD) column into the forever
tables.

## Context

The last open acceptance criterion of [[0114]], deferred here so 0114 could close
(its core defect is fixed, deployed, and verified). 0114's own analysis downgraded
this from a blocker to *"a cheap regression guard… not a blocker, does not gate the
recovery"* — the pre-Soroban tail is legitimately USD-less (2018–2019 has almost no
USDC and `oracle_prices` starts 2025-09), so the gate should **warn, not hard-fail**
by default. It belongs in 0088's flow, which runs ~2026-08-01, and can only be
exercised once that pre-roll runs — which is why it doesn't fit as a 0114 blocker.

## Implementation

- A coverage `SELECT` over the pre-roll's target span on `price_ohlcv_1m FINAL`
  (`countIf(close_usd > 0) / count()` for `volume_quote > 0`, ideally split by
  quote class so the exotic floor doesn't mask a real regression).
- Wire it into the step-3 pre-roll as a pre-flight: emit a loud warning below a
  threshold; make hard-refuse opt-in (the pre-Soroban tail is expected-low).
- Document it in `continue-soroban-backfill.md §step 3` as a checklist item.

## Acceptance Criteria

- [ ] Step-3 pre-roll runs a 1m USD-coverage check over its target span before
      writing the coarse tables.
- [ ] Below-threshold coverage **warns loudly** (hard-refuse opt-in), and the
      expected-low pre-Soroban tail does not spuriously block the recovery.
- [ ] Documented in the 0088 pre-roll runbook.
