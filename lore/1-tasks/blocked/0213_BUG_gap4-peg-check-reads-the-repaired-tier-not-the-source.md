---
id: "0213"
title: "The USD peg check reads _1h — the tier 0182 repaired — so it publishes 0 over 1.5M wrong _1m rows"
type: BUG
status: blocked
related_adr: []
related_tasks: ["0204", "0212", "0209", "0182", "0172"]
tags: ["priority-medium", "effort-small", "observability", "clickhouse", "data-correctness", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/rollup-freshness-probe/src/usd_sanity.rs"
  - "../../../infra/src/lib/stacks/observability-stack.ts"
history:
  - date: 2026-08-20
    status: backlog
    who: okarcz
    note: >
      Spawned from 0204 gap 4 during the pre-deploy prod baseline. The
      peg-applied ladder reads price_ohlcv_1h, which 0182's repair wrote, while
      the peg values live in price_ohlcv_1m, which that repair never touched —
      1,564,045 of them. The check would publish a confident 0. The stranded
      direction is unaffected and is what found 0209.
  - date: 2026-08-20
    status: blocked
    who: okarcz
    note: >
      Code complete and green; BLOCKED on 0212 (and 0209 behind it) for deploy
      only. The peg direction now reads price_ohlcv_1m over a 48 h window with
      its own guards and refusal, the stranded direction is untouched on
      price_ohlcv_1h over 7 days, and the two publish independently. 51 unit
      tests (+14) and 19 integration tests (+4) against CH 26.3.10.60. Verified
      NON-VACUOUS by reverting PEG_TABLE to price_ohlcv_1h: 6 ITs and 2 unit
      tests fail. Acceptance criterion 3 cannot be met until 0212 repairs the
      1.5 M rows — deploying before that ships a permanently-breached ladder.
  - date: 2026-08-20
    status: blocked
    who: okarcz
    note: >
      Prod measurements taken. 🔴 THE DEPLOY BLOCK WAS FALSIFIED — with 0212
      unlanded the ladder reads scanned 684 / peg_applied 0, because the 1.5 M
      peg rows all sit at timestamps <= 2026-08-13 and the 48 h window does not
      reach them. The "ships permanently breached" claim was inherited from the
      unbounded-repoint argument and never re-checked against the query that
      ships; corrected in usd_sanity.rs, observability-stack.ts, the PR body and
      the alarm text. All 684 rows are close_usd = 0, so the direction reads 0
      for want of input, not want of defects — a green peg ladder is NOT
      evidence of a healthy leg while 0209 stands. Empty-scan concern also
      closed: longest silent gap in 30 days is 6.5 h against a 48 h window.
      NOW BLOCKED ON ONE THING ONLY: the peg scan's read cost on prod, which
      decides whether the probe's 1-minute timeout still holds.
---

# The peg check reads the repaired tier, not the source

## Summary

[[0204]] gap 4 alarms on two directions. The **stranded** one works: a zero in
`_1m` rolls up as a zero in `_1h`, so reading the coarse tier detects it — that
is how [[0209]] was found. The **peg-applied** one does not, because a *repaired*
value in `_1h` says nothing about the row it was rolled from.

Measured on prod 2026-08-20:

| table | USDT-quoted rows at `close_usd / close ≈ 1.0` |
|---|---|
| `price_ohlcv_1h` — what the check reads | **0** |
| `price_ohlcv_1m` — where enrichment writes | **1,564,045** |

[[0182]]'s repair wrote the five coarse tiers directly and never touched `_1m`
([[0212]]). So the alarm reads clean over 1.5 M wrong values and would have gone
on doing so indefinitely.

⚠️ **This is the task's own founding failure, reproduced inside the guard built
against it** — a check scoring healthy because it looked at the surface least
able to show the defect.

## Why the obvious fix is wrong

⛔ **Do not simply repoint `SANITY_TABLE` at `price_ohlcv_1m`.**

1. It would read 1,564,045 immediately — above every rung of
   `usdSanityEscalationCounts` (`[1, 100, 10000]`) — and sit permanently in
   ALARM. A permanently-firing alarm gets muted, which is the exact end-state
   [[0204]] exists to prevent.
2. `_1m` is **retention-managed at 7 days** while `_1h` is a forever-table. The
   check's 7-day `LOOKBACK_SECONDS` sits exactly on that boundary, so the window
   reasoning has to be redone rather than inherited.
3. The stranded direction is *correct* on `_1h` and would be made worse by moving
   — the 48 h grace is calibrated to BE's loss window on the hourly tier.

## Implementation

- Split the two directions: keep `stranded` on `_1h` unchanged; give
  `peg_applied` its own `_1m`-scoped query, window and ladder.
- ⚠️ Sequence it **after [[0212]]** has repaired the 1.5 M rows, or the new
  ladder ships permanently breached — which is the muting failure above.
- Re-verify the scan cost by what it **reads**, not what it returns; `_1m` at
  7 days on one quote leg is a different shape from `_1h`.
- The IT already writes a par-valued candle into a real ClickHouse
  (`usd_sanity_counts_both_induced_defects`); extend it to `_1m` so the tier
  distinction is induced rather than reasoned about.

## Acceptance Criteria

- [x] The peg-applied metric is computed from **`price_ohlcv_1m`**, and an IT
      proves it counts a par-valued `_1m` row that no coarse tier carries —
      `a_peg_row_only_in_1m_is_counted_although_every_coarse_tier_reads_clean`.
- [x] The stranded metric still reads `_1h` and its 48 h grace still means
      BE's loss window. `STRANDED_TABLE`, `STRANDED_LOOKBACK_SECONDS` and
      `STRANDED_GRACE_SECONDS` are unchanged in value; only the names moved.
- [x] The ladder reads **0** on prod. ⚠️ **But the criterion's stated logic
      ("i.e. [[0212]] landed first") was FALSIFIED.** Measured 2026-08-20 with
      0212 unlanded: `scanned` 684, `peg_applied` **0**. The 1.5 M peg rows all
      sit at timestamps ≤ 2026-08-13, entirely outside the 48 h window — the
      window choice moved this task out from under its own blocker, and nobody
      noticed until the measurement. 🔴 **It reads 0 because all 684 rows are
      `close_usd = 0`** (0 peg-valued, 0 correctly priced): the leg is dark, so
      the direction has nothing to judge. A green ladder is NOT evidence of
      correct USD valuation — recorded at `PEG_TABLE` and in the alarm text.
- [ ] ⛔ **PRE-DEPLOY MEASUREMENT: the peg scan's read cost on prod.** The
      probe's 1-minute timeout was sized on the `_1h` scan alone (~1.37M rows /
      ~70 MiB / 41-50 ms) and was not revisited for this second `FINAL` scan
      against the **735M-row** `_1m`. ⚠️ **The 48 h window does not bound the
      read**: both tables are `PARTITION BY toYYYYMM(timestamp)` with
      `ORDER BY (asset_id, quote_asset_id, source, timestamp)`, so `timestamp`
      is not a primary-key prefix and the scan prunes to a whole **monthly
      partition** (two across a month boundary). Measure by `EXPLAIN` /
      `read_rows`, never by rows returned. A timeout here is not a Rust `Err` —
      it kills the invocation with nothing published, and step 4 (MV drift)
      never runs.
- [x] ✅ **MEASURED 2026-08-20 — `scanned == 0` is not reachable.** The longest
      stretch with no USDT-quoted `_1m` row in the preceding 30 days is
      **6.5 hours** against the 48 h window: 7.4x margin, so the empty-scan
      refusal will not page on a healthy leg. No code change needed. ⛔ Do not
      shorten the window below ~24 h without re-taking this. Original concern:
      **can `scanned` legitimately reach 0 on the peg tier?** `EmptyScan` was calibrated for a 7-day window on `_1h`; the peg
      direction applies the same `scanned == 0` refusal to **48 h on a sparse
      leg** — measured at ~16 USDT-quoted `_1m` rows for 2026-08-17, so ~32 in
      a 48 h window. A quiet spell makes zero legitimate, and the refusal then
      fails the invocation and pages ops via the probe's own `-errors` alarm
      **every 15 minutes on a healthy system** — the muting failure again.
      Measure the daily minimum over ≥30 days before deploying; if zero is
      reachable, the guard needs a different discriminator (candidate: refuse
      only when the *stranded* direction also scanned nothing, which
      distinguishes a quiet window from a renumbered identity without inventing
      a threshold).
- [x] A note records why `_1m`'s 7-day retention does not undermine the
      lookback — `PEG_LOOKBACK_SECONDS`, plus the
      `the_peg_window_stays_clear_of_the_1m_retention_frontier` unit test and
      the `the_peg_window_excludes_rows_a_cleanup_run_could_delete` IT.

## Implementation Notes

Four files. The shape is a **split, not a repoint** — everything that made the
stranded direction correct on `_1h` is preserved verbatim.

| file | change |
|---|---|
| `usd_sanity.rs` | `SANITY_TABLE` → `STRANDED_TABLE` + `PEG_TABLE`; `LOOKBACK_SECONDS` → `STRANDED_LOOKBACK_SECONDS` (7 d) + `PEG_LOOKBACK_SECONDS` (48 h); `sanity_query` → `stranded_query` + `peg_query`; `SanityCounts` → `StrandedCounts` + `PegCounts` behind a `ScanGuards` trait; `sanity_metrics` → `stranded_metric` + `peg_metric` |
| `main.rs` | two reads and two publishes instead of one, each failing independently |
| `rollup_freshness_it.rs` | fixtures parameterised by tier; 4 new ITs |
| `observability-stack.ts` | peg alarm description names `_1m` and 48 h; ladder comment carries the deploy ordering |

**Verified:** `cargo fmt --all --check` · `cargo clippy -p rollup-freshness-probe
--all-targets --features lambda` (0 warnings) · `cargo test --workspace` (92
suites, 0 failures) · 19 ITs against a local CH pinned to **26.3.10.60** ·
`nx run-many -t lint typecheck` · `nx build infra` · `make -C infra
synth-production` · all six USD alarms render 856–937 chars against the 1024 cap.

## Issues Encountered

- 🔴 **The first version of the new tests was VACUOUS, and the fixtures were
  why.** `insert_usdt_minute_candle` was written as
  `insert_candle_into(c, PEG_TABLE, …)`. A fixture expressed in terms of the
  constant under test follows it wherever it points, so reverting `PEG_TABLE`
  to `price_ohlcv_1h` moved the *writes* too and the tier assertions kept
  passing. Found only by doing the revert. Fixed by spelling both tables
  literally in the fixtures. ⚠️ **This is the task's own founding failure a
  third time** — after the check that read the repaired tier, and 0182 verified
  against its own output: a test that cannot distinguish the thing it asserts.
  The non-vacuity check is now the acceptance evidence, not a nicety.
- `main.rs` claimed `"{} of 4 probe checks failed"`. Check 3 can now contribute
  two failures, so the denominator became false; it now reads
  `"{} probe check(s) failed"`.

## Design Decisions

### From Plan

1. **Split the directions rather than repoint one table.** The three costs of a
   bare repoint are recorded verbatim on `PEG_TABLE`, so the next person to have
   the same idea meets the reasoning before the change.
2. **Stranded stays on `_1h`, untouched.** A zero rolls up as a zero, and the
   48 h grace is calibrated to BE's loss window on the hourly tier.

### Emerged

3. **The peg window is 48 h, not 7 days.** The plan said the window "has to be
   redone rather than inherited" but not to what. 48 h keeps five days of margin
   below `_1m`'s 7-day retention, so a cleanup run can never truncate the scan.
   It costs nothing because this is a re-introduction guard, not a historical
   audit — a regressed writer writes continuously and shows up in the newest
   rows. Frozen history is 0212's population.
4. **No grace on the peg direction.** A stranded row is one enrichment has not
   reached *yet*; a peg-valued row is one enrichment has already written
   *wrongly*. Waiting cannot improve the second, so a grace would only delay
   saying so.
5. **The two directions publish and refuse INDEPENDENTLY.** Previously one
   refusal suppressed both metrics — harmless while they came from one query,
   and the muting failure from the other side once they read different tables.
   A `_1m` scan that matched nothing says nothing about `_1h`.
6. **Guards duplicated per direction, via a `ScanGuards` trait.** Sharing one
   `resolved_legs`/`scanned` pair across two tiers would score the tier that was
   never examined as healthy — this task's own defect, one level up.
7. **`SanityRefusal` now carries the table and the lookback.** With two tiers,
   "the scan matched nothing" is no longer a fact about one place, and a refusal
   that did not say which would send the operator to a table that was fine.

## Review Round

A code review of PR #235 returned six findings; each was verified against the
code before acting. Four were defects in this change and are fixed in the PR:

- **`both_queries_read_final` could not fail.** It asserted
  `sql.contains(" FINAL ")`, already satisfied by `FROM assets FINAL` in the
  shared CTE, so deleting `FINAL` from either candle table would have left it
  green. Now asserts each candle table by name. A sibling assertion,
  `!peg_query().contains("STRANDED_GRACE")`, was checking for Rust identifier
  text in generated SQL and could never fail either.
- **One IT assertion tested the window, not the tier.** In
  `each_direction_only_scans_its_own_tier`, the `_1h`-only half seeded at
  `now() - INTERVAL 3 DAY`, which the 48 h peg window excludes whichever table
  is read — so it passed with `PEG_TABLE` reverted. Seeded at `3 HOUR` now.
  ⚠️ **Third instance of this task's founding failure in one change**, after
  the fixture coupling above.
- **The "reads 1,564,045 — above every rung" claim was wrong for the query that
  ships.** That is the all-history population ([[0212]]); bounded to 48 h the
  reading is the recent arrival rate, tens of rows, which clears rung 1 and
  nothing above it. Corrected in `usd_sanity.rs` and `observability-stack.ts`,
  because it is exactly the number someone would use to re-size the ladder.
- **Broken intra-doc links** to the deleted `LOOKBACK_SECONDS`, `SanityCounts`
  and `SANITY_TABLE`, plus a now-false "prunes by partition" claim. Fixed, and
  the crate's unresolved-link count went **9 → 6** (the rest are in `mv_drift`
  and `lib.rs`, out of scope here).

The remaining two were promoted to acceptance criteria. **Both were then
measured on prod, and one of them overturned this task's central claim:**

- ✅ **Empty scan is not reachable.** Longest silent stretch on the USDT `_1m`
  leg over 30 days is **6.5 h** against the 48 h window — 7.4x margin. No code
  change needed.
- 🔴 **The deploy block was wrong.** It read `peg_applied = 0` with 0212
  unlanded, because the peg population is entirely outside the window. ⚠️ The
  claim was carried from the *unbounded repoint* argument and never re-checked
  against the query that actually ships — the same class of error as the
  original defect: reasoning about a surface other than the one in use.
- ⏳ **Read cost still unmeasured** — the one thing this task now waits on.

## Future Work

None spawned. The remaining work is the deploy, which is gated by [[0212]] and
[[0209]] — both already exist and both already carry this ordering. ⛔ Do not
close this task on the code being green.
