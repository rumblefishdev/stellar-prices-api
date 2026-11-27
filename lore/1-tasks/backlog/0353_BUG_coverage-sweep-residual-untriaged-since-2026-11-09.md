---
id: "0353"
title: "coverage-sweep-unclassified has been in ALARM since 2026-11-09 — 16 swap emitters (2,091 events) untriaged and growing"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0323", "0100", "0290", "0336", "0214"]
tags: [layer-backend, priority-medium, effort-medium, observability, coverage, soroban, alarms]
links:
  - "../../../docs/runbooks/0100-coverage-sweep-triage.md"
  - "../../../packages/coverage-sweep-probe/allowlist.toml"
history:
  - date: 2026-11-24
    status: backlog
    who: stkrolikiewicz
    note: >
      Created from the 2026-11-24 board review. 0323's latch works as designed
      (OK on 2026-11-02, ALARM on the next non-zero run), but no one has
      triaged the residual for three Monday runs. The runbook's owner is
      still TBD.
---

# coverage-sweep-unclassified: residual untriaged since 2026-11-09

## Summary

`prices-production-coverage-sweep-unclassified` went to ALARM on
2026-11-09 06:18 CET and has stayed there. Each Monday run finds more swap
emitters that are neither indexed nor allow-listed. Triage them per runbook
0100 §3 until a run reports `unclassified=0` and the alarm returns to OK.

## Evidence (CloudWatch, 2026-11-24)

| Run (UTC) | unclassified | events | allowlisted |
|---|---|---|---|
| 2026-11-09 05:17 | 6 | 1,019 | 15 |
| 2026-11-16 05:17 | 13 | 1,432 | 17 |
| 2026-11-23 05:17 | 16 | 2,091 | 17 |

Emitters from the 2026-11-23 run (`unclassified swap emitter` WARN lines):

| Contract | wasm | events | top_action | top_shape |
|---|---|---|---|---|
| `CDJJA3CJ…H4XZ` | `030cad40` | 1212 | trade / CCCWWXJ5… | sym\|address\|address -> vec |
| `CCVENFSV…F3TR` | `39cf2442` | 231 | router / swap | sym\|sym -> map |
| `CDQ4LAKS…KLT7` | `6692f39b` | 170 | trade / CC5UXGDQ… | sym\|address\|address -> map |
| `CDZBNWTP…M4OJ` | `c6c69bae` | 142 | swap_settled / GBSBQGBN… | sym\|address\|address -> map |
| `CDLXYTF3…ZH5T` | `2025999a` | 126 | trade / CA35B5JS… | sym\|address\|address -> vec |
| `CDEISYZM…NBWU` | `7dea47b2` | 114 | swap / GDZACFEQ… | sym\|address\|address -> map |
| `CDG3X6GS…SMLT` | `d4affd49` | 28 | swap_executed | sym -> vec |
| `CBDQRFSK…AUZX` | `d4affd49` | 23 | swap_executed | sym -> vec |
| `CBOU2RGF…WB6P` | `ac14d1e4` | 16 | swap_executed / CBFY4Q26… | sym\|address\|address\|address -> map |
| `CCA56KMC…TEB4` | `f550cc18` | 13 | swap_exact / CBAEVZTB… | sym\|address\|address\|vec -> map |
| `CDEXKHTL…AXIS` | `d6cfa312` | 4 | trade / CBLLEW7H… | sym\|address\|address -> vec |
| `CBAM736S…DLTG` | `fa3f548b` | 4 | swap / 0 | sym\|u32\|address -> map |
| `CA3ROUTE…FRTR` | `4591d0f3` | 3 | swap_executed / GA3ROUTE… | sym\|address\|address\|address -> map |
| `CBFKHCZD…LKI6` | `d735c339` | 2 | POOL / swap | sym\|sym -> map |
| `CBGQHIZU…AKJJ` | `d735c339` | 2 | POOL / swap | sym\|sym -> map |
| `CCJQWK4R…2WP6` | `b0266a69` | 1 | swapped | sym -> map |

Full addresses come from the runbook §3 Logs Insights query. The run's
`unmatched_allowlist` still lists stale entries (for example
`CDYPJTUT…` from 2026-09-28), which are candidates for pruning.

## Plan

1. Classify each emitter as (a) a pool of an indexed venue, (b) an unindexed
   venue, or (c) a non-AMM contract (runbook §3). Start with `CDJJA3CJ…`,
   which carries 58 % of the events.
2. (a): register the pool and reprice its range. (b): open a task per venue
   and add a `[[wasm]]` entry with `until`. (c): add `[[contract]]` entries,
   or a `not_a_venue` `[[wasm]]` entry when one wasm keeps spawning instances.
3. Prune stale `unmatched_allowlist` entries.
4. Merge the allow-list PR, deploy the probe, invoke it once.

## Acceptance Criteria

- [ ] Every emitter above has an outcome (a), (b) or (c) recorded here
- [ ] Allow-list PR merged; any (b) venue has its own task
- [ ] Production invoke: `coverage sweep complete` with `unclassified=0`,
      alarm OK
- [ ] Runbook §1 owner agreed with the team and written in (the TBD is why
      this went unnoticed for three runs)
