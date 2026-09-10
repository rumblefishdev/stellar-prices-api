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

**Review round 2** (`gsd-code-reviewer`, standard depth, cross-file,
2026-09-07, on `9732c07`: 1 blocker, 3 warnings, 7 info — one commit, every
fix with a test that would have caught it). **CR-02** the falsifier's `_1w`
and `_1M` ceilings (0.999 / 0.9995) came from a "blending" model decision G
does not produce: ONE rate at the bucket END prices the whole bucket, the
March-2023 week ends 03-13 and the month 04-01, so a correctly repaired `_1M`
reads ~1.0 and the gate reported a false failure with a FREEZE rollback
advised beside it. `_1d` is the coarsest grain that can carry the depeg;
`_1h`/`_4h`/`_1d` keep a 0.99 ceiling, `_1w`/`_1M` are judged by the
MECHANISM — `max(version)` past the value the runbook's "before" step now
records (handed in as `POST_RUN_0268_VERSION_BEFORE_1W`/`_1M`, missing = a
finding), no zero with volume, and the par signature only if the series'
bucket-end rate is exactly 1.0. That last clause is the label ambiguity of
Issue 9. Every "blends N days" sentence is gone from the module doc and the
runbook; `judge` stays pure, 13 CI-run tests. **WR-08** `zeros` now mirrors the
tiers' predicate (`close_usd = 0 AND volume_quote > 0`); zero-volume rows are
`unpriceable`, reported as context, never a failure, never masking the rate
check. **WR-09** the "oracle wins" argument named the wrong table: the epoch
is defined on `usd_rate`, the oracle tier reads `oracle_prices`, and the copy
runs behind a watermark. Doc block and Design Decision 16 now name
`oracle_prices` and state the residual exposure; Appendix B precondition 3
queries `oracle_prices` and is BLOCKING; and the reset mode refuses on the same
count (`assert_no_pre_epoch_oracle_rows`, `ResetBlockedByPreEpochOracleRows`),
unbounded below because the external tier's reach is — SQL-string unit test
plus an `#[ignore]` fixture with the reading below `not_before`. The recompute
(option B) stays. **WR-10** two `#[ignore]` fixtures execute the `'UTC'`
calendar branch on the tables the campaign targets: a `_1d` bucket at day
start with rates at day start and +1d (the +1d rate must not win), and a
`_1M` bucket with rates on 09-01, 09-30 and 10-01 (09-30 must win; a 31-day
end would pick 10-01). `every_timezone_sensitive_expression_pins_utc` now
says it inspects the two 0268 builders only — Issue 10. **IN-08** a unit test
walks `GRAINS` and pins the zero-slack pairing (`max(width, 1 day) <=
external_window_s`). **IN-09** one `pub const GRAINS` next to `bucket_width_s`;
the cross-check iterates it, so a grain missing from `bucket_end_expr` fails.
**IN-10** the "intraday buckets average lower still" comment replaced with
the true statement (exactly 0.9681 against a daily series; lower only if 0267
ships hourly rows). **IN-11** the `PARTIAL` assertion is real: par and a
partial pass now produce different findings. **IN-13** the `Candle.method`
test reads the labels off `usd_method_expr`'s rendered `multiIf`
(`pub(crate)` now) instead of a hand list. **IN-14** `--reset-not-after` moved
out of the copy-pasteable flag block into the prose. **IN-12** left, recorded
as Issue 11.

**Review round 3** (`gsd-code-reviewer`, 2026-09-07, on `df6fdf6`: 0
blockers, 1 warning, 4 info — verdict "ready for PR"; the nine invariants
re-proved on the final tree). **WR-11** the CI test for the version-baseline
variable mutated the process environment with `unsafe set_var`, which under
`--include-ignored` could race the operator's after-check and hand `_1M` a
fake baseline; the name derivation and the parser are now two pure functions
tested without touching the environment. **IN-17** the scheduled Lambda's
external tier runs without the pre-epoch guard the reset run has; the doc
block now says so and why precondition 3 covers it. **IN-15** (`max(version)`
proves the pass reached the bucket, not that it covered every row), **IN-16**
(runbook "before" queries and `measure` aggregate over different row sets)
and **IN-18** ("unset" and "unparsable" share one finding) are recorded here
and left as they are: the first two are properties of the mechanism check by
design, the third is wording.

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
    runbook says to do once. For `close_usd` the invariant holds by the
    candidate filter alone (an oracle-priced row is never `close_usd = 0`).
    For `volume_quote_usd` it rests on WR-04's epoch bound — with a caveat
    review round 2 (WR-09) made explicit: `USDC_ORACLE_EPOCH_S` is defined
    against `usd_rate`'s first `oracle` row, but the oracle tier reads
    **`oracle_prices`**, and `usd_rate`'s oracle rows are copied out of it
    behind a watermark, so `oracle_prices` may hold an earlier USDC reading.
    The residual exposure is exactly one shape: a row enriched before
    `close_usd` existed, whose `volume_quote_usd` the oracle tier set from
    such a reading, is a candidate and has that column recomputed from the
    bucket-end rate; its two USD columns then agree, which is the property
    wanted. On the operator's campaign the exposure is zero by measurement:
    the 0268 reset mode refuses (`ResetBlockedByPreEpochOracleRows`) if
    `oracle_prices` holds ANY canonical-USDC reading below the epoch, a count
    independent of `--reset-not-before` because the tier's reach is too, and
    Appendix B precondition 3 is the same query, now blocking. The behavioural
    test `external_tier_never_overwrites_a_candle_the_oracle_tier_priced`
    still proves the `close_usd` outcome;
    `external_tier_recomputes_a_half_priced_row_from_the_one_reference`
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
9. **The read-time label is ambiguous at exactly par — a known limitation of
   decision E, not a repair defect (review round 2, CR-02).** The label arm
   keys on the signature `close_usd = close`. A bucket whose external rate at
   the bucket end is *exactly* 1.0 — common for USDC on calm days, and the
   likely value for the March-2023 `_1M` bucket, which ends on 04-01 — is
   priced correctly by the external tier and then reported `assumed-par` by
   `/ohlcv`, because a measured 1.0 and an assumed 1.0 leave the same bytes.
   **Decision: no tolerance on the `multiIf`.** A tolerance cannot separate
   the two either (both produce exactly `close_usd = close`); it would only
   relabel near-par measured rates as `assumed-par` too, widening the
   ambiguity rather than closing it, so the arm stays an exact comparison and
   nothing was widened silently. The falsifier tolerates the case because it
   can read the series (`bucket_end_rate == 1.0`); the wire cannot. Closing
   it needs stored provenance — a `method` column on the candle tables, which
   D-05 put out of 0268's scope — or a documented reading of `assumed-par` on
   a pre-epoch USDC leg as "the input rate was 1.0, assumed or measured".
   **For Adam to route**; the OpenAPI text was not changed here.
10. **`repair::months_with_zeros` derives its per-month windows in the SERVER
    zone (review WR-10).** `toStartOfMonth` / `addMonths` / `toYYYYMM` there
    are unpinned, matching the tables' `toYYYYMM(timestamp)` partition key,
    which is server-zone too. The reset and the refill share one such window
    inside a month pass, so they stay paired, and pinning the enumeration to
    UTC while the partition key is not would split months across partitions
    on a non-UTC server. The unit test `every_timezone_sensitive_expression_pins_utc`
    now states that it inspects the two 0268 builders only. Not changed; the
    right fix, if any, is a server-zone assertion at startup, which is the
    0114 driver's concern.
11. **`the_runbook_hand_types_the_oracle_epoch_once_and_it_is_the_constant`
    `include_str!`s the runbook from two directories above the crate (review
    IN-12).** It is the only thing keeping IN-06 closed, and it fails to
    COMPILE the crate's tests if the runbook moves, and fails if a log excerpt
    containing the epoch is ever pasted into the runbook. Left as is: the
    crate is not published, the repo's other cross-file guards live in
    `tools/scripts/` and are not run by `cargo test`, and a pasted second copy
    of the epoch is exactly the drift the test exists to refuse. Revisit if
    the runbook is ever split.

## Run-day checklist (operator)

Nothing below can be done from the branch. Work Appendix B of
`docs/runbooks/repair-coarse-usd-values.md` and use this as the index.

1. **Wait for [[0267]]'s `external` rows on prod.** Appendix B precondition 1.
   A count of 0 is a hard refusal, so there is no way to start early by mistake.
2. **Settle the stamping convention** — precondition 2. If 0267 stamps daily
   rows at day END rather than day START, stop: every bucket would resolve to
   the previous day's rate. This is Issue 3.
3. **Confirm no pre-epoch `oracle_prices` reading for USDC** — precondition 3,
   now BLOCKING and mirrored in the tool (`ResetBlockedByPreEpochOracleRows`).
   This is Issue 2, and it validates both the `/ohlcv` label and the external
   tier's `volume_quote_usd` recompute (Design Decision 16).
4. **Confirm the cleanup worker is still dark** (precondition 4) and that the
   **FREEZE snapshots exist and were verified** (precondition 5).
5. **Record the baseline**, per table, including the `native` 2023-03-11 implied
   rate — which must read exactly 1.0 before the run — and, for `_1w` and
   `_1M`, the `max(version)` of the bucket containing the depeg day. The
   after-check takes those two as `POST_RUN_0268_VERSION_BEFORE_1W` / `_1M`
   and refuses to pass without them.
6. **Dry run each of `_1h`, `_4h`, `_1d`, `_1w`, `_1M`.** ⚠️ Zero candidate
   months is a STOP, not an all-clear. Expect the order of magnitude 0247
   measured: ~654,291 candles.
7. **Real run, one table at a time.** Abort if `rows_reset` far exceeds
   `rows_enriched`; roll that table back from its snapshot before touching the
   next.
8. **After-check**: the SQL falsifier per granularity, plus
   `cargo test -p enrichment-worker --test post_run_0268_it -- --ignored` with
   the two recorded versions in the environment. `_1h`/`_4h`/`_1d` are judged
   by the rate (under 0.99); `_1w`/`_1M` by the mechanism (version moved, no
   zero with volume, par signature only if the series says exactly 1.0 at the
   bucket end). This closes **AC 1 and AC 2**.
9. **Record runtime and rows touched in this file**, as [[0182]] did. This
   closes **AC 5**.
10. **Re-measure [[0266]]'s dislocation table** and record the result there.
    This closes **AC 6**.
11. **AC 4 stays open** until [[0267]] step 2 widens the three
    `method = 'oracle'` filters — Issue 1.

---

## Review round 4 (2026-09-09) — verified against a live ClickHouse

Every finding below was checked against ClickHouse **26.3.10.60**, the pinned
production version. The `#[ignore]`d suites this branch relies on had never been
run: three were red, for one root cause.

### 🔴 The campaign was a no-op

`reset_sql` appended the bare predicate `close_usd = close` into a SELECT whose
own projection declares `CAST(0 AS Decimal(38, 14)) AS close_usd`. ClickHouse
resolves identifiers against SELECT aliases **before** table columns
(`prefer_column_name_to_alias` defaults to 0), so the term evaluated as
`0 = close` and matched nothing.

The failure is silent by construction: `count_reset_pending` renders the same
fragment into an alias-free statement, resolves the real column, and reports the
full 654 291-row population; the reset then writes 0 rows, logs "USD reset made
no progress", and the run exits 0. An operator following the runbook would have
seen a clean pass over unchanged values.

Verified directly:

```sql
SELECT CAST(0 AS Decimal(38,14)) AS close_usd, p.close AS close
FROM (SELECT toDecimal128(5,14) AS close_usd, toDecimal128(5,14) AS close) AS p
WHERE close_usd = close      -- 0 rows: evaluated 0 = 5
WHERE p.close_usd = p.close  -- 1 row
```

Fixed by qualifying in `reset_sql` only; `reset_pending_pred` and the month
enumeration keep the bare form (no `p` in scope — the prefix would be a syntax
error). Three integration tests went red→green:
`the_external_reset_never_zeroes_a_bucket_it_cannot_refill` (0 != 1),
`the_bounded_usd_reset_is_not_refused_by_oracle_rows_above_its_window`,
`the_external_reset_touches_only_the_bounded_month`.

⚠️ The string invariant could not catch this: it asserted the two fragments are
**equal**, which is exactly what must not hold. Replaced with a test that pins
the qualified form in one site and the bare form in the other.

### 🔴 `assumed-par` was a value signature, not provenance

`usd_method_expr`'s first arm read `close_usd = close`. That tests the OUTCOME
and reports it as the INPUT. **174 of the 2049 days in 0267's imported series
close at exactly 1.00000000**, so ~8.5% of the re-enriched population would
carry a MEASURED rate under an assumption label — while `dto.rs` and the
published OpenAPI text promise the opposite in as many words. Documentation
right, SQL wrong. This is Issue 9's read half, and it was in scope all along.

The `external` arm now asks whether an imported rate covers the bucket's UTC day
— the same day-set `external_rate_day_pred` resets on — and is tested before the
par signature. Measured on the seeded rows, old vs new label:

| bucket | old | new |
|---|---|---|
| 2023-03-11 (measured 0.96812) | `external` | `external` |
| 2024-06-01 (no imported rate) | `assumed-par` | `assumed-par` |
| **2025-01-15 (measured exactly 1.0)** | **`assumed-par`** | **`external`** |
| 2026-03-11 16:00 (post-epoch) | `oracle` | `oracle` |

A bare `quote_asset_id = usdc -> 'oracle'` arm also went: pre-epoch it reported
a poll that provably did not exist. That state now reports null.

**One residual, documented on the wire rather than hidden:** day coverage is not
bucket coverage, so a bucket on a covered day whose own staleness window found
no rate falls back to $1 and still reports `external`. No read-side expression
can separate those two cases — the candle rows carry no provenance column. See
Future Work.

### 🟠 Also fixed

- `assert_reset_not_shadowed_by_oracle` gained a lower bound at `not_before`,
  but the oracle tier forward-fills a reading up to `window_s` old, so a row
  just below the floor still re-prices candles just above it — the
  re-apply-and-relabel this guard exists to refuse. Floor widened by `window_s`;
  only the upper bound needed to be window-scoped. New test, falsified.
- `--reset-quote-asset-id <USDT> --reset-require-external-rate` was accepted.
  Every site on the external path is USDC-only, so it would zero USDT rows on
  USDC's rate days and leave the peg tier to write $1 back over all of them —
  0182 under a different quote asset. Refused in the library.
- `external_rate_day_pred` admitted a day on any `external` row while
  `external_sql` filters `r.usd > 0` after the ASOF; the day-set now requires a
  positive rate. (0267's loader already rejects a non-positive rate, so this is
  belt-and-braces, not a live defect.)
- Runbook precondition 2 expected `1970-01-01` from `toTime`, which pins to
  **1970-01-02** — it would have sent the operator to STOP on a correct series.
  Now `formatDateTime(timestamp, '%H:%i:%S', 'UTC')`, expecting `00:00:00`.
  (`%M` is the month name in ClickHouse: `'%H:%M:%S'` prints `00:March:00`.)

### Open

- **654 291 is a measured constant, not a query.** It comes from 0247/0168 and
  is repeated in the title, this file, the runbook and two code comments, but
  nothing re-derives it and the runbook carries no re-measuring query. If the
  population has moved since, the abort signals compare against a stale figure.
- Implementation §1 promises the enrichment "records which one it used"; nothing
  is recorded. §2 promises re-enriching `vwap` where derived; `external_sql`
  carries `p.vwap` through unchanged. §5 describes a shadow-column rollout; the
  branch does a versioned in-place re-insert with FREEZE/`ATTACH PARTITION` as
  rollback. The plan changed and the text did not.
- **ADR 0011 §4 says "no fourth word is coined for the same concept on a third
  endpoint" and that `method` stays `traded`/`peg`/`oracle`, additive.** This
  branch coins `assumed-par` and `external` and retires `peg` from the candle
  path. The branches are right for money semantics — `peg` conflated an
  assumption with a measurement — but the ADR must be superseded in the same
  merge, or the repo's own settled contract says the wire is wrong. **No ADR file
  is touched by either branch.** This needs a human decision and an ADR on
  `develop`; it is not something to slip into a feature branch.

### Future Work → to spawn on `develop`

- **Store the pricing tier on the candle row** (Issue 9, complete form). The
  read path reconstructs `method` from the quote asset, the bucket timestamp and
  the imported rate's day coverage. That is now truthful in the common cases and
  still cannot attribute a source (`Candle.source`/`quality` are NULL on every
  quote leg) or separate a covered-day peg fallback from a measurement. A
  `LowCardinality` provenance column on `price_ohlcv_*`, written by every tier,
  is the only complete fix. Needs its own rollout plan over 654k+ rows and a
  wire-contract change — not a rider on this task.

### Design decisions — Emerged

- **Day-set lookup over a stored column, for now.** The complete fix is the
  column above; it is a schema change plus a re-enrichment of every candle, on a
  branch already carrying a 654k-row campaign. The day-set removes the
  systematic 8.5% error today, at one uncorrelated subquery per query, and the
  residual is stated in the OpenAPI text.
- **Null over `oracle` for the unexplained pre-epoch state.** A null says "no
  USD provenance to report", which `dto.rs` already defines; `oracle` would have
  been a claim.

### Round 4, second pass — three defects in the round-4 fixes

Re-reviewed adversarially against the live ClickHouse. The first pass introduced
two defects and left one hole:

- **The day-set asked about the bucket's first day** while the external tier
  resolves at the bucket END within `max(bucket_width, 1 day)`. Every WEEKLY and
  MONTHLY USDC candle priced from a mid-period rate — which
  `external_rate_day_pred` documents as normal — reported `method: null`.
  Under-reaching is the safe direction for a write and a wrong answer for a
  read. The arm now tests the whole window, rounded out to days.
- **The par arm was not epoch-bounded**, so a post-epoch poll reading exactly
  1.0 reported `assumed-par` — the same defect this round removed, on the other
  side of the epoch. `peg_sql` has no epoch bound either, so post-epoch par is
  genuinely produced by both tiers. Bounded.
- **`external_sql`'s ASOF source still admitted a non-positive rate.** The
  day-set gained `usd_rate > 0`; the subquery the ASOF picks from did not, and
  `r.usd > 0` runs only after the pick. A zero rate mid-day would claim the day,
  let the reset zero every later bucket, be dropped by the post-filter, and
  leave the peg tier to write $1 back — 0182 re-created by the commit meant to
  prevent it.
- **CI was red**: `cargo clippy -- -D warnings` failed on a constant read only
  by tests. Now `#[cfg(test)]`, gate passes.
- The oracle-shadow refusal reported `[not_before, …)` while scanning one window
  lower — an operator checking it would get zero rows for a real refusal.

Lesson worth keeping: both defects of this round were **read-side labels that
looked right in the common case**. The tests that "covered" them asserted string
equality of SQL fragments or used fixtures that could not reach the boundary.
Every fix in both passes was falsified against a live ClickHouse before being
believed.

### 2026-09-10 — `/code-review` round, hourly gate, CI, decisions

**The round-4 labelling fix was wrong, and has been reversed.** A `/code-review`
pass found that testing the imported-rate day-set BEFORE the par signature made
two large populations claim a measurement they do not have — both reproduced on
ClickHouse 26.3.10.60 before changing anything:

- every un-repaired pre-epoch candle on a covered day reported `external` while
  still holding `close x $1.00` (522,321 per the prod measurement in
  `queries_ch.rs`) — day coverage says nothing about whether the campaign has
  reached the row;
- every post-epoch peg fallback reported `oracle` (134,193) — a poll that never
  happened.

The par signature now wins at any timestamp and `external` also requires
`close_usd != close`. That reinstates "a measured rate of exactly 1.0 reports
`assumed-par`" — the conservative direction, since the VALUE is identical. The
round-4 note above described the remaining residual as a rare edge case; it was
systematic, and the user's 2026-09-09 acceptance of the residual rested on that
wrong description. The residual that stands now is the conservative one.
Also from that review: the coverage window used `+ INTERVAL`, which ClickHouse
resolves in the SERVER timezone; now UTC-pinned per grain.

**Hourly gate (`ResetRequiresHourlyRates`).** Found while verifying the hourly
path end to end: the reset is one-shot per row — it re-opens only rows still at
`close_usd = close`. On a daily-only load every hour of a covered day is priced
at the day CLOSE and leaves the signature for good, so a later hourly load
cannot correct it. Measured: 2023-03-11 12:00 repaired on a daily-only load
stayed at 0.96812 through an hourly load and a second pass; Chainlink's 12:00
close is 0.90687439. Fresh candles after the hourly load matched Chainlink
exactly at 00:00 / 07:00 / 13:00 / 20:00. A sub-daily table is now refused until
imported rows exist away from UTC midnight (43,046 on the versioned files);
daily and coarser tables are not gated. Runbook Appendix B precondition 1 says so.

**CI** now compiles and lints `coarse-repair` and `load-external-rate`
(`clippy -p enrichment-worker --features aws-mtls --all-targets -D warnings`).
Before, their `required-features = ["aws-mtls"]` kept them out of every CI job.

**Decisions (user, 2026-09-10):**
1. ADR 0011 stays unchanged. The new `method` words are therefore a DELIBERATE,
   accepted deviation from §4 — do not "fix" the code back to the ADR.
2. The labelling residual is accepted (now the conservative one above).
5. The small autonomous choices (the `--allow-daily-after-hourly` escape, local
   merges of 0268 into 0267) are fine.

**Still open:** deploy ordering (run the campaign BEFORE deploying the new API
binary, so no still-$1 value is labelled measured — not yet in the runbook), and
re-measuring the 654,291 population before the run. The `peg` -> `assumed-par`
rename on the candle path is a breaking wire change and needs a release note.
