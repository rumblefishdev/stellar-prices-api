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
  - "../../../packages/prices-clickhouse/src/rollup_sql.rs"
  - "../../../docs/runbooks/0286-candle-definitions-rollout.md"
  - "../../../docs/runbooks/0286-reingest-history.md"
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
      close_usd = close × latest priced child's rate (replaces the product
      carry — one MV re-CREATE); missing 1m tail falls back to the
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
  - date: "2026-09-16"
    status: active
    who: akot
    note: >
      Phases 1 and 2 implemented on the branch as four GSD slices, 15
      commits 5cc01b5..844a3a1: pf columns on all seven tables with DEFAULT
      migration; price-forming bound + transaction index on every ingest
      path; 1m candle from first/last price-forming fill; one Rust generator
      for rollups.sql/preroll*.sql with pf-gated six MVs, 1M rolled from 1d
      (mv_ohlcv_1d_to_1M), rate-form close_usd; enrichment carrying all 18
      columns with the new predicates; /ohlcv pf-gated with pf_trade_count,
      pf_vwap, close_divergent; offer price for order-book fills; runbooks
      for the rollout and the phase-3 re-ingest. Workspace 1047 tests green
      (+~120 new), 160 #[ignore] tests green on local 26.3.10.60. Local
      end-to-end check on the real 2026-04-02 ledgers: XLM/USDC 1d close
      0.16310 = Horizon's last order-book fill (was 5/34), 0 OHLC-order
      violations on every tier, 100 % of order-book fills offer-priced. One
      post-verification fix: a price that rounds to 0 at 14 dp forms no
      price. Operator steps (MV re-CREATE, rollout order, first-week
      measurement, phase 3) remain open.
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
  close_usd > 0)` — the latest priced child's *rate* (ADR §5), not a carried
  `argMaxIf(close_usd, …)`: re-create the six MVs once, under [[0142]]'s
  re-apply checklist ([[0142]] and [[0137]] are live).
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
- [x] A minute holding two 17-stroop fills at 1/17 next to nothing else has
      `vwap = 1/17`, `trade_count = 2`, `pf_trade_count = 0` and price fields
      0; the same minute beside two ordinary fills has `open`/`close` = the
      first/last ordinary fill, `low` untouched by the dust, and reads back
      from ClickHouse with `pf_trade_count = 2`, not the DEFAULT (unit test in
      `bucket.rs` + `#[ignore]` round trip, both red on `develop`).
      → `bucket.rs` unit tests + `candle_write_it` (43215eb, 280c77e).
- [x] The bound holds exactly at 1/2000 + 1/2000, fails just above, and
      `(10³⁸, 5)` is not price-forming; a Soroban fill is classified on its
      raw `i128` units before scaling (unit tests).
      → `price.rs` tests incl. the i128 form and the Soroban raw-unit case (42cf7fa).
- [x] Key `(ledger, transaction_index, operation_index, claim_index)` on
      every path; the 2026-04-02 ledger (path payment with a high claim
      index first, manage-offer second) picks the manage-offer fill as
      `close`; `version` is unchanged by the widened key (unit tests, the
      version one red before the key widens); the events-backfill query
      carries the `application_order` join and its fallback (unit test on
      the SQL, fallback path exercised).
      → `filter.rs`/`soroban.rs`/events-backfill tests; the 2026-04-02 shape pinned in `a_path_payment_sweep_closes_before_a_later_manage_offer` (42cf7fa, 7a9b348); version test red before the key widened.
- [x] The same fills in any arrival order give an identical candle, and
      `low ≤ open, close ≤ high` holds on every tier by construction — pinned
      on the 1m tier by a property test and on all six rollup statements by
      an `#[ignore]` test on 26.3.10.60, including a 1M month that does not
      start on a Monday.
      → `arrival_order_does_not_change_the_candle` (1m) + `rollup_pf_it` on 26.3.10.60 incl. an April 1M month (5d4ad5e).
- [x] A 1m minute with `pf_trade_count = 0` contributes to no
      `argMin`/`argMax`/`max`/`min` of its parent; a coarse bucket whose last
      minute is dust-only closes at the last price-forming child's `close`
      and gets `close_usd = close × the latest priced child's rate`, not the
      child's `close_usd` (`#[ignore]` tests on 26.3.10.60 reproducing
      2026-04-02).
      → `rollup_pf_it`, `rollup_chain_it`, `preroll_close_usd_guard_it` (rate form vs carried product distinguished by fixture) (5d4ad5e, 84efe8c).
- [x] After `INIT_SQL` on a database holding old-shape rows, every candle
      table carries the three pf columns after `version`, an old row reads
      `pf_trade_count = trade_count`, and re-applying is a no-op; the Rust
      writer's field names equal the canonical column list (tests).
      → `candle_migration_it` pins column order and a legacy 1d row; `CANDLE_COLUMNS` == writer field names test (93c4e35, 280c77e).
- [x] A dust-only minute (`close = 0`, `volume_quote > 0`) gets
      `volume_quote_usd` priced and is **not** re-selected by the next
      enrichment pass on any tier; the pf columns survive oracle / peg /
      external / pivot / reset rewrites; the pivot ignores a dust-only
      XLM/USDC minute (`#[ignore]` tests on 26.3.10.60, red on `develop`).
      → `ch_enrich_it` (51 tests): candidate predicate, pf survival through oracle/peg/external/pivot/reset, pivot reference (0cefa0f, 84efe8c).
- [x] `/ohlcv` returns a `pf_trade_count = 0` bucket with the price fields
      absent and volume present, never carried; a multi-source bucket whose
      dust-only source has the larger volume does not supply the prices;
      `pf_trade_count`, `pf_vwap` and the divergence flag are on the wire
      and null on the USDC peg series (unit + `#[ignore]` tests).
      → `queries_ch.rs` unit tests + `ohlcv_it` (5 new); confirmed on the local API against real 2026-04-02 data (8ddc2fa).
- [x] `docs/database-schema/database-schema-overview.md`, the OpenAPI
      description of `open`/`close`/`high`/`low`/`vwap`, and the general
      overview describe the ADR 0287 definitions, `pf_trade_count` and
      `pf_vwap`; the deploy order is recorded in the repo.
      → 1f267ae; deploy order in `init.sql` and `docs/runbooks/0286-candle-definitions-rollout.md` (5375279).
- [x] ADR accepted for the meaning of `open`/`close`/`high`/`low` —
      ADR 0287, 2026-09-15, amended 2026-09-16 (first/last price-forming
      fill, no carry-forward, no settle pass).
- [ ] Six MVs re-created with APPEND + `sum(version)` + aligned windows
      verified, per-MV freshness recovered ([[0142]]'s re-apply checklist), the
      rollout run in the deploy order above with the ingest last.
- [ ] Measured on the first week of new data: residual high/low bias vs
      Bitstamp on XLM, share of minutes with `pf_trade_count = 0` by source
      (a Soroban token with 0–3 decimals may never form price under the
      raw-unit bound — record any such asset), share of candles the
      divergence flag marks. Recorded here.

Phase 2:
- [x] An order-book fill of 17 stroops against an offer at 0.0794
      (`Price { n: 397, d: 5000 }`) prices at 397/5000 (oriented) and forms
      price; a 34-stroop pool fill (5/34) does not form price and still
      counts in volume; the lookup works across `TransactionMeta` V0…V4,
      prefers `State` over `Updated`, and a missing entry or `d ≤ 0` falls
      back to the ratio + bound (unit tests on XDR fixtures).
      → `filter.rs` XDR fixture tests (18, first in the repo): meta V0…V4 + LedgerCloseMeta V0/V1/V2, State over Updated, ClaimAtom V0, two claims in one operation, all five claim carriers, op-count mismatch, `d ≤ 0`, negative amount (6005f58, 7a9b348).

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

## Implementation Notes

Branch `fix/0286_candles-are-built-from-dust-fills-in-the-wrong-order`,
15 commits `5cc01b5..844a3a1` (2026-09-16), one commit per GSD plan task:

- **S1 ingest + schema** (`93c4e35`, `42cf7fa`, `43215eb`, `280c77e`):
  `CANDLE_COLUMNS` (18 names) + `CANDLE_GRAINS` in `prices-clickhouse`;
  idempotent `ADD COLUMN IF NOT EXISTS … DEFAULT` on all seven tables;
  `price_forming_i64` / i128 bound; `transaction_index` on classic, Soroban
  live and events-backfill (bounded LEFT join on `application_order`
  collapsed to one row per id, counted fallback); 1m candle sorted by
  `lex_key` with saturating decimals clamped to `Decimal(38,14)`.
- **S2 rollups + enrichment** (`5d4ad5e`, `0cefa0f`, `5375279`, `84efe8c`):
  `prices-clickhouse/src/rollup_sql.rs` renders `rollups.sql`,
  `preroll.sql`, `preroll-live-gap.sql` (file == generator pinned);
  `argMinIf/argMaxIf/maxIf/minIf` on `t.pf_trade_count > 0`, pf sums,
  Float64 `vwap`/`close_usd` with the rate form; `mv_ohlcv_1d_to_1M`
  (400-day window over 1d); `CANDIDATE_PRED` and every writing statement's
  own literal per ADR §6; `insert_columns()` on all five re-inserts.
  `preroll-incremental.sql` / `preroll-amm-reprice.sql` marked historical.
- **S3 read surface + docs** (`8ddc2fa`, `1f267ae`): pf gate in both arms
  with explicit NULL wrappers; `pf_trade_count`, `pf_vwap`,
  `close_divergent` before `source`/`quality`; `close_divergent` computed in
  Rust; OpenAPI `FIELDS`; schema overview, general overview.
- **S4 offer price + phase-3 runbook** (`6005f58`, `5c26272`, `7a9b348`):
  `PriceSource { Offer{n,d} | AmountRatio }` on `RawTrade`, per-operation
  offer map from `operations[i].changes`; `OfferLookupCounts` logged per
  partition/run; `decode_probe` reports the offer-priced share;
  `docs/runbooks/0286-reingest-history.md`.
- **Post-verification** (`844a3a1`): `price_survives_column_scale` — a
  price rounding to 0 at 14 dp forms no price (ratio, offer and AMM arms).

Verification (2026-09-16, local 26.3.10.60 only): `cargo test --workspace`
1047 / 0; 12 `#[ignore]` suites 160 / 0; real backfill of ledgers
61926675..61942890 (2026-04-02): XLM/USDC 1d open/high/vwap identical to
Horizon, close 0.16310 = Horizon's last order-book fill, low 0.1604 vs
Horizon's 0.1471 (the 5/34 dust, now excluded); 1d close = last
price-forming 1m close on all 14 393 pairs; 10.5 % of SDEX minutes have
`pf_trade_count = 0`. Report kept in the gitignored
`.planning/VERIFY-0286-local.md`.

## Design Decisions

### From Plan

1. **Candle = first/last price-forming fill of its own bucket** (ADR 0287
   third amendment); no carry-forward, no settle pass.
2. **1M rolls from 1d**, not 1w — a week straddling a month must not carry
   the next month's trades into the 1M close.
3. **Coarse `close_usd` = close × latest priced child's rate**, not a
   carried product; it lands in the one MV re-CREATE.
4. **Deploy order** schema → enrichment + API → MV re-CREATE → ingest last.

### Emerged

5. **`vwap` is an explicit zero, never NULL** in the rollups — the column is
   not Nullable; `ifNull(toDecimal128OrZero(toString(...)), 0)`.
6. **Read-path `vwap` is recomputed over price-forming rows and clamped
   into `[low, high]`**, so it can differ from the stored `vwap` of a
   bucket that also holds dust (documented in `queries_ch.rs`).
7. **`pf_vwap` is `nullIf(…, 0)`** — a zero would be clamped up to `low`
   and invent a price. The 1 % divergence flag is computed in Rust.
8. **A sub-resolution price forms no price** (`844a3a1`): a fill of
   3.3e10 base for 0.0001622 quote passes the bound but rounds to 0 at
   14 dp; without the rule it produced `pf_trade_count = 1` with all
   prices 0 — a shape the ADR excludes. Found by the local end-to-end run.
9. **Offer-priced fill with a non-positive amount is not price-forming**
   (review finding, `7a9b348`).
10. **The rollup generator validates `db` as a bare identifier and takes
    typed bounds** (`Bound::{Param, Timestamp}`), rendering byte-identical
    SQL to the shipped files.
11. **A candle is selectable once per tier by the enrichment** — a
    dust-only row, once its volume is priced, is never re-admitted; the
    peg-before-external ordering (pre-0286) applies to every row
    (rollout runbook §9).
12. **`--mode combined` is banned during the phase-3 re-ingest** — it would
    race `events-backfill` at an equal `version` (phase-3 runbook).
13. **`views.sql` / `current.sql` keep no pf gate** (out of scope) —
    `/ohlcv` can now refuse a price that `price_usd_series` and
    `current_prices` still publish.

## Issues Encountered

- **Name-routed writer** (clickhouse 0.13): a struct omitting a column
  silently takes its DEFAULT, and `pf_trade_count DEFAULT trade_count`
  would declare a dust-only row price-forming — the writer test pins the
  18 names.
- **Decimal division overflows silently** past ~1.7e10 on 26.3.10.60; the
  rollups compute `vwap`/`close_usd` in Float64 and convert with
  `toDecimal128OrZero`.
- **`argMaxIf`/`maxIf` over zero rows return 0, not NULL** — every read-path
  aggregate carries an explicit `countIf = 0 → NULL` wrapper.
- **Events-backfill join fan-out** (S1 review CR): the `application_order`
  join collapsed to one row per transaction id.
- **The local `prices` DB held stale MV bodies** from the reverted first
  implementation; the six local MVs were dropped and re-created.
- **Rounding**: `Decimal::round_dp` is half-to-even, the writer's own rule;
  the sub-resolution test first assumed half-up.
- **Environment-only reds**: `post_run_0228_it`, `post_run_0268_it`
  (production after-checks), `execution_bound_error_it` (needs Caddy),
  `endpoints_it::backfill_status_maps_both_streams` (time-rotted fixture
  dated 2026-06-15 against the 7-day rule) — none touched by this task.

**Broken/modified tests:** `rollup_drift_it` mutated `argMax(close_usd, …)`
which no longer exists; it now narrows the rate predicate. The "last child
is dust" fixtures in `rollup_pf_it` / `preroll_close_usd_guard_it` were
changed so the rate form and the carried product give different answers.
`ohlcv_it`'s alias-tail guard follows the new positional tail. All
intentional.

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
