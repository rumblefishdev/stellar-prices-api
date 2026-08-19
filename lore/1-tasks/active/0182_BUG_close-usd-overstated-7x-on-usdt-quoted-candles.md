---
id: "0182"
title: "44,657 stored candles across 495 assets carry a close_usd ~7.4x too high — the USDT peg fix stops new ones but does not correct history"
type: BUG
status: active
related_adr: []
related_tasks: ["0172", "0196", "0165", "0145", "0111", "0114", "0201", "0208"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "enrichment", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-12
    status: backlog
    who: okarcz
    note: >
      Spawned from 0172. That task fixed the WRITER (USDT moved from the peg
      tier to the pivot tier, so new candles are priced at the measured rate),
      but every close_usd already written through the old peg path is still on
      disk and still ~7.4x too high. Filed separately because correcting it is a
      re-enrichment run with its own risk profile, not a code change.
  - date: 2026-08-13
    status: active
    who: okarcz
    note: >
      Activated. BE answered the two open consumer questions (see "BE answers"
      below): they filter close_usd > 0 and render "--" on a miss, so a zero is
      read as absent rather than as a zero valuation; and historical correctness
      DOES matter to them, for the 30D/1Y pool charts. They have not deployed
      yet, so no consumer has seen the inflated TVL. Advised them not to hold the
      deployment — the two are independent.
  - date: 2026-08-13
    status: active
    who: okarcz
    note: >
      Dry run executed on all five forever-tables and it is NON-VACUOUS — 67
      months each, so the AC is met and the 0114 driver gap is closed. Oracle
      gate re-verified green (0 rows for asset_id 111) and the flat implied_rate
      = 1.0 baseline recorded. But the enumeration is an OR, so the same command
      would ALSO drain ~32M fillable XLM-quoted rows that the 0088 pre-Soroban
      backfill left unenriched below 2024-02 — a second campaign roughly the size
      of all of 0114, and 300x this task's own 567,232 rows. Decision: keep 0182
      scoped to the USDT correction, spawn 0201 for the 32M, RUN NOTHING YET. New
      blocking item: the tiers take no per-quote-leg filter, so 0182 cannot
      currently run without 0201 — routes a/b/c recorded in the task. No FREEZE
      taken, no partition written.
  - date: 2026-08-17
    status: active
    who: okarcz
    note: >
      MECHANISM DECIDED — route (c), sequenced: 0201's pass first (no
      `--reset-*`), then 0182's reset pass over a table already at its floor.
      Taken together with the completion order for the whole chain (0201 -> 0182
      -> 0172 as one campaign). Route (a) was rejected because a combined run
      swamps the `rows_reset ~= rows_enriched` check ~32M against ~357k, and that
      check is the only signal that catches values zeroed and never recomputed;
      route (b) is the better tool but costs a PR before any prod run and would
      leave 0201 outstanding anyway.
      ⛔ THE RUN IS NO LONGER BLOCKED ON THE MECHANISM. What remains before it
      can start: a CH admin must take the FREEZE (67 months x 5 tables;
      `prices_writer` cannot and cannot be granted it), and the operator must
      settle one snapshot before pass 1 vs a second between the passes — one is
      cheaper, two make the passes independently revertible. Execution host is
      fishuser-hero, planned for the morning of 2026-08-18. Still no FREEZE
      taken and no partition written.
  - date: 2026-08-18
    status: active
    who: okarcz
    note: >
      SNAPSHOT COUNT SETTLED — TWO. One FREEZE before pass 1, a second between
      the passes, so pass 2 is revertible without discarding pass 1's ~32M
      recovered rows. Two consequences, both recorded below. The second
      snapshot has to sit BETWEEN the passes, which rules out interleaving them
      per table unless the admin is called back five times — so the ordering is
      now the hybrid: `_1h` alone first as the review gate, then the remaining
      four batched, three admin windows total. And the two snapshots need
      DISTINCT names, because the freeze script deliberately keeps a
      pre-existing snapshot rather than overwriting it, making a re-freeze under
      the same name a silent no-op. In-span footprint measured on prod: 16.82
      GiB across the five tables (799 active parts), so both snapshots peak at
      ~33.6 GiB plus ~4-6 GiB of new parts, ~9% of the 430.6 GiB free. BE
      messaged about the volume. Still no FREEZE taken and no partition written.
  - date: 2026-08-18
    status: active
    who: okarcz
    note: >
      RUN EXECUTED AND VERIFIED — all five forever-tables corrected, from
      fishuser-hero, in roughly four hours rather than the 10-15 h budgeted.
      567,760 rows re-opened and recomputed against the 567,232 sized on
      2026-08-13. The implied rate now tracks USDT's measured value on every
      tier (2026 cluster 0.1494-0.1529, was a flat 1.000000), and on `_1h` the
      monthly series shows par for exactly 15 months then the June 2022 break to
      ~0.13 — matching 0172's independent measurement, and confirming the epoch
      left the pre-depeg par window untouched. Route (c) held: pass 1 drained
      each table to its floor first, so `rows_reset ~= rows_enriched` stayed
      meaningful and every table was checked on it. 18 rows across `_1d`/`_1w`/
      `_1M` came out at close_usd = 0 and tripped the shortfall guard; all 18 are
      dust (close of 0 or below ~4e-14, underflowing at Decimal(38,14)) and none
      had a representable price to lose. That false positive is now documented in
      the runbook. Remaining: BE notification, snapshot cleanup, the guard test,
      and volume_quote_usd.
  - date: 2026-08-19
    status: active
    who: okarcz
    note: >
      RE-VERIFICATION BEFORE RELEASING THE SNAPSHOTS FOUND 157 DESTROYED
      CANDLES, and they were repaired the same day. The epoch 1612656000 is
      2021-02-07 00:00, nineteen hours before the USDT/USDC reference market's
      first candle at 19:00; the pivot joins at-or-before, so every bucket in
      that gap was zeroed by the reset and had nothing to refill from. 121 rows
      on `_1h` and 36 on `_4h`. The three coarser tiers were untouched because
      their reference candle falls in the same bucket as the row being priced,
      and `_1M`'s February bucket is stamped below the epoch entirely — that
      distribution is what confirmed the mechanism. Restored to par by versioned
      insert (par is the measured value for this window per 0172, and what the
      rows held before this task ran); snapshot rollback was rejected because on
      a ReplacingMergeTree it would mean DROP PARTITION across four tables for
      157 rows. at_boundary is now 0 on every tier. The reason 2026-08-18 missed
      it: stranded_with_real_close was run only on the three tables that tripped
      the shortfall guard, which are precisely the three structurally incapable
      of showing this defect. Corrected epoch is 1612724400 and the file's line
      claiming 1612656000 "cannot strand rows" is now marked wrong. Snapshot
      cleanup also completed — all 485 released, shadow/ 31G to increment.txt
      alone, with two procedure corrections recorded (SYSTEM UNFREEZE is
      disabled server-side; the success signal is du, not the entry count).
      Spawned 0208. Remaining: the guard test and volume_quote_usd.
  - date: 2026-08-19
    status: active
    who: okarcz
    note: >
      BE closed the last open question and corrected one of our claims. They do
      NOT read volume_quote_usd — verified against their merged code, the only
      prices surfaces they consume are price_usd_series and price_usd_series_1h
      and the only columns are the identity triple, bucket and close_usd. So the
      scope stays close_usd and the criterion closes as "documented, not
      widened"; since the reset zeroed both USD columns together there is no
      mismatch left in the repaired span, and the pre-epoch window's $1
      volume_quote_usd is correct for that era. Correction to our own record:
      "nothing they deployed ever served the inflated numbers" was wrong. Their
      release shipped before the 2026-08-18 history repair — the list and detail
      pages were never affected because the writer fix preceded their deploy,
      but the 30D/1Y charts on USDT-legged pools did serve the inflated history
      in the interim. Compute-at-read with no caching, so they corrected
      themselves when the repair landed and nothing follows. They also measured
      the 0201 fill from the consumer side (52,580 pools pinned 2026-08-19):
      priceable-ever 51,935 to 52,112 (99.1%), never-priced down to 468,
      priceable-90d 71.0%, 48h flat as expected — external corroboration for
      0201's closure call. USDT exclusion lifted from their Horizon
      cross-validation. Only the re-introduction guard test now remains.
---

# `close_usd` is ~7.4× too high on every USDT-quoted candle ever written

## Measured on prod (2026-08-12)

```
price_ohlcv_1d WHERE quote_asset_id = 111 (canonical USDT):
  candles              44,657
  distinct base assets    495
  span                 2018-05-15 -> 2026-08-12   (i.e. still being written)
  priced (close_usd>0) 44,653
  implied USDT rate     0.999999   <- every one valued at par
```

The correct rate is USDT's measured market value — ~$0.13 in 2026-08, and
varying over time (see [[0172]] for the full monthly series). So each affected
`close_usd` is overstated by roughly `1 / 0.13 ≈ 7.4×`, with the exact factor
depending on the bucket's date.

## Why this is not fixed by 0172

0172 changed how candles are enriched **going forward**: `stable_ids()` no longer
contains USDT, and a second pivot pass prices USDT-quoted candles from the
measured USDT/USDC market. But enrichment only writes rows where
`close_usd = 0` — already-enriched rows are deliberately skipped (that filter is
what makes the pass idempotent and restartable). So the 44,657 wrong values are
inert: nothing will revisit them.

## 🛑 TWO ORDERING CONSTRAINTS — violating either makes this task destructive

Both surfaced in the 2026-08-13 review of 0172's PR #205. Neither is optional,
and neither is visible from inside this task's own plan.

### 1. `oracle_prices` must be purged FIRST ([[0196]])

The enrichment **oracle tier runs before the peg-pivot tier and wins where it
applies** (`ch_enrich.rs:19-22`). [[0196]] measured **46,378 mis-attributed
Reflector rows** on the USDT identity in `prices.oracle_prices`, covering
2026-03 → present and current to the hour.

So zeroing the rows and re-enriching *while those exist* re-applies
`close_usd = close × ~$1.00` to every 2026-03 → 2026-08 USDT-quoted candle and
labels it `method = 'oracle'` — a consumer reads that as **more** authoritative
than the peg placeholder it replaced. Same failure mode already recorded for
[[0168]].

### 2. Pre-2021 candles have NO pivot reference — do not zero them

The USDT/USDC market begins **2021-02-07** (2,011 daily candles). USDT-quoted
candles begin **2018-05-15**. The pivot's `ASOF LEFT JOIN` + `AND r.usd IS NOT
NULL` drops any candle with no reference at or before its bucket, so
**2018-05-15 → 2021-02-07 has nothing to pivot on**.

Those rows currently hold `close × $1` from the old peg path. Zero them and they
stay at `close_usd = 0` permanently — the ambiguous zero read unguarded by ~130
`argMax(close_usd, …)` sites ([[0145]]), which 0172's own rationale argues is
*worse* than a wrong-but-visible number.

⚠️ **And the old `$1` is CORRECT for that window.** 0172 measured USDT at par
from 2021-02 until the June 2022 break, and the depeg is what makes the peg
wrong *after* it — not before. So this is not "wrong data we cannot fix", it is
**right data this task would destroy**.

Options: bound the re-enrichment to ≥ 2021-02-07; or give the pivot a dated peg
epoch (par before the break, measured after). Decide before writing the driver.

⚠️ Also unresolved: `volume_quote_usd` is preserved write-once
(`if(p.volume_quote_usd > 0, …)`), so a row whose `close_usd` this task corrects
keeps a `volume_quote_usd` computed at $1 — the same row then carries two USD
columns that disagree by 7.4×. This task's scope is `close_usd` only; either
widen it or spawn a follow-up.

## ✅ BE answers 2026-08-13 — two open questions closed, one constraint relaxed

Asked BE how they consume `close_usd` and whether history matters. Their reply
settles decisions this task had been holding open.

**1. Nothing on prod has seen the inflated values yet.** BE have *not* deployed
the consuming changes. So this is not an incident with live blast radius — it is
a correctness debt with a window to fix it in. Priority is unchanged but the
urgency framing in [[0172]] ("102 priceable pools silently 7.4× high") describes
what *would* ship, not what has.

**2. A zero reads as ABSENT on their side, not as a zero valuation.** Verbatim:
*"for TVL we take the last `close_usd > 0` value from the last 48h, and if there
was no price for 48h we show an empty `--` TVL."*

⚠️ This **partially relaxes** the pre-2021 constraint above, but does not remove
it. The "ambiguous zero" argument ([[0145]], ~130 unguarded `argMax(close_usd,…)`
sites) is about **our own views**, not BE's client. BE is guarded; we are not.
So zeroing a row is safe *for BE* and still unsafe *for us*. Do not cite this
answer as clearance to zero the pre-2021 window.

**3. Missing hourly rows are harmless.** *"So missing hourly rows don't break
anything on our side."* Removes the pressure that shaped [[0165]]'s peg-fallback
rationale, for this consumer at least.

**4. History matters — for the charts, not the list view.** *"historical
correctness matters for the 30D/1Y charts, so this re-enrichment would be useful
so those USDT pool charts are correct and also show that drop."* The pool-list
TVL takes the last value only, so it has no historical dependency.

→ **This selects the scope.** The deepest surface BE reads is **1 year**. A 1Y
chart never reaches 2021-02-07, so **constraint 2's pre-2021 window is out of
scope for every consumer we know of.** Bound the run at `>= 2021-02-07` and
document the epoch boundary; the "dated peg epoch" option is unnecessary
complexity for a window nobody reads.

**5. They offered to hold their deployment. We declined.** Advised BE to ship:
the writer fix is live as of 2026-08-13, so their pool-list TVL (last
`close_usd > 0` within 48h) is correct the moment they deploy. Only the charts
read history, and that history is equally wrong whether they ship now or later.
The repair is additive and non-destructive, so it lands underneath a deployed
consumer with no coordination.

~~⚠️ **Open question sent back to BE, not yet answered:** do they read
`volume_quote_usd`?~~ ✅ **ANSWERED 2026-08-19 — no. Document and leave it.**
See the BE answers dated 2026-08-19 below.

## Estimate given to BE 2026-08-13 — ~2-4 h of run, 2-4 working days end to end

Extrapolated from [[0114]]'s **measured** figures, not from first principles.

| | 0114 (measured) | 0182 (this task) |
|---|---|---|
| candidates / month, `_1h` | 2,788,693 | ~10k (44,657 in `_1d` over ~99 mo, ×24) |
| batches / month | ~101 | ~2-6 |
| wall clock / month | **6m56s** | ~30-60 s (est.) |

Scan cost is **per batch, not per row** (0114: `--batch-size` is the lever), so a
100× smaller candidate set does not run 100× faster — it runs however long a
handful of partition `FINAL` scans take.

**Scope: five tables, not six.** `price_ohlcv_1m` (7 day) and `_15m` (30 day) are
retention-bounded (`cleanup-worker/src/lib.rs:31-32`), so they hold no history to
correct. The forever tables are `1h/4h/1d/1w/1M`.

⚠️ Cleanup is currently **disabled** on prod, so `_1m`/`_15m` do physically hold
old rows today. Do **not** repair them — they are expired-by-design and will be
dropped when cleanup re-enables. Repairing them spends hours on rows with a
scheduled death.

**Span:** 2021-02-07 → today ≈ 67 months (per BE answer 4 above).
**Run:** 67 months × 5 tables at ~30-60 s ⇒ **~2-4 h**.

**Why the calendar figure is days, not hours** — the run is not the long pole:

1. **The [[0114]] driver will not pick these rows up.** `repair.rs:193` selects
   `(volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0`. Our rows are
   **wrong-but-nonzero**, so the driver's own enumeration reports them as clean.
   Needs a targeted zero/version-bump step first — small, but it needs a test and
   a dry run before it touches prod.
2. **Snapshots need a CH admin.** `prices_writer` cannot `FREEZE` and **cannot be
   granted it** — the user is XML-defined and that storage is readonly
   (`ACCESS_STORAGE_READONLY`, hit and documented in 0114). The operator takes all
   snapshots up front, then the tool runs `--skip-snapshot`. That is a
   coordination step, not a code step.
3. **`--skip-snapshot` prints a warning that is false under this path.** Verify
   `ls /var/lib/clickhouse/shadow/ | grep repair_` plus a non-trivial `du` before
   trusting it (0114).

## What needs deciding

- [x] **Scope — DECIDED 2026-08-13: the five forever tables, direct.** Repair
  `1h/4h/1d/1w/1M` each in place; skip `_1m`/`_15m` (retention-bounded, no history
  to correct — see the estimate section). **Not** "`_1m` + re-roll": the rollup MVs
  are `sum(version)`-based ([[0095]], [[0136]]) so a re-insert into `_1m` does not
  propagate, and `_1m` no longer holds the source rows anyway. This is the same
  shape 0114 already solved — repair each coarse table directly.
- [x] **Span — DECIDED 2026-08-13: `>= 2021-02-07`.** Per BE answer 4, the
  deepest consumer surface is a 1Y chart. Leave the pre-2021 window at its
  existing `close × $1` (which 0172 argues is *correct* for that era) and
  document the epoch boundary. Dated-peg-epoch option dropped as unnecessary.
- [x] **Mechanism — DECIDED 2026-08-17: route (c), sequenced.** The [[0114]]
  `CoarseRepairDriver` is the right shape and is reused (the gap below is closed
  by the reset step). [[0201]]'s pass runs first without `--reset-*`, then
  0182's reset pass over a table at its floor. Full reasoning in the decision
  section after the dry run.
- [x] **Zeroing first — DECIDED and shipped 2026-08-13: the version route.**
  Enrichment skips `close_usd > 0`, so rows must be re-opened before the
  corrected pivot can fill them. `reset_sql` re-inserts them at `version + 1`
  with both USD columns at 0 — an insert, not `ALTER … UPDATE`, so it composes
  with the `FREEZE` snapshot (the pre-reset row survives under its old version
  and `ATTACH PARTITION` restores it). Avoids the RMT-tie problem the [[0097]]
  pre-roll notes flag for the DELETE route. Ran on prod 2026-08-18: 567,760
  rows re-opened, `rows_reset <= rows_enriched` on every table.

### ⚠️ The 0114 driver gap — it reports these rows as CLEAN

`repair.rs:193` enumerates months with
`(volume_quote_usd = 0 OR close_usd = 0) AND volume_quote > 0`. Every row this
task targets has `close_usd > 0` — wrong, but nonzero. So a `--dry-run` today
returns **"no months with enrichable zeros"** and looks like a green all-clear.

That is the same failure mode 0114 pre-registered for itself ("`enriched ≈ 0`,
`zeros_after` unmoved → the silent no-op"). Anyone reaching for the existing tool
without reading this will conclude the data is already fine.

The driver needs a **selection predicate** it does not currently have: rows whose
quote leg is the USDT identity, in the affected span. Zeroing those is what makes
them visible to the existing, tested enumeration — so the change is a targeted
pre-step, not a rewrite of the driver.

## Blast radius — read this before prioritising

⚠️ **Volume weight is NOT the impact measure.** Measured over 2026-05-01+, the
assets that actually *depend* on the USDT leg for their USD price are tiny:
`SPCXLM` ($12 total volume, 100% via USDT), `SCOP`/`RCC` ($0, 100%), `BAT`
(90.9%, $652), `LINK` (86.1%, $2,487), `BTC`-anchor (59.6%, $3,090), `MXN`
(36.8%, $4,022), `GYEN` (17.2%, $9,520). Everything with real volume — XLM
($470M), AQUA ($14.7M), SHX ($12.5M), XRP ($12.3M), yXLM ($10.3M) — draws ~0%
through the USDT leg, so **published prices for the major assets are unaffected**
(XLM measured at 0.0078% of weight).

**But BE values holdings, not flow.** A pool can hold a large position in an
asset that barely trades. 0172's opening recorded **106 pools with a USDT leg,
102 priceable** — every one of those positions is valued ~7.4× high regardless of
how thin the trading is. Prioritise on that, not on the volume table above.

## Implementation (2026-08-13) — the tool, not yet the run

Branch `fix/0182_close-usd-overstated-7x-on-usdt-quoted-candles`. The repair is
an **opt-in reset step** ahead of the existing tiers, so the corrected pricing
logic 0172 already shipped is what recomputes the values — no second
implementation of the pivot, nothing bespoke.

1. **`ch_enrich.rs::UsdResetSpec`** — `{ quote_asset_id, not_before }`. Both
   fields are bounds, not options: one quote leg per run (the blast radius must
   be nameable before the statement runs) and an epoch below which stored values
   are untouchable.
2. **`ch_enrich.rs::reset_sql`** — re-inserts matching rows with **both** USD
   columns at 0 and `version + 1`. An insert, not `ALTER … UPDATE`, so it
   composes with the `FREEZE` snapshot: the pre-reset row survives under its old
   version and `ATTACH PARTITION` restores it.
3. **`ChEnrichConfig.usd_reset: Option<UsdResetSpec>`**, default `None` — a pass
   is byte-identical to its pre-0182 behaviour unless an operator names the leg
   and the epoch. `run_coarse_sweep` **pins it to `None`** rather than merely
   leaving it unset, so the hourly Lambda cannot inherit one.
4. **`repair.rs::months_with_zeros`** now enumerates via the shared
   `repair_target_pred`, so a reset run can actually find the months.
5. **`coarse-repair.rs`** — `--reset-quote-asset-id` / `--reset-not-before`,
   mutually `requires`d, plus a loud pre-run warning.
6. **Runbook** `docs/runbooks/repair-coarse-usd-values.md` — new appendix for
   reset mode. Also corrects a line that still described USDT as a $1 peg.

### Design decisions

**From plan**

1. **Reuse the [[0114]] driver rather than write a repair.** Partition bounding,
   `FREEZE`, dry-run, per-month reporting and the deadline all already exist and
   are proven on prod. The gap was only that its enumeration could not see these
   rows.
2. **Epoch bound rather than a dated peg epoch.** Per BE answer 4 — nothing reads
   deep enough for the pre-2021 window to matter.

**Emerged**

3. **The reset zeroes `volume_quote_usd` too, resolving the open scope question
   without waiting for BE.** The tiers preserve `volume_quote_usd` write-once, so
   zeroing `close_usd` alone would leave the row carrying two USD figures derived
   from *different* rates — disagreeing by ~7.4× on exactly these rows. Both
   columns describe one candle at one instant; fixing one and pinning the other
   produces a row that is incoherent whichever figure a consumer reads. Zeroing
   both costs nothing (same statement) and lets the pivot recompute both from one
   reference. **This supersedes the "widen scope or document the mismatch"
   question** — there is no mismatch left to document. BE's answer is still worth
   having, but it is no longer a blocker.
4. **The oracle ordering constraint is a runtime gate, not a doc note.**
   `ChEnrichError::ResetBlockedByOracleRows` refuses the run while
   `oracle_prices` holds rows for the leg. A warning would not do: the failure is
   silent *and looks like success* — the oracle tier re-fills what the reset
   zeroed, and the run reports a healthy repair over unchanged values, relabelled
   `method = 'oracle'`. This is the [[0168]] trap arriving through a new door.
5. **`rows_reset` is reported alongside `rows_enriched`, never instead of it.**
   `rows_reset` ≫ `rows_enriched` is the signature of the one outcome worse than
   the defect: values discarded and not recomputed, turning a wrong-but-visible
   number into an ambiguous zero.
6. **The reset predicate mirrors the pivot's `volume_quote > 0`**, so it cannot
   re-open a row the pivot is structurally unable to refill. Unit-tested against
   `pivot_sql` directly, so the two cannot drift.

### Review of PR #212 — seven findings, all fixed

Two were correctness-blocking and both were mine.

**1. The branch did not build the production Lambda.** `usd_reset` is a
non-optional field and every construction site was updated *except* `main.rs`,
which sits behind `#[cfg(feature = "lambda")]` — invisible to the
`cargo clippy --workspace --all-targets` I validated with. CI's "Build Lambda
bootstraps" step would have caught it; my local sweep could not.

⚠️ **Lesson worth keeping: `--all-targets` is not `--all-features`.** This crate
has three feature-gated entrypoints. The check that would have caught it, now
run before every push here:

```bash
for F in "" "--features lambda" "--features aws-mtls" "--all-features"; do
  cargo check -p enrichment-worker $F --all-targets || echo "FAIL: $F"
done
```

**2. Nothing verified the reset target was refillable.** The oracle gate answers
"will something overwrite this?" but not "can anything restore this?".
`resolve_reference_ids()` ran *after* the reset, inside the tier section, and a
leg with no reference merely `warn!`s and skips — leaving the zeroes published.
A typo was enough: `--reset-quote-asset-id 11` for `111` **passes the oracle
gate**, because "no oracle rows" is exactly what that gate wants to see. Now
`ResetTargetHasNoPricingPath`, checked before any write.

**3. The safety check the runbook prescribes did not exist.** The warning and the
appendix both said "compare `rows_reset` against `rows_enriched` in the summary"
— but `rows_reset` was dropped at the driver boundary and never reached
`MonthRepair`, `RepairSummary`, or the printed table. It surfaced only in a
`tracing::info!`. Now carried through, printed as a per-month column and a
closing line, and the tool computes the comparison itself rather than leaving it
to the operator's arithmetic.

**4. `reset_sql`'s doc comment was inserted *inside* `pivot_sql`'s.** So
`reset_sql`'s rustdoc opened by describing the pivot and stated a four-parameter
bind order where `reset_sql` binds two — the precise setup for a positional-bind
bug on the next edit — and `pivot_sql` was left undocumented. Restored.

**5. A bounded pass could zero rows and skip the refill tier.** Tier 2 is gated on
`oracle_drained`; a bounded pass that exhausts its budget while still making
progress defers it. Unreachable from the CLI (`one_shot: true` is hard-coded) but
`usd_reset` is a public field on a public `run_through`. Now
`ResetRequiresOneShot`.

**6. `--skip-snapshot` was accepted with `--reset-*`** — while the tool's own
warning said "keep the FREEZE snapshot". Rollback for a bad reset *is* `ATTACH
PARTITION` from that snapshot.

⚠️ **My first fix for this was wrong and is worth recording.** I refused the
combination outright — which would have made reset mode **unusable on prod**.
`prices_writer` cannot `FREEZE` and cannot be granted it, so the established
procedure ([[0114]], Step 3b) is precisely *admin snapshots out of band, tool runs
with `--skip-snapshot`*. Refusing it would have sent the driver into a
`FreezeDenied` on the only cluster the mode exists for. Caught by running the
command rather than by re-reading the diff.

The actual fix is a `--snapshots-verified` acknowledgement, required only when
`--skip-snapshot` meets `--reset-*`. The prod path still works; what changed is
that "the admin froze it" and "nobody froze it" no longer look identical on the
command line. The tool cannot check this itself — it has no filesystem access to
the CH host — so it is explicitly an operator assertion.

**7. `--pivot-window-s` was unvalidated against bucket width.** The 1-day default
drops a `_1w`/`_1M` reference sitting in the previous bucket. Before a reset that
left a row unenriched; with one it discards the value first. Now refused below
7d/31d/1d for `_1w`/`_1M`/`_1d`.

Findings 5-7 share a shape worth naming: **every way this tool can discard a
value now refuses rather than warns.** A warning is the wrong instrument when the
failure is silent and looks like success.

New tests, both verified non-vacuous by removing the guards:
`the_usd_reset_refuses_a_quote_leg_that_no_tier_can_reprice`,
`the_usd_reset_refuses_a_bounded_pass`. 16 ITs, 39 unit tests, clippy clean on
all four feature combinations.

### ⚠️ Known limitation — reset mode is not a fixed point across runs

Within a run it converges (a reset row zeroes both columns and stops matching).
A *second* invocation sees the refilled rows and re-opens them, recomputing
values that are already correct — value-idempotent, but not free, and it bumps
`version` each time. Making it converge would need a "this row was already
repaired" marker the schema does not have. Deliberate: reset mode is a one-off
operator action, documented as such, and structurally excluded from the sweep.

### Tests

- `ch_enrich.rs` unit ×8: `reset_sql_targets_written_values_not_zeros`,
  `..._zeroes_both_usd_columns`, `..._honours_the_epoch_and_the_quote_leg`,
  `..._will_not_reopen_a_row_the_pivot_cannot_refill` (asserts against
  `pivot_sql` itself), `..._is_a_versioned_insert_not_a_mutation`,
  `..._threads_the_partition_window`,
  `reset_pending_pred_stops_matching_once_a_row_is_zeroed`,
  `repair_target_pred_{is_unchanged_without_a_reset,sees_months_that_hold_only_written_values}`.
- `ch_enrich_it.rs` ×3 on the 26.3.10.60 pin:
  - `an_ordinary_pass_cannot_see_a_wrong_but_written_close_usd` — the control,
    and a standing regression for the trap itself.
  - `the_usd_reset_recomputes_written_values_but_respects_the_epoch` — asserts
    the corrected 1.3 **and** that the pre-epoch row keeps its 10.0. Asserting
    only the first would also pass for a reset with no epoch bound at all.
  - `the_usd_reset_refuses_to_run_while_the_oracle_still_shadows_the_quote_leg`
    — plus that nothing was written before the refusal.

**Verified non-vacuous** — each defect restored, each caught:

| Defect restored | Test that failed |
|---|---|
| reset step never runs (0172 writer fix alone) | `..._recomputes_written_values_...` (`rows_reset` 0, not 1) |
| epoch bound dropped from `reset_sql` | same test, on the pre-epoch assertion |
| oracle gate removed | `..._refuses_to_run_while_the_oracle_...` (pass succeeded) |

All green: 39 workspace unit tests, 14 enrichment ITs, clippy clean on the crate.

## ✅ SIZED ON PROD 2026-08-13 — 567,232 rows, and the estimate holds

Measured across the five forever tables. `affected` = quote leg 111, at or after
the 2021-02-07 epoch, still holding a written USD value, with volume to price.

| table | affected rows | pre-epoch left alone | first | last |
|---|---|---|---|---|
| `_1h` | **357,002** | 33,131 | 2018-05-15 | 2026-08-13 |
| `_4h` | 157,683 | 13,176 | 2018-05-15 | 2026-08-13 |
| `_1d` | 41,505 | 3,168 | 2018-05-15 | 2026-08-13 |
| `_1w` | 8,253 | 641 | 2018-05-14 | 2026-08-10 |
| `_1M` | 2,789 | 225 | 2018-05-01 | 2026-08-01 |
| **total** | **567,232** | **50,341** | | |

**Cross-check:** `_1d` gives 41,505 + 3,168 = 44,673 against the 44,657 this task
was filed on — a day of new candles. The predicate is selecting the intended
population, not a neighbouring one.

⚠️ **The `months` column in that query was not usable and is omitted here.** It
reported 91-92 for every table, but the inner `GROUP BY` emits a row per month
whether or not that month holds an affected row, so `uniqExact` counted *every*
month with a USDT-quoted candle (2018-05 → 2026-08 ≈ 100). Affected months are
2021-02 → 2026-08 ≈ **67**. The dry run will report the real figure per table —
do not re-derive it from that query.

**Estimate revised, in the same range but tighter: ~1.5-3 h.** `_1h` carries 63%
of the work at ~5,300 affected rows/month, so ~1 batch per month per tier at the
default 10k batch — the run is **scan-bound, not row-bound**, exactly as [[0114]]
found. Per month-table that is a handful of partition `FINAL` scans at 0114's
measured ~4 s each. Expect ~45-60 min for `_1h`, ~20-30 for `_4h`, ~10-15 for
`_1d`, single-digit minutes for `_1w`/`_1M`.

### ⚠️ The 50,341 pre-epoch rows carry an assumption we cannot verify

They keep `close × $1`. For **2021-02 → 2022-04** that is a *measured* par
([[0172]]). For **2018-05 → 2021-02** it is an extrapolation: this IOU's USDC
market does not begin until 2021-02-07, so there is no in-house observation of
its value at all before then, at any granularity. Real Tether was at par in that
window and the IOU was at par the moment it became visible, so the extrapolation
is reasonable — but it is an inference, not a measurement, and it should not be
described as verified.

Out of scope here per BE answer 4 (nothing reads a 1Y chart back to 2021), and
correcting it is impossible rather than merely deferred — there is no reference
to correct it *to*. Worth its own note if a consumer ever reads that deep.

## ✅ DRY RUN 2026-08-13 — non-vacuous, and it exposed a second campaign

Ran all five tables, `202102`–`202608`, `--reset-quote-asset-id 111
--reset-not-before 1612656000 --pivot-window-s 2678400 --dry-run`. Log:
`/tmp/0182_dryrun.log`.

**The AC is met: 67 months on every table, no "no enrichable zeros" all-clear.**
The 0114 driver gap is closed — the tool can see the defect it could not see
before. Preconditions checked first and all green: `oracle_prices` 0 rows for
asset 111 under any `oracle_name`, `usd_rate` 0 rows for the USDT identity
(0196's purge has not regrown), and the identity still resolves to `asset_id =
111`.

Baseline probe recorded before any write — `implied_rate` **flat 1.000000 for
202102 → 202602**, then 0.9989–1.0002 for 202603 → 202608. That tail is the
*oracle*-written value (Reflector ~0.9996), not the peg path; same defect, two
labels.

### ⚠️ But `zeros_before` is NOT the 567,232 — the enumeration is an OR

`repair_target_pred(Some(spec))` is `(CANDIDATE_PRED) OR (reset_pending_pred)`.
So the dry run counts **every pre-existing enrichable zero in the span as well
as** the reset targets:

| table | dry run reports | USDT reset targets | ratio |
|---|---|---|---|
| `_1h` | 107,516,845 | 357,002 | 0.33% |
| `_4h` | 53,506,597 | 157,683 | 0.29% |
| `_1d` | 17,907,401 | 41,505 | 0.23% |
| `_1w` | 4,391,892 | 8,253 | 0.19% |
| `_1M` | 1,463,985 | 2,789 | 0.19% |

This is **not** a defect in the tool — the OR is exactly what makes the reset
rows visible, and it is unit-pinned
(`repair_target_pred_sees_months_that_hold_only_written_values`). But it means
the command in the run memo does **two** jobs, and the second one is ~300× the
size of the first.

### What the other 99.7% is — measured on `_1h`, 2021-02 → 2026-08

| class | 2021-02..2024-01 (0114 never ran) | 2024-02.. (0114 ran) |
|---|---|---|
| **XLM pivot → fillable** | **31,982,165** | 1,100,131 |
| exotic → stays 0 | 24,056,634 | 50,020,562 |
| USDC peg → fillable | 196 | 403 |
| USDT → fillable | 8 | 31 |

The 74M exotic rows are nearly free — no USD reference, so each tier early-exits
after a couple of no-progress batches (the `no_reference` floor 0114 documented).

⚠️ **The span in this table was FALSIFIED 2026-08-18** during pass 1: almost none
of the 31,982,165 are below **2022-04** — months 202110-202203 hold 0-13
XLM-quoted candidates each and enriched nothing, while 202204 enriched 761,735.
The rows exist, but the window is ~2022-04 → 2024-01, not 2021-02 → 2024-01. Full
measurement and the two falsified explanations are in [[0201]].

⛔ **The epoch claim that used to sit here was WRONG and it cost 157 candles.**
It read: *"the USDT/USDC reference is dense and non-NULL from 202102, first
candle `2021-02-07 19:00`, so `1612656000` cannot strand rows."* Both halves of
the premise are true; the conclusion does not follow. `1612656000` is
**2021-02-07 00:00**, nineteen hours *before* that first candle, and the pivot
joins its reference at-or-before. See the boundary-repair section dated
2026-08-19 — **the correct epoch is `1612724400`.**

**The 31,982,165 fillable XLM-quoted rows are the [[0088]] pre-Soroban backfill's
output**: candles rolled into the coarse tables and never enriched, because
0114's repair span started at 2024-02. That is real missing data and it is worth
recovering — but it is a second campaign comparable in size to the whole of 0114
(~30M rows, 4-5 h on one table), and across five tables plausibly **10-15 h**.

**→ Spawned as [[0201]]. 0182's scope is unchanged: the USDT correction only.**
Neither [[0175]] nor [[0176]] covered it, so it was unclaimed until now.

### Two consequences for this task's own run

1. **The ~1.5-3 h estimate above is wrong for the command as written.** It
   assumed the run visits only the reset rows (~5,300/month on `_1h`). It would
   in fact drain the 32M as well. The estimate stands only for a run bounded to
   the reset leg.
2. **The `rows_reset ≈ rows_enriched` safety check stops working.** In a combined
   run `rows_enriched` is ~32M against ~357k reset, so the shortfall signal the
   runbook prescribes is swamped and cannot detect the failure it exists for
   (values zeroed and not recomputed). Verification would have to fall back to
   the implied-rate probe alone.

### ⛔ Decision taken 2026-08-13: document, do not run

The operator's call: record the finding, keep 0182's scope, spawn [[0201]] for
the 32M, **run nothing yet**. No FREEZE was taken and no partition was written —
the dry run is read-only by construction (`preflight` is `SELECT 1`, the
grant-probe is gated on `snapshot && !dry_run`, and `run()` hits the `dry_run`
guard before `freeze_partition` and `run_through`).

**The mechanism question this created.** The tiers fill any zero in the partition
they visit; they take no per-quote-leg filter. So there is no way today to run
0182 without also running [[0201]]. Three routes were open:

- **a.** Take both at once — one FREEZE, one campaign, 10-15 h, and 0182's
  verification leans entirely on the implied-rate probe.
- **b.** Add a scope flag threading the reset leg into the tier SQL, so 0182 is
  surgical and fast. Costs a PR with tests before any prod run.
- **c.** Sequence them: [[0201]] first as the bigger job, then 0182's reset over
  a table that is already at its floor — at which point the combined run *is*
  the bounded run, and `rows_reset ≈ rows_enriched` works again.

⚠️ A single-month pilot (`--start-month 202606 --end-month 202606`) would price
route **a** cheaply, but note it re-opens that month a second time in the full
run — value-idempotent, one extra `version` bump.

### ✅ MECHANISM DECIDED 2026-08-17: **route (c)**

The operator's call, taken with the completion order for the whole repair chain:
**0201 → 0182 → 0172 close as one campaign, sequenced.** Recorded here because
this file — not the session note — is what a fresh session reads, and until now
it still said the mechanism blocked the run.

**Why (c) over (a):** (a) destroys the `rows_reset ≈ rows_enriched` safety check.
In a combined run `rows_enriched` is ~32M against ~357k reset, so the shortfall
signal — the one that detects *values zeroed and never recomputed*, the worst
outcome this task can produce — is swamped by two orders of magnitude and
verification falls back to the implied-rate probe alone. Sequencing restores it.

**Why (c) over (b):** (b) is the better tool and buys a fast, surgical 0182, but
it costs a PR with tests before any prod run, and the 32M pre-Soroban rows are
real missing data we want recovered anyway. (b) would leave [[0201]] still to do.

**What route (c) means operationally — two passes of the same binary:**

1. **Pass 1 = [[0201]]** — run `coarse-repair` **without** `--reset-*`. Fills the
   ~32M fillable pre-Soroban zeros. The table ends at its floor: only the
   genuinely unfillable exotic-quote rows are left at zero.
   ⚠️ **0201 is the operator's own task** — see the ownership note; this entry
   records the sequencing, it does not claim the work.
2. **Pass 2 = 0182** — run **with** `--reset-quote-asset-id 111
   --reset-not-before 1612656000`. With no other fillable zeros left,
   `rows_enriched` is dominated by the reset rows and `rows_reset ≈
   rows_enriched` is meaningful again.

This also keeps the "**run reset mode once per table**" rule intact: pass 1 is
not a reset invocation, so pass 2 is still the single reset run per table.

**FREEZE span under (c): the same 67 months × 5 tables**, since both passes visit
the same partitions.

### ✅ SETTLED 2026-08-18: **two snapshots** — and the three things that forces

One `FREEZE` before pass 1, a second between the passes. One snapshot is
cheaper, but rolling pass 2 back under it would discard pass 1's ~32M recovered
rows as well; two make the passes independently revertible. What follows is not
obvious from the decision itself.

**1. The two snapshots must have DIFFERENT names.** The freeze script's
`already-frozen` branch deliberately *keeps* a pre-existing snapshot rather than
overwriting it — otherwise a re-freeze would replace a pre-repair rollback point
with a post-repair one. That same protection makes a second freeze under the
same name a **silent no-op**: it prints `already-frozen`, and you believe you
hold two rollback points while holding one. Use `repair_0182_pre_…` for the
first and `repair_0182_mid_…` for the second.

**2. It rules out interleaving the passes per table.** The second snapshot has
to sit between pass 1 and pass 2, so a per-table interleave (`_1h` pass 1, `_1h`
pass 2, `_4h` pass 1, …) needs the admin back **five times** across 10-15 h —
and `prices_writer` cannot `FREEZE`, so every one of those is a second person.
Ordering adopted instead, three admin windows at predictable points:

| # | Step | Who |
|---|---|---|
| 1 | `FREEZE` #1 `repair_0182_pre_…`, all five tables | admin |
| 2 | Pass 1 ([[0201]], no `--reset-*`) on `_1h` | operator |
| 3 | `FREEZE` #2 `repair_0182_mid_…`, **`_1h` only** | admin |
| 4 | Pass 2 (0182, with `--reset-*`) on `_1h`, then **review** | operator |
| 5 | Pass 1 on `_4h`, `_1d`, `_1w`, `_1M` | operator |
| 6 | `FREEZE` #2 `repair_0182_mid_…`, those four | admin |
| 7 | Pass 2 on those four | operator |

`_1h` — the table BE consumes — is proven end to end before the remaining ~10 h
is committed, and no table's post-pass-1 state is pinned before its pass 1 runs.

**3. Disk — measured on prod 2026-08-18, not estimated.**

| table | active parts | on disk, 202102-202608 |
|---|---|---|
| `price_ohlcv_1h` | 204 | **9.56 GiB** |
| `price_ohlcv_4h` | 212 | 4.92 GiB |
| `price_ohlcv_1d` | 154 | 1.71 GiB |
| `price_ohlcv_1w` | 106 | 481.71 MiB |
| `price_ohlcv_1M` | 123 | 164.42 MiB |
| **total** | **799** | **16.82 GiB** |

⚠️ **A `FREEZE` costs nothing when it is taken and accrues its cost afterwards.**
It hardlinks the partition's parts under `shadow/` — the bytes exist once, the
operation is instant and blocks nothing. But parts are immutable, so as this
run's `version + 1` inserts merge and supersede them, the originals cannot be
reclaimed while the hardlink holds the inode. The cost builds over the 10-15 h,
and its ceiling is the size of what was frozen.

Freeze #1 ≤ 16.82 GiB, freeze #2a (`_1h`) ≤ 9.56 GiB, freeze #2b (the other
four) ≤ 7.26 GiB — **≤ 33.6 GiB of snapshot**, plus ~4-6 GiB of new parts before
merges reclaim. ~40 GiB peak against 430.6 GiB free, about **9%**. Comfortable
against the volume — but note it is a **~68% increase on our own 58.93 GiB
footprint**, which is the shape BE would see if they went looking.

## ✅ THE RUN — executed and verified 2026-08-18

Executed from fishuser-hero as `prices_writer` over mTLS, route (c), two
snapshots. **~4 hours end to end**, against a 10-15 h budget — the estimate
assumed 32M rows spread over 36 months and they were concentrated in about 20.

### Pass 2 — the correction this task exists for

| table | reset | recomputed | at zero (dust) |
|---|---|---|---|
| `_1h` | 357,274 | 358,315 | — |
| `_4h` | 157,858 | 158,319 | — |
| `_1d` | 41,573 | 41,565 | 8 |
| `_1w` | 8,266 | 8,261 | 5 |
| `_1M` | 2,789 | 2,784 | 5 |
| **total** | **567,760** | | **18** |

**567,760 against the 567,232 sized on 2026-08-13** — 528 apart after five days
of live writes, and `_1M` landed on 2,789 exactly. `rows_reset <= rows_enriched`
on every table.

### Verification — the acceptance criterion

Implied rate for 2026, all five tiers, where the pre-run baseline was a flat
`1.000000` on every one:

| tier | rate | candles |
|---|---|---|
| `1h` | 0.1529 | 27,335 |
| `4h` | 0.1520 | 14,609 |
| `1d` | 0.1506 | 4,507 |
| `1w` | 0.1494 | 791 |
| `1M` | 0.1525 | 205 |

A 2.3% spread, which is bucket-weighted averaging differing between
granularities. On `_1h` the full monthly series reads **par for exactly 15
months** (202102-202204), then 0.973 → 0.734 → 0.294 across the June 2022 break,
settling at 0.134 by 202608. That is [[0172]]'s measured series arrived at from a
completely different direction — and it confirms the epoch: the par window below
the break kept its correct `$1` rather than being zeroed.

### The 18 rows that tripped the shortfall guard

`_1d`, `_1w` and `_1M` each warned that rows were re-opened but not recomputed.
All 18 are **dust** — `close` of exactly 0, or below ~4e-14 where `rate × close`
underflows at `Decimal(38, 14)` — across four asset ids (1552, 55249, 15952,
43381). Five of `_1d`'s eight were already at `close_usd = 0` before the reset
touched them. `stranded_with_real_close` returned **0** on all three tables:
nothing with a usable price ended at zero.

⚠️ **The guard cannot distinguish "recomputed to zero" from "value destroyed"**,
because `rows_enriched` is a population difference and a row written with a zero
value never leaves the zero population. It also advises checking
`--pivot-window-s`, which was never involved. Triage procedure and worked example
are now in `docs/runbooks/repair-coarse-usd-values.md`.

## ⛔ 2026-08-19 — the run DESTROYED 157 candles, and the re-check is what found it

The run was declared verified on 2026-08-18. A re-verification the next morning,
run before releasing the snapshots, found **157 candles this task had zeroed and
never refilled**. Repaired the same day. Recorded here in full because the way it
was missed is more reusable than the defect.

### The defect — the epoch is 19 hours too early

`--reset-not-before 1612656000` is **2021-02-07 00:00** UTC. The USDT/USDC
reference market's first candle is **2021-02-07 19:00**. The pivot's `ASOF LEFT
JOIN` matches its reference *at or before* each bucket, so every bucket stamped
inside that nineteen-hour gap was zeroed by the reset and had nothing to price
against. `AND r.usd IS NOT NULL` then dropped it.

This is exactly the mechanism constraint 2 pre-registered at the top of this file
— *rows with no pivot reference must not be zeroed* — arriving through the one
door nobody was watching, because the epoch was believed to close it.

| tier | destroyed | why |
|---|---|---|
| `_1h` | **121** | the 19 hourly buckets 00:00-18:00 × 15 assets |
| `_4h` | **36** | buckets 00:00/04:00/08:00/12:00 × 9 assets; the 16:00 bucket **contains** the 19:00 reference trade, so it priced |
| `_1d` | 0 | — |
| `_1w` | 0 | — |
| `_1M` | 0 | — |

⚠️ **The tier pattern is what confirms the mechanism rather than merely fitting
it.** `_1d`/`_1w` are clean because their reference candle falls in the *same
bucket* as the row being priced, and at-or-before is inclusive. `_1M`'s February
bucket is stamped `2021-02-01`, *below* the epoch, so it was never touched at
all. **Only tiers finer than the gap can be stranded, and exactly those two
were.** Any other cause would not produce that distribution.

These were not dust. Asset 953 on `_1h` held `close` ≈ 39,521 with real
`volume_quote`, and 2021-02-06 — every hour of it — was fully priced on both
sides of the boundary.

### 🔴 Why the run's own verification passed over it

`stranded_with_real_close` was run on 2026-08-18 against `_1d`, `_1w` and `_1M`
**and returned 0 on all three** — correctly. Those were the three tables that had
tripped the shortfall guard, so those were the three that got checked.

They are also **precisely the three tiers structurally incapable of showing this
defect.** The two tiers that could show it were never checked, because they had
not warned. The guard selected the sample, and the guard is blind to this failure
mode: `_1h` and `_4h` produced **no shortfall at all** — 357,274 reset against
358,315 enriched — because the 121 and 36 are swamped by rows the pass
legitimately enriched in the same run.

**Rule: run the damage check on every table, never only on the ones that
complained.** A guard that fires tells you where to look; a guard that stays
quiet tells you nothing.

### The repair — restore par, additively

Par is the *measured* value for this window ([[0172]]: USDT at par from 2021-02
until the June 2022 break), it is what these rows held before this task touched
them, and it is what their neighbours on both sides still hold. So the fix is a
versioned insert in the same shape as `reset_sql`, not a rollback:

```sql
INSERT INTO prices.price_ohlcv_1h (…)
SELECT p.timestamp, …, p.volume_quote AS volume_quote_usd, p.close AS close_usd,
       p.vwap, p.trade_count, p.version + 1 AS version
FROM prices.price_ohlcv_1h AS p FINAL
WHERE p.quote_asset_id = 111 AND p.close_usd = 0 AND p.volume_quote > 0
  AND p.timestamp >= toDateTime(1612656000)
  AND p.timestamp <  toDateTime('2021-02-07 19:00:00')
```

Ran against `_1h` (121 rows) and `_4h` (36). A count-only dry run with the
identical `WHERE` was gated on returning exactly 121 and 36 first.

⚠️ **The snapshot rollback was considered and rejected.** On a
`ReplacingMergeTree` an `ATTACH` alone loses to the higher `version` already
written, so rolling back means `DROP PARTITION` then `ATTACH` — a drop on prod
partition `202102` across four tables to repair 157 rows. Wrong risk for the
size of the problem, and the forward write reaches the same state.

### Verified

`close_usd = 0 AND close > 5e-14` bucketed by age, all five tables:
`at_boundary` **0** everywhere. The 82 rows still at zero are all in-flight
buckets — `_1h` 38, `_4h` 22, `_1d` 14 in the last 48 h, and `_1w` 8 in the
current week bucket (stamped `2026-08-17 00:00`, ~55 h old, which is why it falls
outside a 48-hour window and briefly looked unexplained).

⚠️ **Left open, not investigated:** some of those `_1h` rows are 17 h+ old, which
is longer than an hourly sweep should leave a candle unpriced. Either enrichment
lag or the pivot finding no USDT/USDC reference inside its default 1-day window.
Not damage from this task, and it postdates the run entirely.

## ✅ BE answers 2026-08-19 — the last question closed, and one of our claims corrected

Their 2026-08-13 reply on `volume_quote_usd` never arrived; this is the restated
version, plus a correction and an independent measurement of the fill.

**1. `volume_quote_usd` — no, they do not read it. Document the inconsistency
and leave it.** Verified against their merged code: the only prices surfaces
they read are `price_usd_series` and `price_usd_series_1h`, and the only columns
are the identity triple, the bucket, and `close_usd`. They never touch
`price_ohlcv_*` or any volume column. They have recorded the same trap in their
own notes so future work there does not reach for it either.

→ **This closes the question, and it closes it as "no widening".** The reset
already zeroed both columns together (design decision 3 above), so within the
repaired span the two figures are coherent anyway. What remains documented rather
than fixed is the **pre-epoch window**: rows below 2021-02-07 keep a
`volume_quote_usd` computed at $1, which for that era is correct, so there is no
live inconsistency there either. The mismatch this question was filed about does
not exist in the data as it now stands.

**2. ⚠️ CORRECTION TO OUR OWN RECORD — "nothing they deployed ever served the
inflated numbers" is WRONG.** BE's release shipped **before** the 2026-08-18
history repair, not after. The distinction they drew:

- **List and detail pages: never affected.** Our writer fix (2026-08-13)
  preceded their deploy, so the 48-hour `close_usd > 0` path always served
  correct USDT values.
- **30D/1Y charts on USDT-legged pools: DID serve the inflated history**, for
  the interim between their deploy and 2026-08-18.

No action follows — their charts are compute-at-read with no caching, so they
corrected themselves the moment the repair landed, and nothing on their side
cached or derived from `close_usd`. But the acceptance-criterion line below
claimed a clean record that we did not have, and the framing in the BE-answers
section of 2026-08-13 ("not an incident with live blast radius") was true when
written and stopped being true when they deployed. **A statement about a
consumer's exposure has a shelf life; this one outlived its premise.**

**3. Independent confirmation of the [[0201]] fill, measured on their side**
(pinned 2026-08-19, 52,580 pools):

| metric | before | after |
|---|---|---|
| priceable-ever | 51,935 | **52,112** (99.1%) |
| never-priced | — | **468** |
| priceable-90d | — | 71.0% |
| priceable-48h | — | flat, as expected — it never read history |

The 48h figure staying flat while the deeper windows move is exactly the
signature the repair should produce, arrived at from the consumer side without
reference to our numbers. ⚠️ Worth carrying into **[[0201]]'s closure call**,
which is the operator's own — this is external corroboration of the 53,965,024
figure that does not depend on our own enumeration.

**4. They have lifted the USDT exclusion from their Horizon cross-validation**,
and report the correction signature on USDT-legged pools matches expectations.
From their side the USDT thread is closed.

## 🚧 What is NOT done

Remaining, in order:

1. ~~**Size the run.**~~ ✅ done 2026-08-13 — 567,232 rows, see above.
2. ~~**Dry run per table**, non-vacuous.~~ ✅ done 2026-08-13 — 67 months on all
   five, and it surfaced [[0201]]. See the dry-run section above.
3. ~~**Decide the mechanism** — routes a/b/c above.~~ ✅ **DECIDED 2026-08-17:
   route (c)**, sequenced — 0201's pass first, then 0182's reset over a table at
   its floor. See the decision section above for why, and for what each pass
   runs. **The run is no longer blocked on this.**
4. ~~**Snapshots**~~ ✅ **taken 2026-08-18** — `repair_0182_pre_` across all five
   (335 = 67 × 5, 17G, matching the 16.82 GiB measured), then `repair_0182_mid_`
   after each table's pass 1. Operator holds CH admin, so the three windows were
   self-served rather than a second person.
5. ~~**The run**~~ ✅ **done 2026-08-18** — `_1h` first and reviewed before the
   rest, `--pivot-window-s 2678400` throughout. See the results section above.
6. ~~**Verify** the implied-rate probe moves off 1.0~~ ✅ **done** — all five
   tiers at ~0.15 for 2026. ✅ **BE told 2026-08-18** and they replied
   2026-08-19: `volume_quote_usd` closed (they read `close_usd` only), the fill
   independently confirmed on their side, and one of our claims corrected. See
   the BE-answers section dated 2026-08-19.
7. ~~**UNFREEZE and reclaim**~~ ✅ **done 2026-08-19** — all 485 released
   (335 `repair_0182_mid_*` + 150 stale `repair_0114_*`), `shadow/` 31G → 7.7M
   of empty husks → swept to `increment.txt` alone. See the cleanup section for
   the two things that were wrong in the procedure as written.
8. **The 82 in-flight zeros** — never investigated. Some `_1h` rows sit unpriced
   for 17 h+, longer than an hourly sweep should leave them. Enrichment lag, or
   the pivot finding no USDT/USDC reference inside its default 1-day window.
   Postdates the run; not damage from this task.
9. **Guard against re-introduction** — still the one open acceptance criterion.
   Now wants a second check alongside it: no USDT-quoted candle at
   `close_usd = 0` with a representable `close`, which is the defect [[0208]]
   exists to make impossible.

## 🧹 Cleanup — the snapshots do not expire and nothing reclaims them

⚠️ A `FREEZE` left behind is the 2026-08-13 incident ([[0202]]) with our name on
it instead of BE's: dead bytes pinned on a shared volume, invisible to
`system.parts`, visible only to `df`. ~33.6 GiB of it here. Neither ClickHouse
nor this tool ever removes a snapshot — an **admin** must, and `prices_writer`
cannot.

Per table, in this order:

1. Once that table's pass 2 is verified (implied-rate probe off 1.0), drop its
   **pre** snapshot — it protects a rollback you have just decided not to take.
   `_1h` alone returns 9.56 GiB.
2. Keep its **mid** snapshot until the whole campaign is verified *and* BE have
   been told the corrected window.
3. Then drop the mid snapshots too.

```sql
ALTER TABLE prices.price_ohlcv_1h
  UNFREEZE WITH NAME 'repair_0182_pre_prices_price_ohlcv_1h_202403'
```

— per partition, or by removing the `shadow/<NAME>/` directories on the host.
Verify with `du -sh /var/lib/clickhouse/shadow/` returning to its **pre-campaign
size**, not merely shrinking.

⚠️ **Put this on the admin's list when the FREEZEs are arranged**, not after.
It is the half of the procedure with no deadline attached, which is why it is
the half that gets forgotten.

### ✅ DONE 2026-08-19 — and two corrections to the procedure above

485 snapshots released in one pass, `shadow/` 31G → 7.7M → `increment.txt` alone.

**1. `SYSTEM UNFREEZE WITH NAME` does not work on this server.**
`Code: 344 … Support for SYSTEM UNFREEZE query is disabled. You can enable it
via 'enable_system_unfreeze' server setting.` That is a **server** setting on a
ClickHouse BE own 96% of — not something to change for a one-off cleanup. The
`ALTER TABLE … UNFREEZE WITH NAME` form written above is **not** gated by it and
is the one to use. Generate the statements from the host listing rather than
typing them; the table is encoded in the snapshot name
(`repair_0182_mid_prices_price_ohlcv_4h_202606` → `price_ohlcv_4h`), so
`awk -F'_' '{print … $(NF-1) …}'` recovers it for either prefix and preserves
`1M`'s case.

**2. 🔴 The success signal is `du`, NOT the entry count.** `UNFREEZE` releases
the hardlinks but leaves an **empty directory husk** (~16K), so
`ls shadow/ | wc -l` is *unchanged* after a successful release — which reads
exactly like a no-op. Compare a released entry against an untouched sibling
instead: **16K against 36M** is the proof. Sweep the husks afterwards with
`find /var/lib/clickhouse/shadow -mindepth 1 -depth -type d -empty -delete`
(`-type d -empty` cannot touch a snapshot that still holds data).

⚠️ **`df` moves far less than the `du` figure suggests, and that is correct.** A
frozen part only costs *extra* disk once the live table has superseded it; most
of the 31G was still the active data, counted once either way. What the pin
actually did was block reclaim **going forward** as merges superseded more parts.
So "`df` barely moved" is not evidence the unfreeze failed.

⚠️ **`prices_writer` cannot FREEZE, but the UNFREEZE ran fine over `CHQ`**,
which connects as the container's `default` user. The admin constraint applies to
taking a snapshot, not to releasing one — provided you have a `default` path.

## Acceptance Criteria

- [x] Decision recorded: **correct history from 2021-02-07 on**, leave the
      pre-2021 window at `close × $1` and document the epoch boundary. BE
      confirmed 2026-08-13 that history matters (30D/1Y charts) and that their
      deepest surface is 1Y, which never reaches the boundary.
- [x] The **five forever granularities** (`1h/4h/1d/1w/1M`) consistent
      afterwards — ✅ 2026-08-18, all five at 0.1494-0.1529 for 2026 where the
      baseline was a flat `1.000000`, and `_1h`'s monthly series reproduces
      [[0172]]'s par-then-depeg shape. ⚠️ Not "all six" — `_1m`/`_15m` are
      retention-bounded and deliberately excluded.
- [x] The `--dry-run` preview is **non-vacuous** — ✅ 2026-08-13, 67 months on
      every one of the five tables, not the "no enrichable zeros" all-clear the
      unmodified 0114 driver returns for these rows (see the driver gap above).
      The tool can see the defect. ⚠️ Its `zeros_before` counts are **not** the
      567,232 — see the dry-run section for why, and for [[0201]].
- [x] **Mechanism decided** — ✅ 2026-08-17, **route (c)**: sequence [[0201]]'s
      pass first, then 0182's reset over a table already at its floor. Chosen
      because a combined run (route a) swamps the `rows_reset ≈ rows_enriched`
      check ~32M against ~357k, and that check is the only thing that detects
      values zeroed and never recomputed. **No longer blocks the run.**
- [ ] Guard against re-introduction: the [[0172]] regression tests already pin
      the writer; add a data-level check that no USDT-quoted candle carries
      `close_usd / close ≈ 1.0`. ⚠️ **Widen it** — 2026-08-19 showed the
      opposite failure is just as real, so also assert no USDT-quoted candle
      sits at `close_usd = 0` with a representable `close`.
- [x] **The epoch boundary is sound** — ✅ 2026-08-19, *after* a repair. The
      original epoch `1612656000` stranded **157 candles** (121 `_1h`, 36 `_4h`)
      in the 19 hours before the reference market's first trade; restored to par
      by versioned insert, `at_boundary` now 0 on every tier. **The correct
      epoch is `1612724400`.** Spawned [[0208]] so the tool refuses an epoch
      below its reference's first candle rather than trusting the operator.
- [x] **BE notified** — ✅ 2026-08-18. Told them: corrected from 2021-02-07 on,
      all five granularities, 567,760 candles; values below that boundary
      unchanged and already correct. Also warned them that ~54M previously-
      unpriced candles now carry values, which is directly visible to them since
      they render `--` on a `close_usd = 0` miss.
      ⚠️ **One thing we told them was wrong.** We said *"nothing they deployed
      ever served the inflated numbers"*; BE corrected it 2026-08-19 — their
      release shipped before the history repair, so the **30D/1Y charts on
      USDT-legged pools did serve the inflated history** in the interim. The
      list and detail pages never did. Self-corrected on their side
      (compute-at-read, no caching), so no action follows.
- [x] **Snapshots removed** — ✅ 2026-08-19. All 485 released
      (`repair_0182_pre_` 2026-08-18, then `repair_0182_mid_` + 150 stale
      `repair_0114_`), `shadow/` 31G → `increment.txt` alone. ⚠️ Two procedure
      corrections recorded in the cleanup section: `SYSTEM UNFREEZE` is disabled
      server-side, and the success signal is `du`, not the entry count.
- [x] `volume_quote_usd` resolved — ✅ **2026-08-19, documented, not widened.**
      BE confirmed against their merged code that they read `close_usd` only,
      from `price_usd_series`/`price_usd_series_1h`, and never touch
      `price_ohlcv_*` or any volume column. The reset zeroed both USD columns
      together, so within the repaired span they are coherent; the pre-epoch
      window keeps a `volume_quote_usd` at $1, which is correct for that era.
      No mismatch remains in the data to document.
