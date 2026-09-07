---
title: "Phase 4 — invariants that stop a silent synthetic series from shipping again, with thresholds from real distributions and passing tests"
type: synthesis
status: developing
spawned_from: S-phase0-root-cause.md
spawns: []
tags: [pricing, stablecoin, ci, monitoring, tests]
links:
  - "../analysis/guardrails.py"
history:
  - date: 2026-09-04
    status: seed
    who: akot
    note: >
      Thresholds measured over Chainlink USDC/USDT/DAI, Bitstamp, Kraken and
      OKX daily bars; tests run green on 2026-09-04 (`python3 analysis/guardrails.py`).
---

# Phase 4 — guardrails

## The one lesson from the sweep

Of the six candidate signals, **only one separates USDC from every honest
series on its own: `trade_count == 0`.** Everything price-shaped has a
legitimate twin on real data:

| signal | why it false-positives alone [data] |
|---|---|
| O = H = L = C | 68 % of Chainlink USDC days (one heartbeat round per day), 88 % of USDT days; 20 thin Stellar assets with a single trade a day |
| close exactly 1.0 | 16 % of Kraken days (4-decimal prints), up to 67 % of a 30-day Kraken window; 8–10 % on every other venue |
| zero realised vol | never on real data, but a 30-day window can be as quiet as 0.23 bps |
| long constant run | Kraken 18 days, OKX 10 days |

So the invariants are joined with `trade_count`, and the price-shaped ones
carry thresholds sized off the *noisiest real window*, not off intuition.

## The invariants (`analysis/guardrails.py`)

| check | threshold | real-data extreme (any 30-day window) | our USDC |
|---|---|---|---|
| `zero_trades_share_30d` | ≥ 0.9 → **reject** | 0.00 on every venue | 1.00 |
| `flat_and_no_trades_share_30d` | ≥ 0.9 → reject | 0.00 | 1.00 |
| `exact_peg_share_30d` | ≥ 0.9 → alert | 0.67 (Kraken) | 1.00 |
| `constant_close_run_days` | > 30 → alert | 18 (Kraken) | 1 864 |
| `min_rvol_30d_bps` | < 0.1 bps → alert | 0.23 (Bitstamp), 0.33 (Chainlink), 0.58 (Kraken), 0.85 (OKX); 0 never | 0.00 |
| `zero_volume_share_30d` | ≥ 0.9 → alert | 0.00 | 1.00 |

"Reject" means the series fails CI (a fixture test over `/ohlcv` for every
asset with a fiat-peg code). "Alert" means a CloudWatch alarm on the
production series, one evaluation per day, which fits the [[0125]] dashboard
(the metric emission hook the ledger-processor already has).

## Tests that prove separation (all pass, 2026-09-04)

| test | asserts |
|---|---|
| `test_our_usdc_series_fails` | our `/ohlcv` USDC series trips zero-trades, exact-peg, constant-run and rvol |
| `test_our_native_control_passes_price_checks` | XLM, a non-stablecoin, trips nothing |
| `test_real_usd_sources_pass` | Chainlink USDC/USDT/DAI, Bitstamp, Kraken, OKX and the composed series trip nothing over their full history |
| `test_stress_window_does_not_trigger` | 2023-02 → 2023-04 on Chainlink trips nothing (a depeg is not an anomaly of the *series*) |
| `test_a_quiet_real_quarter_does_not_trigger` | the quietest 30 days ever printed on Bitstamp (0.23 bps) pass |
| `test_synthetic_injection_is_caught` | a 45-day splice of exactly 1.0 into Bitstamp trips the constant-run check |

## Where they go in this repo [est.]

1. **CI fixture**: a Rust integration test in `prices-api` that pulls
   `/ohlcv?timeframe=1y&granularity=1d` for canonical USDC (and every
   `peg_coded` asset from the sweep) and asserts the two *reject* invariants.
   It fails today, which is the point; it passes once [[0247]]'s rows are in
   `usd_rate` and the peg branch stops producing `trade_count = 0` history.
2. **Daily alarm**: the four *alert* invariants over the trailing 30 days,
   emitted as one custom metric per invariant from the enrichment worker
   (it already publishes `Prices/Enrichment`), alarmed at the thresholds
   above. Quiet weeks do not fire: the thresholds sit at 2–4× the quietest
   real window.
3. **The peer basket** ([[R-peer-stablecoins]]): a fifth alert, *USDC moved
   > 50 bps and DAI did not move with it* (beta ≈ 0.9 expected), catches a
   bad print on the primary; *DAI moved and USDC did not* catches a dead
   primary. Not implemented in the PoC; the data to size it is in
   `data/peer_corr.csv`.

Thresholds must be re-derived if the surface changes grain or precision:
Kraken's 4-decimal prints are what put `exact_peg_share_30d` at 0.9 rather
than 0.5.
