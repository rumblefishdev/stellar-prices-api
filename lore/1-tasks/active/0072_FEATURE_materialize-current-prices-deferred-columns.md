---
id: "0072"
title: "Materialize current_prices v1-deferred columns (sources breakdown, price_xlm, change_24h_pct) in the MV"
type: FEATURE
status: active
related_adr: []
related_tasks: ["0040", "0039", "0068", "0108", "0117", "0118", "0120", "0123"]
tags: ["phase-future", "effort-medium", "priority-high", "milestone-M2", "vwap", "clickhouse", "materialized-view"]
milestone: 2
links: []
history:
  - date: 2026-06-30
    status: backlog
    who: claude
    note: >
      Spawned from 0040 future work. The prices-api /price endpoint ships in
      0040 with sources={} (and price_xlm / change_24h_pct as zero stubs)
      because mv_current_prices (current.sql, task 0039) writes only the v1
      subset. This task materializes the deferred columns producer-side so the
      API upgrades to pass-through without adding a hot-path query.
  - date: 2026-07-20
    status: backlog
    who: okarcz
    note: >
      **Absorbed task 0068** (duplicate — same four DEFAULT columns, same MV)
      during the 0108 post-M1 grooming sweep. Salvaged from it: the §5.5
      inter-source median-outlier filter on vwap_24h, the "a refreshable MV's
      definition is fixed at create time → DROP VIEW + re-CREATE" redeploy
      gotcha, and the explicit TO(...) column-list footgun from the 0039 review.
      0068 archived. Verified still open: current.sql:52 lists only the six v1
      columns, and price_xlm / change_24h_pct / change_7d_pct / sources remain
      at their table DEFAULTs (current.sql:25-27).
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Tagged `milestone-M2` during the [[0117]] Tranche 2 task-set definition
      and raised to priority-high. This task is the **critical path** of M2:
      overview §9's "Full VWAP formula wired into the Current Price Updater"
      and "Outlier detection" both land here, and §4.1/§4.2's documented
      response shape cannot be honoured while `sources` / `price_xlm` /
      `change_24h_pct` / `change_7d_pct` are DEFAULT stubs. It also gates
      [[0118]] (threshold sits on top of the outlier filter), [[0120]]
      (AC 1 conformance) and [[0123]] (AC 4 reconciliation) — and it is where
      §9's "Aquarius appearing as a named source in VWAP" is actually
      delivered, since `sources` is the only place any source is named.
      Land 0072 before 0118 so `current.sql` is dropped and re-created once,
      not twice.
  - date: 2026-07-23
    status: active
    who: okarcz
    note: >
      Promoted to active as the **first task of Milestone 2** — it is the M2
      critical path per [[0117]]. Four `current_prices` columns
      (`sources`, `price_xlm`, `change_24h_pct`, `change_7d_pct`) sit at their
      table DEFAULTs, so §4.1/§4.2's documented response shape is unmet and no
      price source is named anywhere; §9's "Aquarius appearing as a named
      source in VWAP" is delivered here, not in an extractor. Gates [[0118]]
      (threshold sits on top of the outlier filter), [[0120]] (AC 1
      conformance) and [[0123]] (AC 4 reconciliation).

      Two things to have in hand before touching `current.sql`:
      (1) a refreshable MV's definition is fixed at create time, so this is a
      `DROP VIEW` + re-`CREATE`, not an `ALTER` — `current_prices` keeps
      serving its last-written rows in the gap, so the exposure is staleness,
      not an outage; and (2) the `TO (...)` explicit column-list footgun from
      the 0039 review — an MV inserts **positionally** without it.

      ⚠️ **Sequencing note:** [[0114]]'s coarse repair is running against prod
      CH at promotion time (month 202412 of 30). It writes ~1M RMT rows per
      month into `price_ohlcv_1h` and leaves background merge pressure behind
      it on the SHARED cluster. This MV reads `price_ohlcv_1m` (a different
      table), so there is no correctness interaction — but do not benchmark
      refresh cost, and do not judge merge load, until that run is done.
  - date: 2026-07-29
    status: active
    who: okarcz
    note: >
      **All seven acceptance criteria are met in code; the task stays active
      until the prod rollout is verified.** PR #150 (2026-07-24, `47ad4e1`)
      landed six of them. The seventh — AC 4, the `current_price_usd` read
      surface — was missed by that PR and is closed here: the view still
      exposed only `price_usd` + `updated_at`, and since BE consumes
      `prices.*` views IN-CLUSTER (their 0199 contract — named views, no
      HTTP), `sources` / `price_xlm` / `change_*_pct` / `vwap_24h` were
      unreachable to that consumer no matter what the MV wrote.

      Rollout is now UNBLOCKED: PR #150 deferred the prod apply and the
      refresh-cost measurement until [[0114]]'s coarse repair finished, and
      0114 is complete and archived (`ee97db2`). Steps are written up in
      `docs/runbooks/0072-current-prices-mv-rollout.md` — cost probe, MV
      DROP + re-CREATE, view apply, Compute-stack deploy, per-layer rollback.

      ⚠️ The API deploy is `make deploy-production-compute`, which is exactly
      the deploy [[0132]] avoided by shipping its egress fix through a
      surgical `aws lambda update-function-code`. Running it now also heals
      that CFN drift, and ships every Lambda in the stack from the tree.
  - date: 2026-07-30
    status: active
    who: okarcz
    note: >
      **Rollout started and paused halfway — the new MV is LIVE on ch-prod-01
      and healthy; the read view and the API are not yet rolled.** Steps 0-3
      of the runbook are done: rollback artifacts captured and made
      replay-able, cost probe measured (176-211 ms / 1.69 M rows vs the v1
      MV's 85 ms / 930 k — comfortably inside the 60 s refresh), MV dropped
      and re-created, refresh verified with an empty `exception` and 3,023
      rows written. `sources` (2,278), `price_xlm` (2,257), `change_24h_pct`
      (1,792) and `vwap_24h` (2,278) all populate correctly in production.

      Paused at step 4 because verification surfaced a nine-day production
      incident unrelated to this task: `change_7d_pct` is 0 for every asset
      because `price_ohlcv_1h` — and every other coarse table — has had no
      rows since 2026-07-21. Spawned **[[0136]]**. The remaining
      discriminating test for that incident is a state change on the shared
      production cluster and was deliberately NOT run.

      PR #158 merged (`34ebcbc`) after a `/code-review` that produced six
      findings; five were fixed in-PR (rollback artifacts that could not
      replay, a false-abort `price_xlm == 1` check, the `SELECT *` arity
      claim, the sentinel contract for the seven new columns, and a real
      upgrade-path test with an `IF NOT EXISTS` control). The sixth — view
      DDL needs a `DROP VIEW` grant — went to [[0134]], where measuring the
      prod grants showed the runtime users have no DDL at all and the fix is
      option 2, documentation rather than a BE grant request.

      Also folded the un-enriched-tip `price_usd = 0` defect (21 assets) into
      [[0135]]. Pre-existing, not a regression — v1 used the same unfiltered
      argMax.
---

# Materialize current_prices v1-deferred columns in the MV

## Summary

`prices.mv_current_prices` (the sole writer of `current_prices`, task 0039)
currently populates only the v1 subset
(`price_usd, volume_24h_usd, vwap_24h, market_cap_usd, updated_at`). Four
columns are left at their table DEFAULTs: `price_xlm` (0), `change_24h_pct` (0),
`change_7d_pct` (0), and `sources` ('') — see `current.sql:25-27`. Materialize
the columns the public API needs so `GET /assets/{id}/price` can return them by
pass-through instead of deriving them per request.

## Context

Parent task **0040** (Prices API Gateway + read handlers) ships `/price` with
these fields stubbed (decision 2026-06-30: "D now → A later" — stub in the API,
fill in the MV later) so the load-test endpoint stays a cheap point lookup. The
raw per-source data already exists in `price_ohlcv_1m` (it carries a `source`
column and per-source rows). The natural home for the per-source JSON breakdown
and the XLM-quote / reference-close derivations is the once-per-minute MV, NOT a
per-request API query (which would add a 24h scan + GROUP BY to the hottest
endpoint and undermine the p95<200ms SLO).

## Implementation

- **`sources` (JSON String)**: per-source breakdown over the trailing 24h —
  for each `source`, the latest close (`argMax(close_usd, timestamp)`) and the
  summed volume. Assemble into a JSON object string matching the §4.2 shape
  (`{ "sdex": { "price": ..., "volume_24h": ... }, ... }`). Build inside CH
  (e.g. `toJSONString(map(...))` from a per-source `groupArray` of tuples) so
  the MV stays the single writer.
- **`price_xlm`**: the XLM-quote orientation of the current price (current.sql
  comment §"follow-ups").
- **`change_24h_pct`** (and optionally **`change_7d_pct`**): the 24h/7d
  reference-close self-join.
- **`vwap_24h` refinement** (salvaged from 0068): apply the general-overview §5.5
  inter-source **median-outlier filter** on top of the existing volume-weighted
  mean, so one divergent venue cannot drag the published VWAP.
- **Redeploy mechanics** (salvaged from 0068): a refreshable MV's definition is
  **fixed at create time**, so landing any of this is `DROP VIEW` + re-`CREATE`,
  not `ALTER`. No backfill or migration is needed either way — the MV fully
  recomputes every row on each `REFRESH EVERY 1 MINUTE`, so the first refresh
  after the change populates the new columns for all assets.
- **Positional-insert footgun** (from the 0039 review): every new column must be
  added to the explicit `TO prices.current_prices (...)` column list.
- Decide: extend `mv_current_prices`'s SELECT + target column list in-place, or
  add a companion MV — weigh refresh cost vs. keeping a single ReplacingMergeTree
  row per asset (the supply worker already uses a separate table to avoid row
  contention; mirror that reasoning if the JSON build makes the 1-min refresh
  too heavy).
- Update `prices.current_price_usd` view (and any read surface) to forward the
  newly-populated columns.
- **0040 upgrade**: once columns are live, switch the `/price` handler from the
  `sources: {}` stub to pass-through.

## Acceptance Criteria

- [x] `mv_current_prices` (or companion) writes `sources` as a valid JSON string
      matching the §4.2 `/price` `sources` shape. *(PR #150)*
- [~] `price_xlm` and `change_24h_pct` are populated (non-DEFAULT) for assets
      with sufficient data; `change_7d_pct` decided — **populated**, read from
      `price_ohlcv_1h` rather than `_1m`. *(PR #150)* — **`price_xlm` (2,257) and
      `change_24h_pct` (1,792) verified live 2026-07-30. `change_7d_pct` is 0 for
      all 3,023 assets: the column is correct, but `price_ohlcv_1h` has no rows
      inside the 7-day window — it froze on 2026-07-21. Blocked on [[0136]], not
      re-openable here.*
- [x] Integration test vs prod-pinned CH 26.3.10.60 asserts a seeded multi-source
      asset yields the expected per-source breakdown + scalar fields. *(PR #150,
      `current_prices_mv_writes_0072_columns_and_filters_outliers`)*
- [x] `current_price_usd` view (or read surface) exposes the new columns.
      *(2026-07-29 — missed by PR #150, see Design Decisions)*
- [x] 0040 `/price` handler switched to pass-through; `sources` stub removed.
      *(PR #150 — also `/prices/batch` and `/assets`)*
- [x] `vwap_24h` applies the §5.5 inter-source median-outlier filter (from 0068).
      *(PR #150, `OUTLIER_PCT = 0.20`, arms only at >= 3 sources)*
- [x] All new columns present in the explicit `TO (...)` column list;
      `current_mv_it.rs` extended to assert each (from 0068). *(PR #150)*

**Not an AC, but gates completion:** the prod rollout below is unverified. Do not
archive this task until step 7 of the runbook passes.

## Rollout status

**Halfway. The new MV is LIVE on ch-prod-01 and healthy; the read view and the
API are not yet rolled.** Paused 2026-07-30 after step 3 — not because anything
failed, but because verification surfaced [[0136]] (all six coarse OHLCV tables
frozen since 07-21) and the session was stopped rather than take any further risk
on the shared production cluster. Runbook:
`docs/runbooks/0072-current-prices-mv-rollout.md`.

| Step | State |
|------|-------|
| 0 — capture + repair rollback artifacts | ✅ 2026-07-30 |
| 1 — cost probe on ch-prod-01 (read-only) | ✅ 2026-07-30 |
| 2 — `current.sql` DROP + re-CREATE | ✅ 2026-07-30 |
| 3 — verify the MV | ✅ 2026-07-30 |
| 4 — `views.sql` apply (`current_price_usd`) | ⏳ pending |
| 5 — verify the read surface | ⏳ pending |
| 6 — `make deploy-production-compute` | ⏳ pending |
| 7 — `/price` returns a populated `sources` | ⏳ pending |

**This is a stable state to sit in indefinitely.** The new MV writes all ten
columns; the old six-column `current_price_usd` keeps serving exactly what it
served before. No consumer sees a change until step 4, and the API keeps
serving its stubs until step 6.

**Rollback artifacts** are at `/tmp/0072-rollback-mv_current_prices.sql` and
`/tmp/0072-rollback-current_price_usd.sql` on the operator's machine, both
verified replay-able (they are in `/tmp` — re-capture per step 0 if the machine
has rebooted).

### Measured cost (step 1, ch-prod-01, 2026-07-30)

| | v1 MV | 0072 MV |
|---|---|---|
| refresh duration | 85 ms | **176–211 ms** |
| rows read | 929,852 | **1.69 M** |
| peak memory | — | 153 MiB |
| rows written | 3,024 | **3,023** |

Comfortably inside the runbook's `< 15s` accept band and inside the 60s refresh
interval — no cadence follow-up needed. The ~2× cost is the extra 7-day
`price_ohlcv_1h FINAL` scan for `change_7d_pct`. Write volume is unchanged from
v1, so the MV neither caused nor aggravates [[0136]].

### Step 3 verification (2026-07-30)

```
exception: (empty)        status: Scheduled        last_success_duration_ms: 211
new_cols on mv_current_prices: 4          assets: 3,023
with_sources 2,278 | with_price_xlm 2,257 | with_change_24h 1,792
with_change_7d 0  <- blocked on 0136      | with_vwap 2,278
```

`sources` verified as a JSON object keyed by venue with string-serialised
decimals, e.g.
`{"sdex":{"price":"1.0004648975275","volume_24h":"42025900.56744495766908"}}`.

## Design Decisions

### Emerged

1. **`current_price_usd` moves to `CREATE OR REPLACE`, not `CREATE … IF NOT
   EXISTS`.** The rest of `views.sql` uses `IF NOT EXISTS`, which does **not**
   redefine a view that already exists — against a target already holding the v1
   shape the apply silently no-ops and the new columns never land. Verified on
   local CH 26.3.10.60 against a hand-built v1 view: the `IF NOT EXISTS` form
   left it at 6 columns, `OR REPLACE` took it to 13. A plain view replaces
   atomically, so this needs no DROP window (unlike the refreshable MV).

2. **New view columns are appended, not inserted.** The six columns that shipped
   keep their positions, so a `SELECT *` consumer is not re-ordered underneath
   itself — hence `updated_at` sitting mid-list rather than last.

3. **The other five views in `views.sql` were left on `IF NOT EXISTS`.** They
   carry the same latent footgun (any future edit to them silently won't land on
   a target that already has them), but converting them is beyond this task. See
   Future Work.

4. **Paused mid-rollout after step 3 rather than pressing on.** Steps 4–5 are
   cheap and low-risk, but step 3's verification surfaced [[0136]], and the
   operator's standing rule is that production is not a place to take risks.
   The half-rolled state is coherent and indefinitely stable (new MV + old view),
   so there was no cost to stopping. Recorded because the obvious alternative —
   "finish the two cheap steps while we're here" — was considered and rejected.

## Issues Encountered

- **`change_7d_pct` is 0 for every asset in production.** Not a defect in this
  task: the column reads `price_ohlcv_1h` and that table has had no rows since
  2026-07-21. Diagnosed to a nine-day freeze of all six coarse OHLCV tables →
  spawned **[[0136]]**. The AC is marked `[~]` rather than `[x]`; it cannot be
  closed here.

- **777 of 3,022 assets carry `price_usd = 0`, and 21 of those have a working
  `vwap_24h`.** The 756 with neither are genuinely unpriced and correctly report
  the 0 sentinel. The 21 are a real defect: `unfiltered.price_usd` is
  `argMax(close_usd, timestamp)` with **no `close_usd > 0` filter**
  (`current.sql:203`), while the per-source path filters `src_price > 0`, so an
  un-enriched newest candle zeroes the headline price while `sources`/`vwap_24h`
  demonstrably know it. Observed live on `asset_id 4`
  (`price_usd 0`, `vwap_24h 0.17281880272617`). **Pre-existing, not a
  regression** — the v1 MV used the identical unfiltered `argMax`; 0072 only made
  it visible by putting a real VWAP beside the zero. Folded into **[[0135]]**,
  which already owns `price_usd`'s correctness.

- **The same asymmetry falsified a runbook check.** Step 5 asserted XLM's
  `price_xlm` must be exactly `1`; an un-enriched XLM tip makes it 0. Softened to
  a diagnostic during the PR #158 review before it could trigger a false abort
  mid-rollout.

## Future Work

- Convert the remaining five `views.sql` views to `CREATE OR REPLACE` so schema
  edits actually land on an already-provisioned target — a latent silent-no-op
  footgun, and a plausible contributor to the kind of drift [[0076]] had to
  reconcile by hand. → **[[0134]]**
- `price_usd` is still not outlier-filtered (flagged in PR #150): the headline
  price can come from a manipulated venue while `vwap_24h` is protected, and it
  propagates into `market_cap_usd`. §7 scopes outlier detection to the VWAP, so
  this is a deliberate gap needing its own decision. **Plus the un-enriched-tip
  zero measured above — 21 assets publishing `price_usd = 0` while `vwap_24h`
  knows the price.** → **[[0135]]**
- Every coarse OHLCV table (`_15m` through `_1M`) has been frozen since
  2026-07-21, which is why `change_7d_pct` cannot populate. Found by this task's
  step-3 verification; far wider than this task. → **[[0136]]**
