---
id: "0286"
title: "Candles take every fill at equal weight, including stroop-dust, in the wrong intra-ledger order — rebuild OHLC from price-forming fills with a windowed close, then re-ingest the history"
type: BUG
status: active
related_adr: ["0287"]
related_tasks: ["0278", "0276", "0266", "0228", "0146", "0142", "0137", "0200", "0088", "0282"]
tags: [layer-backend, priority-high, effort-large, ohlcv, ingest, enrichment, clickhouse, data-correctness, api-contract]
links:
  - "../../2-adrs/0287_candle-prices-come-from-price-forming-fills-and-a-windowed-close.md"
  - "../archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/README.md"
  - "../archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/notes/S-close-estimator-and-pivot-reference.md"
  - "../archive/0278_RESEARCH_decide-whether-to-act-on-dust-prints-in-ohlcv/notes/R-measurements-2026-09-15.md"
  - "../../../docs/ohlcv-outlier-prints-analysis.md"
  - "../../../packages/prices-ingest-core/src/filter.rs"
  - "../../../packages/prices-ingest-core/src/tick.rs"
  - "../../../packages/prices-ingest-core/src/bucket.rs"
  - "../../../packages/prices-ingest-core/src/soroban.rs"
  - "../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../packages/prices-clickhouse/schema/rollups.sql"
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: "2026-09-15"
    status: backlog
    who: akot
    note: >
      Spawned from [[0278]] as ONE task for the whole candle fix, by Adam's
      decision (not one per phase). Carries decisions D1–D8 and D10 of 0278
      with their measured basis; D9 (cross-source median on the read path) is
      deliberately out of scope. Phases below are sequencing inside this
      task, not separate deliverables.
  - date: "2026-09-15"
    status: active
    who: akot
    note: >
      Activated by Adam; branch fix/0286_candles-are-built-from-dust-fills-in-the-wrong-order
      cut from develop. Phase 1 first
      (price-forming fills, pf_* columns, windowed close in the rollups, ADR
      0287 already accepted).
  - date: "2026-09-15"
    status: active
    who: akot
    note: >
      Specification review against the code resolved, on the branch, before
      phase 1 starts: window anchored at the last price-forming minute (60-min
      cap from there); open = close on 1m, disclosed; rollups maxIf/minIf, no
      carried high/low, stateless ingest with read-side carry-forward; coarse
      close_usd = close × latest priced child's rate (replaces 0146's product
      carry — one MV re-CREATE for both); missing 1m tail falls back to the
      child's close with close_window_fills = 0; close_median stored; pool
      price post-fill; pf_trade_count everywhere; 0142/0137 already live;
      0228's reset campaign unnecessary after the re-ingest, re-enrichment
      added to phase 3's cost; k = 1 vs 3 on XLM/USDC to measure before
      hard-coding; soroban.rs, writers and docs added to scope.
  - date: "2026-09-15"
    status: active
    who: akot
    note: >
      Three more resolutions, decided by Adam (ADR 0287 second amendment):
      coarse open/close come from a settle pass into price_ohlcv_settle,
      joined by the MVs, instead of MVs reading 1m tails (option b); the
      enrichment candidate predicate excludes close = 0 rows, in scope here;
      pool fills keep their execution ratio with the 0.1 % bound, the
      reserve-based price is dropped. Phase 2 shrinks to offer prices and
      the transaction index.
  - date: "2026-09-16"
    status: active
    who: akot
    note: >
      Redefined by Adam (ADR 0287 third amendment): a candle is built only
      from the price-forming trades of its own bucket. open/close are the
      first and last price-forming fills in fill order, never a window mean
      and never carried from an earlier bucket; a bucket without one has no
      price on the wire. The settle pass, price_ohlcv_settle, close_median
      and the window columns are dropped; the rollups take open/close from
      the child tier; pf_vwap becomes the quality field; D1 (transaction
      index) moves into phase 1 because "last fill" depends on it. A first
      implementation on the branch (four GSD slices, eight commits) was
      reverted the same day at Adam's request — it had built the windowed
      close and the settle machinery; its review findings that still apply
      (name-routed writers, the positional version trap in the fill key, the
      DEFAULT-expression migration, the i128 bound form, the backfill marker
      step in phase 3) are folded in below. Phases and criteria rewritten.
---

# Candles are built from dust fills in the wrong order

## Summary

A candle's `open`/`high`/`low`/`close` are single fills at equal weight, and a
fill's price is the ratio of two integer stroop amounts. A fill of a few
stroops therefore prints an exact small fraction (1/17, 5/34) up to hundreds of
percent off the market; when it lands last in the day it is the close, and
through the pivot tier the USD reference for every XLM-quoted asset (7 328
daily candles ~26 % low on 2023-03-11). The order key
`(ledger, operation_index, claim_index)` has no transaction index, so the "last"
fill is also the wrong one on 54/60 sampled days. Full analysis:
`docs/ohlcv-outlier-prints-analysis.md`; decisions and measurements: [[0278]].

This task implements ADR 0287 (as amended 2026-09-16) as one change in three
phases, and re-ingests the history in place so old and new candles mean the
same thing. The rule it implements: **a candle's prices come only from the
price-forming trades of its own bucket** — `open`/`close` are the first and
last such fills in fill order, `high`/`low` their extremes; a bucket with none
has no price.

## What changes (ADR 0287 §1–§8, in implementation order)

### Phase 1 — price-forming fills, fill order, one definition per tier

- **Price-forming fills in the ingest** (`bucket.rs`, `tick.rs`, `soroban.rs`
  — AMM ticks enter through `amm_trade_to_tick`, and the bound is evaluated
  on the raw `i128` amounts in each token's own decimals, not after
  `AMM_AMOUNT_SCALE`): a fill is *price-forming* iff its price does not come
  from its own rounded amounts, or `1/a + 1/b ≤ 0.001` holds. Evaluate it in
  integer arithmetic as `a > 1000 ∧ b > 1000 ∧ (a − 1000)(b − 1000) ≥ 10⁶`,
  where a product overflowing u128 means the bound holds; the literal form
  `1000(a + b) ≤ ab` with "overflow ⇒ holds" misclassifies 10³⁸ against 5.
  Until phase 2 every fill is ratio-priced, so the bound applies to all.
  Every fill still counts in `volume_*`, `vwap` and `trade_count`.
- **Fill order (D1) in the same phase**: `lex_key =
  (ledger, transaction_index, operation_index, claim_index)`;
  `transaction_index` is the position in `tx_processing` (classic,
  `filter.rs`), the transaction's apply index in `process_ledger` (Soroban
  live, `soroban.rs`), and BE's `default.transactions.application_order`
  joined on `id = transaction_id` in `events-backfill` (LEFT join, FINAL,
  bounded to the chunk; unreadable or missing → WARN once and fall back to
  today's `(transaction_id, event_index)` order). `event_index` is per
  transaction (xdr-parser `types.rs`), so it stays the tiebreak inside one.
  ⚠️ `version = ledger·1000 + operation_index` is unchanged (coarse
  `sum(version)` must stay comparable with existing rows) — give the fill's
  ledger and operation **named fields** before the key widens, or the
  transaction index silently becomes the version's second term.
- **The 1m candle**: `open` = first price-forming fill, `close` = last, in
  `lex_key` order; `high`/`low` = their extremes; `pf_trade_count`,
  `pf_volume` (Σ `volume_base` of price-forming fills), `pf_price_volume`
  (Σ price × `volume_base`). No price-forming fill → price fields 0,
  `pf_*` 0, the row still written with its volume and count. The ingest
  keeps no price state across minutes or chunks. Sort the minute's fills by
  `lex_key` before every sum so nothing depends on arrival order; use
  checked/saturating Decimal arithmetic (rust_decimal panics on overflow),
  clamped to the column's domain.
- **Schema** (`init.sql`): `pf_trade_count`, `pf_volume`, `pf_price_volume`
  on **all seven** tables, appended after `version` via idempotent
  `ALTER TABLE … ADD COLUMN IF NOT EXISTS` (the `AS`-copies do not inherit a
  post-hoc ALTER — same pattern as `close_usd`), with DEFAULT expressions
  `trade_count` / `volume_base` / `volume_quote` so pre-existing rows keep
  the old "every fill forms price" meaning until phase 3. ⚠️ The Rust writer
  is **name-routed** (clickhouse 0.13 inserts by field name), not positional
  as `init.sql` claims: a struct that omits a column silently takes its
  DEFAULT, and `pf_trade_count DEFAULT trade_count` then declares a dust-only
  row price-forming. Every writer changes in the same commit and a test pins
  the writer's field names to the canonical column list. The pre-roll
  scripts insert positionally and fail loudly instead (fix the two
  maintained ones; mark `preroll-incremental.sql` / `preroll-amm-reprice.sql`
  historical).
- **Rollups** (`rollups.sql`, one generator shared with `preroll*.sql`):
  `argMinIf(open, t.timestamp, t.pf_trade_count > 0)`,
  `argMaxIf(close, …)`, `maxIf(high, …)`, `minIf(low, …)`; `pf_*` summed;
  `vwap` and `close_usd` computed in Float64 and converted without throwing
  (Decimal division silently overflows past ~1.7e10 on 26.3.10.60);
  **coarse `close_usd`** = `close × argMaxIf(close_usd / close, timestamp,
  close_usd > 0)` — the latest priced child's *rate* (ADR §5), which
  supersedes [[0146]]'s `argMaxIf(close_usd, …)`: re-create the six MVs
  once, with both, under 0146's checklist ([[0142]] and [[0137]] are live).
  Keep `t.`-qualification inside every aggregate (`ILLEGAL_AGGREGATION`
  otherwise) and the drift detector's constraints on the file.
- **Enrichment** (`ch_enrich.rs`; ADR §6): `CANDIDATE_PRED` becomes
  `(volume_quote_usd = 0 OR (close_usd = 0 AND close > 0)) AND volume_quote > 0`
  — and, because the oracle/peg/external/pivot statements carry their own
  literals, each of them excludes a `close = 0` row once its volume is
  priced (`peg_sql`'s term `p.`-qualified). The pivot's XLM/USDC reference
  adds `pf_trade_count > 0`. **Every re-inserting statement carries all
  candle columns** (the pivot has four nesting levels), pinned by a unit
  test per statement. In scope here, not in the pivot's two-reference change.
- **Read surface** (`/ohlcv`): prices merge only from rows with
  `pf_trade_count > 0` (explicit NULL when no row matches — `maxIf` over
  nothing returns 0, not NULL); a bucket with Σ `pf_trade_count = 0` returns
  `open`/`high`/`low`/`close`/`vwap` absent, volume and `trade_count`
  present (ADR 0011 §5); new fields `pf_trade_count` and `pf_vwap`
  (additive, before `source`/`quality` so the positional RowBinary tail
  stays put) and a divergence flag `|close / pf_vwap − 1| > 1 %`; the USDC
  peg series emits null for them. No carry-forward anywhere.
- **Docs**: `docs/database-schema/database-schema-overview.md`, the OpenAPI
  descriptions of `open`/`close`/`high`/`low`/`vwap` and the general
  overview's dust-print prose (task 0116's "filter on the client") describe
  the ADR 0287 definitions and the new fields.
- **Deploy order** (write it into the schema comment and a runbook): schema
  → enrichment + coarse sweep + API → MV re-CREATE → ingest **last**. The
  old MVs would turn a dust-only minute into `low = 0`, and old enrichment
  would re-insert rows without the pf columns.

### Phase 2 — offer price for order-book fills

- **D2** (`filter.rs`, `price.rs`): order-book fill price = the resting
  offer's `price.n/d` from that operation's `LedgerEntryChanges` (the `State`
  pre-image of the `OfferEntry` with the claim's `offer_id`, `Updated` as
  the second choice; `offer_id` is ledger-unique). Uniform across
  `TransactionMeta` V0…V4 (`operations[i].changes`). Orientation as
  `compute_price`: non-inverted `n/d`, inverted `d/n`; guard `n ≤ 0 ∨ d ≤ 0`.
  Horizon does the same (`findTradeSellPrice`).
- **D3** in its final form: offer-priced fills always form price; pool fills
  and any order-book fill whose offer entry cannot be found take the bound.
  Pool reserves are never used (ADR amendment 2).

### Phase 3 — history, in place (D10)

- Re-ingest the whole chain (~64 M ledgers; 0088's rates give ~5.7 TB and
  ~18 days at home bandwidth) with the phase-2 ingest, **overwriting the
  existing tables**. Old rows outrank a re-ingested row on `version`
  (enrichment bumped them by +1 per pass), so per monthly partition:
  `FREEZE` 1m and the coarse partitions it overlaps → clear that month's
  `backfill_sdex_ledgers` completion markers (else the resume set skips
  every ledger and the run is a silent no-op) → `DROP PARTITION` 1m →
  re-ingest → volumes reconciled against the snapshot → `DROP` the
  overlapping coarse partitions and pre-roll them from the new 1m → next
  month; the 1w tier last, because weeks straddle months. "Reconciled"
  means: SDEX volumes equal to the stroop except the documented backfill
  partition-boundary minutes and the live era, which may come back larger
  where [[0282]] lost trades; AMM sources ≥ with the delta accounted for per
  source and month. The Soroban AMM history enters through
  `events-backfill` (pool registry seeded first — [[0088]]), with the same
  D1–D3 rules; BE's `default.transactions` coverage of the range is a
  precondition, the fallback share recorded.
- Preconditions: phases 1–2 live and measured on new data; cleanup off for
  the whole run ([[0200]]); disk budgeted for the snapshots; [[0282]]
  settled, because the re-ingest writes with the same `version` semantics
  and a later dust-only write of a minute now replaces a priced one.
- **Cost not in the ledger estimate:** re-ingested rows carry
  `close_usd = 0`, so the enrichment worker re-prices the entire history
  afterwards (0268's campaign re-priced 9.94 M candles in ~24 min; 0228's,
  never run, was estimated at ~190 M candles and ~4 h 40 m; this is all of
  them). That same fact makes [[0228]]'s reset-mode campaign (Appendix C)
  **unnecessary**: the refill happens inside the re-enrichment; only its
  after-check (`post_run_0228_it`) still runs, on the repaired XLM/USDC
  reference.

## Out of scope

- D9, the cross-source rule on the read path (median instead of largest
  volume leg). Deferred by 0278; most tokens have one source.
- The pivot's two references (`close_usd` from the end-of-bucket rate,
  `volume_quote_usd` from the bucket VWAP) — 0278 question 5, analysis doc
  §10.3; its own change in `ch_enrich.rs`. (The candidate-predicate change
  above is in scope; this one is not.)

## Acceptance Criteria

Phase 1:
- [ ] A minute holding two 17-stroop fills at 1/17 next to nothing else has
      `vwap = 1/17`, `trade_count = 2`, `pf_trade_count = 0` and price fields
      0; the same minute beside two ordinary fills has `open`/`close` = the
      first/last ordinary fill, `low` untouched by the dust, and reads back
      from ClickHouse with `pf_trade_count = 2`, not the DEFAULT (unit test in
      `bucket.rs` + `#[ignore]` round trip, both red on `develop`).
- [ ] The bound holds exactly at 1/2000 + 1/2000, fails just above, and
      `(10³⁸, 5)` is not price-forming; a Soroban fill is classified on its
      raw `i128` units before scaling (unit tests).
- [ ] Key `(ledger, transaction_index, operation_index, claim_index)` on
      every path; the 2026-04-02 ledger (path payment with a high claim
      index first, manage-offer second) picks the manage-offer fill as
      `close`; `version` is unchanged by the widened key (unit tests, the
      version one red before the key widens); the events-backfill query
      carries the `application_order` join and its fallback (unit test on
      the SQL, fallback path exercised).
- [ ] The same fills in any arrival order give an identical candle, and
      `low ≤ open, close ≤ high` holds on every tier by construction — pinned
      on the 1m tier by a property test and on all six rollup statements by
      an `#[ignore]` test on 26.3.10.60, including a 1M month that does not
      start on a Monday.
- [ ] A 1m minute with `pf_trade_count = 0` contributes to no
      `argMin`/`argMax`/`max`/`min` of its parent; a coarse bucket whose last
      minute is dust-only closes at the last price-forming child's `close`
      and gets `close_usd = close × the latest priced child's rate`, not the
      child's `close_usd` (`#[ignore]` tests on 26.3.10.60 reproducing
      2026-04-02).
- [ ] After `INIT_SQL` on a database holding old-shape rows, every candle
      table carries the three pf columns after `version`, an old row reads
      `pf_trade_count = trade_count`, and re-applying is a no-op; the Rust
      writer's field names equal the canonical column list (tests).
- [ ] A dust-only minute (`close = 0`, `volume_quote > 0`) gets
      `volume_quote_usd` priced and is **not** re-selected by the next
      enrichment pass on any tier; the pf columns survive oracle / peg /
      external / pivot / reset rewrites; the pivot ignores a dust-only
      XLM/USDC minute (`#[ignore]` tests on 26.3.10.60, red on `develop`).
- [ ] `/ohlcv` returns a `pf_trade_count = 0` bucket with the price fields
      absent and volume present, never carried; a multi-source bucket whose
      dust-only source has the larger volume does not supply the prices;
      `pf_trade_count`, `pf_vwap` and the divergence flag are on the wire
      and null on the USDC peg series (unit + `#[ignore]` tests).
- [ ] `docs/database-schema/database-schema-overview.md`, the OpenAPI
      description of `open`/`close`/`high`/`low`/`vwap`, and the general
      overview describe the ADR 0287 definitions, `pf_trade_count` and
      `pf_vwap`; the deploy order is recorded in the repo.
- [x] ADR accepted for the meaning of `open`/`close`/`high`/`low` —
      ADR 0287, 2026-09-15, amended 2026-09-16 (first/last price-forming
      fill, no carry-forward, no settle pass).
- [ ] Six MVs re-created with APPEND + `sum(version)` + aligned windows
      verified, per-MV freshness recovered ([[0146]]'s checklist), the
      rollout run in the deploy order above with the ingest last.
- [ ] Measured on the first week of new data: residual high/low bias vs
      Bitstamp on XLM, share of minutes with `pf_trade_count = 0` by source
      (a Soroban token with 0–3 decimals may never form price under the
      raw-unit bound — record any such asset), share of candles the
      divergence flag marks. Recorded here.

Phase 2:
- [ ] An order-book fill of 17 stroops against an offer at 0.0794
      (`Price { n: 397, d: 5000 }`) prices at 397/5000 (oriented) and forms
      price; a 34-stroop pool fill (5/34) does not form price and still
      counts in volume; the lookup works across `TransactionMeta` V0…V4,
      prefers `State` over `Updated`, and a missing entry or `d ≤ 0` falls
      back to the ratio + bound (unit tests on XDR fixtures).

Phase 3:
- [ ] Every monthly partition of `price_ohlcv_1m` re-ingested; per partition
      and per source `sum(volume_base)`, `sum(volume_quote)`,
      `sum(trade_count)` reconciled against the FREEZE snapshot — equal for
      SDEX (boundary minutes and the live era documented), ≥ with the delta
      explained by [[0282]] for AMM sources; differences otherwise only in
      OHLC.
- [ ] Coarse tiers pre-rolled from the new 1m; XLM/USDC 1d closes on the
      seven dust days of the analysis are within 5 % of Bitstamp (the "last
      price-forming fill" close was not measured on the 1 181-day set —
      this is where it is); the count of XLM-quoted candles priced from a
      quantised XLM/USDC close (0278's before-figure: 22 760 / 143 577 /
      85 699 / 30 064 / 10 938 on 15m / 1h / 4h / 1d / 1w) is zero after the
      re-ingest and pre-roll.
- [ ] The whole history re-enriched (`close_usd > 0` wherever a reference
      exists) and `post_run_0228_it` green on the repaired reference; 0228's
      reset-mode campaign recorded as superseded, not run.

## Notes

- Measured basis for the filter (the 0.1 % bound, the offer price being
  on-market): 0278's `notes/R-measurements-2026-09-15.md` §4. The 2026-09-15
  measurements of k (window size) no longer apply — there is no window.
- The phase-1 unit test for the `vwap` trap only "comes alive" in phase 2
  (before D2 dust is never price-forming), but write it in phase 1 so the
  rollup arithmetic is pinned from the start.
- [[0282]] (candle writes replaced instead of summed across a reconcile run)
  touches the same `version` semantics phase 3 depends on, and under the new
  definition a later dust-only write of a minute replaces a priced one
  outright — settle it before the ingest is deployed.
- Known consequence, not a defect: a Soroban token with 0–3 decimals needs
  > 1 000 raw units on both legs to form price, i.e. 10 whole tokens per
  fill for a 2-decimal token; such an asset gets volume and no price until
  a decimals-aware bound is decided. Watch it in the first-week measurement.
- Reference material from the reverted first implementation (2026-09-16):
  branch `backup/0286-gsd-implementation-2026-09-16` and the gitignored
  `.planning/BRIEF-0286.md` (facts verified against the code and ClickHouse
  26.3.10.60). Its rollup/settle machinery is obsolete; its ingest, schema
  and enrichment findings are folded into phase 1 above.
- 2026-09-15 specification review (ADR 0287's second history entry) and the
  2026-09-16 redefinition (third entry): three 0278 decisions reversed —
  D4 (carry-forward), D5/D6 (windowed VWAP close), D8's shape. D1–D3, D7,
  D10 stand.
