---
id: "0267"
title: "Serve USDC's measured USD history from the Chainlink anchor — load external rates, stop synthesising 1.0, say on the wire what each point is"
type: FEATURE
status: backlog
related_adr: ["0011"]
related_tasks: ["0265", "0247", "0168", "0173", "0111", "0125", "0127", "0266", "0268"]
tags: [layer-backend, layer-api, priority-high, effort-medium, milestone-M3, pricing, enrichment, data-correctness, stablecoin]
milestone: 3
links:
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/memo.md"
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/S-composition-rule.md"
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/notes/S-backfill-migration.md"
  - "0265_FEATURE_price-usdc-from-measurement-not-the-peg/analysis/guardrails.py"
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
  - "../../../packages/prices-clickhouse/schema/views.sql"
history:
  - date: 2026-09-07
    status: backlog
    who: akot
    note: >
      Spawned from [[0265]]'s decision memo. 0265 is the research (why, from
      where, how to compose); this is the implementation. [[0247]] designed
      the usd_rate loading path and becomes step 1 here rather than a
      separate ticket — its acceptance criteria are folded in below.
  - date: 2026-09-09
    status: backlog
    who: claude
    note: >
      CODE HALF COMPLETE on feat/0267_usdc-measured-history-from-chainlink-anchor
      (stacked on 0268's branch), four commits. Schema + both view grains +
      ohlcv_peg_series + Candle.source/quality + the load-external-rate loader
      + the operator runbook. 34 new tests, none needing a ClickHouse; 4 new
      #[ignore] behavioural tests. Cargo.lock unchanged. STILL OPEN: the
      production load, the deploy, and every criterion that can only be
      observed after it — see the Operator Checklist below. Decisions A-F were
      ratified by Adam on the same day; three more emerged, of which the
      composed series running to 2026-09-04 rather than 2026-03-10 is the one
      that changed the design.
---

# Serve USDC's measured USD history

## Summary

`/ohlcv` for canonical USDC publishes a literal `1.0` for every bucket before
2026-03-11 (`queries_ch.rs:933`, reached only through the `is_peg_asset`
gate at `handlers.rs:643`). [[0265]] located the cause, chose the source and
proved the composition rule over the whole history. This task ships it:
load the measured series, make the read path prefer it, and put
`source`/`quality` on the wire so a consumer can tell "was 1.00" from "we
do not know".

The acceptance fixture is the falsifying date: **2023-03-11 must close at
0.9681, not 1.0.**

## Context

- The source decision and its evidence: [[0265]] `notes/memo.md`. Primary
  Chainlink USDC/USD rounds (on-chain, key-less, 2021-02-17 →), fallback
  Bitstamp, cross-check Kraken. USDT-quoted venues and peer stablecoins
  rejected on measured dispersion and correlation.
- The composed daily series already exists (`compose_usdc.py` output,
  2 049 rows 2021-01-25 → 2026-03-10: 98.6 % measured, 1.2 % fallback,
  0.2 % disputed, 0 missing). Re-running the script regenerates it.
- The loading path is [[0247]]'s: rows into `usd_rate` with a `method`
  distinct from `oracle`, view-first, no re-enrichment, ticker→issuer gate
  from [[0173]] satisfied because Stellar USDC `GA5Z…KZVN` is Circle's own
  issuance.
- The cost of not doing it: one-day 99 % VaR of the served series is 0 bps;
  measured it is 6 bps, ES 25 bps, worst day 300 bps.

## Implementation

1. **Loader** (from [[0247]]): the composed daily series →
   `prices.usd_rate`, canonical USDC only, `method = 'external'` added to
   `init.sql`'s vocabulary beside `oracle`/`peg`/`pivot`/`pivot2`, plus
   `source`, `quality`, `n_obs`, `xcheck_spread_bps`, `loaded_at`. Range
   2021-01-25 → 2026-03-10 so no key overlaps our own `oracle` readings.
   Load under a shadow value first (`'external-candidate'`), run
   `guardrails.py` and the 2023-03-11 check against the view, then rename.
   The loader refuses any asset code other than canonical USDC.
2. **Read path**: `ohlcv_peg_series` (`queries_ch.rs:952-957`) and
   `price_usd_series*` (`views.sql:362`, `:570`) accept `external` beside
   `oracle`; the literal `toDecimal128(1, 14)` remains only for buckets with
   neither. `n_obs` and the cross-check spread ride through.
3. **DTO**: `Candle` gains `source` (`chainlink` · `bitstamp` · `oracle` ·
   `` ), `quality` (`measured` · `measured-disputed` · `fallback` ·
   `missing`), `n_obs`, `xcheck_spread_bps`. `method` stays for the
   enrichment vocabulary. `backfill_note` states that stored `close_usd` on
   USDC-quoted candles still carries the assumption until [[0111]]'s
   re-enrichment (defect B).
4. **CI fixture**: an integration test over `/ohlcv?timeframe=all&granularity=1d`
   for canonical USDC asserting close(2023-03-11) ≠ 1.0, plus the two reject
   invariants from `guardrails.py` (30-day share of `trade_count = 0` ≥ 0.9;
   flat-and-no-trades ≥ 0.9) for every asset with a fiat-peg code. Fails
   today by design.
5. **Alarms**: the four alert invariants (exact-1.0 share, constant run,
   30-day realised vol, zero-volume share) as custom metrics on the [[0125]]
   dashboard, one evaluation per day, thresholds as in
   `notes/S-guardrails.md`.
6. **Evidence**: re-include USDC in the [[0127]] spot-check table with the
   0.9681 close.
7. **Ongoing**: a scheduled read of new Chainlink rounds (1–3/day in calm)
   appended as `external` **only** for buckets our own `oracle` did not
   observe — the oracle stays primary from 2026-03-11 on.

## Acceptance Criteria

Ticked = closed by the code on this branch. Unticked = needs the production
load and the deploy (Operator Checklist below), or was descoped — each says
which.

- [ ] `GET /v1/assets/USDC:GA5Z…/ohlcv` returns `close = 0.9681`,
      `source = chainlink`, `quality = measured` for 2023-03-11
      *(DEPLOY-GATED. The query, the read path and the wire fields all ship
      here, and an `#[ignore]` integration test
      (`ohlcv_usdc_publishes_the_imported_measurement_for_the_2023_depeg`)
      asserts exactly this against a seeded ClickHouse. ⚠️ The wire value is
      `0.96812` — the column is `Decimal(38, 14)`, the value is returned
      through `toString`, and ClickHouse TRIMS a Decimal's trailing zeros, so
      neither `0.96812000000000` nor the rounded QUOTATION `0.9681` matches;
      see Emerged decision 10.)*
- [ ] No bucket of that series carries `method = peg` between 2021-01-25 and
      2026-03-10; `trade_count`/`n_obs` reflect rounds, not 0
      *(DEPLOY-GATED for the first half. ⚠️ The second half is DESCOPED and
      SETTLED: `n_obs` was not added to `usd_rate` or to `Candle`, and Adam
      confirmed on 2026-09-09 that it is not needed — see Emerged decision 9.
      Not a deferred item; do not file a follow-up. `trade_count` stays 0 on this synthesized series by the
      pre-existing design, because USDC is not traded as a base and reporting
      its quote volume here would answer a different question.)*
- [x] `usd_rate` carries the external rows with `method = 'external'`, no key
      overlapping an `oracle` row; `price_usd_series` and `_1h` publish them
      and there is no discontinuity artefact at 2026-03-11 *(from [[0247]])* —
      the loader partitions strictly below `USDC_ORACLE_EPOCH_S` so the two
      populations share no KEY — but the epoch is 14:00 and the import is
      stamped at 00:00, so the **1d bucket of 2026-03-11 holds both** and the
      oracle-wins rank does real work there from day one (review IN-01;
      pinned by `the_last_loadable_day_is_the_epoch_day_itself_so_one_daily_
      bucket_holds_both`); both grains widened in one commit with an explicit
      oracle-wins rank, and since review round 1 `/ohlcv` applies the same
      bucket-wide rule; the seam is asserted by
      `ohlcv_usdc_reads_each_side_of_the_oracle_epoch_with_its_own_method`
      (which now seeds that very bucket) and by
      `an_oracle_row_outranks_an_imported_row_in_the_same_bucket`
- [x] Rollback verified: reverting the preference order restores today's output
      byte for byte; rows are not deleted — the promote ADDS a
      ReplacingMergeTree key rather than moving one (`method` is in the
      sorting key), so nothing is ever deleted and the rollback is a read-path
      revert. `the_promote_is_an_additive_insert_select_with_no_destructive_verb`
      asserts the statement carries no DELETE/DROP/ALTER/TRUNCATE; runbook §9
      is the procedure
- [ ] CI fixture green after the deploy, red before it
      *(PARTIAL, and the split is deliberate. CI has no ClickHouse, so the
      behavioural fixtures are `#[ignore]` and the invariants they depend on
      are pinned by SQL-string tests that DO run on every push. The one CI
      test that runs unconditionally is the dry run over the versioned CSV
      (`tests/composed_usdc_csv.rs`), which is green today because it tests
      the loader, not the deployed API.)*
- [ ] The four alarms exist on the dashboard and do not fire on the
      2026-08 → 2026-09 window
      *(OUT OF SCOPE HERE. The alarms are a [[0125]] change and this task
      touches no alarms; `guardrails.py`'s invariants are not implemented as
      metrics. Needs its own task.)*
- [x] The source, its granularity, the fetch date and the ticker→issuer
      decision are recorded in this task *(from [[0247]])* — see Implementation
      Notes; the ticker→issuer gate is CODE
      (`external_rate::check_identity`), not a comment, with its own test
- [ ] [[0247]] closed as folded into this task; [[0265]] archived
      *(0265 IS archived — the versioned CSV is read from its archive
      directory. 0247 is not closed here: four of its six criteria are closed
      by this branch (see below), but its first — rows actually present in
      `prices.usd_rate` — is the production load. Close 0247 when the load has
      run.)*

### [[0247]]'s criteria, individually

| 0247 criterion | Status |
|---|---|
| `usd_rate` carries canonical-USDC rows 2021-01-25 → `2026-03-11 14:00` with a `method` distinct from `'oracle'` | **deploy-gated** — the tool, the method word and the range are all here; the rows are not loaded |
| No overlap with our own readings; one row per bucket either side; no discontinuity artefact | **closed** — strict `<` partition at the shared epoch constant (no shared key); the ONE daily bucket that holds both (the epoch day) is settled by the rank rule on every surface |
| Both `price_usd_series` grains publish the imported rate and stop reporting `peg` for covered buckets | **closed** — both grains widened in one commit, with a single-grain-edit guard |
| The March 2023 SVB window reads materially below par | **deploy-gated** — asserted at `0.96812` by an `#[ignore]` test and by the runbook's curl |
| Source, granularity and fetch date recorded in the task file | **closed** — Implementation Notes below |
| The ticker→issuer decision recorded, with [[0173]]'s reasoning applied | **closed** — and expressed as a refusal in code |

## Out of scope

- Correcting stored `close_usd` on USDC-quoted candles and removing
  `method: peg` from non-stablecoins (defect B) — [[0268]];
  [[0266]]'s ~25 % dislocations on stress dates are a different mechanism
  and stay their own task.
- Any asset other than canonical USDC.

## Implementation Notes

What shipped, on `feat/0267_usdc-measured-history-from-chainlink-anchor`
(stacked on `fix/0268_close-usd-assumes-usdc-is-a-dollar`), in four commits.

**The source, its granularity and its fetch date**, because an imported series
with no provenance is not evidence ([[0247]]'s fifth criterion). Primary
**Chainlink** USDC/USD on-chain rounds, fallback **Bitstamp**, cross-checked
against Kraken; composed by [[0265]]'s `compose_usdc.py` into a **daily** series
and versioned in this repo at
`lore/1-tasks/archive/0265_.../data/composed_usdc_usd_1d.csv`. **2049 rows,
2021-01-25 → 2026-09-04**, fetched and composed 2026-09-07. It is read 1:1 and
NOT copied into a fixtures directory: a second copy is a second thing to keep in
step, and the CI test's whole point is that the tool agrees with the artefact an
operator will actually pass it.

**The ticker→issuer gate is code.** `external_rate::check_identity` refuses
every code and issuer but `USDC` /
`GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN` (Circle's own
issuance), naming the accepted pair in the refusal. [[0173]]'s reasoning applied
rather than restated: asset codes are not unique on Stellar, and an imported USD
history attached to the wrong issuer publishes one asset's price under another's
name.

**Schema.** One idempotent
`ALTER TABLE prices.usd_rate ADD COLUMN IF NOT EXISTS quality
LowCardinality(String) DEFAULT '' AFTER reference_asset`, in the
`current_prices.method` style already in `init.sql`. Positioned `AFTER
reference_asset` so the two PROVENANCE columns sit together — which outside
series, and how good that series says its observation was — ahead of the
`hops`/`version` bookkeeping. Oracle rows keep `''`, and the column comment says
that means "not applicable", never "unknown". The method vocabulary comment
gained the `'external-candidate'` staging word.

**Eight view edit sites, four per grain, and the two grains move together.**
`price_usd_series` and `price_usd_series_1h` each took: the rate subquery
(`method IN ('oracle', 'external')` plus two rank-first `argMax` tuples), arm A
(a `toUInt8(0) AS rate_rank` placeholder), arm B (a numeric rank off the winning
row's method) and the label arm (three-way). ⚠️ Keeping the grains in step is the
point — the pre-0165 hourly variant carried a defect the daily one did not — so
`views_sql_both_series_grains_agree_on_every_task_0267_token` counts every new
token in both statements and fails if they differ. It earned its place
immediately: the first pass edited only the daily arm A (the two `FROM` clauses
differ by table name, so a `replace_all` matched one), and this test is what
caught it.

**The lib/bin split, and the invisibility that forced it.** CI runs
`cargo check --workspace` / `cargo test --workspace` with **no features**, and
Cargo SILENTLY SKIPS a `[[bin]]` whose `required-features` are unmet. The loader
needs `aws-mtls` for the Hetzner transport, so anything inside the binary would
neither compile nor run in CI. Every decision therefore lives in
`enrichment-worker/src/external_rate.rs` — parse, the refusals, the epoch
partition, the rendered statements — and `bin/load-external-rate.rs` keeps clap
wiring, the transport match and `.execute()`. Same split as
`ledger-processor::metrics`. The consequence that matters: the dry run over the
real 2049-row CSV is a NORMAL CI test, not an `#[ignore]` one.

**No new package.** The CSV is machine-generated, ten fixed columns, no quoted
fields across all 2049 rows, so the splitter is ~30 lines by hand and
`Cargo.lock` is byte-identical. It REFUSES a quoted field rather than
mis-splitting one — the guard for the day that stops being true. Adding any
crate here re-opens the package-legitimacy gate.

**The promote is additive, and the help text says so.** `method` is part of
`usd_rate`'s `ORDER BY`, so the `external-candidate` rows and the `external`
rows are DIFFERENT ReplacingMergeTree keys and both survive. `--promote` adds a
key; it does not move one, and it deletes nothing. That is the whole reason the
rollback is free: revert the read path's preference and every loaded row stays
on disk, unread. Re-running the promote is idempotent only because RMT dedups
the identical promoted key on the higher version.

**The two new wire fields, and why every projection had to move.** `Candle`
gains `source` and `quality` as its LAST two fields. `Candle` derives
`clickhouse::Row` and RowBinary is POSITIONAL and carries no types: the
deserializer reads one byte as the `Option` tag, so a projection that disagrees
with the struct either errors with `InvalidTagEncoding` or — for lengths 0 and
1 — SILENTLY MIS-FRAMES the rest of the row. The failure mode is a plausible
WRONG number on a public endpoint, with nothing failing anywhere. So both
`ohlcv` arms emit the two columns as typed NULLs even though neither can
populate them, and the outer projection is now ONE shared constant
(`OUTER_ALIASES`) instead of two hand-maintained copies. The two aggregate
strings were extracted into functions so a ClickHouse-free test can read them,
and `every_projection_and_aggregate_ends_with_the_same_provenance_tail` compares
every site's alias SEQUENCE against that one list.

**A separate runbook file, `docs/runbooks/load-external-usdc-rate.md`**, and the
two reasons are mechanical rather than aesthetic — both are recorded in the new
file's own header so nobody folds it back:
(a) `ch_enrich.rs`'s
`the_runbook_hand_types_the_oracle_epoch_once_and_it_is_the_constant` asserts
the epoch literal appears EXACTLY ONCE in `repair-coarse-usd-values.md` and that
no second ten-digit `177…` number appears anywhere in it, so a new section there
that mentions the epoch turns a green CI red;
(b) **"Appendix B" is referenced by name from eight source and test locations**,
so inserting a section before it and renumbering would silently invalidate every
one of those prose pointers. The new runbook has the same discipline applied to
itself, pinned by a mirror test in `external_rate.rs` — which lives in the LAST
commit because `include_str!` of a file that does not yet exist does not
compile.

`repair-coarse-usd-values.md` gained exactly one pointer, inside Appendix B's
precondition 1, naming the new runbook and warning that the precondition counts
only the PROMOTED word — so an operator who finds zero `external` rows beside
1872 `external-candidate` ones knows the fix is the promote, not a re-load. No
epoch literal, no second `177…` number, no appendix renumbered.

**Review round 1 (2026-09-09, `gsd-code-reviewer` at `standard`: 1 critical,
6 warnings, 5 info — all twelve addressed in one commit).** The Rust half held;
the wire and the operator path did not. What changed:

- **CR-01, the wire.** `source`/`quality` were `toNullable(…)` over columns
  whose oracle value is `''`, so every oracle-priced USDC bucket — every live
  bucket in production — would have carried `"source": ""` against the OpenAPI
  text, and the `#[ignore]` epoch-seam test's `Value::Null` assertion could
  never have held. Now `nullIf(…, '')`, and provenance is emitted ONLY when
  the imported row won the bucket (an outranked import's `bitstamp` must not
  leak onto the oracle's bucket either). Pinned by
  `peg_series_sql_nulls_the_provenance_outside_an_imported_rate`.
- **WR-05 and WR-06 together, one rewrite of `ohlcv_peg_series`.** The single
  `IN ('oracle', 'external')` subquery — rank inside the instant, ASOF recency
  across instants — became TWO method-specific ASOF joins, nested rather than
  chained: `ro` (newest `oracle` before the bucket end, window unchanged from
  0246) and `re` (newest `external` before the bucket end, valid for its WHOLE
  UTC DAY). A valid oracle reading wins the bucket outright — the views' rule,
  and the rule the one overlapping production bucket needs — and an imported
  day now prices every bucket of that day at every grain, which is what 0268's
  external tier already does to the same day's candles. Each side is a single
  method, so nothing needs a collapse before the join and the "no longer
  degenerate `argMax`" is gone with its comment. Issuer binds twice. SQL-string
  tests for both rules and the floors at `1m`/`1h`/`1d`; `#[ignore]` cases
  `ohlcv_usdc_oracle_outranks_a_later_import_in_the_same_bucket` (the
  `views_it` fixture through the API, cross-checked against the view) and
  `ohlcv_usdc_serves_an_imported_day_at_every_hour_of_it` (three hours of the
  depeg day plus the next midnight, which must fall back to `peg`).
- **WR-01..04, the runbook.** A `## Preconditions` section: the schema and
  the widened views are applied FIRST with `prices-clickhouse-init` (named
  with the 0142 caveat of what it re-lands — all of `init.sql`,
  `backfill_progress`, all six views, `DROP VIEW` grant, host loopback as
  `default`), then a `system.columns` check for `quality`; applying the views
  before the promote is harmless because they read a method word that holds no
  rows yet. Step 6 became a data check instead of a fictional `apply-schema`.
  Expected strings are the TRIMMED Decimals `0.96812` and `1`. The midnight
  gate no longer uses `toTime()` (anchored to 1970-01-**02**): both this
  runbook's 4b and 0268's Appendix B precondition 2 now count
  `timestamp != toStartOfDay(timestamp)`, expecting 0 — the latter edited on
  this stacked branch, keeping its epoch literal count at one.
- **IN-01..05.** The "no overlap" prose corrected in the views test, the
  runbook and here, with a CI test that pins 2026-03-11 as the one shared
  bucket; the promote re-applies `timestamp < toDateTime(EPOCH)`; refusals
  name the real path; `DEPEG_DAY_START_S` defined once; a rate finer than
  `Decimal(38, 14)` is a new refusal, `RateTooPrecise`.

## Design Decisions

### From Plan

Ratified by Adam on 2026-09-09, before implementation.

1. **Decision A — the loader is a Rust binary, `load-external-rate`, in
   `enrichment-worker`**, a sibling of `bin/coarse-repair.rs` with the same
   `--features aws-mtls` transport surface. Shadow / promote / dry-run modes and
   six hard refusals, each with its own CI unit test. Rejected: a Python one-off
   — no gate, no tests, and nothing a reviewer can re-run.
2. **Decision B — `quality` becomes a column on `prices.usd_rate`**, a
   `LowCardinality(String) DEFAULT ''` added by an idempotent ALTER. Rejected:
   packing it into `reference_asset` (which already means the source), and
   dropping it (the `fallback` and `measured-disputed` days are exactly what a
   consumer must be able to see).
3. **Decision C — read-path widening with an EXPLICIT preference.** All three
   sites read `method IN ('oracle', 'external')`, and where a bucket holds both,
   oracle wins by a rank rather than by timestamp. Label arms become three-way;
   `peg` stays the fallback for buckets with neither.
4. **Decision D — `Candle` gains nullable `source` and `quality`**, populated
   only on canonical USDC's own synthesized series. `null` on every other asset,
   because the candle tables carry no provenance column — that gap is [[0268]]'s
   Issue 9 and is deliberately out of scope. OpenAPI text in the same commit.
5. **Decision E — the branch is stacked on [[0268]].**
   `prices_clickhouse::USDC_ORACLE_EPOCH_S` is IMPORTED, never redefined; PR
   #293 merges first.
6. **Decision F — the loader PARTITIONS the CSV at the epoch rather than
   refusing it.** Rows below are loaded (1872), rows at or above are counted and
   reported as skipped (177), never written. Hard refusals are reserved for the
   invariants that mean the FILE is wrong. The CSV stays [[0265]]'s artefact
   1:1 and the boundary lives in code, in the same constant the label arm keys
   on.

**Scope note (locked).** HISTORICAL data only. No ongoing append of Chainlink
rounds is built, planned, or recorded as future work — item 7 of the
Implementation section above is retired, not deferred. Our own polling is
primary from the epoch on.

### Emerged

7. **The composed series runs to 2026-09-04, not to 2026-03-10 — and that is
   what made Decision F necessary.** This task's own text (and [[0247]]'s first
   criterion) assumed the composer stopped at the oracle boundary; it did not,
   because the composer wrote the full history it could fetch. 177 of the 2049
   rows sit at or above the epoch. Refusing the file over them would have made
   [[0265]]'s artefact unusable without editing it, and editing it would have
   broken the 1:1 provenance the same criterion demands. Partitioning is the
   only option that keeps both.
8. **The staging word has the promoted word as a PREFIX, so only an EQUALITY
   predicate is safe.** `external-candidate` starts with `external`. Every
   current reader uses `=` or an `IN` list and is fine, but a
   `LIKE 'external%'` or `startsWith(method, 'external')` written later would
   select the UNVERIFIED staged rows and hand them to [[0268]]'s re-enrichment
   of hundreds of millions of candles. Not noticed until the guard test was
   written. It is now asserted explicitly
   (`only_an_equality_predicate_keeps_the_staged_rows_out_of_the_read_path`) and
   stated in the module docs. Renaming the staging word to something without the
   shared prefix (`candidate-external`) would remove the hazard entirely.

   **RATIFIED: the word STAYS (Adam, 2026-09-09.)** Raised explicitly, with the
   rename costed at one constant — `external_rate::SHADOW_METHOD` is the only
   real SQL literal, the other nine occurrences being comments, docs and test
   fixtures — and declined. Decision A's literal stands.

   Two consequences are therefore permanent rather than provisional, and neither
   should be quietly relaxed:
   - **`only_an_equality_predicate_keeps_the_staged_rows_out_of_the_read_path`
     is the STANDING guard**, not temporary scaffolding. It asserts the read
     predicate is an equality and carries no `LIKE`, `startsWith` or `%`. Do not
     delete it as redundant — it is the only thing that fails if someone reaches
     for a prefix match.
   - **Every future reader of `usd_rate.method` must use `=` or an `IN` list.**
     `startsWith(method, 'external')` and `LIKE 'external%'` are the idioms that
     use an index, so they are the ones somebody will reach for, and both select
     the unverified staged rows. The module docs say so beside the constants.

   ⚠️ If the rename is ever reopened, the window is **before the first shadow
   load**. `--promote` reads `method = SHADOW_METHOD`, so renaming after a load
   orphans the staged rows — the promote finds zero and the shadow load has to
   be re-run. Free now, irritating later.
9. **`n_obs` and `xcheck_spread_bps` were NOT added**, to either `usd_rate` or
   `Candle`, though this task's Implementation §1 and §3 list them. Decisions B
   and D name exactly one new column and exactly two new wire fields, and they
   were ratified after that text was written. Both figures are in the CSV and
   can be added later without a migration; adding them now would have widened a
   published response schema with fields nobody asked for, and a field removed
   from a published schema is a breaking change for every generated client.

   **RATIFIED: not needed (Adam, 2026-09-09.)** Raised explicitly, with the cost
   of adding them later stated — two idempotent ALTERs, two appended `Candle`
   fields, three aggregates and two projections, the alias guard catching a
   missed site — and with the observation that `quality` already carries most of
   the cross-check signal: `measured-disputed` means precisely "the two sources
   disagreed beyond tolerance". `quality` answers *whether*, and
   `xcheck_spread_bps` would have answered *by how much*, which nobody needs.
   **Closed, not deferred — no follow-up task is filed and none should be.**
   Implementation §1 and §3 above are superseded on this point by Decisions B
   and D.
10. **The wire value is the stored decimal with its trailing zeros TRIMMED.**
    `usd_rate.usd_rate` is `Decimal(38, 14)` and reaches the wire through
    `toString`, which ClickHouse prints without trailing zeros — so 2023-03-11
    publishes `0.96812`, and the peg publishes `1`, not `1.00000000000000`
    (`views_it.rs` pins both forms). ⚠️ The first draft of this decision, the
    runbook and the BRIEF all said `0.96812000000000`; that was wrong, and an
    operator following the runbook literally would have halted a correct load
    at the 4c gate (review WR-02). The `0.9681` this task quotes is a
    four-significant-figure rounding and does not match either. The integration
    test asserts the exact trimmed string plus a numeric check.
11. **A refusal returned as a `LoadError` from `main` prints via `Debug`, not
    `Display`** — Rust's `Termination` impl for `Result<_, E: Debug>` — so the
    first smoke run showed the operator
    `ForeignIdentity { code: "USDT", … }` instead of the sentence explaining
    what to do. Every carefully written refusal message was invisible at exactly
    the moment it mattered. Fixed by rendering through `to_string()` at the
    `main` boundary, matching `coarse-repair`'s `format!(…).into()` shape. Not a
    plan deviation, just a bug the plan could not have predicted.
12. **The rate is rendered as `toDecimal128('<literal>', 14)`, never through a
    float.** The column's fourteenth place is the entire reason it is a decimal;
    an `f64` intermediate loses it silently, and a rate wrong in the last place
    looks exactly like one that is right. Asserted by
    `the_shadow_insert_names_every_usd_rate_column_and_stamps_the_run`, which
    also asserts the statement contains no `toFloat64`.
13. **The INSERT is chunked at 250 rows.** 1872 rows in one request against a
    SHARED production cluster is inconsiderate and the load is not urgent. A test
    proves every row survives the chunking exactly once, and an empty slice
    produces NO statement — an `INSERT … VALUES` with no values is a syntax
    error, not a no-op.
14. **Two stale comments in the read path were corrected rather than left.**
    `views.sql`'s "this consumer chooses measured or nothing" and
    `queries_ch.rs`'s "**Only `method = 'oracle'` is accepted now**" both
    described the pre-widening rule. They now say that "measured" is two words,
    that a COMPUTED `pivot` is still refused, and that provenance rather than
    authorship is what separates them — a comment that contradicts the code
    beneath it is worse than no comment.

15. **`/ohlcv` applies the views' bucket-wide oracle preference, not a
    tie-break at one instant (review WR-05).** The first implementation ranked
    `oracle` only within an `rts` group and let the ASOF's recency decide across
    instants — a different rule from `views.sql`'s rank-first tuple, held
    equal to it only by the loader's day-start stamping. The reviewer offered
    either rule with the comment corrected; the views' rule was taken, because
    the one production bucket that holds both (the epoch day) must read the
    same on both surfaces, and a rule that depends on another tool's invariant
    is not one either surface can be trusted on. Implemented as two nested
    method-specific ASOF joins rather than a bucket-level pre-aggregation, so
    the 0246 oracle window is untouched and no `argMax` has to choose between
    methods at all.
16. **An `external` row is valid for its whole UTC day on `/ohlcv`, at every
    grain (review WR-06).** The import is daily; 0268's external tier prices
    every candle of the day from it; a one-bucket window on the self-series
    published `peg`/$1 for 23 of 24 hours of an imported day while the same
    hour's XLM/USDC candle said 0.96812. The external floor is
    `toStartOfDay(bkt)`; the oracle floor is unchanged. The window ends at the
    day's end — the next midnight without a row of its own falls back to `peg`,
    asserted, because forward-filling past the day would be 0246's defect in a
    new place. ⚠️ `price_usd_series_1h` was NOT widened the same way — see
    Issues.
17. **The promote carries the epoch bound itself (review IN-02).** `partition`
    runs in a different invocation, possibly of a different binary over a
    staging set some other file produced; the promote is the one write the read
    path serves, so it is the last place the code can hold Decision F's
    boundary. Asserted with the one bound on the shared constant.

## Issues Encountered

- **`price_usd_series_1h` and `/ohlcv` now disagree on the hours of an
  imported day, and a decision is owed.** Since review round 1, `/ohlcv` at
  `granularity=1h` serves an imported daily row for all twenty-four hours
  (Emerged 16), agreeing with 0268's hourly candles; the hourly VIEW still
  buckets `usd_rate` by the hour and publishes `1`/`peg` for the twenty-three
  hours after midnight. Task 0246's cross-surface criterion ("the same value
  for the same bucket") is therefore met for oracle-priced hours and NOT for
  imported ones. Options, for Adam: (a) widen the hourly view's rate subquery
  so an `external` row expands to every hour of its day (an `ARRAY JOIN
  range(24)` on the external rows, keeping the oracle-first tuple) — a
  views.sql change with its own grain-agreement test; (b) load the hourly
  March-2023 CSV for the depeg window, which fills the hours with real
  measurements but only for that month; (c) accept the divergence and
  document it, as the runbook's step 8 and the OpenAPI text do today. Not
  decided here; the runbook says not to "fix" it in the load procedure.
- **The `1d` bucket of the epoch day is the one place both provenances meet,
  and its label is `oracle`** — the day's polls outrank the import at 00:00 on
  every surface. A consumer reading 2026-03-11 sees a measured poll, not the
  composed series' value for that day. That is the ratified rule (Decision C),
  recorded so nobody reads it as a seam artefact.

- **`prices-clickhouse`'s `init_sql_parses_into_statements` counts statements**
  and asserted 33. The new ALTER makes 34. Updated with a comment naming which
  statement was added — the count is a real guard (it catches a statement lost
  to a stray `;`), not noise.
- **`usd_rate_it.rs` asserts `usd_rate`'s exact column vector and is
  `#[ignore]`**, so CI could not have caught the break the new column causes. It
  was updated in the SAME commit as the ALTER, deliberately, because the gap
  between the two is where this kind of breakage normally lives.
- **`cargo clippy --all-targets -p prices-clickhouse` fails on
  `tests/rollup_append_it.rs`** with two `doc_lazy_continuation` errors in a
  module doc comment. PRE-EXISTING and unrelated to this task — that file is
  untouched here, and `prices-clickhouse` is not in `ci.yml`'s clippy list, so
  nothing regressed. The plan's own verify (`clippy --no-deps -p
  prices-clickhouse`, lib only) is clean. Left alone under the scope boundary;
  worth a one-line follow-up whenever that crate is added to CI.
- **`enrichment-worker` and `prices-clickhouse` are still absent from `ci.yml`'s
  clippy list.** Adding them touches `.github/workflows/` and needs a push token
  with the `workflow` scope, which is out of scope here. Noted rather than
  silently omitted — the same footnote [[0125]] carried.

## Operator Checklist

Full procedure with the queries: `docs/runbooks/load-external-usdc-rate.md`.
Everything below is deploy-gated; none of it ran on this branch.

- [ ] 0. **Preconditions** (runbook): on the host, as `default`,
      `prices-clickhouse-init` from THIS branch — re-lands `init.sql` (the
      `quality` ALTER), seeds `backfill_progress`, `CREATE OR REPLACE`s all six
      views; then `system.columns` shows `quality` and both view grains carry
      the widened predicate. Every INSERT names `quality`, so step 4 fails on
      its first chunk without this
- [ ] 1. Build: `cargo build -p enrichment-worker --features aws-mtls --bin load-external-rate`
      (the bin does NOT exist in a default build)
- [ ] 2. Export the four mTLS variables (`CH_DOMAIN`, `MTLS_CERT_PATH`,
      `MTLS_KEY_PATH`, `MTLS_CA_PATH`) and `CH_DATABASE`
- [ ] 3. Dry run against the versioned CSV. ⚠️ **A figure that does not match
      2049 / 1872 / 177 / 24 / 4 / `0.96812` is a STOP, not a note**
- [ ] 4. Shadow load (`--shadow` is the default) — writes
      `method = 'external-candidate'`, which nothing reads
- [ ] 5. Verification query A: 1872 staged rows, 2021-01-25 → 2026-03-11, all
      below the epoch
- [ ] 6. Verification query B: `countIf(timestamp != toStartOfDay(timestamp))`
      is 0 over 1872 rows (0268 resolves at the bucket END with a strict
      `rts < bend`, so a day-END stamp is off by a day and fails nowhere; not
      `toTime()`, which anchors to 1970-01-02)
- [ ] 7. Verification query C: 2023-03-11 reads `0.96812` (trailing zeros
      trimmed), `chainlink`, `measured`
- [ ] 8. Promote. ⚠️ It ADDS a key; the staged rows survive and are inert. Do
      not "clean them up" — leaving them is what makes step 12 free
- [ ] 9. Confirm 1872 rows under EACH of the two method words
- [ ] 10. Confirm `price_usd_series` publishes `0.96812`/`external` for
      2023-03-11 (the schema and views landed in step 0), THEN deploy
      `prices-api` — the new binary reads `usd_rate.quality` directly
- [ ] 11. Run the runbook's 2023-03-11 curl. **This observation is acceptance
      criterion 1.** Then spot-check a `fallback` and a `measured-disputed` day
- [ ] 12. (rollback, if needed) Revert the read path's preference to
      oracle-only and re-apply the views. No row is deleted
- [ ] 13. Close [[0247]] (its first criterion is the load) and re-include USDC
      in [[0127]]/[[0128]]'s spot-check table with the 0.96812 close
- [ ] 14. Then, and only then, [[0268]]'s campaign —
      `docs/runbooks/repair-coarse-usd-values.md` Appendix B, whose precondition
      1 counts exactly the rows step 8 produces
