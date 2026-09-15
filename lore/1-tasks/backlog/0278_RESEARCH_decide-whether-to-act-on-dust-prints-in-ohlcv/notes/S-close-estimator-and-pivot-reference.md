---
title: "Fix the fill price at the source, estimate close from an adaptive window, and give the pivot two references"
type: synthesis
status: developing
tags: [ohlcv, enrichment, estimator, dust-prints, pivot]
links:
  - "../../../../docs/ohlcv-outlier-prints-analysis.md"
  - "../../../../packages/prices-ingest-core/src/filter.rs"
  - "../../../../packages/prices-ingest-core/src/bucket.rs"
  - "../../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: "2026-09-15"
    status: developing
    who: akot
    note: >
      Drafted from a working session with Adam on how a statistician would
      treat the problem: the rounding error is bounded and identifiable from
      fill size, so filter or precision-weight rather than robustify. Adds
      what Horizon does (analysis doc §10.1) and the two-references finding
      (§10.3). Recommendation only; nothing decided in the task yet.
  - date: "2026-09-15"
    status: developing
    who: akot
    note: >
      "Refinements after review" added: the window VWAP must use the §1
      price (not the amount ratio), the window is minute-granular with k = 3
      measured and a 60-min cap, ordering no longer touches candle fields,
      §1 is phase 2, §3 is one rule per tier. Decisions themselves live in
      the README.
---

# Fix the fill price at the source, estimate close from an adaptive window, and give the pivot two references

## Conclusion

Treat the seven questions in the README as one estimation problem with a
known error model, and answer it in this order:

1. **Order key with the transaction index** (question 1). One `enumerate()`
   in `filter.rs`, no contract change.
2. **Fill price from the resting offer (order book) or the pool's reserves
   (AMM), plus a 0.1 % rounding bound for what still forms a price**
   (question 2). Removes cause A at the root; fixes all four OHLC fields from
   1m up, from deployment on.
3. **Close = VWAP of an adaptive window of price-forming fills; high/low =
   extremes of minute-level price-forming VWAPs** (questions 3–4). Contract
   change → ADR; a new `settle` field is the cleaner option (CME: last trade
   ≠ settlement).
4. **Two pivot references** (question 5): end-of-bucket rate for `close_usd`,
   bucket VWAP for `volume_quote_usd`. Then [[0228]]'s campaign.
5. **History** (question 6): candle-level repair from 1m where it survives,
   the last-hour rule where only 1h exists; full re-ingest only if 1–2 must
   reach history fill by fill.

## The error model, and why it dictates the method

Latent market price P(t). A fill prints p_i = b_i / s_i with integer stroop
amounts; the network rounds one leg against the initiator, so

```
|p_i − P(t_i)| / P(t_i)  ≲  1 / min(s_i, b_i)        (rounding bound ε_i)
```

on top of a microstructure noise floor σ_m ≈ 0.15–0.2 % (analysis doc §2,
fills ≥ 1 000 stroops). Four properties decide the method:

- **Bounded and known per fill** from a covariate (size). Not a random outlier.
- **Heteroskedastic**: 17 stroops → up to 6 %; 10⁶ stroops → 10⁻⁶.
- **Sign depends on trade direction** (both signs among the seven outliers),
  so it is not a constant bias to subtract.
- **Deterministic in magnitude**, not Gaussian.

When the error is identifiable from a covariate, the estimator of choice is a
**filter or precision weighting**, not a robust statistic. Robust statistics
(weighted median) are reserved for the residual the covariate cannot see:
genuine off-market trades on thin pairs (yBTC 2023-03-18, [[0266]]).

## 1. Price of a fill: a better source than the amount ratio

- **Order-book fill:** the resting offer's `N/D`, read from the offer's
  ledger-entry change in `LedgerCloseMeta`. The maker's chosen price, exact.
  This is what Horizon does (`findTradeSellPrice`).
- **Pool fill:** the pool's spot price from the reserves before the trade,
  `R_quote / R_base` (or the geometric mean of pre and post). A dust fill moves
  the reserves by ~10⁻⁹, so the spot is dust-immune by construction, and it is
  economically the price the pool quotes.
- **Fallback** when the change cannot be read: the amount ratio with its
  bound ε_i.

The size bound stays for the residual: a dust offer posted at a price
*favourable* to the taker will fill — a real trade, not a market price. The
0.1 % threshold (both legs ≳ 2 000 stroops) is justified statistically: the
rounding bound must sit below σ_m so it adds no variance, and the size table
in §2 shows fills from ~1 000 stroops are indistinguishable from large ones.

Precision weighting `w_i ∝ v_i / (σ_m² + ε_i²)` is the threshold-free
alternative: dust vanishes on its own (weight ~ε⁻²) and a minute with only
dust yields an estimate with a huge variance instead of nothing. Elegant, but
harder to explain to an API consumer. Recommendation: hard threshold for the
published OHLC, precision weighting at most internally.

## 2. The close estimator

Estimand: **P(t_end)**, the price at the end of the bucket — not the typical
price of the bucket. That is why the bucket VWAP is the wrong close (drift
bias; §6: 97–158 days > 5 %).

For a window h before t_end:

```
MSE(h) ≈ σ_drift² · h  +  σ_m² / n(h)
```

Liquid XLM: n grows fast, the optimum is a very short h — §6 measures 1 min
(median 0.15 %, worst 4.5 %) beating 5, 15 and 60 min. Thin market: n(h) is
small, h must grow. Hence:

```
window = the last minute with a price-forming fill, extended backwards until
         ≥ k fills (k ≈ 3) or ≥ V volume, capped at H_max
close  = Σ p_i·v_i / Σ v_i  over fills in the window with ε_i ≤ 0.1 %
```

A nearest-neighbours-in-time estimator: bandwidth adapts to liquidity. On XLM
it degenerates to "VWAP of the last traded minute", already measured best.
On yBTC it takes the last few real trades instead of one.

Residual off-market trades on thin pairs: the **volume-weighted median** in
the same window. Breakdown at 50 % of volume — one large off-market trade
still moves it, and then it is the market.

**Optimal version, if confidence bands are wanted: a Kalman filter.** State
log P(t) as a random walk with variance q per unit time; observations log p_i
with noise r_i = σ_m² + ε_i². The posterior mean at t_end is the close, the
posterior variance a ready-made candle-quality metric. The filter itself
down-weights dust (large r_i), trusts a fresh observation more after a long
gap (q·Δt), and stays near the last estimate with growing uncertainty on a
thin market. For a random walk with observation noise it is an EWMA whose
gain follows the signal-to-noise ratio, so the window estimator above is its
rectangular approximation. Computable in ingest minute by minute. Not
recommended as the published definition of `close` (hard to explain), but
better as an internal quality flag than §6's `|close/vwap − 1|` heuristic.

**Minute with no price-forming fill:** the candle exists (volume is real);
price fields carry forward from the last price-forming minute; a new
`price_trade_count = 0` says so; rollups use
`argMaxIf(close, ts, price_trade_count > 0)` — the same pattern [[0146]]
introduces for `close_usd`.

## 3. High and low

The maximum of n noisy observations is biased upward by roughly
σ·√(2 ln n). With 35 000 fills a day, even σ_m = 0.2 % gives ~0.9 %, and any
fat tail blows it up. That is why high/low are our worst fields (704 / 560
days > 50 % / 30 % off Bitstamp) and why Horizon keeps 5/34 in its low despite
its filter.

Estimator: **max / min over minute-level price-forming VWAPs**, not over
fills. §6 measures 534 / 598 → 2 / 0 days > 10 %, median 5.5 % → 0.16 %. A
definition change ("highest minute price", not "highest print") → same ADR as
close.

## 4. Pivot: two references

Analysis doc §10.3. `close_usd` needs the rate **at the bucket end**;
`volume_quote_usd` needs the **average** rate over the bucket. Today one
reference serves both. Consequence for question 5: "VWAP of the bucket" is
right for USD volume and a stop-gap for USD close while the close is a single
print; once §1 lands, the XLM/USDC close is again the correct reference for
`close_usd` (same instant, same estimand). For history the substitute is the
VWAP of the last traded hour (§10.2), not the day VWAP.

## 5. Ordering

`filter.rs:25` iterates `tx_processing` in the order stellar-core applied the
transactions — the order Horizon's TOID encodes — but drops the index;
`claim_to_raw_trade` receives only `op_idx`, which restarts at 0 per
transaction. Fix:

```
for (tx_idx, tx_ref) in tx_processing.enumerate()
RawTrade / TradeTick += transaction_index
lex_key = (ledger_sequence, transaction_index, operation_index, claim_index)
```

Operations are sequential within a transaction, `ClaimAtom`s are in execution
order within an operation (path hops), so this is a total order consistent
with ledger application.

Every fill in a ledger shares `closed_at` (~5 s), so the order decides only
who wins the tie for open/close. With §1's filter, cause B's effect on close
drops to noise (§3: 54/60 days wrong, all but the dust day within 0.25 %), so
**no separate history repair for ordering** — §6's candle repair covers it.
The Soroban extractors (Aquarius, Soroswap, Phoenix) still need their own
audit (§8).

## 6. Validation before anything ships

Out-of-sample, not the 1 181 XLM days the parameters were read from: XLM
plus 3–5 assets of different liquidity (AQUA, yXLM, a single-venue token)
against Bitstamp / Binance / CoinGecko. Metrics: median |error|, p99, worst
day, days > 5 %. Acceptance: the tail disappears and the typical error
(0.15 %) does not grow. Plus README step 1's count of candles the pivot priced
from a quantised close, to size what the campaign repairs.

## Refinements after review (2026-09-15)

Recorded in the README's "Decisions on the candle fix". The ones that change
this note's text:

- **§2's VWAP must not be `Σ volume_quote / Σ volume_base`.** That ratio is
  the amount-derived price §1 replaces. The window estimator is
  `Σ pᵢ·vᵢ / Σ vᵢ` over price-forming fills with pᵢ the §1 price, carried
  in the 1m row as `pf_price_volume` and `pf_volume`.
- **The window is in minutes.** "≥ k fills" is evaluated as "the last minutes
  until Σ pf_trade_count ≥ k", capped at 60 minutes and at the bucket; the 1m
  candle's close is its own price-forming VWAP. Measured k: 1…3 equivalent,
  ≥ 5 worse (R-measurements-2026-09-15).
- **§5 (ordering) no longer affects any candle field** once close/open are
  minute-granular window VWAPs. It stays for fill-level correctness.
- **§1's payoff is mostly the minutes that hold only dust** and an exact
  end-of-bucket price for AMM sources; a correctly priced dust fill still
  weighs ~nothing. Phase 2.
- **§3 is one rule on every tier**: high/low from price-forming fills, no
  minute-VWAP variant unless the post-§1 residual bias measures too high.

## Alternatives considered

- **Robust statistics alone (median of prints).** Rejected as the primary
  tool: the error is identifiable from size, so filtering is strictly better;
  a median still fails when the window holds one print.
- **Bucket VWAP as the pivot reference for all grains** (question 5 as
  written). Right for volume, wrong for close on 1d and coarser (§10.2).
- **Horizon's 10 % slippage filter.** Too loose — it passes the fills whose
  error is largest relative to σ_m (5/34 in Horizon's own low).
- **Full re-ingest for history.** Only if fill-level fidelity is required;
  candle-level repair from 1m + the last-hour rule covers the measured cases.
