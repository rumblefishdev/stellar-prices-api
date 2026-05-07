---
id: "0004"
title: "Run dump-swap-events against a wider ledger range to detect Phoenix"
type: RESEARCH
status: backlog
related_adr: []
related_tasks: ["0001", "0002"]
tags: [priority-low, effort-small, soroban, amm]
links:
  - "../active/0001_RESEARCH_dump-amm-swap-events/notes/R-swap-topic-shapes.md"
history:
  - date: 2026-05-07
    status: backlog
    who: okarcz
    note: "Spawned from 0001 future work."
---

# Run dump-swap-events against a wider ledger range to detect Phoenix

## Summary

Lore 0001 sampled ~3.5 days of mainnet (62016000–62079999) and found no
Phoenix-style swap events. Pull a wider sample (e.g. one full month) and
re-run `tools/dump-swap-events --histogram` to confirm whether Phoenix
uses yet another topic symbol or is simply low-volume.

## Context

Per `0001/notes/R-swap-topic-shapes.md`, only `swap` (1 emitter) and
`trade` (29 emitters) were observed. Phoenix's absence may be:
1. Low volume in the sampled window (plausible — Phoenix is the smallest
   of the three target AMMs).
2. A third distinct topic symbol we haven't seen yet.
3. Phoenix using one of the symbols we already saw (i.e. one of the 29
   `trade`-emitters IS Phoenix).

This task only resolves (2). Resolution of (3) is the job of task 0002
(venue attribution).

## Implementation

- Pull ~1 month of `.xdr.zst` from the public Galexie archive for a
  recent range, into `.temp/`.
- Run `dump-swap-events --histogram` on the new range.
- Look for swap-like topics not seen in 0001 (e.g. `Swap`,
  `SwapExecuted`, `swap_executed`, etc).
- Update `0001`'s `R-swap-topic-shapes.md` (in archive) with the new
  histogram, or write a fresh note here referencing it.

## Acceptance Criteria

- [ ] At least 30 days of mainnet ledgers scanned with `--histogram`
- [ ] All swap-like topic_0 symbols enumerated and counted
- [ ] Either: Phoenix-specific topic symbol identified, OR documented
      conclusion that Phoenix uses one of the symbols already seen in
      0001 (with at least one example contract for follow-up by 0002)
