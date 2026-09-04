---
id: "0265"
title: "USDC's whole price history is asserted, not measured — every candle is peg-derived with zero trades, and a real depeg reads as $1.00"
type: FEATURE
status: backlog
related_adr: ["0011"]
related_tasks: ["0127", "0165", "0170", "0128", "0197", "0172"]
tags: [layer-backend, layer-api, priority-medium, effort-large, milestone-M3, pricing, enrichment, data-correctness, stablecoin]
milestone: 3
links:
  - "../../../packages/prices-clickhouse/schema/views.sql"
  - "../../../docs/prices-api-general-overview.md"
history:
  - date: 2026-09-04
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0127]] AC 4, where the operator chose to exclude USDC from
      the Tranche 2 spot-check table and state why, rather than block the
      milestone on this. Deferred to M3 deliberately: the fix is structural, not
      a patch. Measured 2026-09-04 against the deployed API, not inferred.
---

# USDC is priced by assertion, and the one date that would prove it wrong says $1.00

## Summary

`GET /v1/assets/USDC:GA5Z…/ohlcv?timeframe=all` returns **2,042 daily points
spanning 2021-02-01 → today, and not one of them has a trade behind it.**

| | USDC | `native` (control) |
|---|---|---|
| points | 2,042 | 2,414 |
| **`trade_count > 0`** | **0** | **2,414 (all)** |
| `volume_base > 0` | 0 | all |
| `derived: true` | **all** | — |
| distinct closes | 177 — **1,865 of them exactly `1`** | real market values |
| method mix | `peg` 1,864 / `oracle` 178 | — |

2021 through 2025 is **100% `method: peg`** — a flat, asserted `1`. Only from
mid-2026 does `oracle` produce varying values, and those still carry
`trade_count: 0`.

🔴 **The falsifying case.** On **2023-03-11**, when USDC broke its peg to
roughly **$0.87-0.88** after the Silicon Valley Bank failure, we return:

```
2023-03-10  close=1  method=peg  trades=0
2023-03-11  close=1  method=peg  trades=0     ← reality was ~$0.87
2023-03-12  close=1  method=peg  trades=0
```

`native` on the same day shows the market genuinely moving (`0.0782 → 0.0588 →
0.0836`, 36,208 trades). **The underlying data exists. We do not use it for
USDC.**

⚠️ **State the claim narrowly and keep it narrow.** For ~99% of dates our
answer is right. This is not "USDC pricing is broken". It is: **the answer is
right by construction rather than by measurement, and on the one well-known
date where those differ, it is wrong.** A price feed that cannot report a
depeg is not reporting a price — it is repeating an assumption.

## Context — why it happens

USDC is our **top-preference quote asset**. Pairs canonicalise as
base=X / quote=USDC and never the inverse, so USDC essentially never appears as
a *base* leg — [[0165]] measured `price_ohlcv_1d WHERE asset_id = <USDC>` at
**0 candles**, unconditional on quote. With no candles, the peg fill covers the
entire history.

Two prior tasks touched the edges and neither fixed this:

- [[0165]] rewrote `price_usd_series`, which `/ohlcv` never reads.
- [[0170]] fixed the `/ohlcv` USDC **self-pair** — before it, the endpoint
  returned `200` with `data: []`. It now returns the peg series instead. That
  is an improvement and also how this defect became visible: an empty response
  is obviously empty, a flat `1.00` looks like data.

🔑 **The circularity is the hard part.** USDC is the denominator. Pricing USDC
*in USD* from candles that are themselves quoted in USDC is circular — see the
recorded finding that the candles cannot price USDC (654,291 rows at exactly
1.0). Any real fix has to break out of that loop with an independent reference,
which is why this is M3 and not a patch.

## Implementation

Sketch, not a plan — the first job is deciding which of these is right.

- **Establish what independent evidence exists, per era.** The oracle feed
  already produces varying USDC values from mid-2026 (`method: oracle`, 178
  points). Determine how far back oracle coverage reaches — [[0167]] recorded
  `usd_rate` coverage starting **2026-03-11**, which does not reach 2023.
- **Price USDC against a non-USDC leg.** USDC/XLM trades exist on SDEX;
  inverting them against an independently-priced XLM gives a measured USDC/USD.
  ⚠️ This inherits XLM's own USD derivation — check it is not itself
  peg-dependent, or the circle closes again.
- **Decide what to publish when nothing independent exists.** Options: keep the
  peg but label it (`method: peg` is *already* on the wire — a consumer can
  distinguish, if they read it), return no point, or return the peg with an
  explicit confidence field. 🔑 Whatever is chosen, `trade_count: 0` +
  `derived: true` are already truthful signals; the question is whether a
  reasonable consumer would notice them.
- **Do the same audit for the other pegged assets.** USDT is the known
  counter-example: [[0172]] established it **really did depeg** in June 2022
  (par for 15 months, then 0.13). If USDT is measured and USDC is asserted, the
  two are inconsistent, and [[0197]] already asks whether USDT's rank-1 quote
  preference is still justified.
- Re-run the 2023-03-11 check as the acceptance test. It is the cheapest
  possible falsifier and it currently fails.

## Acceptance Criteria

- [ ] `GET /v1/assets/USDC:GA5Z…/ohlcv` on **2023-03-11** returns a close that
      reflects the actual depeg, or returns no point — but does **not** return
      `1` as though it were measured.
- [ ] The independent reference used is documented, per era, with the date its
      coverage begins and what is published before that date.
- [ ] The circularity is addressed explicitly: whatever prices USDC does not
      itself depend on USDC being $1.
- [ ] The other pegged assets are audited the same way, and USDC/USDT are
      consistent with each other.
- [ ] A consumer can tell an asserted price from a measured one without reading
      the source — decide whether `method` + `derived` + `trade_count: 0`
      already suffice, and record the answer either way.

## Notes

- 🗄️ **Tranche 2 does not wait for this.** [[0127]] AC 4 excludes USDC from the
  spot-check table and states why, per the operator's decision 2026-09-04.
  That is a weaker package — USDC is the reviewer's named example and the
  easiest close to verify independently — and the exclusion is stated openly
  rather than quietly. Do not reopen that decision here; this task is the
  durable fix, on M3's timescale.
- ⚠️ **`/ohlcv` returning a flat `1.00` is more dangerous than returning
  nothing**, which is what it did before [[0170]]. An empty response prompts a
  question; a plausible-looking constant does not. Worth remembering when
  weighing "return no point" against "return the peg".
- The peg fill itself is not a defect. It exists because a stablecoin with no
  base-leg candles would otherwise have no series at all. The defect is that
  nothing distinguishes *"$1 because we measured it"* from *"$1 because we
  assumed it"* at the point a consumer reads the number.
