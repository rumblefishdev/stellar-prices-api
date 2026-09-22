---
id: "0286"
title: "Candles take every fill at equal weight, including stroop-dust, in the wrong intra-ledger order — rebuild OHLC from price-forming fills with a windowed close, then re-ingest the history"
type: BUG
status: active
related_adr: ["0287"]
related_tasks: ["0278", "0276", "0266", "0228", "0146", "0142", "0137", "0200", "0088", "0282", "0285"]
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
  - date: "2026-09-21"
    status: active
    who: okarcz
    note: >
      Phase-3 operator decisions, recorded from the operator's session and
      a teammate's M3 analysis (see the "Phase 3 — operator decisions" section).
      (1) ORDER: oldest-first from ledger 1, as the runbook says — the
      "Soroban era first" alternative was weighed and dropped. (2) ACCESS:
      the admin identity the re-ingest needs is the new prices.*-scoped CH
      user `prices_admin` (BE tasks 0567/0568, PRs #468/#469, live on prod
      2026-09-21), NOT dev_shared; its cert plus prices_writer's are on the
      campaign machine. (3) Precondition 9 (0285's reverse question) is
      answered: live writes nothing for unregistered pools. (4) Phase-1
      rollout conditions are green (0282 verified 09-19/09-20, #320 merged,
      55/55 alarms OK); who runs it and when is tomorrow's daily.
  - date: "2026-09-22"
    status: active
    who: okarcz
    note: >
      Phase-1 rollout scheduled for TODAY, run end to end by the operator,
      who also runs phases 2 and 3 (no hand-off). All five phase-1
      preconditions verified on production this morning and recorded in the
      "Phase 1 — preconditions verified" section: the drift baseline is
      CLEAN (six `ok`, APPEND intact, no undeclared writer), the snapshot
      costs a measured 26.7 GiB against 276 GiB free, and the running
      ledger-processor is confirmed PRE-#320 — the 09-18 Compute deploy
      shipped stale assets, so the estate is still internally consistent.
      Two corrections fall out: since #325 the next Compute deploy really
      rebuilds, so the schema step is what keeps ingest alive rather than
      mere ordering hygiene; and prices-api cannot ship in the middle step
      because it shares the Compute stack with the ingest (runbook fixed,
      PR #333).
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
  partition-boundary minutes; AMM sources ≥ with the delta accounted for per
  source and month; and, as a SEPARATE check, the days written live between
  2026-07-16 and the #313 deploy (2026-09-17 12:04 UTC) come back HIGHER
  than the snapshot by [[0282]]'s measured shortfall — there the snapshot is
  the damaged side, so "equal" would itself be a finding. The Soroban AMM history enters through
  `events-backfill` (pool registry seeded first — [[0088]]), with the same
  D1–D3 rules; BE's `default.transactions` coverage of the range is a
  precondition, the fallback share recorded.
- Preconditions: phases 1–2 live and measured on new data; cleanup off for
  the whole run ([[0200]]); disk budgeted for the snapshots; [[0282]]'s
  fix (PR #313) deployed and checked over a full day, because a later
  dust-only write of a minute now replaces a priced one; and, for the AMM
  side of the live-era months only, [[0285]]'s reverse question answered
  (does the live path write candles for pools absent from `pool_registry`?
  `events-backfill` could not put those back after the `DROP PARTITION`) —
  SDEX and the pre-live AMM months do not wait for it.
- **Cost not in the ledger estimate:** re-ingested rows carry
  `close_usd = 0`, so the enrichment worker re-prices the entire history
  afterwards (0268's campaign re-priced 9.94 M candles in ~24 min; 0228's,
  never run, was estimated at ~190 M candles and ~4 h 40 m; this is all of
  them). That same fact makes [[0228]]'s reset-mode campaign (Appendix C)
  **unnecessary**: the refill happens inside the re-enrichment; only its
  after-check (`post_run_0228_it`) still runs, on the repaired XLM/USDC
  reference.

### Phase 1 — preconditions verified, rollout 2026-09-22

Measured on production the morning of the rollout. **The operator runs phase 1
and every later phase**; the only external dependency left is Adam pushing the
phase-3 script.

| # | Precondition | State |
| --- | --- | --- |
| 1 | 0282's fix deployed, full-day check recorded | ✅ 09-19 and 09-20 at exactly zero |
| 2 | S2 branch merged | ✅ #320 in `develop` |
| 3 | Drift binary green before the rollout | ✅ 2026-09-22 10:20 UTC, six `ok` |
| 4 | 0142 / 0137 alarms known-green | ✅ `mv-drift` OK; APPEND intact on all six |
| 5 | Disk headroom for the snapshots | ✅ 26.7 GiB needed, 276 GiB free |

**The drift baseline, and why it needed a pre-#320 binary.** `ROLLUPS_SQL` is
`include_str!`'d, so the tool compares the live views against whichever
`rollups.sql` its build carried. Built from `develop` it would report DRIFT on
all six *before the rollout starts*, correctly and uselessly. The baseline was
therefore taken with a binary built from `d11d20ab^1` (the commit before #320
merged), cross-compiled `x86_64-unknown-linux-musl` (`static-pie linked` — the
host's glibc is far older than a current laptop's), `scp`'d to ch-prod-01 and
run there against `localhost:8123` as `default`, password over ssh stdin. There
is no path from a laptop: prod's HTTP endpoint is mTLS-only behind Caddy and
`client()` builds a plaintext client. Result: six `ok`, *"6 rollup MVs in sync
with rollups.sql, and nothing else in prices writes into their targets"*. No
`CRITICAL`, which is how APPEND mode is reported, so precondition 4 is settled
by direct measurement rather than by reading the alarm.

⚠️ `Config::from_env()` defaults to `localhost:8123`, so the same command on a
machine with a local ClickHouse running checks **dev** and exits 0 looking
identical to a clean prod result. Read the startup `url=` line before the
verdict.

**Measured sizes** (2026-09-22 10:00 UTC, `chq`): `1m` 20.05 GiB / 783 M rows;
coarse tiers `1h` 11.32, `15m` 7.38, `4h` 5.41, `1d` 1.91, `1w` 0.49, `1M` 0.20
GiB — **26.7 GiB** for the step-4 snapshot of all six, against 276.35 GiB free
(15.7 %, the `ch-disk-free` alarm firing since 09-21 18:21 UTC on a threshold of
20 %). The alarm is a percentage on a disk shared with BE, who lost ~175 GiB
between 09-21 09:36 and 15:36 and are handling it; the absolute headroom is
ample and phase 1 does not wait for it.

**Incidental finding:** six stale `price_ohlcv_*_bak` tables from **2026-07-17**
(newest row 13:00 that day) hold **15.6 GiB**. Owner and campaign not
identified; if that campaign is closed they are free space. Step 4's snapshots
use the distinct prefix `rollout_0286_bak_*` so the two schemes cannot be
confused during a rollback.

**The running ingest is PRE-#320, and that is what kept the estate consistent.**
The live `reconcile run complete` line carries `held_back` and `open_minute`
(#313) but not `order_book_fills` / `offer_lookup_misses` / `pool_fills`
(#320), so the 2026-09-18 Compute deploy shipped stale Lambda assets — [[0141]]
exactly. Two consequences:

- **#325 (merged 09-21) makes the next Compute deploy rebuild for real.** No
  production component applies `INIT_SQL` (every call site is a test), so the pf
  columns do not exist on prod; a Compute deploy before the schema step would
  put the name-routed writer in front of columns that are not there and stop
  ingestion. The schema step is a hard gate on ingest liveness, not ordering
  hygiene.
- **`order_book_fills` on that log line is the cleanest proof step 7 landed.**

**`prices-api` moves to the end of the order.** `api-handler` and
`ledger-processor` are one CDK stack, so `make deploy-production-compute` ships
both or neither and the runbook's middle step could not contain the API without
also shipping the ingest into pre-0286 MVs. Safe because the two mis-orderings
are not symmetric: a new ingest too early writes a zero `low` across every
coarse tier and must be rolled back, while a new API too late only prolongs the
behaviour production already has. Runbook corrected in PR #333; `init.sql` and
the `rollups.sql` header keep their wording, whose constraint — ingest last — is
unchanged.

**Rollback amendment.** The runbook's `FREEZE` + `ATTACH PARTITION FROM '/path/'`
is a syntax error (Code 62) and `prices_admin` has no filesystem access; step 4
uses backup tables plus `REPLACE PARTITION` instead.

### Phase 3 — operator decisions (2026-09-21)

Recorded by the operator after a teammate's M3 analysis (Slack, 2026-09-21)
and the operator's own session the same day. These refine the phase-3
section above without changing its rules.

- **Order: oldest-first, from ledger 1** — as the runbook says. The
  "Soroban era first" alternative (~14 M ledgers, fixes the 0282 gap and
  every AMM candle first) was weighed and dropped. Reason: the pre-Soroban
  history is ~79 % of the work and SDEX-only, so it never touches
  `pool_registry`; running it first gives ~2 weeks of buffer in which
  [[0290]]'s 133 SushiSwap pools get written and `--discover-pools` is run to
  the current tip — BEFORE the loop reaches 2024-02. Done the other way
  round, ~31 Soroban-era months would be rebuilt a second time once the
  registry is complete. If the registry is still incomplete when the loop
  reaches 2024-02, the AMM side of those months is deferred consciously,
  not silently.
- **Re-ingest is not an M3 criterion.** None of the 9 M3 ACs judges candle
  correctness, so the M3 claim does not wait for it; the evidence pack
  declares 0282/0286 as a known issue ("fix merged/deployed, history repair
  in progress").
- **Access: the admin identity is `prices_admin`, not `dev_shared`.** The
  runbook's precondition 7 ("snapshots by the CH admin, `prices_writer`
  cannot") is met by a new prices.*-scoped XML user on the shared cluster —
  BE tasks 0567 + 0568 (PRs #468, #469), live on prod 2026-09-21, applied in
  place (inode kept, hot-reload, no restart). Grants: SELECT/INSERT/ALTER/
  CREATE TABLE/DROP TABLE/TRUNCATE on `prices.*`; SELECT on `default.*`
  (the AMM re-ingest reads BE's events) and on `system.parts`,
  `system.mutations`, `system.columns`, `system.disks`. Cert CN
  `prices-admin-production`, operator-held, revocable by one CN-map line.
  The campaign machine (`fishuser-hero`) holds exactly three bundles:
  `prices-admin-production`, `prices_writer`, `ca.crt` — all three verified
  live from there (currentUser, CREATE/TRUNCATE/DROP of a probe table in
  `prices`, `system.disks` read). The phase-3 script therefore runs with
  `--admin-cert` AND `--reader-cert` = `~/prices-mtls/prices-admin-production`,
  `--writer-cert ~/prices-mtls/prices_writer`, `--ca ~/prices-mtls/ca.crt`;
  no `dev_read` cert is needed there (its 30 s / 4 GB profile would not fit
  the month-wide FINAL sums anyway). Also live: `prices_writer` reads
  `default.soroban_events` / `default.soroban_contracts` (BE 0569, PR #470)
  for the weekly coverage sweep.
- **Precondition 9 is answered** (runbook §1, the [[0285]] reverse
  question): the live path writes nothing for pools absent from
  `pool_registry` (operator's 2026-09-21 measurement), so `DROP PARTITION`
  deletes nothing `events-backfill` cannot rebuild. The runbook can drop the
  precondition; the script's `--ack-0285` flag records the same fact.
- **Before the first month, on `fishuser-hero`:** `git pull` and rebuild
  `sdex-backfill` (and `events-backfill`) from a develop that contains #320
  — the machine keeps its own checkout, and an old binary would rebuild the
  history under the OLD definition without a single error.
- **Phase-1 rollout conditions are green** as of today: [[0282]] verified
  (09-19 / 09-20 exactly zero loss), #320 merged, 55/55
  `prices-production-*` alarms OK. Not yet checked: the drift binary and
  disk headroom on the CH host. Until the schema step has passed, Compute
  is NOT deployed from develop (the 09-18 incident); queued behind it:
  [[0291]] AC 2–3, #324 ([[0290]]), #331 (0256).
- **API impact is unmeasured.** Compute runs on `fishuser-hero`, but the CH
  box takes the inserts, the per-month pre-rolls, `events-backfill` and the
  final re-enrichment. The July backfill saw 1–4 API req/day, so there is no
  signal yet. Proposal: one CloudWatch read of `IntegrationLatency` p95 at
  the first pre-roll.
- **M3 review touchpoints:** the phase-1 rollout day lights `mv-drift` by
  design — not on a review or video-recording day (describe it in the 7-day
  report if it falls in the window); the end of the re-ingest (TRUNCATE
  1w/1M, live-era month drops) also not during a review.
- **Open for the daily (2026-09-22):** who runs the phase-1 rollout and
  when; who starts the re-ingest on `fishuser-hero` afterwards. Also
  0296's "start" date and 7-day window (the only clean window before the
  load tests is 09-10 → 09-16; 1-min metrics for 09-04 → 09-20 get exported
  before 09-25 regardless).

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
      SDEX (boundary minutes documented), ≥ for AMM sources; the days
      written live between 2026-07-16 and 2026-09-17 12:04 UTC checked
      separately and HIGHER by [[0282]]'s measured shortfall, expected vs got
      logged per source; differences otherwise only in OHLC.
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
- [ ] No USDT-quoted `price_ohlcv_1m` row carries the $1 peg after the
      re-enrichment ([[0212]]'s query: `peg_written = 0`, `pivot_written > 0`,
      measured on 1m and on one coarse tier) — the re-ingest replaces the
      1.56 M rows 0172/0182 never reached, so 0212 closes here (re-ingest
      runbook §7e).

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
14. **The coarse `close_usd` rate is held to the precision floor on BOTH legs**
    (`rollup_sql::RATE_BEARING_CHILD`, found by [[0151]]'s audit, 2026-09-17).
    The gate was `close_usd > 0 AND close > 0`; the prod row
    `close = 5e-14, close_usd = 4e-14` that `PRICE_FLOOR_SQL`'s own doc quotes
    passed it, and as the latest "priced" child re-priced a healthy parent
    close by 0.8 on every tier above. Now `>= 1e-12` on both, the line
    `/ohlcv`'s `convertible` already draws. Changes all six MV bodies — lands
    in the same re-CREATE.
15. **The 0268 par signature needs a price**: `close_usd = close AND close > 0`
    at both sites. A dust-only candle is `0 = 0` before and after every
    refill, so the campaign re-opened it on every run and its pending count
    never drained. Its volume stays as the tier that priced it left it — dust
    volume, deliberately not worth a second signature.
16. **The live offer-lookup tally follows the candles** (`a697c9c`, after the
    merge of #313). Runs now re-decode the ledgers of the open minute, so the
    process-wide counter diffed across a run counted held-back fills twice;
    it is kept per ledger and summed over the flushed minutes, the rule
    `flush_older_than` uses, and carried on `RunStats`.
17. **#313's forced partial flush is an accepted, alarmed erase path.** A run
    that spends its whole `maxIterations` budget inside one minute flushes it
    partial; under ADR 0287 a dust-only remainder then erases that minute's
    price. Needs a sub-2 s close time (12 ledgers/minute measured, budget 32);
    listed in rollout runbook §1a with its alarm,
    `prices-production-ledger-processor-forced-partial-flush`.
18. **[[0285]] gates only the live-era AMM months of phase 3**, not the
    phase: every other month's old rows were written under the registry rule
    the re-ingest applies.

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
- [[0282]] was fixed by PR #313 (merged `2cb5b2b`, deployed 2026-09-17
  12:04 UTC; its full-day raw-vs-stored check is owed on that task). The fix
  does NOT sum writes — a second write of a minute still replaces the first —
  it ends every reconcile run on a whole minute and re-reads the held-back
  ledgers, so the live processor writes each minute once. Under the new
  definition a dust-only second write erases a priced minute, so the ingest
  is still deployed only after that check is recorded.
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
