---
id: "0133"
title: "Guardrail: egress / write-volume alarm on the live pipeline so amplification shows on a dashboard, not a bill"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0132", "0039", "0056"]
tags: [observability, cost, clickhouse, egress, perf, priority-medium, effort-small, phase-future]
links: []
history:
  - date: 2026-07-29
    status: backlog
    who: okarcz
    note: >
      Spawned from 0132 future work (Step 3). 0132 was found by the BE team via
      part_log + Cost Explorer, not by our own monitoring — the ~9,413× asset
      re-emit ran for weeks billing ~$337/mo of AWS→Hetzner egress with nothing
      watching. This adds the missing meter so the next amplification surfaces on
      a dashboard/alarm instead of a surprise invoice.
---

# Egress / write-volume alarm on the live pipeline

## Summary

Task 0132 (live processor re-emitting the whole asset registry every reconcile,
9,413× amplification, ~$337/mo egress) went undetected because nothing watched
write volume or Lambda egress — the BE team found it in `system.part_log` and AWS
Cost Explorer. This task adds a guardrail so a future amplification is caught by an
alarm, not a bill.

## Context

- The cost is invisible to functional tests (output is correct) and only appears in
  the split Lambda→Hetzner topology. The right detection layer is ops metrics, not
  unit tests — see 0132 rationale.
- Alarm plumbing already exists (task 0056 → Slack `#stellar-prices-api-bot`); this
  should reuse it.

## Implementation (sketch — refine when picked up)

- Pick the signal(s):
  - **Lambda egress** — CloudWatch `AWS/Lambda` DataTransfer / a CloudWatch alarm on
    the `EUC1-DataTransfer-Out-Bytes` usage, threshold well above the legitimate
    candle stream (~3 GB/day) but below a runaway (e.g. alert > ~20 GB/day).
  - **CH write volume** — a scheduled probe over `system.part_log`
    (`sum(rows)`/`sum(size_in_bytes)` per table per day) that flags any single table
    exceeding an amplification factor vs its `... FINAL` real row count.
- Wire the breach to the existing 0056 Slack alarm path.
- Document the thresholds + the "what to check first" runbook pointer (link 0132).

## Acceptance Criteria

- [ ] A metric/alarm exists on live-pipeline egress (or per-table CH write volume)
- [ ] Threshold set above the legitimate baseline, below a 0132-class runaway
- [ ] Breach notifies the existing Slack channel (0056)
- [ ] Brief runbook note on interpreting a breach (links 0132)
