---
id: "0247"
title: "Backfill USDC's real USD rate before 2026-03-11 from an external anchor — deep history is denominated in an assumption"
type: FEATURE
status: backlog
related_adr: []
related_tasks: ["0168", "0167", "0173", "0172", "0165", "0111"]
tags: ["priority-low", "effort-medium", "clickhouse", "data-correctness", "read-surface", "history"]
links:
  - "../../../packages/prices-clickhouse/schema/init.sql"
  - "../../../packages/enrichment-worker/src/ch_enrich.rs"
history:
  - date: 2026-08-31
    status: backlog
    who: okarcz
    note: >
      Spawned from [[0168]]. 0168 publishes the measured USDC rate from
      2026-03-11 onward and falls back to a labelled $1 before that. The
      fallback is correct and stays correct; this task is about whether the
      window can be FILLED. Raised as "the candles must know the real rate" —
      measured on prod and disproved, which is what identified the only route
      that does work: an anchor from outside the USDC-denominated system.
---

# Deep history is denominated in USDC, so it cannot price USDC

## Summary

Every `close_usd` before **2026-03-11** rests on the assumption `USDC = $1`. That
makes the assumption unfalsifiable from our own data, and it makes a genuine
depeg invisible — most concretely the **March 2023 SVB weekend**, when USDC
traded near **$0.88** for ~3 days and our history says `$1.0000`.

The fix is not available internally. It needs an external USD price series.

## Context — why the candles cannot supply it (MEASURED on prod 2026-08-31)

Both halves were checked rather than argued, because "just read it off the
candles" is the obvious objection and it is wrong for a non-obvious reason.

**1. There is no USDC candle.** Canonical USDC is our top-preference quote, so
canonicalisation makes it the quote on every pair it appears in:

```
SELECT count() FROM prices.price_ohlcv_1d FINAL WHERE asset_id = <canonical USDC>
-> 0
```

That is the same fact that forced [[0165]] to invent the peg arm.

**2. The quote-side candles are circular.** Pricing USDC from an XLM/USDC candle
needs XLM's USD price, which before the oracle window is *defined* as XLM's price
in USDC (`ch_enrich.rs`, `pivot_sql`):

```sql
SELECT timestamp, sum(close * volume_base) / sum(volume_base) AS usd
FROM price_ohlcv_1m WHERE asset_id = <XLM> AND quote_asset_id = <USDC>
```

Measured over every USDC-quoted candle before 2026-03-11:

| implied rate (`close_usd / close`) | candles |
|---|---|
| **1.00000000** | **654,291** |

**One distinct value.** The candles carry exactly zero information about USDC's
dollar price — deriving a rate returns the assumption that produced them, and
would arrive labelled `'oracle'` instead of `'peg'`. Strictly worse.

**3. It is not our pruning.** `oracle_prices` retention is 13 MONTH, which would
reach ~2025-07; the table simply starts at `2026-03-11 14:00`. There is nothing
further back to copy.

## Implementation

Load an external USDC/USD daily (or hourly) series into `prices.usd_rate` for the
pre-oracle window, keyed on the canonical Stellar identity.

- **Source:** USDC/USD on centralized venues trades against actual dollars and
  reaches back to 2018 — CoinGecko, Kraken, CryptoCompare. Pick one, record which
  and at what granularity; a daily close is enough for a peg.
- **`method` must be its own value**, NOT `'oracle'`. `'oracle'` means a
  Reflector reading we polled ourselves. Something like `'external'`, added to
  `init.sql`'s vocabulary block alongside `oracle`/`peg`/`pivot`/`pivot2`. A
  consumer must be able to tell a first-party measurement from an imported one.
- **`usd_rate` is append-only for this purpose** — no re-enrichment. See below.
- Range: from SDEX genesis (2021-02) to `2026-03-11 14:00`, stopping where our
  own readings begin so the two never overlap at the same key.

## 🔑 Why this is unusually cheap for `price_usd_series`

The peg arm reads `usd_rate` **at read time** ([[0168]]). So loading these rows
fixes the view's whole deep history **immediately, with no re-enrichment** — an
INSERT, not a pass over hundreds of millions of candle rows. Most history
corrections in this repo are 0182-class work measured in days; this one is not.

⚠️ **That is only true of the VIEW.** `close_usd` on the candles themselves is a
stored product and still says `1.0000` — correcting it IS the re-enrichment job
([[0168]]'s "known adjacent gap", gated behind [[0111]]). So shipping this alone
makes the view and the candles disagree in deep history, which is a fresh
instance of the same defect class 0168 exists to fix. **Decide deliberately
whether to ship the view-only half**, and if so, write down that the disagreement
is intended and where it ends.

## 🔴 The gate — this is a ticker→issuer claim

An external feed prices the **ticker** "USDC". We would file it under the Stellar
issuer `GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN`. That is
exactly the shape of claim that produced the **7.4× USDT error** ([[0172]]):
Reflector prices Tether's own token at par, we filed it under a Stellar IOU
really worth ~$0.13.

The USDC case is much stronger — the canonical Stellar USDC *is* Circle's own
issuance, not a third-party IOU sharing a code — but "much stronger" is a
judgement, and the USDT error was also obvious in hindsight. **Route it through
[[0173]]'s symbol→issuer mapping gate rather than waving it through**, and do not
generalise the loader to "stablecoins" or "assets with a `usd_rate` row".

## Sizing — do this before committing to it

USDC held par tightly across almost all of 2021-02 → 2026-03. The real content is
a handful of event days plus sub-0.1% wobble. Establish the payoff first:

- How many days in our span deviate more than, say, 0.5% from par?
- What is the largest deviation, and how many candles sit in those buckets?

If the answer is "SVB and nothing else", that is still worth having — those are
the days where being wrong is most visible — but it changes the priority and it
argues for a daily grain rather than an hourly one.

## Acceptance Criteria

- [ ] `prices.usd_rate` carries canonical-USDC rows from SDEX genesis to
      `2026-03-11 14:00`, with a `method` distinct from `'oracle'`.
- [ ] No overlap with our own readings at the same key; the join in
      `price_usd_series*` picks exactly one row per bucket either side of the
      boundary, and there is no discontinuity artefact at 2026-03-11.
- [ ] `price_usd_series` and `price_usd_series_1h` publish the imported rate for
      deep history and stop reporting `method = 'peg'` for covered buckets.
- [ ] The March 2023 SVB window reads materially below par — the acceptance
      fixture, because it is the one span where the difference is unmistakable.
- [ ] The source, its granularity, and the date it was fetched are recorded in
      the task file. An imported series with no provenance is not evidence.
- [ ] The ticker→issuer decision is recorded, with [[0173]]'s reasoning applied.

## Out of scope

- Correcting `close_usd` on the candles (re-enrichment) — [[0168]]'s known
  adjacent gap, gated behind [[0111]].
- Any asset other than canonical USDC. Widening is [[0173]].
