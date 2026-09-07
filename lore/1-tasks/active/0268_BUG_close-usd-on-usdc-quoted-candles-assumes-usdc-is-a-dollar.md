---
id: "0268"
title: "close_usd on every USDC-quoted candle before 2026-03-11 assumes USDC = $1 — re-enrich 654,291 candles from the external rate, and stop stamping method: peg on assets that are not stablecoins"
type: BUG
status: active
related_adr: ["0011"]
related_tasks: ["0265", "0267", "0168", "0182", "0111", "0266", "0247"]
tags: [layer-backend, priority-medium, effort-large, milestone-M3, clickhouse, enrichment, data-correctness, stablecoin, history]
milestone: 3
links:
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/S-phase0-root-cause.md"
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/S-backfill-migration.md"
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../packages/prices-clickhouse/schema/init.sql"
history:
  - date: 2026-09-07
    status: backlog
    who: akot
    note: >
      Spawned from [[0265]]'s Future Work as "defect B". [[0168]] named this
      the "known adjacent gap" it deliberately did not close; [[0267]] closes
      USDC's own series (defect A) and leaves this one, because it is a
      re-enrichment of stored candles, not an INSERT. Sized from the sweep in
      0265 phase 0 (135 of 232 assets carry method: peg).
  - date: 2026-09-07
    status: active
    who: akot
    note: "Activated; taken by akot after closing 0265."
  - date: 2026-09-07
    status: active
    who: claude
    note: >
      Code half shipped on fix/0268_close-usd-assumes-usdc-is-a-dollar in three
      commits: the method vocabulary split with a read-time 'external' label
      arm; a third enrichment tier pricing USDC legs from the measured rate;
      and the reset scoped by a shared day-set predicate so it can only zero a
      bucket it can refill. 23 new SQL-string unit tests (bare cargo test, no
      ClickHouse), 8 new #[ignore] behavioural tests, plus
      tests/post_run_0268_it.rs as the operator's falsifier. Runbook Appendix B
      added. AC 3 closes; 1, 2, 5 close on the prod run (waits on 0267's rows);
      4 belongs to 0267 step 2; 6 is a post-run operator step. STATUS STAYS
      ACTIVE — six of seven run-day steps are the operator's.
---

# Stored USD prices assume USDC is a dollar

## Summary

Every USDC-quoted candle enriched before the oracle window (2026-03-11)
has `close_usd = close × 1.00`. The `1.00` is the enrichment peg tier's
literal, not a measurement ([[0247]] measured it: **654,291** pre-oracle
USDC-quoted candles, one implied rate, exactly 1.0). On 2023-03-11, when
USDC traded at 0.87–0.97, every such asset's USD price is overstated by
the depeg. XLM had 36,208 trades that day and reads `method: peg`.

Two symptoms, one fix:

1. **The stored number is wrong** on the stress days and slightly wrong
   everywhere else (USDC sits 1–10 bps off par on ordinary days).
2. **The label lies to consumers.** `method: peg` appears on 135 of 232
   swept assets (`native` on 1,865 days, canonical USDT on 1,858) and there
   means "USD denomination assumed", while on USDC it means "no measured
   rate". A reader cannot tell the two apart without the source code
   (0265 phase 0, `data/synthetic_profile.csv`).

## Context

- [[0267]] loads the measured USDC/USD series (Chainlink primary, Bitstamp
  fallback) into `usd_rate` as `method = 'external'` and fixes the *view*
  and `/ohlcv` for USDC. It does not touch stored `close_usd`, so after it
  ships the view and the candles disagree for 2021-01 → 2026-03. That
  disagreement is intended and announced in `backfill_note`; this task ends
  it.
- [[0182]] is the precedent: 567,760 candles corrected across five
  granularities after the USDT peg fix. Same class of job, same risks (it is
  what the cleanup worker shredded in the 0182/0201 campaign, which is why
  that worker is dark).
- [[0111]]'s enrichment redesign is the gate: the re-enrichment must run as a
  bounded, resumable pass, not a full-table rescan.
- [[0266]]'s ~25 % dislocations on the same dates are a different mechanism
  (two unrelated assets moving by an identical ratio) and stay separate;
  after this task lands, re-measure 0266's table to see what remains.

## Implementation

1. **Rate source**: the `external` rows [[0267]] loads (daily; hourly for
   2023-03 if 0267 ships it). The enrichment reads `usd_rate` for the bucket
   with preference `oracle` → `external` → literal 1.0, and records which
   one it used.
2. **Re-enrich** `close_usd` (and `volume_quote_usd`, `vwap` where derived)
   on every candle with `quote_asset_id = USDC` and `timestamp < 2026-03-11`,
   on all forever granularities, in bounded batches with a resumable cursor
   — the 0182 runbook shape, under 0111's constraints. Do not touch candles
   already enriched from an `oracle` reading.
3. **Vocabulary**: split `method` so it no longer overloads `peg`:
   - `traded` / `oracle` / `pivot` / `pivot2` keep their meaning;
   - `external` = the imported USDC/USD rate was the input;
   - `assumed-par` (or drop `peg` from non-stablecoins entirely) = the
     literal 1.0 was the input, i.e. nothing measured. Add it to
     `init.sql`'s vocabulary block and to the API docs.
4. **Verification fixtures**: XLM and yBTC daily closes on 2023-03-11 and
   2023-03-15 against 0266's Binance reference; the 0265 guardrail
   invariants over the swept peg-coded assets; a count of remaining
   `implied_from_candles == 1.00000000` rows that must be zero for
   USDC-quoted candles before 2026-03-11.
5. **Rollout**: shadow column or shadow partition first, compare, then swap;
   rollback is the previous parts (never delete before the swap is verified).
   Keep the cleanup worker dark for the duration.

## Acceptance Criteria

Five of the six close **on prod, after the operator run** — the branch delivers
the code, the tests, the runbook and a dry-runnable tool, and the run itself
waits on [[0267]]'s rows. Which half is done is stated per criterion.

- [ ] No USDC-quoted candle before 2026-03-11 carries `close_usd == close`
      exactly where an `external` rate exists for its bucket *(code half DONE:
      the external tier, its four SQL-string invariants and its three
      behavioural tests. Closes on the run — Appendix B.)*
- [ ] `native` on 2023-03-11 publishes a USD close that reflects the USDC
      rate that day (≈ 3 % below the USDC-denominated close), on every
      granularity *(the falsifier is now runnable code —
      `packages/enrichment-worker/tests/post_run_0268_it.rs`, both tests
      `#[ignore]`d. Expected to FAIL until the pass has run; that is its
      purpose.)*
- [x] `method: peg` no longer appears on any non-stablecoin; the new value
      is documented in `init.sql` and the API reference — **the emission half
      is the whole code half and it shipped**: the candle path now says
      `assumed-par` or `external`, and the vocabulary is documented in
      `init.sql`, `views.sql`, the OpenAPI `Candle.method` description,
      `dto.rs` and `docs/database-schema/database-schema-overview.md`, all in
      one commit with the wire change. `peg` survives only on USDC's own
      series, where 0165's meaning still holds.
- [ ] The view and stored `close_usd` agree in deep history; the
      `backfill_note` caveat from [[0267]] is removed *(**not this branch** —
      needs `views.sql:521,728` and `queries_ch.rs:960` widened from
      `method = 'oracle'` to include `'external'`, which is [[0267]] step 2.
      See Issues 1.)*
- [ ] The pass is bounded and resumable; runtime and rows touched recorded
      here, as [[0182]] did *(code half DONE: every new statement carries the
      0111 partition bound on its candidate side, asserted by an occurrence
      count; the reset obeys `time_window`. The figures go here after the
      run.)*
- [ ] [[0266]]'s dislocation table re-measured after the pass, with the
      result recorded there *(deferred, ratified: post-run operator step.)*

## Out of scope

- USDC's own series — [[0267]].
- Any quote asset other than canonical USDC.

## Implementation Notes

What shipped on `fix/0268_close-usd-assumes-usdc-is-a-dollar`, in three commits.
The prod run is not in them — it waits on [[0267]]'s rows and is an operator
step with its own runbook appendix.

**The tier, and where it slots.** A third tier in `ch_enrich.rs`,
`run_external_tier`, between the oracle tier and the peg-pivot tier. The order
is an evidence ranking: a reading we polled, then a reading someone else
measured, then an assumption. It is gated on the SAME `oracle_drained` flag as
the peg tier, for the same two reasons — an un-drained oracle tier may still
hold an unapplied in-window oracle price, and once `close_usd > 0` a row never
re-enters the oracle tier on a later pass. `refs` is resolved once and shared
with the peg-pivot block, and `remaining`/`batches` are handed down, so the peg
tier only ever sees what neither tier above could price.

`volume_quote_usd` is write-once (`if(p.volume_quote_usd > 0, …)`). That single
expression IS the "oracle wins" guard: a candle the oracle priced keeps its
depeg-aware value even if the statement reaches the row.

**Why not the obvious shortcut.** Inserting the imported series into
`oracle_prices` under a pseudo oracle name would have been less code and let
tier 1 do the work. Rejected twice over: [[0247]] forbids publishing an import
as `oracle`, because `oracle` means "we polled Reflector"; and `oracle_prices`
staying untouched is what preserves the meaning of the reset's oracle-shadow
guard, which exists to refuse a reset the oracle tier could undo.

**Why the bucket's END.** `timestamp` is the bucket's START but `close` is the
period's LAST close, so resolving at `timestamp` prices a weekly or monthly
candle with the rate from the day the period opened — up to a month stale, and
systematically so. The bucket end is already the settled rule on both other
surfaces (`views.sql`'s `price_usd_series`, `queries_ch::ohlcv_peg_series`);
this reuses it rather than coining a third convention. ⚠️ Calendar functions
for the calendar grains: `timestamp + 2_678_400` overshoots every 30-day month
and every February, so eleven months of twelve would resolve against the wrong
day's rate — producing a plausible number and failing nowhere.

**Why the staleness bound is derived and not configured.** `external_window_s`
returns `max(86_400, bucket_width)` from the table name. The floor of one day is
0267's cadence. `pivot_window_s` is configurable, and that is exactly why
`coarse-repair` needs an explicit refusal for a value shorter than the bucket
width: with a reset in play, too narrow a window discards a stored value and
then fails to recompute it. A derived bound cannot be set wrong, so there is no
refusal to write and no way for an operator to reach the destructive
combination — and it avoids threading a knob nobody should turn through six
`ChEnrichConfig` literal sites.

**The read-time label, and why a timestamp arm is sound.** Candles store no
`method`; `/ohlcv` reconstructs provenance from the quote leg and the rate
signature. The classification is now one pure fn, `usd_method_expr`, so its arm
order is unit-testable. Four arms: `close_usd = close` on a USDC leg →
`assumed-par`; a USDC leg below `USDC_ORACLE_EPOCH_S` → `external`; any other
USDC leg → `oracle`; a pivot leg → `traded`; then the empty fallback.

Arm 2 is sound only because `prices.usd_rate` holds no `oracle` row for
canonical USDC before that instant. That is in-repo prose, **not** a live
measurement — which makes it falsifiable, and Appendix B precondition 3 is the
query that falsifies or confirms it on prod. It is Issue 2 below.

⚠️ Arm ORDER is load-bearing and nothing enforces it but a test. `multiIf` takes
the first match, every arm is valid SQL in any order, and a reordering relabels
whole populations on the wire without failing at compile, query or render time.
The three byte offsets are asserted.

**The shared day-set predicate, and the three sites it must stay equal across.**
`external_rate_day_pred` is the one definition of "an imported rate exists for
this bucket's UTC day", spliced verbatim into `reset_pending_pred` (the loop's
termination test), `reset_sql` (the statement that zeroes) and — through
`repair_target_pred` — `repair::months_with_zeros` (the month enumeration).
Those three disagreeing is what hid [[0182]] for a month: the driver enumerated
`CANDIDATE_PRED`, every row 0182 targeted had `close_usd > 0`, so `--dry-run`
reported "no months with enrichable zeros" — a green all-clear over 44,657 wrong
values. A test asserts the same fragment appears in all three.

It is an **uncorrelated** `IN (SELECT …)` because `months_with_zeros` splices
the predicate into the bare `WHERE` of its own grouped scan, where no outer
alias is in scope: a correlated `EXISTS` or a JOIN is a syntax error there, not
a style choice. The fragments carry BARE column names rather than `p.`-qualified
ones so the string is literally identical in all three renderings, and "the same
string" is the only form of agreement a test can prove.

The grain is the UTC day, which for `_1w`/`_1M` is deliberately NARROWER than
the tier's own rule. The reset skipping a refillable row costs one stale value a
later run can still fix; the reset zeroing an unrefillable row is the incident.
Only one of those two errors is recoverable, so the predicate is biased toward
it.

**The two new `UsdResetSpec` fields and the bounded guard.** `not_after:
Option<u32>` and `require_external_rate: bool`, both APPENDED so a spec that
asks for neither renders byte-identically to the pre-0268 statement — proven
against the existing `usdt_reset()` fixture, because the 0182 path has already
run against production and must not change by a byte.
`assert_reset_not_shadowed_by_oracle` now counts `oracle_prices` rows inside
`[not_before, not_after)` only. The guard exists because the oracle tier runs
first and wins, so it only needs to refuse where the oracle tier can actually
REACH; USDC has held live oracle rows since 2026-03-11, and an all-time count
refuses every reset of the 2020–2025 history those rows cannot touch. With
`not_after = None` it still counts to the end of time, so 0182 keeps its
all-time refusal. The error message now names the window it counted over —
without it the operator sees a number with no denominator and cannot tell
whether widening or narrowing is the fix.

**The fourth refusal.** `ResetRequiresExternalRates`: when
`require_external_rate` is asked for and `usd_rate` holds zero `external` rows,
the pass returns an error rather than running. An error, never a `warn!` — with
no external rows the day-set predicate matches nothing, so the run would report
a clean, healthy, entirely empty repair, which is the same false all-clear that
hid 0182. It is placed last of the four because it costs one more count and only
applies to this mode.

**Tests.** Every new SQL invariant has a pure SQL-string unit test that runs
under a bare `cargo test` with no ClickHouse (CI runs exactly that): 11 for the
tier, 6 for the reset, 5 for the read-time label, 1 for the epoch constant. The
behavioural tests are `#[ignore]`d as the harness requires — 3 for the tier, 4
for the reset, 1 for the API labels. `tests/post_run_0268_it.rs` is new and is
the operator's after-check, expected to fail until the prod pass has run.

**Files.** `ch_enrich.rs`, `bin/coarse-repair.rs`, `tests/ch_enrich_it.rs`,
`tests/post_run_0268_it.rs`, `prices-clickhouse/src/lib.rs`, `schema/init.sql`,
`schema/views.sql` (comment-only), `prices-api`'s `queries_ch.rs`, `dto.rs`,
`handlers.rs`, `openapi/descriptions.rs`, `tests/ohlcv_it.rs`,
`docs/database-schema/database-schema-overview.md`,
`docs/runbooks/repair-coarse-usd-values.md`.

**Review round 1** (`gsd-code-reviewer`, standard depth, 2026-09-07: 1
critical, 7 warnings, 7 info — all closed in one commit, every fix with a test
that would have caught it). **CR-01** `bucket_end_expr` floored every
sub-daily grain to `+ 3600`, so a `_1m` candle at 23:30 UTC resolved to the
NEXT day's rate — each fixed grain now uses its own width, an all-grain
cross-check test pins it, and a `_1m` fixture at 23:30 with two daily rates
proves it. **WR-01/02** the operator falsifier could not deserialise its own
`Nullable(Float64)` aggregate and its `±0.04` band contained 1.0 — rewritten
around a typed row (`rate`, `rows`, `zeros`), a per-grain ceiling par cannot
satisfy, a `close_usd = 0` count (the 0182 outcome), and the judgement
extracted into a pure fn with six CI-run unit tests. **WR-03** `peg` is back
in the `Candle.method` documentation (OpenAPI and DTO), scoped to the USDC
self-series, with a test over `FIELDS`. **WR-04** the external tier's
candidate set is bounded by `USDC_ORACLE_EPOCH_S`, so the population it
writes and the population the label arm calls `external` are identical by
construction. **WR-05** `UsdResetSpec::validate()` refuses an empty
`[not_before, not_after)` window, called from `reset_step` AND from the CLI
before any connection — the driver enumerates months with the same predicate
before `reset_step` runs, so a library-only check would never fire. **WR-06**
both USD columns are recomputed from the one reference (Emerged 16).
**WR-07** `addDays`/`addWeeks`/`addMonths`/`toDate` carry an explicit `'UTC'`.
**IN-03** a comment: the read side needs `GROUP BY rts` because it may widen
its `method` filter; here `method` is pinned and last in the sorting key, so
`FINAL` yields one row per `rts` — not a correctness risk while the filter is a
single equality. **IN-05** blank line. **IN-06** the runbook hand-types the
epoch once, as `SET param_epoch`; every query reads `{epoch:UInt32}` and a
unit test pins the literal to the constant. **IN-07** table and span are one
array of pairs. **IN-01** left as is: the negative assertions plus the two
exact-equality checks already prove the 0182 shape; a full-string pin of
`reset_sql` would fail on every whitespace edit for no added proof. **IN-02**
and **IN-04** are scope decisions, recorded as Issues 7 and 8.

## Design Decisions

### From Plan

1. **D-01 — a new external tier**, after the oracle tier and before peg-pivot;
   oracle values always win. Rejected: inserting the import into
   `oracle_prices` under a pseudo oracle name (labels an import `oracle`, which
   [[0247]] forbids, and defeats the reset guard); view-only computation (that
   is the [[0267]] state this task exists to end).
2. **D-02 — the vocabulary split.** `external` = the imported rate was the
   input; `assumed-par` = the literal 1.0 was the input; `peg` reserved for
   USDC's own series, where 0165's "no measured rate was available" is still
   exactly what happened.
3. **D-03 — build against fixtures; the prod run is an operator step.** The
   tool refuses on 0 external rows rather than running and doing nothing.
4. **D-04 — reset scope**: USDC quote leg, below the oracle epoch,
   `close_usd = close` exactly, AND an external row exists for the bucket's day.
5. **D-05 — the label is reconstructed at read time.** Candles store no
   `method`; the `multiIf` gains a timestamp arm keyed on one named epoch
   constant. Rejected: a `usd_rate` join in `/ohlcv` (hot read path); a `method`
   column on six candle tables plus the rollup chain (out of scope).
6. **D-06 — `UsdResetSpec` gains `not_after`**; the oracle-shadow guard counts
   inside `[not_before, not_after)`; the external predicate is expressed once
   and reused in all three sites. Settled as a field on the existing type, not a
   sibling.
7. **D-07 — the rate resolves at the bucket's END**, not "the bucket's UTC day".
8. **D-08 — a SQL-string unit test for every new invariant**, because CI runs a
   bare `cargo test` with no ClickHouse. The second vocabulary document is
   `docs/database-schema/database-schema-overview.md` (BRIEF §6 named
   `docs/prices-api-general-overview.md`, whose two `method` hits are about API
   Gateway per-HTTP-method caching and are unrelated). `views.sql:521,728` and
   `queries_ch.rs:960` belong to [[0267]] step 2.

### Emerged

9. **`USDC_ORACLE_EPOCH_S` lives in `prices-clickhouse`, not beside
   `usdc_identifier()`.** D-05 asked for the latter, which is `handlers.rs:706`.
   Placed in `prices-clickhouse` beside `USDC_ISSUER` instead, because
   `usdc_identifier()` is itself built from `prices_clickhouse::USDC_ISSUER`,
   `handlers.rs` depends on `queries_ch.rs` and not the reverse, and
   `enrichment-worker` — which needs the same value for `--reset-not-after`'s
   default — cannot read a private fn in the API crate at all. The INTENT (one
   named constant, beside the USDC identity, read by both sides) is honoured
   more strongly here than in either alternative. A cross-reference comment
   above `usdc_identifier()` points at it. Its test derives the instant from
   civil-date arithmetic rather than re-typing the literal, so a mistyped digit
   cannot pass by appearing on both sides of an `assert_eq!`.
10. **The staleness bound is derived, not a `ChEnrichConfig` knob.** The
    research proposed an `external_window_s` field mirroring `pivot_window_s`.
    Argument for deriving it is in Implementation Notes above; the short form is
    that `pivot_window_s` needs a runtime refusal *because* it is configurable,
    and a derived bound cannot be set wrong. `coarse-repair` carries an
    assert-shaped guard over the derivation so the refusal already exists if
    someone later makes it a knob.
11. **`assumed-par` kept as D-02's spelling.** It is the only candidate that
    names the INPUT rather than the outcome, which is how `external`, `oracle`
    and `traded` are all defined. `method` is a plain `String` on the wire, so
    the hyphen costs nothing.
12. **`ChPassStats` deliberately NOT given a per-tier external counter.**
    `oracle_misses` keeps its documented meaning (candidates the ORACLE tier
    could not cover) and the `enrichment-worker::metrics` mapping stays
    byte-identical, so no CloudWatch metric changes shape. A per-tier count is
    its own task if it is ever wanted — see Issues 6.
13. **`reset_pending_pred` and `repair_target_pred` gained a `db` parameter.**
    The plan expected `repair_target_pred` to need no change because it composes
    `reset_pending_pred`; in fact neither fn had a database in scope, and the
    day-set subquery must qualify `{db}.usd_rate`. Relying on the session's
    default database instead would have been the only alternative, and it would
    have made these two the *only* statements in the module that do. Both
    callers already had the database to hand. The plan's actual intent — one
    definition, composed, so the three sites cannot drift — is unchanged.
14. **A vacuously-true assertion in `ohlcv_it.rs` was repaired, not left.**
    `ohlcv_peg_signature_on_a_pivot_leg_is_not_labelled_peg` asserted
    `method != "peg"` on a candle-path response. After the rename that guard can
    never fire — a regression labelling an XLM leg would emit `assumed-par`. The
    assertion for the new spelling was added beside it. Not a behaviour change;
    a guard that had quietly stopped guarding.
15. **The existing runbook appendix was retitled "Appendix A".** The new one is
    Appendix B and refers to it by name nine times; leaving the original as a
    bare "Appendix" would have made every one of those references dangle.
16. **The external tier recomputes BOTH USD columns from the one reference
    (review WR-06, option B) — a deliberate departure from the BRIEF §5
    mechanism.** The BRIEF named the write-once `volume_quote_usd > 0` guard as
    how "oracle values win". Keeping it would leave a row enriched before
    `close_usd` existed (`volume_quote_usd = volume_quote × $1`, `close_usd =
    0`) with `close_usd` at 0.9681 beside a `volume_quote_usd` at par — the
    incoherent row `reset_sql`'s own doc block rejects — or, under the
    reviewer's option A (skip those rows), hand them to the peg tier AFTER the
    reset step has already run, so the operator's one-shot campaign would end
    with rows carrying the par signature and AC 1 would fail after a run the
    runbook says to do once. The invariant survives by a different mechanism:
    WR-04's epoch bound. `USDC_ORACLE_EPOCH_S` is by definition the first
    oracle row for canonical USDC, so no candidate row can carry an oracle-set
    value, and the reset run's oracle-shadow guard refuses the run outright if
    that premise is ever false on prod. The behavioural test
    `external_tier_never_overwrites_a_candle_the_oracle_tier_priced` still
    proves the outcome; `external_tier_recomputes_a_half_priced_row_from_the_one_reference`
    proves the coherence.

## Issues Encountered

1. **AC 4 cannot close from this branch.** "The view and stored `close_usd`
   agree in deep history; the [[0267]] `backfill_note` caveat removed" needs
   `views.sql:521,728` and `queries_ch.rs:960` widened from `method = 'oracle'`
   to include `'external'`. D-08 assigns those three lines to [[0267]] step 2,
   and this branch does not touch them. **For Adam to route.**
2. **The label arm's soundness is an assumption, not a measurement.** Arm 2
   reports a scaled pre-epoch USDC leg as `external` on the premise that prod
   holds no `oracle` row for USDC before `USDC_ORACLE_EPOCH_S`. That is in-repo
   prose (`queries_ch.rs:763`, `views.sql:81`), never verified against the
   cluster. Appendix B precondition 3 is the query. If it comes back non-zero,
   some genuinely oracle-priced candles will be labelled `external`.
3. **[[0267]]'s daily-row stamping convention is assumed, not verified** — it
   cannot be until 0267 ships. The tier resolves at the bucket's end with
   `rts < bend`, so a day-START stamp gives every bucket in a day that day's
   rate; a day-END convention resolves every bucket to the PREVIOUS day's rate,
   producing entirely plausible numbers. Appendix B precondition 2 and the
   integration test `external_tier_prices_a_usdc_leg_from_the_measured_rate`
   (whose fixture stamps at day start, and says so) are the two checks.
4. **BRIEF §6 phase 5's `analysis/guardrails.py` re-run is not actionable
   here.** That script is not in this branch's working tree — `analysis/` does
   not exist on it; it lands with PR #289 / `feat/0265`. It is an operator step
   after that PR merges. **For Adam to route.**
5. **The uncorrelated `IN (SELECT …)` shape has not been executed against a
   ClickHouse in this environment.** No container was available and
   `localhost:8123` did not answer, so every ClickHouse-touching test here is
   `#[ignore]`d — which is what CI runs anyway. The `#[ignore]` tests and
   Appendix B's dry run are the proofs. An illegal predicate shape errors
   loudly rather than silently, which is the acceptable failure mode; a wrong
   *result* would not be, which is why the dry-run STOP condition is written
   the way it is.
6. **No per-tier row count exists for the external tier.** `rows_enriched` is
   `candidates_before - remaining` across all three tiers, so a run cannot
   report how many candles the imported rate priced versus how many fell to the
   peg. Deliberate (Emerged 12 — it keeps the metric mapping unchanged), but it
   means Appendix B's abort signal is a whole-pass figure. **For Adam to route
   if the breakdown is wanted.**
7. **Three `usd_rate` reads are unbounded by `time_window` (review IN-02).**
   Constraint §5 says every statement is bounded by the month partition. The
   candle side of every new statement is; the `usd_rate` side is not, in three
   places: the day-set subquery inside `external_rate_day_pred` (re-run on
   every batch of `count_reset_pending` and inside every `reset_sql`), the
   ASOF reference in `external_sql`, and `assert_external_rates_are_loaded`
   (once per month of the campaign). The reference side being unbounded is
   correct and deliberate — a month's first buckets need an anchor in an
   earlier partition, `pivot_sql`'s proven shape. The other two are unbounded
   for simplicity: `usd_rate` holds ~2,000 `external` rows for USDC, the
   identity tuple is a sorting-key prefix, and the scans are negligible.
   **Recommendation:** leave as is and record the constraint's letter and the
   code as intentionally divergent here; revisit only if 0267's series grows
   by orders of magnitude (hourly for all of history), at which point the
   day-set subquery could take `toYYYYMM(timestamp) IN (month, month - 1)`.
   Not changed in this task.
8. **`not_after` bounds the bucket START, so a `_1w`/`_1M` bucket can straddle
   the epoch (review IN-04).** The March-2026 monthly bucket starts 2026-03-01,
   below `USDC_ORACLE_EPOCH_S`, so it is eligible for reset; ten of its days
   sit above the epoch, where the oracle priced things. If it carries the par
   signature it is zeroed and repriced from the last `external` row — a
   measurement, and the label arm keys on the same bucket start so wire and
   store agree — but a blend the operator did not ask for. Narrow; the
   `_1d`-and-shorter grains cannot straddle. Related: `external_window_s` for
   `_1M` is 31 days, so a sparse series lets a monthly bucket resolve to a rate
   at the bucket's START while still passing the staleness filter — harmless
   with a daily series, a trap if 0267's series ever has month-long gaps.
   **Recommendation:** for `_1w` and `_1M`, end the campaign at
   `--end-month 202602` (now in Appendix B) and inspect the one straddling
   bucket by hand if it matters; do not move the bound to the bucket END —
   that would make the reset and the label disagree, which is the worse
   error. Not changed in this task.

## Run-day checklist (operator)

Nothing below can be done from the branch. Work Appendix B of
`docs/runbooks/repair-coarse-usd-values.md` and use this as the index.

1. **Wait for [[0267]]'s `external` rows on prod.** Appendix B precondition 1.
   A count of 0 is a hard refusal, so there is no way to start early by mistake.
2. **Settle the stamping convention** — precondition 2. If 0267 stamps daily
   rows at day END rather than day START, stop: every bucket would resolve to
   the previous day's rate. This is Issue 3.
3. **Confirm no pre-epoch `oracle` row for USDC** — precondition 3. This is
   Issue 2, and it is the one check that validates the `/ohlcv` label rather
   than the stored value.
4. **Confirm the cleanup worker is still dark** (precondition 4) and that the
   **FREEZE snapshots exist and were verified** (precondition 5).
5. **Record the baseline**, per table, including the `native` 2023-03-11 implied
   rate — which must read exactly 1.0 before the run.
6. **Dry run each of `_1h`, `_4h`, `_1d`, `_1w`, `_1M`.** ⚠️ Zero candidate
   months is a STOP, not an all-clear. Expect the order of magnitude 0247
   measured: ~654,291 candles.
7. **Real run, one table at a time.** Abort if `rows_reset` far exceeds
   `rows_enriched`; roll that table back from its snapshot before touching the
   next.
8. **After-check**: the SQL falsifier per granularity, plus
   `cargo test -p enrichment-worker --test post_run_0268_it -- --ignored`. This
   closes **AC 1 and AC 2**.
9. **Record runtime and rows touched in this file**, as [[0182]] did. This
   closes **AC 5**.
10. **Re-measure [[0266]]'s dislocation table** and record the result there.
    This closes **AC 6**.
11. **AC 4 stays open** until [[0267]] step 2 widens the three
    `method = 'oracle'` filters — Issue 1.
