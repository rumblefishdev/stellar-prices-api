---
title: "R — every site that reads or writes the close_usd zero"
type: research
status: mature
spawns: []
tags: [clickhouse, close-usd, sentinel, adr-0292]
links:
  - "../../../../2-adrs/0292_close-usd-zero-is-a-named-sentinel-with-a-published-reason.md"
  - "../../../../../docs/database-schema/close-usd-zero-guardrails.md"
history:
  - date: "2026-09-17"
    status: mature
    who: akot
    note: "Three read-only audit passes over the 0286 code; A1 and A2 verified by hand and fixed in 0286, the rest carried into the guardrail inventory."
---

# R — every site that reads or writes the close_usd zero

Baseline: branch `fix/0286_…` at `e8874b3` (+ uncommitted 0212 after-check).
Three read-only audits (storage/rollups, enrichment, read surface). Nothing was
built or run. **Verified by hand** = I opened the lines; everything else is from
the audit reports and must be re-checked before it is cited in the ADR.

Meanings of zero: 1 = not yet priced · 2 = never priceable (exotic quote floor)
· 3 = genuinely zero · 4 = no price-forming fill (`close = 0`,
`pf_trade_count = 0`) — new with 0286.

## A. Defects in the 0286 code — fix on the 0286 branch, before the MV re-CREATE

| # | site | defect | status |
|---|---|---|---|
| A1 | `prices-clickhouse/src/rollup_sql.rs:338-339` (×17 renderings) | coarse `close_usd` rate admits a child on `t.close_usd > 0 AND t.close > 0` — no `1e-12` floor, no pf term — while the four price aggregates in the same SELECT carry the floor. `lib.rs:73-81` documents the prod row `close = 5e-14, close_usd = 4e-14` whose ratio 1.25 "looks perfectly ordinary"; it passes. Headers claim ONE floor line "in all three languages". No test (the sub-floor fixture has `close_usd = 0`). | **verified by hand** |
| A2 | `enrichment-worker/src/ch_enrich.rs:525`, `:2475` | 0268 par signature `close_usd = close` has no `close > 0`: a dust-only USDC-quoted row (0 = 0) is re-selected by every reset run. No test. | **verified by hand** |
| A3 | `enrichment-worker/tests/post_run_0228_it.rs:299-300` | the after-check's reference vwap has no pf / `close > 0` filter; after phase 3 it diverges from the write path on any bucket holding dust — and 0286 phase 3 requires it green. | report |
| A4 | `ch_enrich_it.rs` | `pf_trade_count > 0` in the pivot reference is pinned by a string test only (the IT fixture also has `close = 0`); the "peg" row of `pf_columns_survive_every_enrichment_rewrite` is priced by the external tier, so `peg_sql` is never exercised; dust-only rows are tested behaviourally on the oracle tier / 1m only. | report |
| A5 | comments | `rollups.sql:67-68` says the coarse sweep prices a bucket with no priced child — false for an all-dust bucket (`close = 0` stays 0). `init.sql:165` describes meaning 1 only. `usd_sanity.rs:221` and `drift.rs:517` (pre-existing) still describe the pre-0286 rollup / task 0146. | report |

## B. What the ADR (0151) must decide or enumerate

- **No stored state separates meaning 1 from 2.** Inferred only: row age
  (`count_remaining_at_volume_zero`'s recency window), "a pass made no progress"
  (frontier `Exhausted`, month grain), quote-leg identity. Meaning 3 is excluded
  by assumption, not by code.
- **Rows that can never leave the candidate set:** (a) the exotic floor — counted
  every pass, never written; (b) a product that rounds to 0 at 14 dp, or an
  oracle reading of 0 (`oracle_sql` has no `o.price_usd > 0`; its `IS NOT NULL`
  is dead under `join_use_nulls = 0`) — row ends `close > 0, close_usd = 0`, is
  re-inserted at `version + 1` every pass and heads `ORDER BY timestamp LIMIT`;
  (c) frontier oscillation `Exhausted → Pending` on floor months.
- **Coarse `close_usd = 0` with `close > 0`** still conflates 1 and 2;
  `sum(volume_quote_usd)` over partly enriched children looks complete.
- **Published contract, per surface:**
  - `price_usd_series*`, `usd_reference*` — row omitted (true to the header).
  - `/ohlcv` — JSON `null` price fields, `pf_trade_count` tells 4 from 1/2.
  - `current_prices` / `current_price_usd` / `/price` / `/assets` — a literal
    `0` / `"0"` with `method = ''`. `views.sql:973-974` claims the series
    contract; `:224-228` says "sentinels, not NULLs". One of them is wrong.
- **Surfaces disagree on the same row:** views gate on `close_usd > 0`,
  `/ohlcv` on `>= 1e-12` + pf + the par-signature rule. Legacy sub-floor rows
  and par-signature rows are published by the views and refused by `/ohlcv`.
  The only cross-surface test covers USDC.
- **Guardrails that exist and are tested** (to be listed in the ADR): two-term
  pf gate in every rollup/pre-roll rendering; ingest filter + floor
  (`bucket.rs`, `tick.rs`, `soroban.rs`); writer field names == `CANDLE_COLUMNS`;
  explicit `vwap` zero; no bare `argMax(close_usd` in pre-rolls;
  `CANDIDATE_PRED`'s `close > 0`; pivot reference reads `close` with three
  terms; rate legs `> 0`; `/ohlcv` `countIf(valid) = 0 → NULL` wrappers,
  `pf_vwap nullIf`, peg-series NULL tail; `usd_sanity` stranded metric scoped
  to `close > 0`.
- **MV bodies have no behavioural test** — text equality with the generator +
  pre-roll tests only.

## C. Belongs to 0147 (coverage gate), not to the ADR

- `views.sql:400/:689` — weighted mean over the priced subset only, labelled
  `traded`, no coverage signal; `w = volume_base`, not `pf_volume`.
- `views.sql:724` — the 1h `close_usd > 0` filter has no test; "an asset with
  only unpriced rows is absent" is pinned at neither grain.
- `usd_reference` has no pf term while the enrichment pivot has one; a
  dust-only XLM/USDC day becomes `no_reference` ("systemic blackout").
- `current.sql:274-275` — a dust-only minute keeps a venue "live" and its USD
  volume weights `vwap_24h`.
