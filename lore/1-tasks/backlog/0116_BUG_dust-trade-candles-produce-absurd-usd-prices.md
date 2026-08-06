---
id: "0116"
title: "Dust-trade candles produce absurd close_usd values (up to $29.6M) in every OHLCV granularity"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0114", "0115", "0026", "0144", "0147"]
tags: [clickhouse, data-quality, sdex, enrichment, priority-medium, effort-small]
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
---

# Dust-trade candles produce absurd `close_usd` values

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

- [ ] A dust threshold is derived from measured distribution, not assumed, and
      validated against a sample of genuinely-expensive tokens so they are not
      swept up.
- [ ] Absurd `close_usd` rows are identifiable by consumers (flag column or
      documented read-time filter).
- [ ] `volume_quote_usd` behaviour is explicitly unchanged (it is already
      correct).
- [ ] Verified against both a repaired historical month (202502) and a
      live-written month (202607) — the defect exists in both.

## Notes

- Do **not** treat this as a [[0114]] regression. The repair's own AC was
  corrected on 2026-07-23 to test *reference correctness* rather than a value
  ceiling, precisely because a ceiling can never pass on data the repair is not
  responsible for.
- Distinct from [[0115]] (exotic quotes with no USD path at all). That is about
  rows we *cannot* price; this is about rows we price correctly from a
  meaningless input.
