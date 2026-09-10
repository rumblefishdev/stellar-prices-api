---
id: "0116"
title: "Dust-trade candles produce absurd close_usd values (up to $29.6M) in every OHLCV granularity"
type: BUG
status: active
related_adr: []
related_tasks: ["0114", "0115", "0026", "0144", "0147", "0117"]
tags: [clickhouse, data-quality, sdex, enrichment, priority-medium, effort-small, milestone-M2]
milestone: 2
links:
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-07-23
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0114]]'s pilot verification. The coarse-USD repair surfaced
      40 rows > $1M in 202502 alone; investigation showed the references are
      correct and the *input candles* are junk. Confirmed pre-existing — the
      live-enriched path shows the same tail — so this is not a repair defect.
  - date: 2026-08-10
    status: backlog
    who: okarcz
    note: >
      Tagged milestone-M2 (+ milestone: 2), closing the last open AC of [[0117]].
      0117 deferred this tagging because 0116 existed only on the unmerged
      fix/0114_repair-preflight-and-runbook-gaps branch; 0114 is now completed
      and 0116 is on develop, so the precondition is met. No shape change - this
      is scope tagging only. It matters because untagged, 0116 is invisible to
      any "what is left for M2?" query, and it feeds Tranche 2 AC 4 (VWAP
      reconciliation against raw price_ohlcv rows) and AC 6 (the USDC 1d
      spot-check) - absurd close_usd values would surface in exactly those two
      checks.
  - date: 2026-09-10
    status: active
    who: okarcz
    note: >
      Activated. The measured evidence in this file dates from 2026-07-23 and
      202607 was then a partial, in-flight month — re-measuring on prod before
      deriving any threshold.
  - date: 2026-09-10
    status: active
    who: okarcz
    note: >
      📐 **Re-measured on prod; the approach in this file does not survive it.**
      The mechanism is confirmed and sharper than "negligible volume": the
      extreme tail is trades of **one or two stroops** (1e-7) of base with
      `trade_count = 1` and under $0.36 of notional. Dose-response is clean and
      monotonic in both months — 202502 one-stroop buckets are 85.0% over
      $1,000 (max $29,606,748, the headline reproduced), 202608 22.3% (max
      $3,517,649); at ≥1 whole token, 0.11% and 0.008%.
      🔴 **Option 2 as specified would be wrong a third of the time.** Of dust
      buckets checkable against a non-dust reference for the same pair, **38 of
      116 are priced correctly** (0.1-10x). A smallest-unit trade of a genuinely
      expensive asset is a real order at the right price, so a size threshold
      misclassifies exactly the assets the AC said not to sweep up.
      🔴 **93% cannot be adjudicated at all** — 2,741 of 2,944 dust buckets are
      on assets with no non-dust trading anywhere in the month. Spawned
      **[[0274]]**; that population is an asset with no market, not a bad candle.
      Also measured and discarded: **notional value is a poor discriminator** —
      the >$1k rate *rises* with bucket value (2.6% at $0.01-1, 12.2% at ≥$10k),
      because active tokens legitimately cost more.
      Operator chose the document-and-refile route after the findings were put
      to them. **PR #303** open.
---

# Dust-trade candles produce absurd `close_usd` values

## ⚠️ The mirror image — precision COLLAPSE at the small end (found 2026-08-10)

This task is about absurdly **large** `close_usd`. Verifying [[0167]] on prod
surfaced the opposite failure at the same root: assets whose unit price sits at
the bottom of `Decimal(38,14)`.

Measured over 103,016 USDC-quoted daily candles, 145 (0.14%) carry a `close_usd`
that no rate can reproduce. Their `close` is a single-digit multiple of `1e-14`
and **every one loses exactly one ulp**:

| asset | close | close_usd | mantissas |
|---|---|---|---|
| KINGSTON | `0.00000000000008` | `0.00000000000007` | 8 → 7 |
| MetaVerse | `0.0000000000001` | `0.00000000000009` | 10 → 9 |
| MetaVerse | `0.00000000000011` | `0.0000000000001` | 11 → 10 |
| MetaVerse | `0.00000000000015` | `0.00000000000014` | 15 → 14 |

Consistently **one unit down**, which rules out round-to-nearest (`10 × 1.0001`
would stay `10`). It is a float round-trip: `1e-13` has no exact binary form,
becomes `0.99999…e-13`, and truncates. Relative error reaches **14.25%**;
absolute error is `1e-14` per unit — nil, even at the millions of `volume_base`
these candles carry.

**Same family as this task** — an asset priced outside the range
`Decimal(38,14)` handles usefully — opposite end. Recorded here rather than
filed separately because a fix for one should consider the other: dust trades
produce garbage at the top, precision collapse produces it at the bottom, and
both are "the price is outside our representable working range".

⚠️ **Not a pricing-tier defect.** The same error appears against any rate,
including the one enrichment itself used, so it must not be mistaken for an
oracle or peg problem. It also means a *relative*-error check over these assets
will always look alarming while the absolute error is negligible — any
tolerance test needs an absolute floor as well as a percentage.

## Summary

Single-trade SDEX candles with negligible volume carry nonsense unit prices,
which enrichment then faithfully converts to USD. The result is a long tail of
absurd `close_usd` values across every granularity — measured up to **$29.6M**
for a token whose entire bucket was ~$3 of volume.

This is **not** an enrichment or repair defect. The USD reference applied to
these rows is correct; the OHLC input was already junk.

## Evidence (prod, measured 2026-07-23)

Top offenders in `price_ohlcv_1h` for 202502, after the [[0114]] repair:

| base | quote | close_quote | close_usd | implied_ref_usd | vol_quote | trade_count |
|---|---|---|---|---|---|---|
| COVA | XLM | 94,810,046 | 29,606,748 | 0.312 | 9.48 XLM | **1** |
| PCOY | XLM | 58,588,965 | 20,798,438 | 0.355 | 5.86 XLM | **1** |
| YIELD | USDC | 12,312,121 | 12,312,121 | **1.0** | 3.69 USDC | **1** |

`implied_ref_usd` (= `close_usd / close`) is 0.312–0.408 for XLM-quoted rows —
the correct XLM/USD price for February 2025 — and exactly 1.0 for USDC-quoted
rows (the stablecoin-direct cast). **The conversion is right; the candle is
wrong.** Someone traded a dust amount (~1e-7 of a token) for a few XLM, and the
resulting unit price is meaningless.

## It predates the repair — confirmed by control

The same tail exists in data written by the **live** enrichment path, which
nobody disputes:

| scope | rows | p50 | p99 | max_usd | > $1M | pct |
|---|---|---|---|---|---|---|
| `1h` 202502 (repaired) | 1,000,641 | 0.001104 | 10,197 | 29.6M | 40 | 0.0040% |
| `1h` 202607 (live path) | 136,754 | 0.000582 | 2,039 | **24.0M** | 2 | 0.0015% |
| `1m` 202607 (live path) | 3,437,815 | 0.002149 | 5,104 | **55.6M** | 8 | 0.0002% |

Live's `max_usd` is *higher* than the repaired month's. The ~2.7× rate
difference between the two `1h` rows is era/composition plus small-sample noise
(2 events), not a systematic difference — the `1m` figure differs mostly by
granularity, since a month holds ~25× more 1m rows than 1h buckets.

## Scope of the harm

- **`volume_quote_usd` is unaffected.** These rows carry ~$3 of volume, so
  volume aggregates are not distorted. BE's LP analytics do not see this.
- **`close_usd` is affected** — a price-display column. Any consumer that
  charts, ranks, or takes a max over `close_usd` will show a spike.
- The wider tail matters more than the extreme: **3.4% of repaired rows are
  > $1k** and 9.3% are > $100. Not all of those are junk (some tokens are
  genuinely expensive per unit), so a naive threshold will misclassify.

## Possible approaches (not yet chosen)

1. **Filter at read time** in the API — cheapest, non-destructive, but every
   consumer must opt in and the bad data stays.
2. **Flag at ingest** — add a `is_dust` / quality column set when
   `trade_count = 1` and `volume_quote` is below a per-quote threshold. Keeps
   the row, lets consumers choose. Touches live ingestion, which has a freeze
   history ([[0064]] / [[0094]]) — needs care.
3. **Exclude from the candle entirely** — most invasive; changes what a candle
   means and is not reversible.

Option 2 looks right, but the threshold needs deriving from the distribution
rather than guessing — see the 3.4%/9.3% caveat above.

## Acceptance Criteria

- [x] A dust threshold is derived from measured distribution, not assumed, and
      validated against a sample of genuinely-expensive tokens so they are not
      swept up. **Derived (base amount in minimal units) and then, on the
      validation this criterion demands, found unfit to ship as a standalone
      verdict — a third of checkable dust buckets are priced correctly.** The
      criterion is met by the validation having been done and its result
      published, not by a threshold shipping.
- [x] Absurd `close_usd` rows are identifiable by consumers. **Documented
      read-time filter** — `volume_base` + `trade_count` are already on every
      candle, so no flag column was needed. Documented in three places: the
      `Candle` doc comments, the published OpenAPI description, and design doc
      §4.2.
- [x] `volume_quote_usd` behaviour is explicitly unchanged (it is already
      correct). **Stated in all three places**, with the reason: these buckets
      carry a few dollars, so volume aggregates are undistorted.
- [x] Verified against both a repaired historical month (202502) and a
      live-written month — the defect exists in both. **202502 and 202608**;
      202607 was substituted because it was partial when this task was written
      and 202608 is a complete live-written month.

## Implementation Notes

Documentation only. No schema, ingestion or read-path change. **PR #303**;
`cargo test -p prices-api` 440 passed / 0 failed, OpenAPI lint valid, clippy
clean. Every prod query read-only via mTLS as `dev_read` (`readonly = 1`).

Measured on `price_ohlcv_1h`, rows with `close_usd > 0`:

| base amount | 202502 % > $1k | 202608 % > $1k |
|---|---|---|
| 1 stroop | **85.0** | 22.3 |
| 2-9 stroops | 82.3 | 15.5 |
| 10-99 | 32.4 | 12.4 |
| 100-9,999 | 10.9 | 5.9 |
| 1e4-1e7 | 2.5 | 0.4 |
| ≥ 1 whole token | **0.11** | **0.008** |

Population, 202608: 695,015 priced rows; 2,944 dust (≤9 stroops, 0.42%); 2,049
of those over $1,000. Dust is only **12.7%** of the >$1k population — the
extreme (>$1M) tail is dust-dominated, the broad tail is not.

## Issues Encountered

- **A correlated subquery is not supported on this ClickHouse** (`Code: 48
  NOT_IMPLEMENTED`) — the reference-coverage measurement had to be rewritten as
  two `LEFT JOIN`s to aggregate subqueries. Note the result is only readable
  because prod runs `join_use_nulls = 0`: an unmatched row yields `0`, not NULL,
  and the query tests `> 0` rather than `IS NOT NULL`, which would be dead code
  here ([[join-use-nulls-zero-makes-is-not-null-dead-code]]).
- **`SELECT toString(close_usd) AS close_usd … WHERE close_usd > 1000` fails**
  with `Code: 386 NO_COMMON_TYPE` — the alias shadows the column in `WHERE`.
  Aliases prefixed `s_` instead.
- **A first attempt at a junk metric measured nothing.** Defining junk as ">100x
  the pair's own median" gave a flat ~0.3-2.5% across every size bucket with
  ratios up to 1.6e14 — the median itself is contaminated by the
  precision-collapse rows this file records at the small end. Replaced with a
  reference built **only from non-dust rows**, which is what produced the
  usable answer.

## Design Decisions

### From Plan

1. **Derive the threshold from the distribution rather than guess**, and
   validate against genuinely-expensive tokens — the task's first AC, and the
   step that produced the decision below.

### Emerged

2. **Option 2 (flag at ingest) rejected on measurement, not preference.** Of
   dust buckets with a non-dust reference for the same pair, 33% are priced
   correctly. Shipping a size-based `is_dust` column would publish a flag that
   is wrong a third of the time it fires, and wrong specifically about valuable
   assets. Recorded in `Candle::volume_base`'s doc comment so it is not
   rediscovered from scratch.

3. **Option 1 (documented read-time filter) chosen, and it needed no new
   field.** `volume_base` and `trade_count` are already returned on every
   candle, including price-less ones — the raw material was on the wire the
   whole time; only the interpretation was missing.

4. **Documented in three places, not one, because the repo separates two
   audiences.** `openapi/descriptions.rs` exists precisely so published text
   carries no task numbers or ADR references, enforced by
   `every_published_text_is_present_and_reader_facing`. Writing this only in the
   doc comments would have left integrators — the people who chart a $3.5M
   candle — unable to see it.

5. **The 93% split out as [[0274]] rather than absorbed here.** It is a
   different claim about a different subject: 0116 asks whether a bucket's close
   is a market price, 0274 asks whether the asset has a market at all. It also
   belongs beside [[0147]] and [[0252]], which is a decision 0274 is asked to
   settle rather than assume.

6. **`volume_quote_usd` called out as unaffected in every place**, not left
   implicit. The natural reaction to "prices are wrong" is to distrust the
   volume beside them; the measurement says not to.

## Future Work

- [[0274]] — assets with no meaningful trading are published as priced (the 93%).

## Notes

- Do **not** treat this as a [[0114]] regression. The repair's own AC was
  corrected on 2026-07-23 to test *reference correctness* rather than a value
  ceiling, precisely because a ceiling can never pass on data the repair is not
  responsible for.
- Distinct from [[0115]] (exotic quotes with no USD path at all). That is about
  rows we *cannot* price; this is about rows we price correctly from a
  meaningless input.
