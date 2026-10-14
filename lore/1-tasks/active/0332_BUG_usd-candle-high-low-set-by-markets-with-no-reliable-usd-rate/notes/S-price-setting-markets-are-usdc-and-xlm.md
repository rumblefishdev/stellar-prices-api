---
title: "Decision: only USDC- and XLM-quoted markets set USD open/high/low/close"
type: synthesis
status: mature
spawned_from: notes/R-usdt-legs-widen-usd-candles.md
spawns: []
tags: [ohlcv, methodology, high-low, adr-0011]
links:
  - "../../../../2-adrs/0011_base-currency-is-a-denomination-not-a-pair-filter.md"
history:
  - date: "2026-10-14"
    status: mature
    who: akot
    note: >
      Agreed on Slack: Marek ("wyklucz usdt w liczeniu high/low"), Oskar (same
      reason USDT was dropped from the USD reference).
---

# Decision: only USDC- and XLM-quoted markets set USD prices

## Conclusion

Keep the candle definition (high/low = extremes of real trades) but take them
only from trades whose USD value we know.

1. **Allowlist, not a denylist.** Price-setting quote assets are canonical USDC
   and XLM (XLM/USDC is liquid, so XLM's USD rate is well known). Every other
   quote leg, USDT included, counts for volume only. A new token (e.g. USDT0)
   joins the list once its rate is reliable.
2. **Per bucket.** If a price-setting market has a valid row, open, high, low,
   close and `method` come from price-setting markets only. Otherwise the bucket
   is computed as today.
3. **Volume, trade counts:** every market. **vwap, pf_vwap:** every convertible
   market, clamped into the new `[low, high]`.
4. **Read path only** (`queries_ch.rs`). No stored value changes.
5. **Contract:** amend ADR 0011, the `/ohlcv` description, and tell clients the
   methodology changed.

## Reasoning

- The defect is the input to max/min, not max/min itself.
- USDT's USD rate is not measurable (see the R- note), so any USDT extreme in
  USD is a guess, and max/min selects the worst guesses.
- Open/close today come from the highest-volume market, which can be USDT; the
  same argument applies to them.

## Alternatives considered

- **Volume-weighted high/low.** Rejected: not a traded price.
- **Per-minute USDT rate.** Rejected: USDT has no single USD price even within a
  minute.
- **Leave as is.** Rejected by Marek and Oskar.

## Step 2 (not decided here)

Convert USDC/XLM markets at the rate of the trade's minute, not the bucket close.
Matters for XLM-quoted tokens, since XLM moves within the day. Needs `_1m`
priced end-to-end. Decided after step 1 is measured.
