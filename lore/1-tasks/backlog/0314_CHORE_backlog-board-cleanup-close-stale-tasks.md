---
id: "0314"
title: "Backlog board cleanup — close the 13 tasks that are already done, superseded or decided against"
type: CHORE
status: backlog
related_adr: []
related_tasks: ["0062", "0113", "0115", "0129", "0130", "0169", "0175", "0201", "0205", "0209", "0212", "0266", "0289", "0154", "0139", "0286"]
tags: [layer-docs, priority-medium, effort-small, lore, board]
links: []
history:
  - date: "2026-09-24"
    status: backlog
    who: okarcz
    note: >
      Created after a read-only triage of all 80 backlog tasks on 2026-09-24
      sorted them into stale (13), quick-check (13) and keep (54). This task
      closes the 13 stale ones in one PR.
---

# Backlog board cleanup — close the stale tasks

## Summary

The backlog holds 80 tasks, and the board is hard to read. A triage on
2026-09-24 checked every one against the code, git history, archived tasks and
the active 0286 task. Thirteen can be closed now: their work was done elsewhere,
their premise is gone, or the decision went the other way. This task archives
them as `completed`, each with a history entry naming the evidence.

## Context

The same triage sorted the rest into **quick-check** (13 — one cheap check each
before they can close) and **keep** (54 — real work left). Those are out of
scope here; only the stale group moves.

## Tasks closed

| ID | Why it can close |
|----|------------------|
| 0062 | [[0111]] cut `count_candidates` from 9.27 s / 735 M rows to 0.08 s / 4.27 M rows — the cost this task was about |
| 0113 | Written for [[0088]] step 3 (archived); `preroll-incremental.sql` is historical and [[0302]] owns the full-range cap |
| 0115 | The same exotic-quote pivot as [[0154]]; its depeg and two-hop criteria are carried there |
| 0129 | Same root cause (asset_id allocator collisions) as [[0139]], which owns the fix |
| 0130 | [[0218]] moved the coarse sweep to its own Lambda, 15m in CDK; `tables_swept=6 tables_failed=0` on 2026-08-24 |
| 0169 | `deploy-board.yml` on the default branch has `schedule` + `workflow_dispatch` (#308); scheduled runs fire, last 2026-09-24 09:59Z |
| 0175 | Soroswap history completed by [[0097]]; the 07-06→07-11 hole is [[0101]]'s, and [[0286]] phase 3 re-ingests the Soroban era |
| 0201 | Its own 2026-08-18 history entry: 53 965 024 rows recovered in [[0182]]'s pass 1 |
| 0205 | Archived [[0309]]: the 2026-09-23/24 `cdk diff`s show no change to `/api/{proxy+}` — the greedy proxy is live |
| 0209 | Root cause fixed by [[0215]] ("closes the substance of 0209 and 0212"); the USDT pivot has written since |
| 0212 | Moved into [[0286]] (`4c7709e3`); its `peg_written = 0` check is a phase-3 acceptance criterion there |
| 0266 | Shared factor was a dust close on XLM/USDC; fixed by [[0286]] / ADR 0287, verified by its "seven dust days" criterion |
| 0289 | Decided against on 2026-07-14 (PR #104, task 0091): `xdr-parser` stays on `branch = "develop"` deliberately |

## Acceptance Criteria

- [ ] All 13 tasks archived with `status: completed` and a history entry citing the evidence
- [ ] 0115's open criteria noted on [[0154]] before 0115 closes; 0129 pointed at [[0139]]
- [ ] Merged to `develop`, so the board drops them
