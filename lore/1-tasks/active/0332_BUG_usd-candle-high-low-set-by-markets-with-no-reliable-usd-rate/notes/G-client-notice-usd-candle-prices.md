---
title: "Client notice draft: USD candle prices come from USDC- and XLM-quoted markets"
type: generation
status: developing
spawned_from: notes/R-step-1-measured-on-production.md
spawns: []
tags: [ohlcv, client-notice, high-low]
links:
  - "S-price-setting-markets-are-usdc-and-xlm.md"
history:
  - date: "2026-10-14"
    status: developing
    who: akot
    note: >
      Draft for Adam to send. Effective date left open until the compute
      deploy is scheduled.
---

# Client notice draft

Adam sends this. Fill in the date once the deploy is scheduled.

---

**Subject:** Prices API: USD candles now take prices only from USD-measurable markets (effective `<DATE>`)

From `<DATE>`, `GET /assets/{asset}/ohlcv` in USD mode (`base_currency=USD`,
the default) changes which markets set a candle's prices.

**What changes**

A candle merges every market of an asset: each trading pair on each venue.
Each market is converted to USD before the merge. A market quoted in Stellar
USDT (the Apay bridge token, `GCQTGZ…`, not Tether) has no reliable USD rate,
so its converted high and low could land far from where the asset traded. On
XLM, 2022-06-18, the API returned a high of $0.1767 and a low of $0.0815;
Binance shows $0.1135 and $0.1001.

From `<DATE>`:

- `open`, `high`, `low`, `close` and `method` come only from markets quoted
  in USDC or XLM, when one of them formed a price in the bucket. That same
  day now returns $0.1137 / $0.1005.
- A bucket in which no USDC- or XLM-quoted market formed a price is computed
  exactly as before.
- `volume_base`, `volume_quote_usd`, `trade_count` and `pf_trade_count` are
  unchanged and still count every market.
- `vwap` and `pf_vwap` still weigh every market and stay within the published
  `low` and `high`. When a USDT-quoted market traded outside the USDC/XLM
  range, the reported mean is the nearer bound. `close_divergent` can flag
  such a bucket.
- `method` follows the markets that set the prices. A bucket that traded
  mostly against USDT can now report the `assumed-par` of its USDC market.
- `base_currency=XLM` is unchanged.

The response shape does not change: no field is added, removed or renamed.

**Who is affected**

Measured on daily candles since 2021-02: 0.34 % of priced buckets (37,582),
across 419 assets that traded against USDT and against USDC or XLM on the same
day. High or low moves in 16,136 of those buckets, across 340 assets. For XLM,
the 95th-percentile gap to Binance falls from 23 % to 1.2 % for the daily high
and from 44 % to 6.4 % for the daily low.

**What you need to do**

Nothing, unless you store candles. Candles you fetched before `<DATE>` for
the affected assets can differ from what the API returns now; fetch them
again to get the new values.

The `/ohlcv` description in the API reference states the rule.
