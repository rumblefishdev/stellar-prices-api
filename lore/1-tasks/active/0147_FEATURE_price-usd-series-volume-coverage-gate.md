---
id: "0147"
title: "Replace price_usd_series*'s close_usd > 0 filter with a volume-coverage gate"
type: FEATURE
status: active
related_adr: ["0292", "0287"]
related_tasks: ["0144", "0118", "0131", "0116", "0146", "0150", "0061", "0151", "0286"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "be-interop", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-08-05
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0144]] future work (phase 5) — BE 0199 finding 3i. Both of
      BE's own proposed fixes were measured and rejected; this is the
      implementable form of their intent.
  - date: "2026-09-17"
    status: backlog
    who: akot
    note: >
      Re-read against [[0286]] (merged, ADR 0287) and ADR 0292 from [[0151]],
      which DECIDES what this task SHIPS. Not solved by 0286: it deliberately
      left `views.sql` / `current.sql` without a price-forming gate, and BE's
      yXLM print (0.764 units = ~7.6M stroops) clears the 0.1 % bound. [[0146]]
      is closed as superseded — the rollup half is 0286's rate-form `close_usd`.
      From ADR 0292: a candle is PRICED iff `close >= 1e-12 AND
      close_usd >= 1e-12`; `priced_volume_share = Σ pf_volume(priced) /
      Σ pf_volume(eligible)`, eligible = quote asset has a conversion path, so
      unpriceable legs AND dust-only candles are outside the denominator; the
      gate is the default, the share is always on the wire, an absolute floor
      sits beside the ratio; X is measured on our own history after the 0286
      rollout (research found no outside number to copy — nobody else has an
      async conversion step; a median would not have fixed the yXLM case). Also
      handed over: the same floor and a pf gate in `views.sql` arm A and
      `usd_reference*`, WITH the cross-surface test under the floor (a test of
      agreement cannot land before its fix); `current.sql`'s `src_is_live` /
      `src_volume` counting dust-only minutes; the untested 1h filter; and the
      additive wire fields `price_status` (priced | carried | unpriced) and
      `as_of` on the current-price surfaces. Gap list:
      `docs/database-schema/close-usd-zero-guardrails.md`.
  - date: "2026-09-18"
    status: backlog
    who: akot
    note: >
      **ADR 0292's read-time rules were prototyped in SQL on ClickHouse
      26.3.10.60, on the real column types (`CREATE TABLE … AS
      prices.price_ohlcv_1m`), and they WORK** — throwaway scratch database,
      nothing kept. §3: four rows all at `close_usd = 0` classify as `priced` /
      `pending` / `unpriceable` / `no_price` from columns they already have.
      §4: BE's 7.7x hour reproduced — one priced 0.764-unit print at 0.05
      beside 1000 units still pending at a true 0.0065: today's
      `WHERE close_usd > 0` publishes 0.05; the share reads 0.076 % and the gate
      withholds it; once enrichment catches up it publishes 0.006573 at 100 %.
      A bucket holding ONLY the 0.764 print reads 100 % and is still withheld —
      that is what the absolute floor is for. §5: `price_status` and an age in
      minutes fall out of `argMaxIf` / `maxIf` on the same priced predicate.
      **Three traps the prototype hit, for whoever builds this:**
      (1) the naive weighted mean THROWS — `Decimal * Decimal / Decimal` on
      these columns raises `DECIMAL_OVERFLOW` (code 407); cast to `Float64` per
      row BEFORE multiplying, as `views.sql` already does for `v` and `w`.
      (2) `as_of` can itself be a zero-sentinel: `maxIf(timestamp, <priced>)`
      over no matching row returns `1970-01-01`, not NULL — published as-is
      that is a 56-year-old price, the very class ADR 0292 is about; use
      `maxIfOrNull` or an explicit guard, and pin it with a test.
      (3) "the quote asset has a conversion path" does not exist as data — the
      prototype used a one-row table; this task must decide its source of truth,
      and `unpriceable` changes retroactively when a reference market appears.
      The thresholds the prototype used (50 %, 10 units) were invented for the
      demonstration and are NOT a proposal: X is still measured after the 0286
      rollout.
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      Activated. Phase 1 runs from `.planning/BRIEF-0147.md` (binding, written
      2026-09-22): one priced predicate shared with `/ohlcv`, `pf_volume`
      weights, gate = `priced_volume_share >= X` (placeholder 0.5) AND
      `priced_volume_usd >= 100` (0118's `min_volume_usd` default),
      `priced_volume_share` appended last, `price_usd_series_coverage*` views
      with `priced | pending | unpriceable`, yXLM RED→GREEN IT on 26.3.10.60.
      Branch `feat/0147` is STACKED on `feat/0216` (PR #337 still open, both
      touch `views.sql`); the PR waits for #337. Phase 2 (measure X on prod)
      after the 0286 week, ≥ 2026-09-29. No deploy in phase 1.
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      **Phase 1 landed on `feat/0147_price-usd-series-volume-coverage-gate`** —
      6 commits, local only, NOT pushed and NOT deployed anywhere. Shipped: one
      priced predicate shared with `/ohlcv`, `pf_volume` weights at every
      weighted surface, `priced_volume_share` appended LAST (7 → 8 columns on
      both series grains), the publish gate, `price_usd_series_coverage{,_1h}`
      (6 → 8 views), and the same floor + pf terms on `usd_reference*`. The
      yXLM case is RED→GREEN on ClickHouse 26.3.10.60 with the RED output
      captured verbatim (see Implementation Notes). Test counts: 66 lib, 22
      `views_it` `#[ignore]` (16 pre-existing + 6 new; every pre-existing
      assertion byte-identical — only fixture `volume_quote_usd` was raised),
      224 `prices-api` lib, 43 `ohlcv_it` `#[ignore]`, 12 `openapi`, and 1,127
      passing across `cargo test --workspace`. **Both `X = 0.5` and `FLOOR_USD = 100` ship as
      PLACEHOLDERS** carrying a phase-2 marker pinned by a test; the task stays
      `active` until phase 2 measures them on prod (≥ 2026-09-29) and the
      rollout runs. The task's PR still waits for #337 (0216).
---

# Volume-coverage gate for `price_usd_series` / `price_usd_series_1h`

## Summary

Both views filter

```sql
WHERE p.close_usd > 0
```

before volume-weighting. The filter was written to stop un-enriched rows
dragging the weighted mean toward zero, and it does — but it makes the
denominator `sum(volume_base)` run **over the enriched subset only**, so the
weighting population depends on how far the hourly enrichment pass happened to
have got. Whichever rows are enriched become 100% of the weight.

BE measured the pathological case on yXLM (2026-08-04 13:00): a **0.764-unit
dust print at 1.3085 USD** was the hour's only enriched row, so the view
returned 1.3085 against ~0.170 in every neighbouring hour — **7.7×**.
Reproduced on CH 26.3.10.60 ([[0144]] `repro/`, TEST B).

The weighting arithmetic is sound — BE said so and they are right. The same
dust print sits in the fully-enriched 12:00 bucket beside 42,038 units of real
volume and moves the result by nothing. It is the population that is wrong.

## Both of BE's proposed fixes fail — measured

- **"Exclude the bucket until every row is enriched" cannot terminate.**
  Enrichment documents a **permanent** exotic-quote floor: candles whose quote
  is not USDC/USDT/XLM and which have no oracle keep `close_usd = 0` forever,
  by design (`ch_enrich.rs:31-32`). Any bucket containing one such row would be
  suppressed in perpetuity. This is the kind of gate that passes every test and
  then strands real assets on prod.
- **"Remove the filter and weight over everything" is worse than the status
  quo.** Measured: **0.000023** against a true ~0.170, because an unpriced row
  enters as a zero numerator against a full-weight denominator.

## The implementable version of their intent

Publish the bucket only when the enriched rows account for **≥ X% of the
bucket's `volume_base`**.

- Prices a bucket as soon as its *real* volume is priced.
- Ignores a permanently-unpriceable dust tail, so it terminates.
- Being a **weight-share** test rather than a row-count test, it is immune to
  the dust-print case **by construction** — 0.764 units against 42,038 can
  never clear the bar alone.

Additionally expose **`priced_volume_share`** so a consumer can set its own bar
rather than inheriting ours. The `views.sql` header already promises
value-or-absent semantics; "partially enriched" is a third state that today
masquerades as a good value.

## Implementation

- Pick X from **[[0144]] query C's real distribution on prod**, not from taste.
  Check what `priced_volume_share` looks like across a normal day first.
- Apply to both `price_usd_series` and `price_usd_series_1h`. Both are
  `CREATE OR REPLACE VIEW` ([[0134]]), so delivery is safe.
- **Ship one definition of "priced enough", not three.** [[0118]]
  (`min_volume_usd` inclusion threshold) and [[0131]] (pre-roll USD coverage
  gate) are proposing the same predicate in two other places. Unify the
  threshold and its naming across all three, or reconcile explicitly why they
  differ.
- [[0116]] is what makes the dust rows junk in the first place; this gate stops
  a junk row *being* the answer. Complementary, not alternative.

## Still needed after [[0286]]

[[0286]] (which superseded [[0146]]) fixes zeros manufactured by the rollup
chain. This gate covers the case
where the **base table's own rows** are unpriced, which no rollup fix can reach.

## Implementation Notes (phase 1)

Branch `feat/0147_price-usd-series-volume-coverage-gate`, 6 commits, local only.
Nothing deployed; both gate constants are placeholders. See
`.planning/quick/260922-kdo-0147-phase-1-volume-coverage-gate-priced/` for the
brief, plan and summary, and `.planning/CONTRACT-0147-be.md` for the note to BE.

### The RED proof, verbatim

Test `a_dust_print_cannot_price_a_bucket_whose_volume_is_unpriced` was written
FIRST and run against the UNCHANGED `views.sql` on the local ClickHouse
26.3.10.60. Fixture: one 0.764-unit print priced at 1.3085 beside 1000 unpriced
units of the same identity in the same daily bucket.

```
thread 'a_dust_print_cannot_price_a_bucket_whose_volume_is_unpriced' panicked at
packages/prices-clickhouse/tests/views_it.rs:2318:9:
price_usd_series SETTINGS compile_expressions = 0: the only priced row in this
bucket is a 0.764-unit print holding 0.0763 % of its eligible volume, so the
bucket must be WITHHELD — got [("FOO", 1.3085)] (1.3085 is the dust print's own
price, BE's 7.7x yXLM defect)

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 16 filtered out
```

GREEN, same fixture, after the gate: `price_usd_series` has **no row** for that
identity/bucket, and `price_usd_series_coverage` reads
`priced_volume_share = 0.000763`, `priced_volume_usd = 1`, `status = 'pending'`
— in **both** JIT modes. Enrich the 1000 units at their true 0.0065 and the
bucket publishes at **0.00749394** with share `1` and `status = 'priced'`.

⚠️ 0.00749394, not 0.0065: that is the bucket's volume-weighted mean over its
now-complete population. The dust print is a real trade and keeps its 0.0764 %
of the weight, worth +0.001 on the published price. The residual is the point —
before the gate the same print WAS the price, at 200x the truth.

### What shipped

1. **One priced predicate, spelled once.** Arm A and `usd_reference*` use
   `/ohlcv`'s `valid` for the same row: `close >= 1e-12 AND close_usd >= 1e-12
   AND pf_trade_count > 0 AND pf_volume > 0`, plus convertibility (quote is the
   canonical USDC, or `close_usd != close`). `usd_reference*` take the same
   floor and pf terms minus the USD leg, which they do not read.
2. **`pf_volume` weights, and the population is explicit.** Arm A no longer
   filters: it reads every candle of the bucket and sums conditionally, so the
   unpriced rows are in the denominator. `priced_volume_share = Σ pf_volume
   (priced) / Σ pf_volume(eligible)`, appended LAST after `method`, never NULL.
3. **Publish iff `share >= X` AND `priced_volume_usd >= FLOOR_USD`**, else the
   bucket is ABSENT with a row in `price_usd_series_coverage{,_1h}` saying
   `priced | pending | unpriceable`. Eligibility is computed in the view (no new
   table) and is retroactive per ADR 0292. The peg arm is outside the gate and
   reads `priced` with share 1.

### What phase 2 owes

**Both `X = 0.5` and `FLOOR_USD = 100` are PLACEHOLDERS**, each carrying
`-- ⚠️ PLACEHOLDER, measured in phase 2` beside it in the `views.sql` header and
pinned by `views_sql_marks_both_gate_constants_as_phase_2_placeholders`.

`FLOOR_USD` is a placeholder too, and that is an amendment made during planning
on evidence from our own code: `current.sql:130-150` records that 0118's
identical 100 USD, applied UNCONDITIONALLY over a 24 h window, would have
blanked **2,960 of 3,068 priced assets (96.5 %)** on prod 2026-08-27 — which is
why 0118 made its threshold conditional on a clearing sibling. 0147's floor is
per BUCKET (per hour at `_1h`, ~$2.4k/day) and has no sibling notion, so it is
strictly harsher. 100 is cited for PROVENANCE, not as a measurement.

Phase 2, after the 0286 measurement week (≥ 2026-09-29): run the
`priced_volume_share` and `priced_volume_usd` histograms on prod over a 7-day
window, **per grain**, read-only via `dev_read` and with **no script committed
to develop**; pick X where the bimodal mass between the peaks is smallest;
record how many buckets X withholds against how many the floor does; replace
both constants, drop both markers, and record the distribution in the view
header and here. Rollout (`.planning/rollout-2026-09/ROLLOUT-0147.md`) waits for
that.

## Acceptance Criteria

- [x] Neither view can return a bucket whose published price rests on a
      negligible share of that bucket's volume — regression test on CH
      **26.3.10.60** reproducing BE's yXLM case.
      → `a_dust_print_cannot_price_a_bucket_whose_volume_is_unpriced` (RED
      captured above), `only_the_dust_print_is_priced_and_the_absolute_floor_withholds_the_bucket`,
      `the_gate_and_the_coverage_view_behave_the_same_at_the_hourly_grain`.
- [x] A fully unpriceable bucket is absent; a *pending* bucket is
      distinguishable from a *priced* one — not conflated.
      → `price_usd_series_coverage{,_1h}`'s `status`;
      `a_bucket_quoted_only_in_an_ineligible_asset_reads_unpriceable_with_a_zero_share`.
- [ ] X justified against query C's measured distribution, recorded in the
      header. **OPEN — owned by phase 2** (≥ 2026-09-29, after the 0286
      measurement week). X ships as a marked placeholder until then, and
      nothing is deployed on it.
- [x] `priced_volume_share` exposed to consumers. **Confirmed as wanted by the
      only consumer** — BE, 2026-08-06: *"please do expose the coverage share,
      we'll set our own bar on it."* Not optional; ship it with the gate.
      → `0144/notes/S-be-0199-response-received.md`
- [x] Threshold definition reconciled with [[0118]] and [[0131]].
      → `FLOOR_USD` cites 0118 by name AND carries 0118's own 96.5 %
      measurement in the `views.sql` header, which is why it is a placeholder
      too; [[0131]] has a history note pointing at this definition; the
      guardrails inventory rows 79-82 are closed against their tests.
- [ ] BE told the gate has shipped and what X is. **OPEN — owned by Adam**,
      who sends `.planning/CONTRACT-0147-be.md` (written, deliberately not
      committed) after phase 2 fixes the numbers. Sending it now would give BE
      two values that are about to move.
