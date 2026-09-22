---
title: "R — the six ADR decisions against outside practice"
type: research
status: mature
spawns: []
tags: [clickhouse, close-usd, sentinel, adr-0292]
links:
  - "../../../../2-adrs/0292_close-usd-zero-is-a-named-sentinel-with-a-published-reason.md"
  - "../../../../../docs/database-schema/close-usd-zero-guardrails.md"
history:
  - date: "2026-09-17"
    status: mature
    who: akot
    note: "Four web-research passes; the 15 sources ADR 0292 cites were re-opened and their quotes checked, everything else here is as reported."
---

# R — the six ADR decisions against outside practice

Four web-research passes (sources opened by the agents; items they could not
open are marked unverified in their reports and here). Companion to
[[R-zero-sentinel-code-audit]]. The 15 sources ADR 0292 lists under References
were re-opened on 2026-09-17 and their quotes checked; everything else below is
as the research passes reported it — re-open a source before quoting it.

## D1 — keep the non-nullable `DEFAULT 0` sentinel; reject Nullable and the rate table
- Store-at-write of a converted amount is the norm: Kimball multi-currency
  (pair of columns, converted in ETL), dbt forum (materialise `price_usd`), Dune
  `dex.trades.amount_usd` (built with the model), Uniswap subgraphs, Coin Metrics.
  Kimball's late-arriving-dimension pattern (placeholder, then type-1 overwrite)
  is our `version + 1` re-insert.
- Sentinel side: ClickHouse docs ("Nullable … almost always negatively affects
  performance", recommends a default), a 1B-row benchmark 13.7 s vs 10.2 s
  (rushter.com), kdb+ typed in-band nulls, Uniswap `0`. ClickHouse issue #78169:
  MV selecting NULL into a non-nullable column throws 349 even with
  `insert_null_as_default = 1` (unverified by us) — a partial Nullable move
  breaks MVs loudly.
- NULL side: Kimball ("null-valued measurements behave gracefully"), Dune (NULL
  `amount_usd` at DEX-wide scale), Arrow validity bitmap, ClickHouse `argMax`
  skipping NULL. Strongest argument (Fesperman via Wragg): a sentinel has no
  system support, so every new reader is a fresh chance to repeat the bug.
- Rate table: FOR — ClickHouse's denormalisation warning (one rate change
  rewrites many rows = the 0268 9.94M-candle campaign). AGAINST — everyone
  materialises; a CH JOIN builds the right table in RAM on top of FINAL; our
  pivot rate is itself derived from candles, so it would be a second cache.
- VERDICT: keep. Record the wire-format question separately (D5).

## D2 — four meanings of zero; `close = 0` as "no price"
- No vendor publishes price 0 for "no price". Verified encodings of an empty
  interval: null price + volume (Kaiko), carry-forward (Coin Metrics), omit
  (Coinbase, Horizon, Polygon/Massive, Alpaca stocks). TradingView lists zero
  prices as a common datafeed mistake. Our `/ohlcv` = the Kaiko convention.
- "Excluded from price, counted in volume" precedent: Uniswap v2/v3
  (`volumeUSD` vs `untrackedVolumeUSD`, `txCount` always), Kaiko null+volume.
  Horizon filters pool trades at 1000 bps rounding slippage and DROPS their
  volume too (`trade_aggregation.go`, flag `rounding-slippage-filter`).
- AMEND: the marker is `pf_trade_count = 0`; `close = 0` is its consequence.
  Until phase 3 re-ingests, legacy rows carry `pf_trade_count = trade_count`
  from the DEFAULT with `close = 0`, so the two-term gate stays.

## D3 — pending vs never: inferred, or a status column?
- Reason exposed to the consumer ONLY where a status channel exists: Pyth
  (status enum + prev_price/prev_timestamp), LSEG (Ok/Suspect), FIX tag 276,
  CoinGecko (`is_stale`, `is_anomaly`), Kaiko (extrapolated flag).
  Null/omit: Horizon, Massive, SEP-40/Reflector (deliberately delegates to the
  consumer), Dune after its forward-fill cap. In-band sentinel: Uniswap
  (`derivedETH = 0`, open issue v3-subgraph #87), kdb+.
- Carry-forward WITH a marker is the norm (Chainlink `updatedAt`, Pyth,
  LSEG Suspect, Kaiko flag, Dune age caps 2d/7d/30d, CF Benchmarks `(*)`).
  Coin Metrics carries forward silently and lists that as a limitation.
- TWO independent passes recommended `usd_status Enum8(pending|priced|
  unpriceable|no_price)` in storage (Darwen's Reason column; no null map).
  Counter: name-routed writer + DEFAULT = the `pf_trade_count` trap again;
  needs an aggregation rule in six MVs; "unpriceable" is a claim as-of, not
  permanent.
- VERDICT: status computed at READ time now (from `pf_trade_count`, quote
  identity, `close_usd`), exposed on the wire with `as_of`. Storage column
  recorded as the option with a window: decide before phase 3, which rewrites
  every row anyway.

## D4 — "priceable" and the coverage gate (feeds 0147)
- Nobody gates on "converted share of volume" — no one else has an async
  conversion step. Quorums are counts or absolute floors: Dune $10k/interval,
  Uniswap 2 / 20 ETH locked, Coin Metrics 1% (CEX) / 5% (DEX) market share,
  CoinGecko/CCCAGG outlier logic from 3 sources, FTSE DAR >4 of 65 obs.
  Denominators are always over the ELIGIBLE set (whitelist first).
- Gate vs confidence: hard suppress (Kaiko default, Uniswap, Dune); carry with
  flag (CF `(*)`, FTSE DAR, Chainlink); always publish with a number (Pyth,
  DefiLlama, CoinGecko); both = Pyth only.
- Median would NOT fix the yXLM case (one priced observation). Coverage, not
  estimator.
- VERDICT: priced row = `close >= 1e-12 AND close_usd >= 1e-12`; denominator =
  eligible (quote convertible in principle) volume; both sides on `pf_volume`
  (report said keep dust in — rejected: a dust-only candle can never be
  converted, and on 0–3-decimal Soroban tokens dust is real size); gate +
  always-exposed `priced_volume_share`; add an absolute floor; X from our own
  history after the 0286 rollout (0147's job).

## D5 — per-surface publication contract
- Per-surface encodings are accepted if documented (Alpaca: stocks omit,
  crypto returns quote-priced zero-volume bars, one FAQ). No guide demands one
  encoding API-wide; Zalando 123 / Azure only want null == absent.
- The literal `0` on current-price surfaces is the outlier: no surveyed API
  publishes 0 for unknown; Google AIP-149 names the ambiguity ("0 is distinct
  from no rating"). DEX Screener: `priceUsd` nullable beside required
  `priceNative` — closest analogue.
- Changing `0` → `null` in place is breaking (AIP-180, GitHub, Stripe).
- AMEND: keep the `0`, add `price_status` (priced|carried|unpriced) + `as_of`
  (+ optional `is_stale`); document `0 + method ''` = unpriced in OpenAPI;
  record `0 → null` as a future versioned change (Zalando 189/190 headers).

## D6 — guardrail inventory
- MADR 4 has a "Confirmation" section (how compliance is confirmed; fitness
  functions). Azure/AWS/Nygard: ADRs short, append-only, link out rather than
  become design guides.
- AMEND: ADR carries invariants + mechanisms (generator pin, drift detector,
  IT suites) + rejected options with revisit triggers; the site-by-site
  inventory becomes a living doc the ADR links to.
- ClickHouse `CHECK` constraints: enforced on INSERT; whether the refreshable
  APPEND path evaluates them is unverified. Counter: a violated CHECK fails the
  whole insert → a stalled MV tier = availability incident. Preferred: a
  scheduled assertion in the existing `rollup-freshness-probe` (`usd_sanity`)
  with an alarm — same detection, no write-path risk.
