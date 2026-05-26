---
id: '0035'
title: 'Periodically re-survey Phoenix factory; catch first stable pool when deployed'
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ['0032', '0018']
tags:
  [layer-research, priority-low, effort-small, phoenix, stable-pool, monitoring]
links:
  - '../archive/0032_RESEARCH_phoenix-stable-pool-first-observation/notes/S-no-stable-pool-deployed.md'
history:
  - date: 2026-05-15
    status: backlog
    who: oski
    note: 'Spawned from 0032 — replaces 0032 as the ongoing concern.'
---

# Periodic Phoenix factory re-survey

## Summary

Task 0032 confirmed no stable pool is deployed on Phoenix mainnet as
of 2026-05-15. This task is the ongoing-monitoring follow-up: re-run
the same survey on a cadence so the moment a stable pool is added it
gets captured and the consumer's stable-pool decoder can be validated
against real data.

## Context

The survey is fast and idempotent (one `query_pools` call + one WASM
fetch per new pool). It is the cheapest possible "watcher" for the
stable-pool event. Until a pool with `Config.pool_type != 0` (or a
new WASM hash that isn't one of the two known XYK hashes) appears, no
follow-up action is required.

## Implementation

Two options, smallest first:

1. **Manual rerun** — re-execute the survey procedure documented in
   [0032 S-note §Method](../archive/0032_RESEARCH_phoenix-stable-pool-first-observation/notes/S-no-stable-pool-deployed.md)
   monthly (e.g. via a scheduled lore-task or calendar reminder).
   Append a new dated evidence file to the archived 0032 directory
   when changes are observed; otherwise no action.
2. **Lightweight cron/Lambda** — write a small script (Rust binary or
   shell) that runs `query_pools`, fetches each new pool's WASM hash,
   and emits an alert (Slack/log line) only when a new pool is
   detected. Persist the previous-survey hash set in a small object
   (S3 or DynamoDB) to detect deltas.

Prefer option 1 until a stable-pool decoder ships and starts running
in production; switch to option 2 if Phoenix is actively shipping new
pool types.

## Acceptance Criteria

- [ ] Decision recorded on which option (manual vs cron) is used.
- [ ] If cron: deployed and verified to detect a synthetic new-pool
      delta.
- [ ] When a stable pool is detected, open a new task to run
      `dump-swap-events` against it and complete the original 0032
      goal (decode and archive a 6-event swap grouping).
