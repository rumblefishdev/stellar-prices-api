---
title: "Decision memo (ADR draft) — price USDC from measurement, not the peg"
status: proposed
deciders: [akot]
related_tasks: ["0265", "0247", "0168", "0172", "0173", "0111", "0125", "0127"]
related_adrs: ["0011"]
tags: [pricing, stablecoin, data-correctness, oracle, decision]
links:
  - "S-phase0-root-cause.md"
  - "R-source-candidates.md"
  - "R-peer-stablecoins.md"
  - "S-composition-rule.md"
  - "S-guardrails.md"
  - "S-backfill-migration.md"
history:
  - date: 2026-09-04
    status: proposed
    who: akot
    note: >
      Draft. Gets an ADR id on develop once the implementation task is filed;
      ADRs in this repo are written post-factum. Every number below comes from
      analysis/*.py run on 2026-09-04; sources with a URL are in data/sources.csv.
---

# Price USDC from measurement, not the peg

**The one number.** Every limit, haircut or backtest sized off our USDC
history was sized off a series whose 1-day 99 % VaR is **0 bps**. Measured
over the same 1 864 days it is **6 bps**, the expected shortfall **25 bps**,
and the realised worst day **300 bps** (2023-03-11). Our endpoint published
`1.00` on that day.

## Context

`GET /v1/assets/USDC:GA5Z…/ohlcv?timeframe=all` returns 2 042 daily candles,
**every one** with `trade_count = 0`, O = H = L = C, `derived: true`; 1 864
of them are `method: peg` and read exactly 1.0. Realised vol of the series is
0.80 bps against 9.11 bps on Bitstamp over the same days.

The 1.0 is ours, not a provider's. `handlers.rs:643` routes canonical USDC —
and only it — to `ohlcv_peg_series`, which joins `usd_rate` for
`method = 'oracle'` and, finding nothing before **2026-03-11**, inserts a
literal `toDecimal128(1, 14)` (`queries_ch.rs:933`). USDC has no base-leg
candles (it is always the quote), and every USDC-quoted `close_usd` before
the oracle window was computed *from* the assumption USDC = $1 ([[0247]]:
654 291 candles, one implied rate, 1.0). Nothing inside the system can price
USDC; the fix needs an external anchor.

Two defects share the symptom. **A:** the fully synthetic USDC series —
exactly one asset, confirmed by a 232-asset sweep (`trade_count = 0` on 100 %
of days: USDC only). **B:** `method: peg` stamped on 135 of 232 assets,
including `native` (1 865 days) and canonical USDT, meaning "USD denomination
assumed"; a consumer cannot tell it from "USDC unmeasured".

## Decision

1. **Primary anchor: Chainlink USDC/USD on Ethereum mainnet**, read as
   rounds (3 240 rounds, 2021-02-17 → today; 0.25 % deviation, 23 h
   heartbeat). On-chain, real USD, no licence, no API key, verifiable by a
   reviewer with any RPC. It recorded the depeg: 243 rounds on 2023-03-11,
   low **0.8800** at 07:51:23 UTC.
2. **Fallback: Bitstamp USDC/USD**, only where the primary has no
   observation (24 days: the 2021-01-25 → 02-16 tail and a few sparse days).
   **Cross-check: Kraken and Bitstamp.** A daily close more than 25 bps from
   a cross-check keeps the primary value and is flagged `measured-disputed`
   (4 days in five years). No averaging: every published number is one
   venue's print.
3. **Gaps publish no price** (`quality = missing`), never a carried-forward
   one. The single permitted carry is inside the primary's own heartbeat on
   the hourly grain, with `n_obs = 0` on the row.
4. **Backfill the full history via [[0247]]'s path**: ~2 049 daily rows into
   `usd_rate` with `method = 'external'` (never `'oracle'`), 2021-01-25 →
   2026-03-10, view first. `ohlcv_peg_series` accepts `external` beside
   `oracle`. Rollback is the view's preference order; rows are never deleted.
5. **The response says what each point is**: `source`, `quality`
   (`measured | measured-disputed | fallback | missing`), `n_obs`,
   `xcheck_spread_bps`, beside the existing `method`. Existing
   `method`/`derived`/`trade_count` are truthful but insufficient (defect B).
6. **Guardrails**: two CI rejects (`trade_count = 0` share ≥ 0.9 over 30
   days; flat-and-no-trades ≥ 0.9) and four daily alarms (exact-1.0 share
   ≥ 0.9, constant run > 30 days, 30-day realised vol < 0.1 bps, zero-volume
   share ≥ 0.9). Thresholds sit 2–4× outside the noisiest real window; six
   tests pass, including the March 2023 window and the quietest Bitstamp
   month.

## Rationale

- **Dispersion decides the source order.** USD sources (Chainlink, Bitstamp,
  Kraken, Coin Metrics) agree to < 1 bps median; USDT-quoted venues agree
  with each other to 1 bps but sit **3–4 bps** off the USD cluster in calm
  and **20–30 bps** off in stress — that offset is USDT/USD. So USDT-quoted
  candles are cross-checks after correction, never primaries. Binance
  additionally had no USDC/USDT book from 2022-09-26 to 2023-03-11 14:00.
- **No peer stablecoin is a proxy.** On Chainlink, DAI's deviation
  correlates 0.994 with USDC's in March 2023 (beta 0.89) — it *is* USDC
  through its collateral, not an independent source. USDT is
  anti-correlated (−0.81) and rose to +90 bps while USDC fell. TUSD, USDP,
  LUSD, FRAX carry 12, 7, 44 and 16 idiosyncratic episodes USDC never had.
  The basket is a validation input only.
- **Daily is not enough for the event but is enough for the acceptance
  test.** Every venue's daily close on 2023-03-11 is 0.96–0.97; the trough
  0.86–0.88 is hourly. The AC asks the daily close not to be 1.0; it will be
  0.9681.
- **Why not paid vendors.** CoinGecko, CoinMarketCap and CryptoCompare gate
  2023 history behind keys and forbid redistribution; Coin Metrics Community
  is CC BY-NC. Pyth Benchmarks require a key since 2026-08-26; RedStone keeps
  30 days; API3 keeps no history. Chainlink costs a few thousand `eth_call`s
  once and one per round thereafter.

## Alternatives considered

| alternative | verdict |
|---|---|
| Derive USDC/USD from XLM/USDC × XLM/USD | rejected: circular; returns 1.0 relabelled `oracle` ([[0247]]) |
| Return no point before 2026-03-11 | rejected: honest but fails AC 1's spirit; the data exists and is free |
| Keep the peg, label it better | rejected as the whole answer, kept as the fallback for `missing` |
| Median of N venues | rejected: changes < 1 bps on 95 % of days, destroys provenance on the rest |
| Backfill from USDT or DAI | rejected on measured correlations (above) |
| Fix-forward only | rejected: leaves 2023-03-11 at 1.00, the falsifying case |

## Consequences

- **Positive:** AC 1–3 close with one INSERT and one query change; the
  reviewer's named example becomes the strongest exhibit; the mechanism is
  reproducible from a clean machine (`analysis/run.sh`).
- **Negative:** the view and stored `close_usd` disagree in deep history
  until the re-enrichment (defect B, [[0168]]/[[0111]]); that disagreement
  must be stated in `backfill_note`. `method: peg` on non-stablecoins stays
  until the vocabulary change ships with it.
- **Cost:** one-off Chainlink read ≈ 3 240 calls (done, 20 min on a public
  RPC); ongoing one call per new round (≈ 1–3/day in calm). No licence, no
  vendor.

## Implementation plan (next ticket)

1. Loader: `analysis/compose_usdc.py` → `usd_rate` rows, `method='external'`,
   shadow first, canonical USDC only, refuses any other code ([[0173]] gate).
2. `ohlcv_peg_series` and `price_usd_series*`: accept `external`; add
   `source`/`quality`/`n_obs`/`xcheck_spread_bps` to the `Candle` DTO.
3. CI fixture: 2023-03-11 close ≠ 1.0 and the two reject invariants over
   `/ohlcv` for every peg-coded asset.
4. Alarms: four invariants as custom metrics on the [[0125]] dashboard.
5. Evidence: re-include USDC in the [[0127]] table with the 0.9681 close.
6. File defect B (re-enrichment + `method` vocabulary) as its own task.

Figures: `figures/fig1` (March 2023 overlay, the opening chart), `fig2`
(deviation from peg, per venue), `fig3` (inter-source divergence), `fig4`
(our quality flags), `fig5` (peer stablecoins), `fig6` (ours vs measured,
full history, quality band).
