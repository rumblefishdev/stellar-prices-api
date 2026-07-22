---
id: "0036"
title: "What is the 237-byte delta between Phoenix XYK WASM builds 167ab414... and 13b158655e...?"
type: RESEARCH
status: completed
related_adr: []
related_tasks: ["0032", "0034", "0099", "0108"]
tags: [layer-research, priority-low, effort-small, phoenix, wasm-analysis, defensive]
links:
  - "../archive/0032_RESEARCH_phoenix-stable-pool-first-observation/notes/S-no-stable-pool-deployed.md"
  - "../archive/0032_RESEARCH_phoenix-stable-pool-first-observation/notes/evidence/phoenix_pool_inventory_2026-05-15.txt"
history:
  - date: 2026-05-15
    status: backlog
    who: oski
    note: "Spawned from 0032 — defensive: confirm 8-event grouping holds on second XYK build."
  - date: 2026-07-20
    status: completed
    who: okarcz
    note: >
      **SUPERSEDED by task 0099 — the question is answered, and the answer was
      "no, the grouping does not hold".** Closed in the 0108 post-M1 sweep.
      This task asked whether the 237-byte delta between the two production
      Phoenix XYK WASMs changes event emission, and predicted that if it did,
      the consumer's 8-event assumption would silently mis-parse. That is
      precisely what 0099 found in production: Phoenix emits VARIABLE-LENGTH
      swap groups, and dispatch_phoenix gated on n >= 8 (the fully-populated
      shape) while Phoenix omits optional fields — discarding 5,175 real
      7-event swaps, ~2.1% of Phoenix volume. Fixed and deployed to live
      2026-07-17 11:57:52.
      So the defensive hypothesis was correct and has been closed empirically,
      via production data rather than the planned swap-dump + wasm-tools diff.
      The step-3 disassembly is moot: we now know the behavioural difference
      directly, which is all the disassembly was ever a proxy for.
      Backward-looking reprice of what live wrote under the bug → 0101.
---

# Phoenix XYK 237-byte WASM delta

## Summary

Task 0032 found two distinct Phoenix XYK WASMs in production:
`167ab414...506c` (36810 B) and `13b158655e...f2ca` (37047 B). The
public Soroban interface and contract meta are byte-identical; the
runtime difference is therefore in implementation code. This task
verifies the practical impact on the consumer.

## Context

If the 237-byte delta changes event emission order, payload, or
count, the consumer's XYK extractor (per task 0018 §3, 8-event group)
would silently mis-parse the PHO/USDC pool today. Even if the delta
is benign (e.g., a minor invariant check, a logging line), it is
worth confirming to avoid future regressions.

## Implementation

1. **Dump a real swap** on the PHO/USDC pool
   (`CD5XNKK3B6BEF2N7ULNHHGAMOKZ7P6456BFNIHRF4WNTEDKBRWAE7IAA`) using
   the same procedure as task 0018 §3 (`dump-swap-events --contract …
   --tx … --show-xdr --pretty` against a Galexie ledger range
   containing the tx). Confirm the 8-event grouping holds and the
   field order matches the XYK spec.
2. If the grouping or field order differs in any way, escalate: this
   becomes a higher-priority finding for task 0034 (multi-WASM
   tolerance) and possibly a new extractor variant.
3. (Optional, only if step 1 surfaces a divergence.) Disassemble the
   two WASMs with `wasm-tools dump` or `wasm2wat` and diff the text
   forms to identify the actual code-level delta.

Defer step 3 unless step 1 motivates it — disassembly is expensive
and meaningless without a behavioral signal.

## Acceptance Criteria

- [ ] One real PHO/USDC swap event grouping decoded and archived as
      evidence.
- [ ] Confirmed: same 8-event grouping, same field order as the
      reference XYK spec from 0018.
- [ ] If divergence found, finding documented and 0034 updated.
