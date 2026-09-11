---
id: "0228"
title: "XLM's measured USD rate is fetched every 5 minutes and thrown away, while 11 M candles are priced by deriving it indirectly through USDC"
type: BUG
status: active
related_adr: ["0011"]
related_tasks: ["0167", "0170", "0172", "0182", "0061", "0227", "0173", "0267", "0268", "0276", "0154"]
tags: ["priority-medium", "effort-large", "oracle", "enrichment", "data-correctness", "usd", "milestone-M2"]
milestone: 2
links:
  - "notes/R-phase0-measurement-2026-09-11.md"
  - "notes/sql/q9_asof_shape.sql"
  - "../../../../packages/oracle-worker/src/lib.rs"
  - "../../../../packages/enrichment-worker/src/ch_enrich.rs"
  - "../../../../packages/enrichment-worker/src/bin/coarse-repair.rs"
  - "../../../../packages/enrichment-worker/tests/post_run_0228_it.rs"
  - "../../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../../docs/runbooks/repair-coarse-usd-values.md"
history:
  - date: "2026-08-26"
    status: backlog
    who: okarcz
    note: >
      Found while checking `usd_rate` coverage for [[0170]]. Measured on prod:
      `oracle_prices` holds **51,235 Reflector readings for XLM** and
      `prices.usd_rate` holds **zero** rows for it, against 48,049 for USDC.
      `peg_identities()` is exactly canonical USDC by design and pinned by a
      test, so XLM is never snapshotted — and `oracle_prices` is pruned at
      INTERVAL 13 MONTH, so those readings age out permanently.
      Meanwhile the enrichment pivot tier prices ~11 M XLM-quoted candles using
      "XLM's volume-weighted close against USDC", i.e. deriving XLM's dollar
      value through our own candle data, which is dollars only because USDC is
      assumed to be $1. We measure the thing directly, discard the measurement,
      and then infer it.
      ⚠️ Filed as a design question, not a wrong-number claim. Whether the
      derived value materially differs from the measured one is UNMEASURED and
      is the first acceptance criterion.
  - date: "2026-09-11"
    status: active
    who: akot
    note: >
      Picked up by akot. Starting with AC 1 — measure the derived pivot
      `ref_usd` for XLM against Reflector's measured reading before any fix
      is chosen.
  - date: "2026-09-11"
    status: active
    who: akot
    note: >
      Phase 0 measured on prod (notes/R-phase0-measurement-2026-09-11.md).
      AC 1 as filed is MET — pivot vs Reflector over 4,413 hours: p50 −1.2 bps,
      p05/p95 ±35 bps — and the premise is half wrong: the oracle tier already
      prices XLM-quoted candles from the measurement inside the oracle window
      (3,601 of 3,643 on 2026-08-01). The real defect is 0268's, one hop over:
      pivot_sql never multiplies by the USDC/USD rate, so every pre-epoch
      XLM/USDT-quoted candle still assumes USDC = $1 — 2023-03-11 is stored
      +3.2 % (7,328 daily candles). RESCOPED: (1) scale the pivot by the
      measured USDC rate at the bucket end, 0268's shape; (2) full re-enrichment
      of the ≈190 M XLM-quoted + ≈3.2 M USDT-quoted pre-epoch candles, bounded
      and resumable; (3) snapshot XLM readings into usd_rate under a set named
      beside peg_identities(). Retention is NOT a TTL — it is the dark
      cleanup-worker's policy, earliest loss 2027-04. Decisions ratified by
      Adam the same day. Converted to a directory.
  - date: "2026-09-11"
    status: active
    who: akot
    note: >
      Branch work complete in 3 commits on
      `fix/0228_xlm-oracle-rate-is-measured-then-discarded-while-the-pivot-derives-it`.
      (1) `pivot_sql` rewritten: a candidate subquery projecting the bucket end,
      the inline reference vwap, then two NESTED method-specific ASOF legs on
      `usd_rate` for canonical USDC — oracle preferred over external, no argMax —
      writing `close_usd = close x vwap x usdc_usd`; a bucket with neither rate is
      left unpriced. (2) A separately named pivot-leg reset mode
      (`require_pivot_usdc_rate`), its CLI flag, five refusals and runbook
      Appendix C, plus `post_run_0228_it.rs`. (3) `measured_identities()` beside
      `peg_identities()` returning `[Native]`, snapshotted by a SECOND, separate
      `populate_usd_rate_from_oracle` call. 723 tests pass across the four
      packages (40 of them new); the two `peg_identities` diff gates against
      origin/develop are empty. The 8 new `#[ignore]` ClickHouse tests were
      written and compile but were NOT RUN —
      no ClickHouse is reachable on this machine, exactly as 0268 recorded. The
      re-enrichment campaign itself is an operator CHORE, not started.
      Independent verifier: human_needed only on the ClickHouse-gated fifth
      must-have; code review: 0 blockers, WR-01 (dry run skipped the pivot
      window guard) fixed in a fourth code commit. STATUS STAYS ACTIVE — the
      campaign, the deploy and the #[ignore] runs are the operator's.
  - date: "2026-09-11"
    status: active
    who: akot
    note: >
      Prove run (`/prove`) on the branch against a rootless ClickHouse
      26.3.10.60 — the prod version. All 43 `#[ignore]` ch_enrich_it tests
      and the 5 usd_rate_population_it tests PASS; a hand-built Appendix C
      campaign with the real `coarse-repair` binary scales every one of the
      six tables to 0.9681 on 2023-03-11 and is value-idempotent at
      version 5. Two defects found and fixed in a fifth code commit:
      (1) `post_run_0228_it` decoded a `Nullable(Float64)` ratio into `f64`
      and reported ±1e230 on a correctly repaired table — the acceptance
      gate could never pass on prod; now `Option<f64>` like 0268's twin,
      NULL is a harness finding, CI test added. (2) The
      `ResetRequiresExternalRates` refusal both flags' `--help` promised
      was unreachable through the CLI (the unloaded series EMPTIES the
      month enumeration, so the run ended green with "0 month(s)"); the
      driver now checks it before enumerating months, dry run included,
      #[ignore] test added. 44 + 5 ClickHouse tests, 724 CI tests, clippy
      -D warnings clean. STATUS STAYS ACTIVE for the same reasons as before.
---

# We measure XLM's dollar price, throw it away, then derive it from USDC

## Phase 0 outcome — 2026-09-11, read this before the rest

The sections below are the task **as filed** (2026-08-26). Phase 0 measured
them on prod — [notes/R-phase0-measurement-2026-09-11.md](notes/R-phase0-measurement-2026-09-11.md)
— and three of the premises did not survive:

1. **The measurement is not discarded where it exists.** Reflector's `XLM`
   resolves to `AssetIdentity::Native`, and the oracle tier prices XLM-quoted
   candles from it directly inside the oracle window. The pivot only prices
   the deep history, where there is no reading to discard.
2. **AC 1 is met as written.** Pivot vs Reflector over 4,413 hours: median
   −1.2 bps, 90 % within ±35 bps. Scaling by USDC/USD moves the median < 3 bps.
3. **Retention is not a 13-month TTL.** It is the dark cleanup-worker's
   policy; the readings are intact and the earliest possible loss is 2027-04.

What phase 0 found instead is **task 0268's defect one hop over**: `pivot_sql`
computes `ref_usd` in USDC units and never multiplies by the measured USDC/USD
rate. After 0268 rescaled every USDC-quoted candle, the XLM- and USDT-quoted
candles are the only stored prices still assuming USDC = $1. On 2023-03-11 that
is **+3.2 % on 7,328 daily candles** (49,090 hourly). The pre-epoch population
is **≈ 190 M XLM-quoted + ≈ 3.2 M USDT-quoted** candles across seven tables.

### Rescoped implementation (ratified by Adam, 2026-09-11)

1. **Scale the pivot** — `pivot_sql` multiplies `ref_usd` by the USDC/USD
   rate from `usd_rate` (`external` before the epoch, `oracle` after) resolved
   at the **bucket end**, the same ASOF shape and staleness rule as 0268's
   `external_sql`. One statement, so XLM and USDT are fixed together. The
   `traded` wire label does not change meaning.
2. **Re-enrich the full pre-epoch population** with 0268's machinery: bounded
   by the 0111 partition window, resumable, FREEZE per partition, a reset that
   zeroes only what the same pass can refill. Two things do not carry over
   and must be built, not loosened: the reset's `require_external_rate` path
   is USDC-only by design (a new, separately named pivot-leg mode with its own
   refusal test), and the par signature `close_usd = close` does not select a
   pivoted row (a new one-definition-three-sites predicate: pivot leg,
   pre-epoch, `close_usd > 0`, a positive USDC rate exists for the day).
   Runs as its own CHORE with a runbook appendix, like 0276.
3. **Snapshot XLM readings into `usd_rate`** as `method = 'oracle'`, `hops = 0`
   — what they factually are — through a set named for what it is beside
   `peg_identities()`, whose test stays byte-identical. The identity gate is
   code (`reflector_key_to_identity("XLM") == Native`, no issuer to
   mis-attribute), pinned by a test to 0173's standard. Before the first row is
   written, check how `views.sql`'s `price_usd_series*` and `/ohlcv` treat a
   native row in `usd_rate`. Correct the stale `202509` comment in
   `oracle-worker/src/lib.rs`.

Out of scope, unchanged: 0227, 0173, and the `redstone` rows the note reports.

---

## Summary

Reflector reports XLM's USD price every 5 minutes and we keep **none of it**. The
readings sit in `prices.oracle_prices` until the 13-month retention prunes them,
and are never snapshotted into the forever-retained `prices.usd_rate`.

At the same time, the enrichment **pivot tier** prices every XLM-quoted candle by
deriving XLM's dollar value from XLM/USDC candles — an indirect route that is
dollars only because USDC is assumed to be $1.

So the direct measurement is discarded and an inferred substitute is used in its
place, on the reference asset that the largest part of the store depends on.

## Measured on prod — 2026-08-26

```
code   oracle_readings   oldest                newest                usd_rate_rows
XLM    51,235            1970-01-21 15:41:56   2026-08-26 10:00:00   0
USDC   51,235            1970-01-21 15:41:56   2026-08-26 10:00:00   48,049
```

⚠️ The `1970` oldest is [[0227]]'s timestamp defect, unrelated to this task.
The `48,049` vs `51,235` gap is the same defect — it is not evidence of a
snapshotting problem for USDC.

🔑 **The number that matters is XLM's `0`.** It is not a shortfall; nothing is
written for XLM at all.

## Why it is zero — by design, and the design is narrow

`oracle-worker/src/lib.rs` snapshots only `peg_identities()`, which is exactly
canonical USDC and nothing else. There is a test pinning that:

> *"the peg set must be exactly canonical USDC. Adding a member is a claim that
> its oracle feed names that ISSUER, not just that code — write the evidence in
> the doc comment above before changing this."*

That test is **correct and should not be weakened**. It exists because of
[[0173]]: Reflector prices the *ticker* USDT — Tether's own token, genuinely at
par — and filing that under the Stellar issuer's address asserts ~$1.00 for an
asset worth $0.13. The lesson is that a symbol is not an identity.

⚠️ So this task is **not** "add XLM to `peg_identities()`". XLM is not a peg, and
the set is named for what it is. The question is whether a *second*, differently
named path should snapshot measured non-peg rates — and what the evidence is that
Reflector's `XLM` symbol names our native asset rather than something else.

## What the pivot does instead

From `ch_enrich.rs`:

> **pivot:** a candle quoted in a *measured* reference asset gets
> `close_usd = close × ref_usd`, where `ref_usd` is that asset's
> **volume-weighted close against USDC** at or before the bucket, forward-filled
> by an `ASOF LEFT JOIN`.

Follow it through: XLM's dollar value comes from XLM/USDC candles, and those are
dollars only via the $1 peg. So ~11 M XLM-quoted candles rest on the peg
assumption one hop away — while an independent measurement of the same quantity
exists and is discarded.

⚠️ **This is a structural observation, not a measured error.** The derived value
may track the measured one closely. Nobody has checked. That check is AC 1 and it
decides whether this task is worth anything at all.

## Why it could matter

- **[[0172]]/[[0182]] is the precedent.** Assuming a stablecoin sits at par cost
  567,760 corrected candles when USDT turned out to trade at ~$0.13. The USDC peg
  is a much better assumption than the USDT one was — but "much better" is what
  was believed about USDT too, and the answer came from measuring.
- **[[ADR-0011]] leans on the rate being measured.** Its load-bearing argument is
  that the USD-per-USDC rate wobbles 0.9976-1.0008 and so a flat $1 is wrong. The
  same reasoning applies to a pivot resting on that peg.
- **Retention makes it irreversible.** Every day of unsnapshotted XLM readings
  ages out for good. Whatever is decided, the readings currently in
  `oracle_prices` are recoverable only until the 13-month window passes them.

## Implementation

- **First, measure the disagreement.** Compare, over the window where both exist,
  the pivot's derived `ref_usd` for XLM against Reflector's measured XLM reading
  at the same bucket. Report the distribution, not a single summary number.
  ⚠️ If they agree to within noise, close this task with that evidence — it is a
  legitimate outcome and cheaper than the alternative.
- If they diverge, options to cost:
  1. **Snapshot measured non-peg rates** into `usd_rate` under a correctly-named
     path, with `method` reflecting that they are measured readings. Requires the
     symbol-to-identity evidence [[0173]] demands.
  2. **Have the pivot prefer a measured rate** where one exists, falling back to
     the derived one otherwise — which reintroduces a provenance question the
     response must carry.
  3. **Backfill `usd_rate` from `oracle_prices`** for XLM before the retention
     window passes, independently of which source the pivot uses. Cheap, and it
     stops the bleeding while the rest is decided.
- ⚠️ Whatever ships must not weaken the `peg_identities()` test. If a new set is
  needed, give it its own name and its own evidence.

## Acceptance Criteria

- [x] The derived `ref_usd` and the measured Reflector reading for XLM are
      compared over the overlapping window, and the distribution of the
      difference is recorded. **This gates the rest of the task** — agreement to
      within noise is a valid close.
      → **Met by phase 0** ([notes/R-phase0-measurement-2026-09-11.md](notes/R-phase0-measurement-2026-09-11.md)):
      4,413 hours, p50 −1.2 bps, p05/p95 ±35 bps, |p99| 144 bps. They agree to
      within noise, which under the criterion as written was a valid close — and
      phase 0 found the real defect one hop over while measuring it.
- [x] If they diverge, the affected candle population is counted before any fix
      is chosen.
      → **The phase-0 population table is the count.** Pre-epoch XLM-quoted:
      1m 72.5 M, 15m 8.9 M, 1h 64.8 M, 4h 29.0 M, 1d 10.4 M, 1w 3.3 M, 1M 1.4 M.
      USDT-quoted: 1.55 M / 0.57 M / 0.70 M / 0.26 M / 70.7 k / 11.8 k / 4.4 k.
      Counted before any fix was chosen; it is what made the campaign a separate
      CHORE rather than part of this branch.
- [x] The evidence that Reflector's `XLM` symbol names our native asset — not
      merely a matching ticker — is written down, to [[0173]]'s standard.
      → In `measured_identities()`'s doc comment, and as CODE:
      `measured_identities_resolve_from_the_reflector_symbol_mapping` calls
      `reflector_key_to_identity("XLM")` and asserts `AssetIdentity::Native`.
- [x] `peg_identities()`'s test is intact, or its replacement carries the same
      "a symbol is not an identity" guarantee with the same explicitness.
      → Intact, **proven by diff against `origin/develop`**, not by eye: both the
      fn body and `peg_identities_is_exactly_canonical_usdc` are byte-identical.
      `measured_identities_is_exactly_the_native_asset` is the new set's own
      pinning test, worded to force the same evidence-first edit.
- [x] Whether the readings currently in `oracle_prices` are preserved before the
      13-month window passes them is decided explicitly, not by default.
      → **Decided: preserve them, now.** The snapshot copy is gap-filling rather
      than watermarked, so the first run after deploy copies all 52,607 existing
      XLM readings in one statement — no backfill tool is needed. The stale
      `202509` paragraph that deferred this is replaced with the measured facts.

### The rescoped criteria (BRIEF §4)

| # | Criterion | Closes |
|---|-----------|--------|
| 1 | `pivot_sql` multiplies by the USDC/USD rate at the bucket end, with `external_sql`'s invariants | ✅ **on the branch** — code + SQL-string tests |
| 2 | An XLM-quoted 1d candle on 2023-03-11 comes out ≈3.2 % low | ⚠️ **test written, NOT RUN** (no ClickHouse here); closes on prod after the campaign, via `post_run_0228_it` |
| 3 | The pivot-leg reset is bounded, resumable, one-predicate-three-sites, refuses what it must | ✅ **on the branch** for the pure parts; the refusal/idempotence tests are `#[ignore]` and unrun |
| 4 | Runbook appendix with preconditions, baseline, expected figures, rollback | ✅ **on the branch** — Appendix C |
| 5 | XLM readings land in `usd_rate` as `oracle`/`hops 0` through a named set | ✅ **on the branch** in code; the rows appear on the **first prod run after deploy** |
| 6 | Lore AC 1 recorded as met, AC 2's count as the phase-0 table | ✅ **this file** |

## Out of scope

- [[0227]]'s timestamp unit bug, though it affects the same table and its
  ~30% loss must be accounted for when measuring the overlap above.
- [[0173]]'s USDT mis-attribution.
- Re-enriching existing candles — that follows the measurement, and only if it
  shows a material difference.

## Implementation Notes

Three commits on
`fix/0228_xlm-oracle-rate-is-measured-then-discarded-while-the-pivot-derives-it`.

### 1. `fix(lore-0228): scale the enrichment pivot by the measured USDC/USD rate`

`packages/enrichment-worker/src/ch_enrich.rs`,
`tests/ch_enrich_it.rs`, `packages/prices-clickhouse/schema/init.sql`.

`pivot_sql` is now four levels instead of one:

1. a **candidate subquery** over `{tbl} FINAL` carrying the existing filters and
   projecting `{bucket_end_expr} AS bend` plus a constant join key;
2. that, ASOF-joined to the **inline reference vwap** subquery — unchanged, still
   no DDL (task 0083), still resolved at the bucket START within the bound
   `pivot_window_s`;
3. that, ASOF-joined to the **oracle** `usd_rate` leg;
4. that, ASOF-joined to the **external** leg, with
   `close_usd = CAST(refusd × toFloat64(close) × toFloat64(rate) AS Decimal(38,14))`.

**Why the factor went inside `pivot_sql` rather than on the read side.** BRIEF
decision A, ratified. Multiplying at read time is the 0267 state 0268 ended:
stored and served disagree, and every unguarded `argMax(close_usd, …)` site —
about 130 of them — reads the stored number, not the view.

**Why the two rate joins are NESTED.** `queries_ch::peg_series_sql` is the only
in-repo precedent for two method-specific ASOF joins and its doc gives the
reason: a nested subquery needs nothing from the multi-JOIN rewrite and reads the
same under both analyzers. The two right sides carry distinct column names
(`orts`/`orate`, `erts`/`erate`) for the same reason. A read-only probe on prod
ClickHouse 26.3.10.60 ([notes/sql/q9_asof_shape.sql](notes/sql/q9_asof_shape.sql))
showed that SAME-level chaining also executes there, which retires RESEARCH
assumption A2 — but nesting is what shipped, because it is the proven form and
it makes "oracle over external" explicit rather than incidental.

**Why the bind count stayed at 4 and the staleness is inlined.** Each `?` is a
separate positional parameter, so referencing a bound staleness from both rate
legs would have cost two more binds. `external_window_s` is derived from the
table by design — its own doc explains that a derived bound cannot be set wrong,
and `coarse-repair.rs` carries an assert-shaped guard that assumes it stays
derived — so it is inlined as a literal. The bind ORDER did change, to
`watermark, watermark, pivot_window_s, LIMIT`: the candidate scan is now a
subquery rendered FIRST, so its watermark sits textually before the reference
subquery's.

**The steady-state behaviour change, stated.** A pivot-leg candle with no USDC
rate in window is now left at `close_usd = 0` where it used to be priced at the
raw vwap. Pre-epoch this cannot happen (every XLM-quoted priced candle is
≥ 2021-01-25 and has an external rate for its day); post-epoch it depends on
USDC's `oracle` series being continuous.

`init.sql`'s `method` vocabulary block records D-05: no word is coined for this
composition, and `/ohlcv` still labels a pivot leg `traded`.

### 2. `fix(lore-0228): add the pivot-leg reset mode, its campaign tooling and runbook Appendix C`

`ch_enrich.rs`, `bin/coarse-repair.rs`, `tests/ch_enrich_it.rs`,
`tests/post_run_0228_it.rs` (new), `docs/runbooks/repair-coarse-usd-values.md`.

`UsdResetSpec` gains `require_pivot_usdc_rate: bool`, appended after
`require_external_rate` so a spec asking for neither renders the pre-0228
strings byte-for-byte — which the existing
`a_0182_shaped_spec_renders_byte_identically_to_the_pre_0268_statement` now
proves for both tasks at once.

**Why the new mode reuses `external_rate_day_pred` verbatim instead of coining a
span predicate.** The bucket's UTC start day is sufficient at EVERY grain: a rate
stamped at that day's 00:00 is at most `bucket_width + 86,400` old at the bucket
end, and `external_window_s = max(86_400, bucket_width)` admits exactly that —
including `_1M`, where 2,678,400 s covers a 31-day March. Reusing the existing fn
means one definition, three sites, and one already-proven test to extend, instead
of a second span-shaped predicate whose boundary behaviour would have to be
re-argued. The alternative (an `arrayExists` span-set mirroring
`usd_method_expr`) would reach slightly more `_1w`/`_1M` buckets; under-reach is
the documented safe direction, because a skipped refillable row costs one stale
value while a zeroed unrefillable row is the incident.

**Why one leg per run.** `UsdResetSpec`'s own doc: the blast radius must be
nameable before the statement runs. `reset_sql` and `reset_pending_pred` keep
their single `quote_asset_id`; the campaign is two passes per table.

Refusals, in `reset_step`: the leg must be in `ReferenceIds::pivot_ids()` (new,
`assert_pivot_rate_leg_is_a_pivot_reference` — the mirror of
`assert_external_rate_leg_is_usdc`), the day-set must be non-empty (reused
verbatim), the span must not be oracle-shadowed and the target must be priceable
(both already unconditional), and the window must be non-empty and the two modes
not combined (both pure, in `validate()`, so the CLI refuses before opening a
connection).

`post_run_0228_it.rs` copies `post_run_0268_it.rs`'s shape: `#[ignore]`d tests
against a real database, a pure `judge()` covered by 12 unit tests CI does run,
a `RatioUnder` ceiling for the grains whose bucket ends inside the depeg and a
`Mechanism` check for `_1w`/`_1M`, plus a second, non-depeg date so a uniformly
low table cannot pass.

### 3. `fix(lore-0228): snapshot XLM Reflector readings into usd_rate`

`packages/oracle-worker/src/lib.rs`, this file.

`measured_identities()` returns `[AssetIdentity::Native]`, with its evidence in
the doc comment and in two tests. The snapshot site issues **two separate**
`populate_usd_rate_from_oracle` calls, each with its own non-fatal arm and its
own per-identity log line, summing into `rates_snapshotted` so
`OracleUsdRatesSnapshotted` keeps its meaning.

**`populate_usd_rate_from_oracle`'s guards, read against the current tree and
reported rather than worked around** (BRIEF decision D asked for exactly this):

| Guard | Where | Peg-specific? |
|-------|-------|---------------|
| Task 0139 identity pre-pass (exactly one `asset_id` per identity, exactly one identity per `asset_id`) | `writer.rs` pre-pass | **No** — an identity-collision guard |
| `o.price_usd > 0` | the copy's `WHERE` | **No** |
| `o.oracle_name = ?` | the copy's `WHERE` | **No** — parameterized, not a literal |
| `o.timestamp > ORACLE_EPOCH_FLOOR` (2020-01-01) | the copy's `WHERE` | **No** — task 0086's junk-1970 floor |
| Rows written `'oracle', '', 0` | the copy's projection | **No** — what a polled reading factually is |
| `identity_columns(Native)` | `writer.rs` | Supported by construction: `("native", "XLM", "", "")` |

Nothing has moved since RESEARCH read them, and none of them assumes peg
semantics. **Confirmed: the copy is gap-filling, not watermarked** — a
`LEFT ANTI JOIN` on `(timestamp, usd_rate)` whose only lower bound is
`ORACLE_EPOCH_FLOOR`, deliberately so that backdated readings from
`sdex-backfill` are not stranded. So the first run after deploy copies all
52,607 existing XLM readings in one statement and no backfill tool is needed.

**D-09 — the views/API gate was EXECUTED and PASSED.** This is a finding, not a
waiver: every reader of `usd_rate` in the repo pins the full canonical USDC
identity tuple, so a native row joins nothing and no served output moves.
Re-verified against the current tree, not taken from RESEARCH:

- `views.sql` `price_usd_series` — the rate subquery is generic over identities,
  but the JOIN is keyed on arm B's identity columns, and arm B's `WHERE` is
  `q.contract_address = '' AND q.asset_code = 'USDC' AND q.issuer_address = 'GA5Z…KZVN'`.
  (Its projection has a `'native'` branch in the `asset_kind` `multiIf`; that
  branch is unreachable under the `WHERE` below it.) Same for the `_1h` twin.
- `queries_ch::peg_series_sql` — both ASOF subqueries pin the USDC tuple.
- `queries_ch::usd_method_expr` — `imported_days` pins it plus `method = 'external'`.
- `current.sql` — `usdc_rate` is allowlisted to USDC by name, behind a fence.
- `handlers.rs` — `is_peg_asset` is true only for canonical USDC, so an XLM
  request never reaches `ohlcv_peg_series`.

**Residual, stated rather than hidden:** the native rows land under
`method = 'oracle'`, which `init.sql` already defines as "a measured reading,
polled from Reflector (hops = 0)" — factually what they are. `init.sql`'s
"ABSENCE IS THE SIGNAL" prohibition targets synthetic `peg` fills, not measured
readings, and nothing in the DDL restricts `usd_rate` to peg identities.

## Design Decisions

### From Plan

1. **D-01 — the USDC/USD factor enters inside `pivot_sql`** (BRIEF decision A,
   ratified). The inline reference vwap stays; the outer statement ASOF-joins
   `usd_rate` for canonical USDC at the candidate's bucket END with 0268's
   staleness rule; `close_usd = close × vwap × usdc_usd`. Rate preference
   `oracle` then `external`; neither → the row is left unpriced. One statement
   covers XLM and USDT.
2. **D-02 — the campaign is the FULL pre-epoch pivot-leg population** (decision
   B, ratified by Adam), bounded per 0111 partition window, resumable, FREEZE
   per partition, dry-runnable. The prod run is an operator CHORE; this branch
   ships code, tests, runbook and tool only.
3. **D-03 — a separately named reset mode for pivot legs** (decision C): one
   predicate rendered at three sites, refusing the USDC leg and an
   oracle-shadowed span, re-opening nothing the same pass cannot refill.
4. **D-04 — a new fn beside `peg_identities()`** returning `[Native]`, with its
   own pinning test and the identity evidence as a test. `peg_identities()` and
   its test byte-identical. Rows written `method='oracle'`, `hops=0`.
5. **D-05 — no new `method` word.** `traded` keeps its meaning; the composition
   is documented in `pivot_sql`'s doc comment and `init.sql`'s vocabulary block.
   The OpenAPI `Candle.method` text was CHECKED, not assumed: it describes
   `traded` as "priced through a reference asset's own trades" and makes no
   claim about dollars via the peg, so no OpenAPI change was needed.
6. **D-06 — idempotence is VALUE-idempotence** (CONTEXT amendment): a second run
   recomputes identical values at `version + 2`; operator-only, never in the
   recurring sweep. See Issues 1 for the deviation this records.
7. **D-07 — `price_ohlcv_1m` is EXCLUDED from the campaign.** `coarse-repair`
   keeps its refusal, untouched. The pivot SQL fix still applies to every table.
8. **D-08 — two separate `populate_usd_rate_from_oracle` calls**, because the
   0139 pre-pass aborts a whole slice on one bad identity.
9. **D-09 — the views/API gate PASSED**, recorded as a finding. See
   Implementation Notes, commit 3.

### Emerged

Everything below was decided by the executor; the plan left it open.

10. **The reset mode is a second `bool` on `UsdResetSpec`, not a sibling type.**
    0268 settled the same question the same way, and appending keeps the 0182
    `usdt_reset()` fixture byte-identical while reusing `not_after` and
    `validate()`.
11. **`assert_hourly_rates_are_loaded` is NOT called for the new mode**, and the
    plan explicitly asked for this to be decided and stated. 0268 needs that gate
    because its candidate is the par signature `close_usd = close`, which a
    sub-daily candle priced from the DAY close stops carrying — so the repair is
    one-shot and a later hourly load can never reach it. A pivoted row carries no
    signature, so this mode's candidate still matches after a repair: loading the
    hourly file later and re-running simply recomputes. **The gate guards an
    irreversibility that does not exist here.** Appendix C says so in
    precondition 1, and recommends loading the hourly file anyway for accuracy.
12. **`assert_no_pre_epoch_oracle_rows` is NOT called for the new mode either.**
    It measures the EXTERNAL tier's own premise — that tier recomputes
    `volume_quote_usd` unconditionally below the epoch and its safety rests on no
    poll having priced USDC there. The pivot keeps `volume_quote_usd` write-once,
    so the premise is not load-bearing here. The window-scoped oracle-shadow
    guard, which runs for every spec, is what stops the oracle tier re-pricing
    anything this mode re-opens.
13. **`repair_target_pred` needed no change**, as the plan anticipated: it
    composes `reset_pending_pred`, so the new fragment reaches the month
    enumeration for free. The three-sites test proves it.
14. **The mutual-exclusion refusal lives in `validate()`, not in clap.** One
    definition, so no driver — CLI or otherwise — can assemble the combination,
    and the CLI still refuses before opening a connection.
15. **`ResetRequiresExternalRates`'s message was generalized** to name both
    flags and both runbook appendices. The variant and its field are unchanged,
    so the existing test still matches; only the operator-facing text moved.
16. **`plan_peg_pivot_step` takes TWO window fragments.** The peg statement scans
    the table directly and needs `p.timestamp`; the pivot's candidate scan is now
    a subquery with no alias in scope and needs the bare column. One string
    handed to both would be a syntax error on whichever it did not fit. Pinned by
    `the_peg_and_pivot_windows_are_qualified_differently`.
17. **`run_peg_pivot_tier`'s no-progress break stays `warn!`**, where
    `run_external_tier`'s twin is `info!`. The plan asked to align them or say
    why not. They are not the same situation: the external tier has the peg tier
    below it, so its break is a handover; the peg-pivot tier is the LAST one, and
    its leftovers are published as `close_usd = 0`, which ~130 unguarded
    `argMax(close_usd, …)` sites read as a real price. What did change is the
    message, which now names the new cause (a pivot leg with no measured USDC
    rate in window) — the old wording said "exotic quotes" and would have been
    wrong about the commonest case after this change.
18. **The falsifier's ceiling is a RATIO, self-calibrating.** The plan's runbook
    baseline is the median implied reference rate `close_usd / close` — cheap and
    uncorrelated, and that is what Appendix C tells the operator to record. But
    for a pivot leg that number is XLM's USD price, which has no fixed expected
    value, so a hard-coded ceiling would be a magic constant tied to one measured
    market price. `post_run_0228_it` therefore divides it by the reference
    market's own bucket vwap from the same table, giving the USDC/USD factor the
    stored value carries: exactly 1.0 if the campaign never ran, 0.9681 after.
    Both are in the runbook; the cheap one is the eyeball, the ratio is the gate.
19. **`_15m` is IN the falsifier's grain list**, unlike `post_run_0268_it`.
    Appendix B says `_15m` has a 30-day retention and holds no 2023 rows, but
    phase 0 measured 8.9 M pre-epoch XLM-quoted `_15m` rows on 2026-09-11. One of
    the two is stale and this branch cannot settle it, so Appendix C tells the
    operator to settle it with a count before deciding. See Issues 5.
20. **New integration fixtures seed a USDC rate of EXACTLY 1.0.** Making the rate
    mandatory in the pivot breaks every pivot fixture that seeds none. A rate of
    1.0 keeps each test testing what it is named for (tier composition, the
    measured-USDT rate, the 0182 reset's refill path) instead of re-deriving new
    expected values, and the scaling itself is proven by dedicated tests.
21. **The falsifier's `ratio` is `Option<f64>`, and `None` behind `matched > 0`
    is a finding that names the HARNESS.** The column is `Nullable(Float64)`
    because the reference vwap divides by `nullIf(sum(volume_base), 0)`; the
    alternative — `assumeNotNull` in the SQL — would keep the struct plain but
    silently turn "no reference" into `0.0`. `post_run_0268_it` already carries
    `rate: Option<f64>` for the same reason, so the twins now share the shape,
    and the operator reading "the harness query is wrong" is a different
    situation from "the table is wrong" — the message says which.
22. **The no-rates refusal lives in `CoarseRepairDriver::run`, not only in the
    pass, via one free fn `assert_external_rates_are_loaded` that both call.**
    Putting it in the CLI would have covered one driver; putting it in `run()`
    covers every driver and the dry run, which WR-01 already established must
    refuse what the real run refuses. One definition, two call sites, so the
    two cannot drift.

## Issues Encountered — for Adam to route

1. **🔑 The D-06 deviation from BRIEF decision C, with its evidence.** Decision C
   required "a second run over a repaired month must find zero candidates". That
   is unattainable for a pivot leg without a schema change, and the reason is
   structural rather than a matter of effort. 0268 gets it free because its
   candidate carries a SELF-ERASING signature, `close_usd = close`: once the row
   is re-priced it stops matching. A pivoted row never carried such a signature —
   it was `close × vwap`, which equals `close` only by coincidence.
   Distinguishing "already scaled" from "not scaled" means comparing
   `close_usd / close` against the bucket's own reference vwap, i.e. a
   **correlated** join — and `repair::months_with_zeros` splices the predicate
   into a bare grouped `WHERE` where no outer alias is in scope, so a correlated
   form is a syntax error there, not a style choice. `version` cannot substitute:
   coarse rollups carry large SUMMED versions (`coarse_repair_row_outranks_large_summed_version`).
   Adam ratified value-idempotence on 2026-09-11; this records the deviation.
   The cost is real and is in the runbook: a second run rewrites correct values,
   bumps `version` by 2 and spends the FREEZE rollback point.
2. **🔑 RESEARCH assumption A4 is a BLOCKING PRECONDITION for the campaign, not
   a code defect.** `assert_reset_not_shadowed_by_oracle` counts `oracle_prices`
   rows for the reset's quote leg in `[not_before − window_s, not_after)`. XLM
   **is** polled and has held readings since 2026-03-11; `--reset-not-after`
   defaults to 14:00 that day. **Any XLM reading stamped earlier that day refuses
   the entire campaign — every table, every month.** Appendix C precondition 5 is
   the query that finds out first, and the remedy is an explicit
   `--reset-not-after <first reading>`, never a purge and never a widened window.
   Unmeasurable from this branch (no prod access).
3. **The `volume_quote_usd` write-once residue.** The pivot keeps its
   `if(p.volume_quote_usd > 0, …)` guard per BRIEF §5, so a row enriched before
   `close_usd` existed keeps a par-derived volume figure beside a newly scaled
   close. Inside the campaign's reset span this is harmless — the reset zeroes
   both columns, so both are recomputed from one reference. Outside it, the
   residue stands. 0268's external tier drops the guard for exactly this reason,
   but its argument rests on an epoch bound the pivot does not have. Kept as the
   BRIEF requires; stated here so it is a known state rather than a discovery.
4. **The `redstone` rows.** `oracle_prices` holds 419,519 rows at `asset_id = 0`,
   `price_usd = 0`, under `oracle_name = 'redstone'`. Untouched, as instructed.
   They are not this task's, but nothing owns them — see the spawn list.
5. **`_15m`'s retention is described two ways in the repo.** Appendix B says a
   30-day retention leaves no 2023 rows; `cleanup-worker`'s `RETENTION` agrees
   (`INTERVAL 30 DAY`); phase 0 measured 8.9 M pre-epoch XLM-quoted `_15m` rows
   on prod on 2026-09-11. The likeliest explanation is that the cleanup worker is
   **dark**, so the policy has never been applied — which would mean the rows are
   real and in scope, and that they will vanish the day it is enabled. Appendix C
   makes the operator count before deciding. Worth settling properly.
6. **D-09 is a PASSED GATE, not a waiver** — repeated here because BRIEF decision
   D framed it as a possible stop-and-report. It was executed, it passed, and the
   evidence is in Implementation Notes. No served output moves when the first
   native `usd_rate` row appears.
7. **Nine pre-existing clippy warnings in `prices-ingest-core`**
   (`canonical.rs`, `soroban.rs` — collapsible `if`s, no-op masks, a manual
   `div_ceil`). Present on `origin/develop`, in files this task did not touch, so
   left alone. `cargo clippy -p enrichment-worker --features aws-mtls
   --all-targets -- -D warnings` is clean, and so are `oracle-worker` and
   `prices-api`.
8. **🔑 The acceptance gate was broken (found by the prove run, fixed).**
   `post_run_0228_it`'s `measure()` returns `ratio` as `Nullable(Float64)` —
   the reference vwap's `nullIf` makes the whole median nullable — while
   `Measurement` declared `f64`. RowBinary decoded one byte off and the
   falsifier reported a carried factor of `-3.9e230` on a table the tool had
   just repaired to exactly 0.9681. On prod it would have failed the campaign
   forever, whatever the data said. Fixed to `Option<f64>` (Decision 21);
   proven both ways on the local engine: passes on the repaired database,
   fails with "factor 1.000000 >= 0.99 — indistinguishable from par" on an
   unrepaired one. CI test `a_null_factor_behind_matched_rows_is_a_harness_finding_not_a_pass`.
9. **`--help`'s "Refused outright when `usd_rate` holds zero `external` rows"
   was false through the CLI, for BOTH modes (found by the prove run, fixed).**
   The refusal sat in the per-month pass, but the day-set predicate is also
   part of `months_with_zeros`, so an unloaded series produced zero months, no
   pass, and `exit 0` with "0 month(s): 0 enriched". Inherited from 0268
   (Appendix B had the same gap). `CoarseRepairDriver::run` now runs the check
   first, dry run included (Decision 22); `#[ignore]` test
   `the_repair_driver_refuses_an_unloaded_series_before_enumerating_months`,
   and the real binary refuses with `ResetRequiresExternalRates`, exit 1, in
   both modes and in `--dry-run`.
10. **A refusal that fires inside the per-month pass leaves the FREEZE behind.**
    The driver freezes the partition, THEN builds the pass whose `reset_step`
    may refuse (`ResetPivotRateLegIsNotAPivotReference` is new here); the
    snapshot under `repair_0114_<db>_<table>_<month>` stays, and the next real
    run on that partition fails with `FreezeDenied … DIRECTORY_ALREADY_EXISTS`.
    Pre-existing 0114 driver order; prod is unaffected (`--skip-snapshot`);
    local/CI hits it. Recorded in Appendix C rather than fixed — reordering
    FREEZE after the pass's refusals is 0114's design to revisit. Spawn list 4.

## Broken/modified tests

Unit tests (`ch_enrich.rs`):

- `pivot_sql_computes_the_xlm_usdc_reference_inline` — kept every assertion that
  still holds (inline reference, no `CREATE TABLE`, the `ref_asset_id` cast, the
  ASOF equality). Dropped the bind-order block, which moved to its own test, and
  changed `WHERE p.quote_asset_id = 5` to the bare form now that the filter lives
  in the candidate subquery. Intentional: the statement was restructured.
- `pivot_sql_bounds_only_the_candidate_side_not_the_reference` — the window
  fragment changed from `p.timestamp` to the bare `timestamp`; the
  `toDateTime(100)` occurrence count stays 1 and now proves three subqueries
  unbounded instead of one; added an unbounded-render assertion. Bind count still 4.
- `pivot_sql_prices_usdt_quoted_candles_from_its_usdc_market` — the pair product
  assertion became the triple product, plus a check that the USDT leg reads both
  rate legs. Intentional: one statement fixes both references.
- `plan_issues_one_peg_and_two_pivots` — signature change only (two window
  parameters). Every assertion is unchanged.
- `reset_sql_will_not_reopen_a_row_the_pivot_cannot_refill` — the pivot's
  `volume_quote > 0` is now alias-free, so the assertion matches the bare form.
- `a_0182_shaped_spec_renders_byte_identically_to_the_pre_0268_statement` — no
  assertion changed; the doc now states that it pins the pre-0228 strings too.
- `usdc_external_reset()` / `usdt_reset()` fixtures — gained
  `require_pivot_usdc_rate: false`, explicitly.
- `post_run_0228_it.rs` (prove-run commit) — `Measurement::ratio` became
  `Option<f64>`; `carried()` wraps in `Some`, the "unmeasurable" fixture sets
  `None` instead of `NAN`, `par.ratio = Some(1.0)`. Intentional: the wire
  column is nullable (Issues 8). No assertion changed meaning.

Integration tests (`ch_enrich_it.rs`), all for the same reason: making the USDC
rate mandatory in the pivot means a fixture that seeds none now leaves its
subject at `close_usd = 0`.

- `enrich_fills_close_usd_across_oracle_peg_and_pivot_tiers` — seeds an
  `external` USDC rate of exactly 1.0 for the fixture's deep day. All asserted
  values are unchanged.
- `usdt_quoted_candles_pivot_on_the_measured_rate_not_a_dollar_peg` — same
  treatment, so it keeps testing the measured-USDT rate rather than the new one.
- `setup_0182` — seeds a rate of 1.0 at each fixture instant's UTC day start, so
  the reset tests built on it still have a working pivot refill path. This is the
  fixture behind six tests (`an_ordinary_pass_cannot_see_a_wrong_but_written_close_usd`,
  `the_usd_reset_recomputes_written_values_but_respects_the_epoch`, and the four
  refusal tests); none of their assertions changed.
- Eight `UsdResetSpec` literals gained `require_pivot_usdc_rate: false`.
- **Audited and NOT changed:** the coarse-repair and sweep tests at `:544-1100`
  and the frontier tests at `:1628-1900`. Every one of them uses FOO/USDC (the
  peg leg) or FOO/EXO (no reference); the single FOO/XLM candle in
  `the_frontier_advances_exhausts_and_never_revisits` is deliberately unpriceable
  (no XLM/USDC market is seeded) and the test asserts it enriches nothing.

New tests, 40 in all:

| Where | Count | What |
|-------|-------|------|
| `ch_enrich.rs` units | 12 | the scaled pivot's invariants (triple product, both rate legs' identity tuple and positivity, bucket-end resolution on every grain, nesting, inlined staleness, unpriced-without-a-rate, versioned insert, bind order, the window alias split) |
| `ch_enrich.rs` units | 6 | the pivot-leg reset mode (three-sites, no par signature, the alias split, mutual exclusion, the partition window, the empty window) |
| `ch_enrich_it.rs` `#[ignore]` | 3 | the 2023-03-11 scaling on `_1d` and `_1h`, oracle-over-external, no-rate-left-unpriced |
| `ch_enrich_it.rs` `#[ignore]` | 5 | the reset mode's four refusals and its value-idempotence |
| `post_run_0228_it.rs` | 12 | the pure `judge()`, which CI runs |
| `oracle-worker/src/lib.rs` | 2 | the measured set's pinning test and its identity evidence |

## Verification run on the branch

```
cargo test -p enrichment-worker -p oracle-worker -p prices-ingest-core -p prices-api   → 723 passed, 0 failed
cargo clippy --no-deps -p enrichment-worker -p oracle-worker -p prices-ingest-core -p prices-api
                                                                                      → clean except 9 pre-existing prices-ingest-core warnings (Issues 7)
cargo clippy -p enrichment-worker --features aws-mtls --all-targets -- -D warnings    → clean
npm run -s verify:staged (via the pre-commit hook, each commit)                       → passed
diff origin/develop peg_identities() body                                             → empty
diff origin/develop peg_identities_is_exactly_canonical_usdc                          → empty
```

### The `#[ignore]` ClickHouse suite — RUN, 2026-09-11 (prove run)

The docker socket is root-only for this user, so the first pass recorded these
as "written, compile, NOT RUN". The prove run then started
`clickhouse-common-static-26.3.10.60` — the exact prod build — as a rootless
process in a scratch directory and ran everything against it:

```
cargo test -p enrichment-worker --test ch_enrich_it -- --ignored              → 44 passed, 0 failed  (43 + the new driver refusal)
cargo test -p prices-ingest-core --test usd_rate_population_it -- --ignored   → 5 passed, 0 failed
cargo test -p prices-api -- --ignored                                          → 9 passed, 1 failed: backfill_status_maps_both_streams
                                                                                 ("stalled" vs "running"; prices-api is untouched by this branch, pre-existing)
```

Beyond the suite, a hand-built Appendix C campaign with the real `coarse-repair`
binary (`--transport local`), on all six tables seeded with an XLM/USDC
reference at vwap 0.0588, a FOO/XLM subject stored UNSCALED at 0.588, and
0267's daily series for March 2023 (0.9681 on the 11th):

```
before:  close_usd 0.588   ratio_to_ref 1.0     version 1   (every table)
after:   close_usd 0.5692428   ratio 0.9681   version 3   (depeg bucket, 15m/1h/4h/1d)
         close_usd 0.5880588   ratio 1.0001   version 3   (recovered bucket; _1w/_1M rate at bucket end)
         FOO/XLM 2020-06-01 (no rate that day)  0.588  version 1   ← untouched
         FOO/USDC par control on the depeg day  5      version 1   ← untouched
rerun _1d:   identical values, version 5                          ← D-06 value-idempotence
post_run_0228_it (fixed):  2 passed on this database; 2 failed on an unrepaired copy
```

And the XLM snapshot path with the real writer and the real
`measured_identities()`: two readings land as `('native','XLM','','', …,
'oracle', '', hops 0)`, the 1970 junk reading is dropped, the peg pass adds
USDC beside them, a second measured pass inserts 0. With those native rows
present, `price_usd_series` / `_1h` serve `native XLM traded` from XLM's own
market and nothing keyed on the `usd_rate` rows (D-09, observed).

`post_run_0228_it` remains the operator's after-check and is EXPECTED TO FAIL
on prod until the campaign runs; its pure `judge()` tests pass in CI today.

One prod query WAS run this session, read-only: the two-ASOF shape probe
([notes/sql/q9_asof_shape.sql](notes/sql/q9_asof_shape.sql)) against ClickHouse
26.3.10.60. It returned, for XLM-quoted 1d candles on 2023-03-11, stored
`close_usd` 0.00144215598065 → scaled 0.00139618 (ref vwap 0.05882353 × rate
0.96812 `external`), i.e. **−3.19 %** — the acceptance figure, measured on the
engine, before any of this code existed.

### Independent review and verification, 2026-09-11

Both ran as separate agents after the three commits, on the branch, with no
access to the executor's notes beyond the plan:

- **Verifier** re-ran every command above itself (0 failed across 36 test
  binaries; clippy clean; both `diff` gates empty; every `#[ignore]` binary
  compiles) and read `pivot_sql` end to end. Verdict: 4 of 5 must-haves
  verified; the fifth — value-idempotence at `version + 2` — is implemented
  and wired but is a ClickHouse-backed test, so it stays **human_needed**
  until the harness runs.
- **Code review** (diff `develop..HEAD` against BRIEF §5): **0 blockers**.
  - **WR-01, fixed on the branch** — `coarse-repair` gated the
    `--pivot-window-s` minimum-width guard behind `!args.dry_run`, so
    Appendix C's "dry run first" would have accepted a window the real run
    then refused. The window guards now run in dry-run mode too; only the
    snapshot guard stays real-run-only (a dry run discards nothing).
    Inherited from the 0182 shape, so Appendix A/B had the same gap.
  - **WR-02, recorded** — `assert_pivot_rate_leg_is_a_pivot_reference`
    resolves the leg from `prices.assets` while `assert_external_rates_are_loaded`
    checks a literal `usd_rate` identity; the two could disagree only in a
    `prices.assets` inconsistency window. Not changed: both refusals are
    conservative, and unifying them means threading `ReferenceIds` into the
    rate check for no measured benefit.
  - IN-01 (`refs.usdc.unwrap_or(0)` sentinel in the new error variant) and
    IN-02 (two snapshot round-trips per pass) recorded, not changed.

## Operator Checklist (the campaign CHORE)

Nothing below can be done from the branch. Work **Appendix C** of
`docs/runbooks/repair-coarse-usd-values.md` and use this as the index.

1. **Deploy the branch first.** The campaign's refill path is the SCALED pivot;
   running it against the old statement would reset rows and rewrite them with
   the same unscaled value, spending the FREEZE point for nothing.
2. **Confirm the session and server are UTC** (precondition 0).
3. **Confirm [[0267]]'s `external` rows are loaded and PROMOTED** (precondition
   1). A count of 0 is a hard refusal, and since the prove-run commit the tool
   really does refuse — before enumerating months, in the dry run too. The
   hourly file is NOT required here — see Design Decision 11 for why, and load
   it anyway if you can.
4. **Confirm the cleanup worker is still dark** (precondition 2) and that the
   **FREEZE snapshots exist and were verified** (precondition 3). This
   population is ~19× Appendix B's; budget the disk first.
5. **Confirm the leg's identity resolves 1:1** (precondition 4), for XLM and
   again for USDT.
6. **🔑 Confirm `oracle_prices` holds no reading for the leg below the bound**
   (precondition 5, BLOCKING). This is Issues 2 and it is the likeliest stop.
   If non-zero, pass an explicit `--reset-not-after <first reading>`.
7. **Settle `_15m`** with the count in Appendix C (Issues 5) before including it.
8. **Measure `--reset-not-before` per leg** — the first candle of that leg's own
   USDC market, on the table being repaired. Not a round date: this is the
   157-candle lesson of [[0182]].
9. **Record the baseline per table**: the reset-candidate count, the median
   implied reference rate on 2023-03-11, and — for `_1w` and `_1M` —
   `max(version)` of the bucket containing the depeg day. The after-check takes
   the last two as `POST_RUN_0228_VERSION_BEFORE_1W` / `_1M` and refuses to pass
   without them.
10. **Dry run each table.** ⚠️ Zero candidate months is a STOP, not an all-clear.
11. **Real run, one table and one leg at a time**, with `--pivot-window-s`
    widened for `_1w`/`_1M`. Abort if `rows_reset` far exceeds `rows_enriched`;
    roll that table back from its snapshot before touching the next.
12. **After-check**: the median implied rate per grain, plus
    `cargo test -p enrichment-worker --test post_run_0228_it -- --ignored` with
    the two recorded versions exported.
13. **Walk `/ohlcv`** for an XLM-quoted asset and confirm `method` still reads
    `traded`. A changed label is a finding (D-05).

## Spawn list for Adam

These are a list rather than task files on purpose: a lore task is created
atomically on `develop` (pull → create → push) and this session works on a
feature branch with no push, so creating them here would put them somewhere
nobody can see.

1. **CHORE — run the 0228 pivot-leg re-enrichment campaign on production.**
   [[0276]]'s shape. Twelve passes (six tables × two legs), ≈4 h 40 m of pure
   work for the XLM leg by the runbook's estimate, gated on Appendix C's six
   preconditions. Closes rescoped criteria 2 and 3 on prod. Blocked on this
   branch being merged and deployed.
2. **Decide what owns the `redstone` rows** — 419,519 rows at `asset_id = 0`,
   `price_usd = 0` in `oracle_prices` (Issues 4). They match no asset, price
   nothing, and no task claims them.
3. **Settle `_15m`'s retention** (Issues 5): whether the 30-day policy has ever
   run, and therefore whether 8.9 M pre-epoch rows are in scope for this campaign
   and for any future one.
4. **Reorder `coarse-repair`'s FREEZE after the pass's refusals** (Issues 10):
   today a partition is frozen before a reset refusal can fire, and the leftover
   snapshot blocks the next real run on that partition with `FreezeDenied`.
   0114's driver, small, low priority; prod never hits it.

## Notes

- Found by a query aimed at `usd_rate` coverage for [[0170]]. The hypothesis
  under test was that a *window* of readings had gone unsnapshotted; that was
  **falsified** — USDC's coverage is complete apart from [[0227]]'s rows. The
  real gap was a whole asset, not a date range.
- ⚠️ Nothing here is a claim that current USD prices are wrong. It is a claim
  that we prefer an inference over an available measurement, and that nobody has
  checked what that costs.
