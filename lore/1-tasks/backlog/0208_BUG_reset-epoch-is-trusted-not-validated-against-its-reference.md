---
id: "0208"
title: "coarse-repair trusts --reset-not-before instead of checking it against the reference market's first candle — the 0182 run destroyed 157 candles through a 19-hour hole"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0182", "0172", "0114", "0145"]
tags: ["priority-high", "effort-small", "clickhouse", "data-correctness", "enrichment", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../docs/runbooks/repair-coarse-usd-values.md"
history:
  - date: 2026-08-19
    status: backlog
    who: okarcz
    note: >
      Spawned from 0182. Its run on 2026-08-18 was given an epoch of 1612656000
      (2021-02-07 00:00) when the pivot's reference market does not begin until
      19:00 that day. The reset zeroed 157 candles in the gap and the pivot
      could not refill them. The tool accepted the epoch without checking it
      against the reference it would later join on, and every existing guard
      passed.
---

# The reset epoch is an operator assertion the tool never checks

## Summary

`UsdResetSpec.not_before` is load-bearing — below it the pivot has no reference,
so a row the reset zeroes can never be refilled and stays at `close_usd = 0`
permanently. The tool takes that boundary on trust. On 2026-08-18 it was wrong by
nineteen hours and **157 candles were destroyed** ([[0182]]).

The fix is small: refuse a `not_before` that falls below `MIN(timestamp)` of the
reference series the pivot will actually use, the same way
`ResetTargetHasNoPricingPath` already refuses a leg no tier can reprice.

## Context

[[0182]] added reset mode with three guards, each closing a way the run could
discard a value: `ResetBlockedByOracleRows`, `ResetTargetHasNoPricingPath`,
`ResetRequiresOneShot`, plus a `--pivot-window-s` floor per bucket width. Its own
review note named the principle — *"every way this tool can discard a value now
refuses rather than warns."*

The epoch was the one that got a **comment instead of a check.** [[0182]]'s file
asserted the reference was "dense and non-NULL from 202102, first candle
`2021-02-07 19:00`, so `1612656000` cannot strand rows." Both halves of the
premise are true and the conclusion is false: `1612656000` is that day's
*midnight*, and the pivot's `ASOF LEFT JOIN` matches at-or-before.

⚠️ **`ResetTargetHasNoPricingPath` does not catch this**, and that is the point.
It asks whether a pricing path *exists* for the leg. One did — starting nineteen
hours after the epoch. The existing guard is about the *asset*; this is about the
*boundary*.

⚠️ **Nor is it visible in the outcome.** `_1h` reported 357,274 reset against
358,315 enriched — **no shortfall at all**, because 121 stranded rows are
invisible against a third of a million legitimately enriched ones. The
`rows_reset ≈ rows_enriched` check is a population comparison and cannot resolve
a defect three orders of magnitude below its own noise.

## Implementation

- In the same pre-flight block as `ResetTargetHasNoPricingPath`, resolve the
  reference series the pivot will use for `quote_asset_id`, take its
  `MIN(timestamp)`, and refuse when `not_before` is below it — reporting both
  timestamps and the suggested epoch, so the message *is* the fix.
- New `ChEnrichError::ResetEpochBelowReference { not_before, first_reference }`.
- ⚠️ **Compare to the hour, not the day.** A day-granular check passes the exact
  case that caused this. The stranding window is
  `[not_before, first_reference)` and it can be any length.
- Runbook: replace the "pick an epoch" guidance with "query the reference's
  `MIN(timestamp)` and use that value", and record the 2026-08-19 boundary
  repair as the worked example.
- Correct `1612656000` → `1612724400` wherever it appears as the USDT epoch.

## Acceptance Criteria

- [ ] A reset whose `not_before` is below the reference's first candle **refuses
      before any write**, naming both timestamps and the correct epoch.
- [ ] Verified non-vacuous: with the guard removed, a test reproducing the
      2026-08-18 shape (epoch at 00:00, reference from 19:00) leaves rows at
      `close_usd = 0` and the test fails.
- [ ] Hour-granular — a case where epoch and first reference share a date but
      differ by hours is still refused.
- [ ] The exact epoch already in flight (`1612656000` vs a reference starting
      `2021-02-07 19:00`) is a named regression test.
- [ ] Runbook and every recorded USDT epoch corrected to `1612724400`.
- [ ] Post-run damage check in the runbook says **every table**, not the tables
      that warned — the sampling error that let this reach prod.
