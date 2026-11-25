---
type: R
status: mature
date: 2026-10-13
who: stkrolikiewicz
---

# R — Pre-run report: dollar tokens traded against XLM before 2021-02

Safeguard 3 of the task. Built from the public API and Binance, not from
ClickHouse (no prod read access in the session that made it). Data and the
script that produced it are in [`../data/`](../data/).

## Method

1. Every issuer of `USD`, `USDT` and `USDC` on Horizon: **1,194** assets.
2. For each, `GET /v1/assets/{id}/ohlcv?granularity=1M&base_currency=XLM` over
   2015-11 → 2021-01: the candles where the token is the base and XLM the quote,
   which is how `canonical.rs` stores every dollar token against XLM except
   canonical USDT and USDC. 1,192 scanned (the two canonical ones skipped), 0
   errors.
3. Daily candles of the tokens that traded. Implied XLM/USD at par = `1 / pf_vwap`.
4. Per day: the reference is the USD-weighted median of those markets; each
   token's deviation from it; Binance XLM/USDT daily VWAP as an outside check
   (Binance data starts 2018-05-31; Horizon and CoinGecko no longer serve this
   period publicly).

**Not covered:** canonical USDT, which is XLM's *quote* and so sits in XLM's own
series. The API has no raw XLM/USDT price for it before 2021 (USD fields are
null), only its volume: 21 days and 4,581 XLM in 2018, 63 days and 8,948 XLM in
2019, 257 days and 5.2 M XLM in 2020.

## Findings

**Eleven dollar tokens traded against XLM before 2021-02.**

| token (issuer prefix) | days | USD volume at par | first → last | median dev. from the day | median dev. from Binance |
|---|---|---|---|---|---|
| USD `GB2O5PBQ` | 427 | 127.6 M | 2019-11 → 2021-01 | 0.0 % | 0.52 % |
| USD `GDUKMGUG` (AnchorUSD) | 902 | 18.7 M | 2018-08 → 2021-01 | 0.0 % | 0.63 % |
| **USDT `GC2PJ6J3`** | 137 | 4.6 M | 2019-06 → 2021-01 | **932 %** | **115,809 %** |
| USD `GDSRCV5V` | 544 | 3.4 M | 2018-09 → 2021-01 | 0.73 % | 1.01 % |
| USD `GBNLJIYH` | 316 | 72 k | 2019-10 → 2021-01 | 3.54 % | 3.47 % |
| USD `GBUYUAI7` | 976 | 19 k | 2016-10 → 2021-01 | 0.75 % | 0.70 % |
| USD `GAEDZ7BH` | 271 | 12 k | 2020-03 → 2021-01 | 1.12 % | 1.05 % |
| USDT `GCCK3X3K` | 30 | 2.8 k | 2020-12 → 2021-01 | 2.44 % | 4.24 % |
| **USD `GD3LHBHY`** | 17 | 208 | 2020-10 → 2021-01 | **65.7 %** | **65.6 %** |
| **USDT `GC3YRTJQ`** | 15 | 17 | 2020-11 → 2021-01 | **24.6 %** | **24.3 %** |
| **USD `GA5O7NTU`** | 2 | 17 | 2020-06 → 2020-11 | **312,462 %** | **312,385 %** |

**Coverage of the reference** (days with at least one dollar market):

| year | days | USD volume | median abs. dev. from Binance | p90 | days > 10 % off |
|---|---|---|---|---|---|
| 2016 | 1 | $0.02 | — | — | — |
| 2017 | 14 | $31 | — | — | — |
| 2018 | 228 | $7.6 M | 1.87 % | 5.57 % | 1 |
| 2019 | 365 | $10.7 M | 0.61 % | 2.47 % | 15 |
| 2020 | 366 | $96.2 M | 0.34 % | 1.88 % | 4 |
| 2021-01 | 31 | $39.9 M | 0.85 % | 2.67 % | 0 |

- **Before 2018 there is no usable reference:** one token, $31 in two years.
  Priced history in practice starts in **August 2018** (AnchorUSD), daily from
  2019.
- **Where it exists, par agrees with the market:** median 0.3–1.9 % from
  Binance by year.
- **The median filter does not stop USDT `GC2PJ6J3`.** It held par in mid-2019
  (2019-06-06: implied XLM/USD 0.1203 against Binance 0.1219) and then lost it: its
  implied XLM/USD is a median **~1,160x** Binance's, ~346 in January 2021. Valued at
  $1 its volume is inflated by the same factor, so it holds more than half the
  day's USD-at-par volume on **19 days**; there it *is* the weighted median and
  sets the reference. (The 932 % in the table is against the day's median, which
  it skews itself.) Simulated
  with `max_dev` 0.10 and 0.25 alike: **20 days more than 10 % off Binance** in
  2018–2020 with all eleven tokens, **1 day** with the four off-peg tokens
  (bold above) left out.

## So what

"Every dollar token" as decided on 2026-10-13 gives ~20 days of XLM/USD wrong by up
to three orders of magnitude in 2019–2020, and with it every XLM-quoted price on those days. The
deviation threshold cannot fix it. It needs a per-token exclusion: either a
list from this report (the four bold tokens), or a rule that drops a token whose
median deviation over its history exceeds the threshold. `max_dev` itself barely
matters here (0.10 and 0.25 give the same days); 0.25 keeps the honest small
markets that sit 3–4 % off.
