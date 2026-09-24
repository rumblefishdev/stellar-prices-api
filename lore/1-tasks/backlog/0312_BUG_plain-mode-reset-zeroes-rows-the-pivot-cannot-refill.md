---
id: "0312"
title: "Plain-mode (0182) reset still zeroes pivot-leg rows that have no USDC rate or no reference inside the pivot window — the epoch guard only covers the start of history"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0208", "0182", "0228", "0268"]
tags: [priority-low, effort-small, clickhouse, data-correctness, enrichment]
links:
  - "../active/0208_BUG_reset-epoch-is-trusted-not-validated-against-its-reference.md"
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../docs/runbooks/repair-coarse-usd-values.md"
history:
  - date: "2026-09-24"
    status: backlog
    who: akot
    note: >
      Spawned from 0208 future work (PR #347 review, WR-04). 0208 refuses an
      epoch below the reference's first candle; an admitted epoch is still only
      a lower bound. Low priority: it bites only when someone runs a plain-mode
      reset campaign, and none is planned. Do this before the next one.
---

# A plain-mode reset still strands rows in the middle of history

## Summary

[[0208]] closed the **start** of the stranding window: `coarse-repair` now
refuses a `--reset-not-before` below the first priced reference candle on the
table. The same damage — rows zeroed by the reset that the pivot cannot put
back, left at `close_usd = 0` permanently — is still possible **above** the
epoch in plain (0182) mode, and nothing refuses it.

## Context

The pivot refills a zeroed pivot-leg candle only when **both** exist for its
bucket:

1. a USDC rate (`usd_rate`, `oracle` or `external`) within the derived bound of
   the bucket end, and
2. a reference candle (e.g. USDT/USDC) at or before the bucket, within
   `--pivot-window-s`.

Example: the USDT/USDC market goes silent for three days in 2022. A plain-mode
reset from an admitted epoch zeroes every USDT-quoted candle in those days;
with a 24 h pivot window nothing refills them.

The 0228 mode (`require_pivot_usdc_rate`) already narrows its candidate set with
`external_rate_day_pred` and `pivot_reference_day_pred`, so it only zeroes days
it can refill. Plain mode has neither. Today the only protection is the
runbook's "An admitted epoch is a lower bound, not a refill guarantee" section
(added by [[0208]]): two day-level counts the operator must run and see at 0 —
an operator assertion, the class 0208 existed to remove.

## Implementation

- Decide between (a) gating plain-mode pivot-leg resets the way 0228 does —
  splice `external_rate_day_pred` / `pivot_reference_day_pred` into
  `reset_sql`, `reset_pending_pred` and the month enumeration, and run
  `assert_external_rates_are_loaded` in `assert_reset_is_admissible` — or
  (b) refusing plain mode for pivot legs outright and pointing the operator at
  0228's mode, if no campaign needs plain mode any more.
- Either way, a refusal/narrowing lives in the single admissibility list and
  holds in a dry run.
- ⚠️ The day predicates are day-granular: a bucket whose reference falls
  outside `--pivot-window-s` on the same day is still admitted. Decide whether
  that residue is acceptable (0228 accepted it) or needs a bucket-level
  predicate.
- Remove the "run these two counts first" caveat from the runbook and
  `--help` once the tool enforces it.

## Acceptance Criteria

- [ ] A plain-mode pivot-leg reset cannot zero a row on a day with no USDC rate
      or no usable reference — refused or excluded before any write, dry run
      included.
- [ ] Non-vacuous IT: a reference gap inside the reset window leaves rows at
      `close_usd = 0` without the change and not with it.
- [ ] Existing 0182/0208 reset ITs still pass; runbook and `--help` caveats
      updated.
