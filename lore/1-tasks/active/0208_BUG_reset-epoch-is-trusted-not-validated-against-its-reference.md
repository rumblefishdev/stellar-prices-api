---
id: "0208"
title: "coarse-repair trusts --reset-not-before instead of checking it against the reference market's first candle — the 0182 run destroyed 157 candles through a 19-hour hole"
type: BUG
status: active
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
  - date: "2026-09-23"
    status: active
    who: akot
    note: >
      Activated. Implementation runs from `.planning/BRIEF-0208.md` via
      /gsd-quick --full on branch fix/0208: refusal in
      assert_reset_is_admissible for pivot-reference legs, compared in seconds
      against the pivot's own reference predicate on the pass's table.
  - date: "2026-09-24"
    status: active
    who: akot
    note: >
      Implemented on fix/0208, PR #347 (12 commits, not merged). All six
      acceptance criteria met: guard last in assert_reset_is_admissible,
      three new refusals, 1612656000 refused on _1h/_4h and admitted on
      _1d/_1w/_1M. enrichment-worker lib 181/0, ch_enrich_it --ignored 66/0
      (8 new). Review: 0 blockers, 4 warnings + 7 info, all fixed. Stays
      active until #347 merges.
  - date: "2026-09-24"
    status: active
    who: akot
    note: >
      WR-04 fixed in #347 instead of a separate task (0312 created and deleted
      the same day): a plain-mode reset of a pivot leg (XLM, USDT) is refused
      with ResetPlainModeOnPivotLeg, pointing to the 0228 mode. Review found
      the check sat behind the oracle-shadow query (the operator would be told
      to purge live readings first) — moved before it. #347 now 26 commits;
      lib 189/0, ch_enrich_it --ignored 70/0, verified 13/13.
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

- [x] A reset whose `not_before` is below the reference's first candle **refuses
      before any write**, naming both timestamps and the correct epoch.
- [x] Verified non-vacuous: with the guard removed, a test reproducing the
      2026-08-18 shape (epoch at 00:00, reference from 19:00) leaves rows at
      `close_usd = 0` and the test fails.
- [x] Hour-granular — a case where epoch and first reference share a date but
      differ by hours is still refused.
- [x] The exact epoch already in flight (`1612656000` vs a reference starting
      `2021-02-07 19:00`) is a named regression test.
- [x] Runbook and every recorded USDT epoch corrected to `1612724400`.
- [x] Post-run damage check in the runbook says **every table**, not the tables
      that warned — the sampling error that let this reach prod.

## Implementation Notes

PR #347, branch `fix/0208_reset-epoch-is-trusted-not-validated-against-its-reference`.
Files: `packages/enrichment-worker/src/ch_enrich.rs`, `tests/ch_enrich_it.rs`,
`src/bin/coarse-repair.rs`, `docs/runbooks/repair-coarse-usd-values.md`.

- `pivot_reference_row_pred(ref_id, usdc_id)` is now the ONE definition of a usable
  reference row; `pivot_sql`'s reference leg and the guard's
  `SELECT toUInt32(minOrNull(timestamp)) … FINAL` are both rendered from it, and a
  unit test pins both clauses exactly.
- `assert_reset_epoch_is_covered` runs **last** in `assert_reset_is_admissible`, so
  the repair driver hits it before month enumeration (dry run included) and
  `reset_step` before `reset_sql`. The query has no month window and no watermark,
  so the per-month re-check gives the same answer every month.
- Pure `check_reset_epoch(not_before, first_reference)` compares seconds;
  equality is admitted (the first reference candle refills itself).
- New `ChEnrichError` variants: `ResetEpochBelowReference` (unix + UTC for both,
  the stranded window, the epoch to re-run with), `ResetEpochHasNoReference`,
  `ResetEpochUsdcUnresolved`.
- `coarse-repair` prints a refusal's Display message and exits 1; before, `main`'s
  `Result` printed the Debug form, so none of the operator guidance reached the
  terminal (all eight refusals).
- Measured per table against the incident's shape: `1612656000` refused on `_1h`
  (first reference 1612724400) and `_4h` (16:00 bucket, 1612713600), admitted on
  `_1d`/`_1w`/`_1M` — exactly the tables that did and did not lose candles.
- Runbook: per-table epoch measurement with the guard's predicate (Appendix C,
  now with the USDT id lookup), the 2026-08-18 worked example, damage/triage
  queries take `<TABLE>` and must be run on every table, and a new section
  "An admitted epoch is a lower bound, not a refill guarantee".

**Tests:** lib 181/0; `ch_enrich_it --ignored` 66/0 on rootless ClickHouse
26.3.10.60 (8 new ITs). Non-vacuity: with the guard call removed the incident IT
fails with 19 rows stranded at `close_usd = 0`; six single mutations of the
guard (empty set read as `Some(0)`, dropping `pf_trade_count > 0` / `close > 0` /
`volume_base > 0`, unresolved USDC treated as pass, always reading `_1h`) each
turn exactly one new IT red.

**Modified existing tests:** the 15 `pivot_reset(DEPEG_DAY - 86_400)` calls in the
0228 ITs used an epoch a day below their own fixture's first reference
(`DEPEG_DAY + 43_200`), which the new guard refuses. They now use
`PIVOT_FIRST_REF`; no asserted value changed (every subject row sits at or above
it). Unit fixtures that embedded `1612656000` as the USDT epoch now use
`1612724400`; where the old value is the refused case it is named
`INCIDENT_EPOCH_0182`.

## Design Decisions

### From Plan

1. **Pivot legs only (D1).** The peg/stable leg (USDC, incl. the 0268 external
   mode) has no reference series; both plain and 0228 modes are checked, because
   0228's `pivot_reference_day_pred` is day-granular and passes the 00:00→19:00 case.
2. **The pivot's exact predicate on the pass's own table (D2).** Per-table is what
   makes `_1d` admit the epoch `_1h` refuses.
3. **No reference refuses (D3); seconds, never dates (D4); new variant with the
   fix in the message (D5).**

### Emerged

4. **Guard runs last in the admissibility list.** Earlier would have changed the
   refusal seven existing ITs expect; the more specific refusals win first.
5. **`minOrNull` into `Option<u32>`** instead of `min`: ClickHouse's `min` over
   zero rows is 1970, which would admit every epoch.
6. **Separate `ResetEpochUsdcUnresolved`** (review IN-02) — "USDC missing from
   `prices.assets`" and "no reference rows on this table" have different fixes.
7. **Reference ids resolved once per admissibility check** (IN-03) and passed to
   every check that needs them; the 0215 missing-reference warning no longer
   repeats per check.
8. **CLI prints Display, not Debug** — found by the verifier; outside the brief,
   but without it AC1's "the message is the fix" never reaches the operator.

## Issues Encountered

- **Existing 0228 fixtures violated the invariant the guard enforces** — see
  Modified existing tests. Not a regression: those ITs were resetting below their
  own reference and passing only because the day-granular predicate masked it.
- **Runbook damage queries hard-coded `price_ohlcv_1d`** while telling the
  operator to check every table — the same sampling error as the incident.
  Replaced with `<TABLE>`.

## Plain mode on pivot legs (review WR-04, fixed in #347)

The epoch guard bounds only where the reference begins. A plain-mode reset
(neither `--reset-require-*` flag) checked neither refill input — a USDC rate
and a reference inside `--pivot-window-s` — so rows in a mid-history reference
gap were still zeroed for good. Adam chose to refuse it rather than gate it:

- `ResetPlainModeOnPivotLeg { quote_asset_id }` from the pure
  `check_plain_mode_leg`, placed right after `assert_reset_target_is_priceable`
  and **before** `assert_reset_not_shadowed_by_oracle`. The first placement
  (after it) was a review blocker: plain mode has no default
  `--reset-not-after`, XLM has live Reflector rows, so the operator got
  `ResetBlockedByOracleRows` ("purge those rows") first — an irreversible
  delete — and only then the plain-mode refusal.
- Plain mode stays for the USDC peg leg, which needs `--reset-not-after` at or
  below USDC's first oracle reading on prod; `ResetBlockedByOracleRows` now
  says bound the window, never purge live readings.
- Tests: the plain-mode USDT/XLM ITs moved to the 0228 mode (none removed);
  new ITs for the 3-day reference-gap refusal, the refusal with live oracle
  rows on both legs, the peg tier refilling a plain USDC reset, and the
  unbounded USDC refusal. Red runs: without the check 3 ITs fail (1 row
  stranded in the gap), with it behind the oracle query 2 fail.
- Runbook Appendix A (0182's plain procedure) is historical and peg-only.

## Future Work

None open. WR-04 was briefly spawned as 0312 and deleted the same day — it is
fixed in #347 (section above).
