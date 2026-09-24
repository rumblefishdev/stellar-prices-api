---
id: "0147"
title: "Replace price_usd_series*'s close_usd > 0 filter with a volume-coverage gate"
type: FEATURE
status: completed
related_adr: ["0292", "0287"]
related_tasks: ["0144", "0118", "0131", "0116", "0146", "0150", "0061", "0151", "0286"]
tags:
  ["priority-high", "effort-medium", "clickhouse", "data-correctness", "be-interop", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: "2026-08-05"
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
  - date: "2026-09-22"
    status: active
    who: akot
    note: >
      **Phase-1 code review applied** (2 further commits, still local, still
      not deployed; 8 on the branch). One latent correctness hole and four
      doc/test gaps. FIXED: the share's denominator now weights on
      `is_priced OR is_eligible`, so `priced ⊆ eligible` holds BY CONSTRUCTION
      at both grains and on both surfaces — `is_priced` admits a row on
      `close_usd != close` for ANY quote while `is_eligible` names a quote set,
      and without the `OR` a priced row outside that set went into the
      numerator only (measured on 26.3.10.60: `priced_volume_share = 5000000`
      in a `Decimal(10, 6)` column, and a fully-priced $5,000 bucket reading
      `unpriceable`). Unreachable through today's write path; reachable the
      moment a symbol is tracked without being made eligible (0173's shape).
      New IT `a_priced_row_outside_the_eligible_quote_set_stays_inside_the_share`
      (RED before the fix), plus two text tests the review found missing:
      arm A is now pinned IDENTICAL between each series view and its coverage
      view (it is spelled four times, nothing compared the copies), and the
      hand-synced USDT/USDC issuer literals are pinned against the crate consts
      by value AND by count (0172/0173's class). DOC corrections, no behaviour
      change: `unpriceable` redefined as "no eligible PRICE-FORMING volume"
      (the dust-only bucket reads it too, and no `usd_rate` row will ever move
      that one); the peg-disjunct sentence corrected to the shipped `pw = 0`
      (a peg member with entirely-unpriced base volume still falls back to the
      peg and reads `priced`/share 1); the predicate is `/ohlcv`'s `valid` PLUS
      `pf_volume > 0`, never "identical to" it; guardrails row 80 now quotes
      the SHIPPED CAST text. Counts after the fixes: **68** `prices-clickhouse`
      lib, **23** `views_it` `#[ignore]`, 224 `prices-api` lib, 43 `ohlcv_it`
      `#[ignore]`. Still open and unchanged: both gate constants are phase-2
      placeholders, nothing is deployed, and `current.sql`'s `src_is_live` /
      `src_volume` dust counting is **unowned — needs a task** (no follow-up
      task exists; guardrails row 84 says so rather than pointing at one).
  - date: "2026-09-23"
    status: active
    who: akot
    note: >
      **No longer stacked; draft PR #346.** #337 (0216) merged, so the
      branch was rebased onto develop `e117e57` with no conflicts (none of
      the 18 intervening commits touch this task's files) and is now eight
      linear commits on develop alone. Re-verified on ClickHouse 26.3.10.60:
      **69** `prices-clickhouse` lib, **23** `views_it`, **10**
      `current_mv_it`, **225** `prices-api` lib, **43** `ohlcv_it`, **12**
      `openapi`, fmt clean. Pushed and opened as a DRAFT PR, which stays
      draft until phase 2 replaces both placeholder constants. Views are
      applied only by hand (`chwf views.sql`); no deploy, init step or 0286
      phase-3 script applies them, so a merge alone would publish nothing.
      Phase 2 (≥ 2026-09-29) measures on the same post-0286 week as 0286's
      AC 10. 0286's phase-3 re-ingest (started 2026-09-23 at 201511) rewrites
      history only, month by month, and does not touch that window.
  - date: "2026-09-23"
    status: active
    who: akot
    note: >
      **Phase 2 measured, both constants final.** Brought forward from
      2026-09-29 at Adam's call. On prod (dev_read, this branch's coverage
      body inlined) the share is binary in all four windows — 0 of 266,011
      eligible buckets between 0 and 1 — so X = 0.5 stays, now as a measured
      value, and the absolute FLOOR_USD is REMOVED from all four executable
      sites: a $1 floor kept ~16 % of today's buckets, $100 ~3 %, and price
      error does not fall with volume (median ~0.6 % in every bin). Test (b)
      flips to `a_fully_priced_small_bucket_publishes_at_full_share`; the
      placeholder-marker test is replaced by
      `views_sql_has_no_placeholder_constant_and_no_absolute_usd_floor`; both
      proven RED against a re-added floor. 70 prices-clickhouse lib, 23
      views_it, 10 current_mv_it, 225 prices-api lib, 43 ohlcv_it, 12 openapi.
      A 1d-grain and weekend re-check stays due 2026-09-29, not gating.
  - date: "2026-09-24"
    status: completed
    who: akot
    note: >
      **Merged and rolled out.** PR #346 merged as c7a733d5 after Oskar's
      review (3 low stale-comment findings, fixed in e684a11). Rolled out on
      prod 2026-09-24 11:16 UTC: the six 0147 views applied one statement per
      request as dev_shared, `current_price_usd` left to 0216. Published rows
      dropped 1.00 % (1d) and 0.35 % (1h), inside the predicted 0.3–1.6 %;
      reference counts unchanged, sampled prices identical. Details in
      "Rollout". The BE contract is sent by Adam; the 2026-09-29 re-check
      moves to [[0313]].
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
now-complete population. The dust print is a real trade and keeps its **0.0763 %**
of the weight (0.764 / 1000.764 = 0.07634 %), worth +0.001 on the published
price. The residual is the point — before the gate that same print WAS the
price: 1.3085 is **201x** the 0.0065 of the leg that carried the volume, and
**175x** the bucket's own enriched weighted mean of 0.00749394.

### What shipped

1. **One priced predicate, spelled once.** Arm A and `usd_reference*` use
   `/ohlcv`'s `valid` for the same row **plus a positive price-forming weight**:
   `close >= 1e-12 AND close_usd >= 1e-12 AND pf_trade_count > 0 AND
   pf_volume > 0`, plus convertibility (quote is the canonical USDC, or
   `close_usd != close`). `pf_volume > 0` is the term `/ohlcv`'s `valid` does
   NOT have (decided by brief D-01): these surfaces WEIGHT by that column and
   `/ohlcv` does not, so a zero-weight row cannot move their mean but can empty
   their denominator. Same rule, not the same expression — do not call the two
   identical. `usd_reference*` take the same floor and pf terms minus the USD
   leg, which they do not read.
2. **`pf_volume` weights, and the population is explicit.** Arm A no longer
   filters: it reads every candle of the bucket and sums conditionally, so the
   unpriced rows are in the denominator. `priced_volume_share = Σ pf_volume
   (priced) / Σ pf_volume(eligible OR priced)`, appended LAST after `method`,
   never NULL. The denominator's `OR priced` is what makes `priced ⊆ eligible`
   true BY CONSTRUCTION: `is_priced` admits a row on `close_usd != close` for
   any quote while `is_eligible` names a quote set, so without it a priced row
   outside that set lands in the numerator only and the ratio leaves [0, 1]
   (measured: 5000000 in a `Decimal(10, 6)` column). Found in review, fixed
   with a test.
3. **Publish iff `share >= X` AND `priced_volume_usd >= FLOOR_USD`**, else the
   bucket is ABSENT with a row in `price_usd_series_coverage{,_1h}` saying
   `priced | pending | unpriceable`. Eligibility is computed in the view (no new
   table) and is retroactive per ADR 0292. The peg arm is outside the gate and
   reads `priced` with share 1.
   ⚠️ **`unpriceable` means "no eligible PRICE-FORMING volume in the bucket"**,
   which is two different buckets under one word: no USD path for the quote
   (retroactive — a `usd_rate` row flips it to `pending`), OR an eligible quote
   whose every candle is stroop-dust, so `pf_volume` sums to 0 and no rate row
   will ever move it. Post-0286 the second is ordinary, not exotic — it is the
   class 0286 created. Read `pf_volume` to tell them apart; the status word
   does not.
   ⚠️ **The peg disjunct tests `pw = 0` — no PRICED weight — not `ew = 0`.** So
   a peg member that also trades as a base falls under the gate only when some
   of that base volume is priced; with its base volume ENTIRELY unpriced it
   still takes the peg fallback and reads `priced` / share 1 on the coverage
   view (measured: 100,000 eligible unpriced units). Not a regression — the
   pre-0147 bucket had `sum(w) = 0` and took the same fallback — but the share
   on such a row describes the peg statement, not the traded population.
   Making those absent means `peg_present = 1 AND ew = 0`: a behaviour change,
   not phase 1's to take.

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

## Phase 2 — the measurement (2026-09-23)

Run read-only as `dev_read` on production, with this branch's own coverage view
body inlined and the candle tier pre-filtered to each window.

| Window | Grain | Eligible buckets | 0 < share < 1 |
| --- | --- | --- | --- |
| 09-23 06:00 → 15:00, incl. the hour in progress | 1h | 13,363 | 0 |
| 09-22 12:00 → 09-23 12:00 (post-0286) | 1h | 24,311 | 0 |
| 09-15 → 09-22 | 1h | 122,166 | 0 |
| 08-23 → 09-22 | 1d | 106,671 | 0 |

- **X = 0.5.** Enrichment prices a bucket whole or not at all, so no X in
  (0, 1] withholds anything today. X guards the yXLM shape if it returns
  (a half-enriched bucket, e.g. USDT-quoted candles waiting on the sweep, [[0209]]).
- **No `FLOOR_USD`.** Median `priced_volume_usd` per bucket is $0.01–0.02;
  a $1 floor keeps 15.7–17.6 % of today's published buckets, $100 keeps
  3.2–3.7 %. Against the median of the same asset's other hourly buckets
  within ±12 h (274,754 buckets, 09-01 → 09-22) the median price error is
  ~0.6 % in every volume bin, and `usd < 100` catches 93 % of > 2x outliers
  by withholding 96 % of buckets — no information. Industry practice agrees:
  CF Benchmarks publishes on one trade; dollar thresholds elsewhere pick
  sources, not whether to publish.
- The share-0 buckets that publish today (63 / 475 / 1,828) are all
  sub-1e-12 prices `/ohlcv` already refuses; with X = 0.5 the views publish
  98.4–99.7 % of today's set.
- `close_usd > 0 AND volume_quote_usd = 0`: 0 in every window.
- Off-market prints (> 2x from the asset's ±12 h median) are 0.3–0.5 % of
  hourly buckets, none on the 7 assets with ≥ $1M 3-week volume; 35 of the 55
  on assets ≥ $100k are one wash-traded-looking token (VORIXLM, 4e-5 → 60).
  Not in 0147's scope — a deviation filter is a separate backlog item.

## Rollout (2026-09-24)

Applied by Adam as `dev_shared` at 11:16 UTC from develop `c7a733d5`, with the
runbook `.planning/rollout-2026-09/ROLLOUT-0147.md` (not committed).
Before/after were compared over fixed windows ending at 2026-09-24 12:00.

| Surface | Before | After |
| --- | --- | --- |
| `price_usd_series` (2 days) | 8,784 | 8,696 (−1.00 %) |
| `price_usd_series_1h` (48 h) | 42,754 | 42,605 (−0.35 %) |
| `usd_reference` / `_1h` buckets | 2 / 48 | 2 / 48 |

- `xlm_usd` moved only in the 14th decimal place (dust weights). XLM,
  canonical USDC (`oracle`, share 1) and the thin `50X` kept `close_usd` and
  `method`.
- `priced_volume_share` is at position 8 in both views, and both coverage
  views exist. Coverage status counts: 1d = 8,696 priced / 89 pending (all
  share 0) / 1,733 unpriceable; 1h = 42,605 / 720 / 10,245.
- **New since phase 2:** at 1h there are now 53 published buckets with a share
  strictly between 0 and 1, and 15 withheld as `pending` with a share above 0.
  So X = 0.5 now has an effect; phase 2 measured 0 such buckets. Follow-up in
  [[0313]].
- `query_log` showed no exception on the six views after the apply.

### Emerged during rollout

1. **Six statements, not `chwf views.sql`.** The HTTP interface takes one
   statement per request, and the file also holds 0216's `current_price_usd`,
   which on prod still had its 2026-09-11 definition. The six 0147 views were
   applied one by one, and the rollback is the four pre-0147 bodies from
   `246ef739`.
2. **Dry run before the write.** Each view body was run as `dev_read` with
   `LIMIT 0` on prod (HTTP 200, while a bad column returns 404), so a missing
   column would have shown up before the change.

## Acceptance Criteria

- [x] Neither view can return a bucket whose published price rests on a
      negligible share of that bucket's volume — regression test on CH
      **26.3.10.60** reproducing BE's yXLM case.
      → `a_dust_print_cannot_price_a_bucket_whose_volume_is_unpriced` (RED
      captured above), `a_fully_priced_small_bucket_publishes_at_full_share`,
      `the_gate_and_the_coverage_view_behave_the_same_at_the_hourly_grain`.
- [x] A fully unpriceable bucket is absent; a *pending* bucket is
      distinguishable from a *priced* one — not conflated.
      → `price_usd_series_coverage{,_1h}`'s `status`;
      `a_bucket_quoted_only_in_an_ineligible_asset_reads_unpriceable_with_a_zero_share`.
- [x] X justified against query C's measured distribution, recorded in the
      header. → phase 2, 2026-09-23 (Adam brought it forward from 09-29):
      the share is BINARY in every measured window — 0 of 266,011 buckets
      between 0 and 1 — so X = 0.5 is kept as the volume-weighted-median rule
      and `FLOOR_USD` is REMOVED (a $1 floor keeps ~16 % of today's buckets,
      and volume does not predict a bad price). Numbers in the `views.sql`
      header and "Phase 2" below. A full-week re-check (1d grain, weekend) is
      due 2026-09-29 but no longer gates the PR.
- [x] `priced_volume_share` exposed to consumers. **Confirmed as wanted by the
      only consumer** — BE, 2026-08-06: *"please do expose the coverage share,
      we'll set our own bar on it."* Not optional; ship it with the gate.
      → `0144/notes/S-be-0199-response-received.md`
- [x] Threshold definition reconciled with [[0118]] and [[0131]].
      → 0118's verdict on an unconditional floor is confirmed per bucket and
      the floor is gone; [[0131]] has history notes pointing at this
      definition and at X = 0.5; the guardrails inventory rows 79-82 are
      closed against their tests.
- [ ] BE told the gate has shipped and what X is. **Handed to Adam at
      close (2026-09-24)**: he sends `.planning/CONTRACT-0147-be.md` with the
      rollout date 2026-09-24; the numbers are final. Closed with this open at
      his call, since nothing left in the repo gates it.
