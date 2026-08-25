---
id: "0225"
title: "GET /ohlcv returns an empty 200 for actively-trading assets that do not trade against the default USDC quote — 12 of 13 remaining 0120 failures"
type: BUG
status: backlog
related_adr: []
related_tasks: ["0120", "0170", "0128", "0210"]
tags: ["priority-high", "effort-medium", "api", "read-surface", "data-correctness", "scf", "milestone-M2"]
milestone: 2
links:
  - "../../../packages/prices-api/src/assets/handlers.rs"
  - "../../../packages/prices-api/src/assets/queries_ch.rs"
history:
  - date: 2026-08-25
    status: backlog
    who: stkrolikiewicz
    note: >
      Found by the [[0120]] conformance re-run after [[0135]] landed. 0120
      blocks on 0135/0170/0178 and expected to go green once they were done —
      it will not. 12 of the 13 remaining failures are this defect, and it is
      owned by none of the three. Measured on prod the same session.
---

# `/ohlcv` reports "no candles" for assets that are trading right now

## Summary

`GET /assets/{id}/ohlcv` filters on **both** legs and defaults the quote to
USDC. An asset that trades actively against **XLM** but not against USDC
therefore gets a `200 OK` with `data: []` — indistinguishable from an asset
that has never traded.

This is **not** [[0170]]. 0170 is the USDC *self-pair* (USDC is never a base).
Here the base leg is present and busy; the requested quote is simply not where
the asset trades.

## Measured on prod, 2026-08-25

The conformance suite asks for `?granularity=1h` over 7 days and `1d` over 30
days, with no `base_currency`, so the quote resolves to USDC. Candle counts in
`price_ohlcv_1d`:

| asset | vs XLM | vs USDC |
|---|---|---|
| AUD | **4,864**, current to today | 1,214, newest **2026-05-20** |
| BOL | **2,085**, current to today | 12, newest **2023-10-27** |
| RON | **203**, current to today | none |
| EQL | **9**, current to today | none |

All four are present as a base leg (7–13 distinct pairs each), so the
`asset_id` filter matches plenty of rows. Only the `quote_asset_id = <USDC>`
conjunct empties the result. The `CBIJBDNZ…` contract asset shows the same
shape at 274 base / 492 quote appearances.

## Why the empty 200 is the wrong answer

The handler already contains the precedent, in [[0170]]'s own citation of
`handlers.rs`:

> its absence is a server-side data gap, not "no candles". Surface it as a 503
> instead of masking it as an empty 200 (which looks like a healthy asset with
> no history).

Same defect, different trigger. A consumer charting AUD sees "no data" for an
asset with 4,864 daily candles.

## Blast radius

- **[[0120]]** — 12 of its 13 remaining failures, and the reason its stated
  unblock plan ("re-run green after 0135/0170/0178") does not hold. 0135 is
  archived and the run still fails.
- **[[0128]]** — the M2 evidence package cites that run.
- Any consumer charting an asset whose liquidity is on XLM rather than USDC,
  which on this network is the common case, not the exception.

## Design question to settle first

Three defensible answers; pick explicitly rather than by accident:

1. **Fall back to the asset's most-liquid quote** and say so in the response.
   Most useful, but makes the response's *meaning* data-dependent — the same
   objection 0170 raises against inverting some assets and not others.
2. **Keep the empty result, change the status.** A 404/503 with a body naming
   the quotes that DO have candles. Honest, cheap, and pushes the choice to
   the caller.
3. **Convert through `usd_reference`** so a USD-quoted series exists for any
   asset with any priced pair. Most work, and it overlaps [[0170]]'s peg-fill
   arm — worth checking whether one implementation serves both.

Whichever is chosen must state what `/ohlcv` returns *in* (§ the endpoint
returns OHLC in the quote asset), because options 1 and 3 change that.

## Acceptance Criteria

- [ ] Decision recorded with reasoning, including what the response is
      denominated in
- [ ] `GET /assets/{AUD}/ohlcv` over a recent window returns a non-empty,
      correctly-denominated series **through the deployed API**
- [ ] The same verified for an asset with NO USDC pair at all (RON or EQL)
- [ ] An asset that genuinely never traded is still distinguishable from this
      case — the whole point; assert both
- [ ] A normal USDC-quoted asset's response is byte-identical to today's
- [ ] [[0120]]'s ohlcv failures drop to 0 on a re-run, or the remainder is
      attributed to a named task

## Notes

- Do not fold this into [[0170]]. They share a handler and a symptom but have
  different root causes and different correct answers; merging them makes the
  eventual fix untestable against either.
- The `CBIJBDNZ…` row also carries [[0210]]'s empty `asset_code`, visible in
  the probe output as a blank code column. Different defect, same asset.
